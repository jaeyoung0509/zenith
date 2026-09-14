//! Which git directory describes a project root.
//!
//! Pure filesystem work: a `.git` entry is resolved through the pointer forms
//! Git records, and everything read is bounded by [`crate::git::metadata`].

use crate::git::metadata::{
    core_config_value, gitfile_target, read_capped, recorded_path, CappedRead,
    MAX_GIT_METADATA_BYTES,
};
use crate::privacy::paths::mask_paths_in_text;
use crate::safety::symlink::SymlinkGuard;
use std::path::{Path, PathBuf};
use zenith_platform::PlatformEnvironment;

/// The git directory that describes a project root.
pub enum GitDirectory {
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

impl GitDirectory {
    /// A git directory that is both the `HEAD` source and the identity, which is
    /// the ordinary case: the repository's own `.git`.
    fn resolved_whole(git_dir: PathBuf) -> Self {
        GitDirectory::Resolved {
            identity: git_dir.clone(),
            git_dir,
        }
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
/// - a target outside it is accepted only when Git's own backlink ties it to
///   this checkout: a linked worktree records `gitdir` naming this project's
///   `.git`, and a submodule records `core.worktree` naming this project;
/// - anything else is refused with a reason, because a path that merely *looks*
///   like a worktree or submodule git directory is a shape, not a relationship.
///
/// `root` is workspace content, not a nominated directory: it comes from an
/// observed agent process working directory.
pub fn git_directory(root: &Path, environment: &PlatformEnvironment) -> GitDirectory {
    let marker = root.join(".git");
    if SymlinkGuard::is_symlink_metadata(&marker).unwrap_or(true) {
        // A link is not followed on faith, and a Windows directory junction is
        // not a `FileType::is_symlink` either, so the guard decides both.
        return match std::fs::canonicalize(&marker) {
            Ok(target)
                if SymlinkGuard::validate_components_between(&target, root, environment)
                    .is_ok() =>
            {
                GitDirectory::resolved_whole(target)
            }
            Ok(target) => match external_git_directory(&target, root) {
                Some((git_dir, identity)) => GitDirectory::Resolved { git_dir, identity },
                None => GitDirectory::Refused(mask_paths_in_text(&format!(
                    "The .git entry is a link to {}, which is not this project's git directory",
                    target.display()
                ))),
            },
            Err(_) => GitDirectory::Absent,
        };
    }
    if marker.is_dir() {
        return GitDirectory::resolved_whole(marker);
    }
    match read_capped(&marker) {
        CappedRead::Absent => GitDirectory::Absent,
        CappedRead::Unreadable => GitDirectory::Refused(
            "The .git pointer exists but could not be read safely".to_string(),
        ),
        CappedRead::OverCap => GitDirectory::Refused(format!(
            "The .git pointer file is larger than {MAX_GIT_METADATA_BYTES} bytes and was not read"
        )),
        CappedRead::Read(contents) => {
            let Some(target) = gitfile_target(root, &contents) else {
                return GitDirectory::Absent;
            };
            if SymlinkGuard::validate_components_between(&target, root, environment).is_ok() {
                return GitDirectory::resolved_whole(target);
            }
            match external_git_directory(&target, root) {
                Some((git_dir, identity)) => GitDirectory::Resolved { git_dir, identity },
                None => GitDirectory::Refused(mask_paths_in_text(&format!(
                    "The .git pointer resolves to {}, which is outside the project and is not this project's linked-worktree or submodule git directory",
                    target.display()
                ))),
            }
        }
    }
}

/// The `(HEAD source, identity)` pair for a `gitdir:` pointer that leaves the
/// project, when Git's own backlink ties it to `root`.
fn external_git_directory(target: &Path, root: &Path) -> Option<(PathBuf, PathBuf)> {
    // The git directory the target descends from: the nearest ancestor holding a
    // `HEAD`, since `worktrees/<name>` and `modules/<path>` both sit directly
    // under one.
    let git_dir = target
        .ancestors()
        .skip(1)
        .find(|candidate| candidate.is_dir() && candidate.join("HEAD").is_file())?;
    let shape = target.strip_prefix(git_dir).ok()?;
    let mut components = shape.components();
    match components.next()?.as_os_str().to_str()? {
        "worktrees" => {
            if components.count() != 1 || !worktree_belongs_to(target, root) {
                return None;
            }
            // A linked worktree reads its own HEAD but is identified by the
            // repository's git directory, exactly as identity resolution did
            // before this check existed.
            Some((target.to_path_buf(), git_dir.to_path_buf()))
        }
        "modules" => {
            components.next()?;
            if !submodule_belongs_to(target, root) {
                return None;
            }
            // A submodule keeps its own git directory and its own identity.
            Some((target.to_path_buf(), target.to_path_buf()))
        }
        _ => None,
    }
}

/// Whether `<target>/gitdir` names this project's `.git`, which Git records when
/// it creates a linked worktree.
fn worktree_belongs_to(target: &Path, root: &Path) -> bool {
    let CappedRead::Read(contents) = read_capped(&target.join("gitdir")) else {
        return false;
    };
    let Some(recorded) = recorded_path(target, &contents) else {
        return false;
    };
    same_location(&recorded, &root.join(".git"))
}

/// Whether the submodule git directory records this project as its working tree,
/// which `git submodule` writes as `core.worktree`.
fn submodule_belongs_to(target: &Path, root: &Path) -> bool {
    let CappedRead::Read(contents) = read_capped(&target.join("config")) else {
        return false;
    };
    let Some(worktree) = core_config_value(&contents, "worktree") else {
        return false;
    };
    let Some(recorded) = recorded_path(target, &worktree) else {
        return false;
    };
    same_location(&recorded, root)
}

/// Compares two paths by what they resolve to, so a recorded relative path and
/// the checkout it names are compared as the same location.
fn same_location(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}
