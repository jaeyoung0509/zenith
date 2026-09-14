use crate::models::ProjectIdentity;
use crate::tooling::{
    git_directory, git_inspection_refusal, read_capped, CappedRead, GitDirectory,
};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zenith_platform::path_algebra::{self, PathFlavor};
use zenith_platform::PlatformEnvironment;

pub fn resolve_project(
    cwd: &Path,
    environment: &PlatformEnvironment,
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
    // A repository whose attribute sources cannot be neutralized is still
    // identified, but no `git` invocation is made in it: the branch is a bounded
    // file read, and the dirty state stays unread.
    let inspection_refusal = if is_repository {
        git_inspection_refusal(&root)
    } else {
        None
    };
    if let Some(reason) = &inspection_refusal {
        crate::diagnostics::log_error("project_identity", reason);
    }

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

    let is_dirty = if is_repository && inspection_refusal.is_none() {
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
    let Ok(mut cmd) = crate::tooling::git_command(root) else {
        // The refusal itself is reported where the project identity is resolved;
        // here the state is simply not read.
        return false;
    };
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

/// Longest branch name or detached label returned across IPC. Git bounds the
/// length of a ref name component, not of a whole hierarchical name, so this is
/// Zenith's own bound on what it hands the interface: a longer value is refused
/// rather than truncated into a name that never existed.
const MAX_BRANCH_NAME_BYTES: usize = 1_024;

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
    // A detached HEAD holds an object id: 40 hex digits in a SHA-1 repository,
    // 64 in a SHA-256 one. Anything else is not a state Zenith can name, so it
    // reports nothing instead of echoing the file back.
    if matches!(trimmed.len(), 40 | 64) && trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
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
    use crate::tooling::MAX_GIT_METADATA_BYTES;

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

    /// Runs `git` for a fixture: the assertions below are about how Zenith reads
    /// what Git itself wrote, so the metadata comes from Git rather than from a
    /// hand-written copy of its layout.
    fn fixture_git(root: &Path, args: &[&str]) -> std::process::Output {
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("git is installed on this host")
    }

    fn fixture_git_ok(root: &Path, args: &[&str]) {
        let output = fixture_git(root, args);
        assert!(
            output.status.success(),
            "fixture `git {args:?}` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn fixture_repository(root: &Path) {
        std::fs::create_dir_all(root).unwrap();
        fixture_git_ok(root, &["init", "-q"]);
        fixture_git_ok(root, &["config", "user.email", "fixture@example.invalid"]);
        fixture_git_ok(root, &["config", "user.name", "Fixture"]);
        std::fs::write(root.join("tracked.txt"), "baseline\n").unwrap();
        fixture_git_ok(root, &["add", "-A"]);
        fixture_git_ok(root, &["commit", "-qm", "initial"]);
    }

    #[test]
    fn a_linked_worktree_resolves_through_the_backlink_git_recorded() {
        let temp = tempfile::tempdir().unwrap();
        let repository = temp.path().join("repository");
        fixture_repository(&repository);
        let worktree = temp.path().join("checked-out-elsewhere");
        fixture_git_ok(
            &repository,
            &[
                "worktree",
                "add",
                "-q",
                &worktree.to_string_lossy(),
                "-b",
                "worktree-branch",
            ],
        );

        let (_, identity) = resolve_project(&worktree, &test_environment()).unwrap();
        assert_eq!(identity.branch.as_deref(), Some("worktree-branch"));
        assert!(identity.is_worktree);
        assert!(!identity.is_dirty, "a fresh worktree has no changes");
        // The identity is the repository's own git directory, derived from the
        // pointer the fixture wrote rather than from a canonicalized spelling.
        let pointer = std::fs::read_to_string(worktree.join(".git")).unwrap();
        let target = Path::new(pointer.trim().strip_prefix("gitdir:").unwrap().trim());
        let repository_git_dir = target.parent().unwrap().parent().unwrap();
        assert_eq!(
            identity.repository_id,
            Some(opaque_id("repository", repository_git_dir))
        );
    }

    #[test]
    fn a_worktree_shaped_git_directory_without_the_backlink_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let repository = temp.path().join("repository");
        fixture_repository(&repository);
        let real_worktree = temp.path().join("real-worktree");
        fixture_git_ok(
            &repository,
            &["worktree", "add", "-q", &real_worktree.to_string_lossy()],
        );
        let worktree_git_dir = repository.join(".git/worktrees/real-worktree");
        assert!(worktree_git_dir.join("HEAD").is_file());

        // A directory that merely points at another repository's worktree git
        // directory has the shape of a worktree without being one: the backlink
        // Git records names the real checkout, not this one.
        let unrelated = temp.path().join("unrelated");
        std::fs::create_dir_all(&unrelated).unwrap();
        std::fs::write(
            unrelated.join(".git"),
            format!("gitdir: {}", worktree_git_dir.display()),
        )
        .unwrap();

        match git_directory(&unrelated, &test_environment()) {
            GitDirectory::Refused(reason) => assert!(
                reason.contains("outside the project"),
                "the refusal must name the reason: {reason}"
            ),
            _ => panic!("a worktree shape without the recorded backlink must be refused"),
        }
        let (_, identity) = resolve_project(&unrelated, &test_environment()).unwrap();
        assert_eq!(identity.repository_id, None);
        assert_eq!(identity.branch, None);

        // The checkout the backlink does name keeps resolving.
        let (_, identity) = resolve_project(&real_worktree, &test_environment()).unwrap();
        assert!(identity.repository_id.is_some());
    }

    #[test]
    fn a_nested_submodule_resolves_through_its_recorded_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let superproject = temp.path().join("superproject");
        fixture_repository(&superproject);
        let dependency = temp.path().join("dependency");
        fixture_repository(&dependency);

        // Nested by more than one path component: the submodule git directory is
        // `.git/modules/<path>`, not `.git/modules/<name>`.
        fixture_git_ok(
            &superproject,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-q",
                &dependency.to_string_lossy(),
                "tools/nested/dependency",
            ],
        );

        let submodule = superproject.join("tools/nested/dependency");
        let submodule_git_dir = superproject.join(".git/modules/tools/nested/dependency");
        assert!(submodule_git_dir.join("config").is_file());
        let head = std::fs::read_to_string(submodule_git_dir.join("HEAD")).unwrap();
        let expected_branch = head
            .trim()
            .strip_prefix("ref: refs/heads/")
            .expect("a fixture repository starts on a branch")
            .to_string();

        let (_, identity) = resolve_project(&submodule, &test_environment()).unwrap();
        assert_eq!(identity.branch.as_deref(), Some(expected_branch.as_str()));
        // The identity is the git directory the pointer names, resolved against
        // the canonicalized checkout the way resolution itself does it.
        let pointer = std::fs::read_to_string(submodule.join(".git")).unwrap();
        let recorded = Path::new(pointer.trim().strip_prefix("gitdir:").unwrap().trim());
        let expected_identity = if recorded.is_absolute() {
            recorded.to_path_buf()
        } else {
            submodule.canonicalize().unwrap().join(recorded)
        };
        assert_eq!(
            identity.repository_id,
            Some(opaque_id("repository", &expected_identity)),
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

        // A ref name longer than Zenith's own bound is refused rather than
        // truncated into one that never existed.
        let long_branch = "c".repeat(MAX_BRANCH_NAME_BYTES + 1);
        std::fs::write(
            git_dir.join("HEAD"),
            format!("ref: refs/heads/{long_branch}\n"),
        )
        .unwrap();
        assert_eq!(read_head_status(&git_dir), (None, false));

        // Git bounds a ref name component, not the whole hierarchical name, so
        // a name longer than 255 bytes is still a ref Git created and is
        // reported as one.
        let hierarchical = format!("{}/{}", "d".repeat(130), "e".repeat(130));
        std::fs::write(
            git_dir.join("HEAD"),
            format!("ref: refs/heads/{hierarchical}\n"),
        )
        .unwrap();
        assert_eq!(
            read_head_status(&git_dir),
            (Some(hierarchical), false),
            "a hierarchical ref name must survive intact"
        );

        // A detached HEAD holds an object id — 40 hex digits in a SHA-1
        // repository, 64 in a SHA-256 one. Anything else is not a state that can
        // be named, so the file is not echoed across IPC.
        for unnamed in ["not an object id\n", "deadbee\n", "0123456789abcdef\n"] {
            std::fs::write(git_dir.join("HEAD"), unnamed).unwrap();
            assert_eq!(read_head_status(&git_dir), (None, false), "{unnamed:?}");
        }

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
