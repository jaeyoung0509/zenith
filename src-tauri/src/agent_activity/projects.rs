use crate::models::ProjectIdentity;
use crate::safety::symlink::SymlinkGuard;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zenith_platform::path_algebra::{self, PathFlavor};
use zenith_platform::PlatformEnvironment;

pub fn resolve_project(
    cwd: &Path,
    environment: &zenith_platform::PlatformEnvironment,
) -> Option<(PathBuf, ProjectIdentity)> {
    let canonical_cwd = cwd.canonicalize().ok()?;
    if !canonical_cwd.is_dir() {
        return None;
    }

    let git_root = find_git_root(&canonical_cwd, environment.user_home().as_deref());

    let root = git_root.unwrap_or_else(|| canonical_cwd.clone());
    let marker = root.join(".git");
    let repository = git_directory(&root, environment);
    if let GitDirectory::Refused(reason) = &repository {
        // A refused pointer is not the same state as a directory that is not a
        // repository, so it is recorded where the user can see it rather than
        // being reported as "no repository". The message can name a path, which
        // the log sanitizer masks.
        crate::diagnostics::log_error("project_identity", reason);
    }
    let is_repository = matches!(repository, GitDirectory::Resolved { .. });
    let is_worktree = is_repository && marker.is_file();

    let display_name = root.file_name()?.to_string_lossy().to_string();
    let location_hint =
        location_hint(&root, environment.flavor()).unwrap_or_else(|| display_name.clone());

    let display_path = crate::privacy::paths::display_path(&root, environment);

    let id = opaque_id("project", &root);
    let worktree_id = if is_worktree {
        Some(opaque_id("worktree", &root))
    } else {
        None
    };
    let repository_id = match &repository {
        GitDirectory::Resolved { identity, .. } => Some(opaque_id("repository", identity)),
        _ => None,
    };

    let (branch, is_detached) = match &repository {
        GitDirectory::Resolved { git_dir, .. } => read_head_status(git_dir),
        _ => (None, false),
    };

    let is_dirty = if is_repository {
        check_git_dirty(&root)
    } else {
        false
    };

    Some((
        root,
        ProjectIdentity {
            id,
            display_name,
            location_hint,
            display_path,
            repository_id,
            worktree_id,
            is_worktree,
            branch,
            is_dirty,
            is_detached,
        },
    ))
}

/// The last two components of a project root, used as a low-information hint.
///
/// Computed through the path algebra so a Windows root is reduced to its own
/// components (`D:\Users\me\projects\app` -> `projects/app`) even when the
/// check runs on another host, and so the hint can never contain a drive
/// letter, a UNC server, or an absolute prefix.
fn location_hint(root: &Path, flavor: PathFlavor) -> Option<String> {
    let normalized = path_algebra::normalize(&root.to_string_lossy(), flavor);
    let components = normalized
        .split(flavor.separator())
        .filter(|component| !component.is_empty())
        // The drive prefix is not a directory name.
        .filter(|component| {
            !(flavor.is_windows() && component.len() == 2 && component.ends_with(':'))
        })
        .collect::<Vec<_>>();
    let display_name = components.last()?.to_string();
    let hint = match components.len() {
        0 | 1 => display_name.clone(),
        len => format!("{}/{}", components[len - 2], display_name),
    };
    Some(hint)
}

/// Finds the nearest repository root, ignoring a dotfiles repository that
/// lives directly in the user home so it cannot absorb unrelated projects.
fn find_git_root(start: &Path, home: Option<&Path>) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|candidate| {
            candidate.join(".git").exists() && home.is_none_or(|home| *candidate != home)
        })
        .map(Path::to_path_buf)
}

fn check_git_dirty(root: &Path) -> bool {
    let mut cmd = crate::tooling::git_command(root);
    cmd.args(["status", "--porcelain=v1", "-z"]);
    if let Ok(output) =
        zenith_platform::subprocess::run_with_timeout(cmd, Duration::from_millis(800))
    {
        if output.status.success() {
            return !output.stdout.is_empty();
        }
    }
    false
}

/// Byte cap for the files a repository pointer makes Zenith read: `<root>/.git`
/// when it is a pointer, and the `HEAD` it resolves to. Both are stated before
/// the read, following the stat-then-cap convention the tree already uses for
/// hashed and audited files.
const MAX_GIT_METADATA_BYTES: u64 = 4_096;

/// Longest branch name returned across IPC. Git refuses to create a ref whose
/// name does not fit in a single path component, so a longer value is not a ref
/// name, and truncating it would invent one that never existed.
const MAX_BRANCH_NAME_BYTES: usize = 255;

/// The git directory that describes a project root.
enum GitDirectory {
    /// Usable: `git_dir` holds the `HEAD` this project reads, and `identity` is
    /// the path hashed into the repository id. The two differ for a linked
    /// worktree, whose own `HEAD` sits in the repository's git directory.
    Resolved { git_dir: PathBuf, identity: PathBuf },
    /// No `.git` entry, or one that names nothing.
    Absent,
    /// Present but refused. The reason is reported instead of the state being
    /// downgraded to "not a repository".
    Refused(String),
}

/// Outcome of a capped read: the size is checked before the read, so an
/// oversized file is refused instead of loaded.
enum CappedRead {
    Read(String),
    Absent,
    OverCap,
}

/// Reads one Git metadata file under [`MAX_GIT_METADATA_BYTES`], without
/// following a link at the path itself.
fn read_capped(path: &Path) -> CappedRead {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return CappedRead::Absent;
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return CappedRead::Absent;
    }
    if metadata.len() > MAX_GIT_METADATA_BYTES {
        return CappedRead::OverCap;
    }
    match std::fs::read_to_string(path) {
        Ok(contents) => CappedRead::Read(contents),
        Err(_) => CappedRead::Absent,
    }
}

/// The git directory for `root`, contained and bounded.
///
/// `.git` may be a *file* holding a `gitdir:` pointer — how a linked worktree
/// and a submodule record where their git directory actually lives. That is a
/// second pointer mechanism, and its legitimate target is usually *outside* the
/// project, so `SymlinkGuard`'s containment rule cannot be the whole decision:
///
/// - a target inside the project is validated by `SymlinkGuard`, the guard that
///   already governs pointer following in this tree;
/// - a target outside it is accepted only in a shape Git itself writes for a
///   repository whose git directory lives elsewhere (`worktrees/<name>` and
///   `modules/<path>`, both beside a real git directory holding a `HEAD`);
/// - anything else is refused with a reason, because a pointer that merely
///   leaves the project names a directory with nothing tying it to this
///   repository.
///
/// `root` is workspace content, not a nominated directory: it comes from an
/// observed agent process working directory.
fn git_directory(root: &Path, environment: &PlatformEnvironment) -> GitDirectory {
    let marker = root.join(".git");
    match std::fs::symlink_metadata(&marker) {
        Ok(metadata) if metadata.is_dir() => GitDirectory::resolved_whole(marker),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            // A link is not followed on faith: its target is judged by the same
            // rules as a `gitdir:` pointer.
            match std::fs::canonicalize(&marker) {
                Ok(canonical)
                    if SymlinkGuard::validate_components_between(&canonical, root, environment)
                        .is_ok() =>
                {
                    GitDirectory::resolved_whole(canonical)
                }
                Ok(canonical) => match external_git_directory(&canonical) {
                    Some((git_dir, identity)) => GitDirectory::Resolved { git_dir, identity },
                    None => GitDirectory::Refused(format!(
                        "The .git entry is a symbolic link to {}, which is outside the project",
                        canonical.display()
                    )),
                },
                Err(_) => GitDirectory::Absent,
            }
        }
        Ok(metadata) if metadata.is_file() => match read_capped(&marker) {
            CappedRead::Absent => GitDirectory::Absent,
            CappedRead::OverCap => GitDirectory::Refused(format!(
                "The .git pointer file is larger than {MAX_GIT_METADATA_BYTES} bytes and was not read"
            )),
            CappedRead::Read(contents) => {
                let Some(value) = contents.trim().strip_prefix("gitdir:") else {
                    return GitDirectory::Absent;
                };
                let value = value.trim();
                if value.is_empty() {
                    return GitDirectory::Absent;
                }
                let pointer = Path::new(value);
                let target = if pointer.is_absolute() {
                    pointer.to_path_buf()
                } else {
                    root.join(pointer)
                };
                if SymlinkGuard::validate_components_between(&target, root, environment).is_ok() {
                    return GitDirectory::resolved_whole(target);
                }
                match external_git_directory(&target) {
                    Some((git_dir, identity)) => GitDirectory::Resolved { git_dir, identity },
                    None => GitDirectory::Refused(format!(
                        "The .git pointer resolves to {}, which is outside the project and is not a linked-worktree or submodule git directory",
                        target.display()
                    )),
                }
            }
        },
        _ => GitDirectory::Absent,
    }
}

impl GitDirectory {
    /// A git directory that is both the `HEAD` source and the identity, which
    /// is the ordinary case: the repository's own `.git`.
    fn resolved_whole(git_dir: PathBuf) -> Self {
        GitDirectory::Resolved {
            identity: git_dir.clone(),
            git_dir,
        }
    }
}

/// The `(HEAD source, identity)` pair for a `gitdir:` pointer that leaves the
/// project.
///
/// Only the two shapes Git writes for a git directory that lives outside the
/// checkout are accepted, and only when the git directory they hang from is a
/// real one. A linked worktree reads its own `HEAD` but is identified by the
/// repository's git directory, exactly as identity resolution did before this
/// check existed; a submodule keeps both under its superproject. Everything
/// else, including a `separate-git-dir` checkout whose pointer has no shape
/// tying it to this repository, is refused.
fn external_git_directory(target: &Path) -> Option<(PathBuf, PathBuf)> {
    let container = target.parent()?;
    let git_dir = container.parent()?;
    if !git_dir.is_dir() || !git_dir.join("HEAD").is_file() {
        return None;
    }
    match container.file_name()?.to_str()? {
        "worktrees" => Some((target.to_path_buf(), git_dir.to_path_buf())),
        "modules" => Some((target.to_path_buf(), target.to_path_buf())),
        _ => None,
    }
}

/// Branch and detached state of the repository whose git directory is
/// `git_dir`, read under the same cap as the pointer file that named it.
fn read_head_status(git_dir: &Path) -> (Option<String>, bool) {
    let CappedRead::Read(head) = read_capped(&git_dir.join("HEAD")) else {
        return (None, false);
    };
    let trimmed = head.trim();
    if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
        if branch.len() > MAX_BRANCH_NAME_BYTES {
            return (None, false);
        }
        return (Some(branch.to_string()), false);
    }
    // A detached HEAD holds a raw object id. Anything else is not a state
    // Zenith can name, so it reports nothing instead of echoing the file.
    if trimmed.len() >= 7 && trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        (Some(format!("Detached ({})", &trimmed[..7])), true)
    } else {
        (None, false)
    }
}

pub fn candidate_project_roots(
    agent_cwds: &[PathBuf],
    dev_listeners: &[crate::models::DevelopmentListener],
    registered_workspaces: &[PathBuf],
    environment: &zenith_platform::PlatformEnvironment,
) -> Vec<PathBuf> {
    let mut candidates = HashSet::new();
    for cwd in agent_cwds {
        if let Some((root, _)) = resolve_project(cwd, environment) {
            candidates.insert(root);
        }
    }
    for listener in dev_listeners {
        if let Some(dir) = listener.working_directory.as_deref() {
            if let Some((root, _)) = resolve_project(Path::new(dir), environment) {
                candidates.insert(root);
            }
        }
    }
    for ws in registered_workspaces {
        if let Some((root, _)) = resolve_project(ws, environment) {
            candidates.insert(root);
        }
    }
    let mut list: Vec<_> = candidates.into_iter().collect();
    list.sort();
    list
}

pub fn opaque_id(namespace: &str, value: &Path) -> String {
    let mut digest = Sha256::new();
    digest.update(namespace.as_bytes());
    digest.update([0]);
    digest.update(value.as_os_str().as_encoded_bytes());
    let encoded = crate::hash::hex(&digest.finalize());
    format!("{namespace}-{}", &encoded[..16])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The host machine these project fixtures live on.
    fn test_environment() -> zenith_platform::PlatformEnvironment {
        zenith_platform::PlatformEnvironment::native()
    }

    #[test]
    fn resolves_deepest_repository_and_hides_absolute_path() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo-name");
        let nested = root.join("src/deep");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/feature/test\n").unwrap();

        let (resolved, identity) = resolve_project(&nested, &test_environment()).unwrap();
        assert_eq!(resolved, root.canonicalize().unwrap());
        assert_eq!(identity.display_name, "repo-name");
        assert_eq!(identity.branch.as_deref(), Some("feature/test"));

        // The hint is exactly the last two components, so the absolute prefix
        // of the root can never appear in it.
        let parent_name = root
            .parent()
            .and_then(Path::file_name)
            .expect("temporary root has a parent")
            .to_string_lossy()
            .to_string();
        assert_eq!(identity.location_hint, format!("{parent_name}/repo-name"));
        assert!(!identity.location_hint.contains("repo-name/src"));
        assert!(!identity.location_hint.contains(&root.display().to_string()));
        assert!(!identity.id.contains("repo-name"));
    }

    #[test]
    fn location_hint_masks_absolute_locations_in_both_flavors() {
        use zenith_platform::path_algebra::PathFlavor::{Posix, Windows};

        // Components only: no drive letter, no UNC server, no leading separator.
        assert_eq!(
            location_hint(Path::new(r"D:\Users\me\projects\app"), Windows).as_deref(),
            Some("projects/app")
        );
        assert_eq!(
            location_hint(Path::new(r"\\?\Z:\Users\me\app"), Windows).as_deref(),
            Some("me/app")
        );
        assert_eq!(
            location_hint(Path::new(r"\\server\share\me\app"), Windows).as_deref(),
            Some("me/app")
        );
        assert_eq!(
            location_hint(Path::new("/Users/me/projects/app"), Posix).as_deref(),
            Some("projects/app")
        );

        let hint = location_hint(Path::new(r"Z:\Users\me\secret-project"), Windows)
            .expect("a drive-rooted project has a hint");
        assert_eq!(hint, "me/secret-project");
        assert!(!hint.contains("Z:"), "drive letter leaked: {hint}");
        assert!(!hint.contains('\\'), "separator leaked: {hint}");
        assert!(!hint.starts_with('/'), "absolute path leaked: {hint}");
    }

    #[test]
    fn a_project_under_a_stated_home_renders_the_same_hint_on_every_flavor() {
        let home = tempfile::tempdir().unwrap();
        let project = home.path().join("projects/app");
        std::fs::create_dir_all(&project).unwrap();

        let native_hint = location_hint(&project, PathFlavor::current()).unwrap();
        assert_eq!(native_hint, "projects/app");
        assert!(!native_hint.contains(home.path().to_str().unwrap()));

        // The same shape under a stated Windows home renders identically, so
        // the hint never falls back to whatever layout the host happens to use.
        assert_eq!(
            location_hint(Path::new(r"D:\Users\me\projects\app"), PathFlavor::Windows).unwrap(),
            native_hint
        );
    }

    #[test]
    fn same_named_projects_receive_distinct_ids() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("one/project");
        let second = temp.path().join("two/project");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let (_, a) = resolve_project(&first, &test_environment()).unwrap();
        let (_, b) = resolve_project(&second, &test_environment()).unwrap();
        assert_ne!(a.id, b.id);
        assert_ne!(a.location_hint, b.location_hint);
    }

    #[test]
    fn a_home_directory_dotfiles_repository_does_not_absorb_child_projects() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let project = home.join("projects/app");
        std::fs::create_dir_all(home.join(".git")).unwrap();
        std::fs::create_dir_all(&project).unwrap();

        assert_eq!(find_git_root(&project, Some(&home)), None);

        std::fs::create_dir_all(project.join(".git")).unwrap();
        assert_eq!(find_git_root(&project, Some(&home)), Some(project.clone()));
    }

    /// A fixture workspace whose `.git` pointer names a git directory, plus the
    /// directories a pointer can legitimately name.
    fn gitfile_workspace(pointer: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join(".git"), pointer).unwrap();
        (temp, project)
    }

    fn write_head(git_dir: &Path, contents: &str) {
        std::fs::create_dir_all(git_dir).unwrap();
        std::fs::write(git_dir.join("HEAD"), contents).unwrap();
    }

    #[test]
    fn an_oversized_git_pointer_is_refused_rather_than_read() {
        let payload = "gitdir: ".to_string() + &"a".repeat(MAX_GIT_METADATA_BYTES as usize);
        let (_temp, project) = gitfile_workspace(&payload);

        let refused = match git_directory(&project, &test_environment()) {
            GitDirectory::Refused(reason) => reason,
            other => panic!(
                "an oversized pointer must be refused, got {}",
                match other {
                    GitDirectory::Absent => "Absent",
                    GitDirectory::Resolved { .. } => "Resolved",
                    GitDirectory::Refused(_) => unreachable!(),
                }
            ),
        };
        assert!(
            refused.contains(&MAX_GIT_METADATA_BYTES.to_string()),
            "the refusal must state the cap it applied: {refused}"
        );

        // The refusal is not presented as a repository: no identity, no branch,
        // and no `git` invocation in a directory whose `.git` was refused.
        let (_, identity) = resolve_project(&project, &test_environment()).unwrap();
        assert_eq!(identity.repository_id, None);
        assert_eq!(identity.branch, None);
        assert!(!identity.is_dirty);
    }

    #[test]
    fn a_git_pointer_outside_the_project_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("elsewhere");
        // The target exists and holds a HEAD, so the containment rule — not the
        // accident of a missing file — is what refuses it.
        write_head(&outside, "ref: refs/heads/private\n");
        let project = temp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join(".git"),
            format!("gitdir: {}", outside.display()),
        )
        .unwrap();

        let refused = match git_directory(&project, &test_environment()) {
            GitDirectory::Refused(reason) => reason,
            _ => panic!("a pointer that leaves the project must be refused"),
        };
        assert!(
            refused.contains("outside the project"),
            "the refusal must name the reason: {refused}"
        );

        let (_, identity) = resolve_project(&project, &test_environment()).unwrap();
        assert_eq!(identity.repository_id, None);
        assert_eq!(identity.branch, None);
        assert!(!identity.is_worktree);
    }

    #[test]
    fn a_linked_worktree_and_a_submodule_keep_resolving() {
        let temp = tempfile::tempdir().unwrap();
        let repository = temp.path().join("repository");
        let git_dir = repository.join(".git");
        // The repository's own git directory, as Git lays it out.
        write_head(&git_dir, "ref: refs/heads/main\n");
        write_head(
            &git_dir.join("worktrees/branch-x"),
            "ref: refs/heads/worktree-branch\n",
        );
        // A submodule keeps its git directory under the superproject's.
        write_head(&git_dir.join("modules/vendor-lib"), "ref: refs/heads/sub\n");

        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}", git_dir.join("worktrees/branch-x").display()),
        )
        .unwrap();

        let (_, identity) = resolve_project(&worktree, &test_environment()).unwrap();
        assert_eq!(identity.branch.as_deref(), Some("worktree-branch"));
        assert!(identity.is_worktree);
        assert_eq!(
            identity.repository_id,
            Some(opaque_id("repository", &git_dir)),
            "a worktree is identified by the repository's own git directory"
        );

        let submodule = repository.join("vendor-lib");
        std::fs::create_dir_all(&submodule).unwrap();
        std::fs::write(
            submodule.join(".git"),
            format!("gitdir: {}", git_dir.join("modules/vendor-lib").display()),
        )
        .unwrap();

        let (_, identity) = resolve_project(&submodule, &test_environment()).unwrap();
        assert_eq!(identity.branch.as_deref(), Some("sub"));
        assert_eq!(
            identity.repository_id,
            Some(opaque_id("repository", &git_dir.join("modules/vendor-lib"))),
            "a submodule keeps its own identity under the superproject"
        );
    }

    #[test]
    fn a_head_read_is_capped_and_a_branch_name_is_bounded() {
        let (_temp, project) = gitfile_workspace("gitdir: ./.git-dir");
        let git_dir = project.join(".git-dir");

        // An oversized HEAD is refused like an oversized pointer.
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(
            git_dir.join("HEAD"),
            "ref: refs/heads/".to_string() + &"b".repeat(MAX_GIT_METADATA_BYTES as usize),
        )
        .unwrap();
        assert_eq!(read_head_status(&git_dir), (None, false));

        // A ref name longer than Git would create is not truncated into one.
        let long_branch = "c".repeat(MAX_BRANCH_NAME_BYTES + 1);
        std::fs::write(
            git_dir.join("HEAD"),
            format!("ref: refs/heads/{long_branch}\n"),
        )
        .unwrap();
        assert_eq!(read_head_status(&git_dir), (None, false));

        // A detached HEAD holds an object id; anything else is not a state that
        // can be named, so the file is not echoed across IPC.
        std::fs::write(git_dir.join("HEAD"), "not an object id\n").unwrap();
        assert_eq!(read_head_status(&git_dir), (None, false));

        std::fs::write(
            git_dir.join("HEAD"),
            "0123456789abcdef0123456789abcdef01234567\n",
        )
        .unwrap();
        assert_eq!(
            read_head_status(&git_dir),
            (Some("Detached (0123456)".to_string()), true)
        );
    }

    #[test]
    fn out_of_home_projects_never_expose_an_absolute_display_path() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("outside-project");
        std::fs::create_dir_all(&project).unwrap();
        let (_, identity) = resolve_project(&project, &test_environment()).unwrap();
        assert!(
            !identity.display_path.starts_with('/') && !identity.display_path.contains(":\\"),
            "display path leaked an absolute location: {}",
            identity.display_path
        );
        // A temporary directory may live under the home directory (Windows) or
        // outside it (macOS); both render masked, never absolute.
        assert!(
            identity.display_path.starts_with("~/") || identity.display_path.starts_with(".../"),
            "display path was not masked: {}",
            identity.display_path
        );
    }
}
