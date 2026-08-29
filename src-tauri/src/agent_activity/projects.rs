use crate::models::ProjectIdentity;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn resolve_project(cwd: &Path) -> Option<(PathBuf, ProjectIdentity)> {
    let canonical_cwd = cwd.canonicalize().ok()?;
    if !canonical_cwd.is_dir() {
        return None;
    }

    let git_root = canonical_cwd
        .ancestors()
        .find(|candidate| candidate.join(".git").exists())
        .map(Path::to_path_buf);

    let root = git_root.unwrap_or_else(|| canonical_cwd.clone());
    let marker = root.join(".git");
    let is_repository = marker.exists();
    let is_worktree = marker.is_file();

    let display_name = root.file_name()?.to_string_lossy().to_string();
    let parent_name = root
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().to_string());
    let location_hint = parent_name
        .map(|parent| format!("{parent}/{display_name}"))
        .unwrap_or_else(|| display_name.clone());

    let display_path = format_display_path(&root);

    let id = opaque_id("project", &root);
    let worktree_id = if is_worktree {
        Some(opaque_id("worktree", &root))
    } else {
        None
    };
    let repository_id = if is_repository {
        Some(opaque_id("repository", &repository_identity_path(&root)))
    } else {
        None
    };

    let (branch, is_detached) = if is_repository {
        read_head_status(&root)
    } else {
        (None, false)
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

fn format_display_path(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if let Ok(rel) = path.strip_prefix(&home) {
            return format!("~/{}", rel.display());
        }
    }
    path.display().to_string()
}

fn check_git_dirty(root: &Path) -> bool {
    let mut cmd = crate::tooling::command("git");
    cmd.arg("-C")
        .arg(root)
        .args(["status", "--porcelain=v1", "-z"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C");
    if let Ok(output) = crate::tooling::run_with_timeout(cmd, Duration::from_millis(800)) {
        if output.status.success() {
            return !output.stdout.is_empty();
        }
    }
    false
}

fn repository_identity_path(root: &Path) -> PathBuf {
    let marker = root.join(".git");
    if marker.is_dir() {
        return marker;
    }
    let Ok(contents) = std::fs::read_to_string(&marker) else {
        return root.to_path_buf();
    };
    let Some(value) = contents.trim().strip_prefix("gitdir:") else {
        return root.to_path_buf();
    };
    let git_dir = Path::new(value.trim());
    let absolute = if git_dir.is_absolute() {
        git_dir.to_path_buf()
    } else {
        root.join(git_dir)
    };
    absolute
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "worktrees"))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or(absolute)
}

fn read_head_status(root: &Path) -> (Option<String>, bool) {
    let marker = root.join(".git");
    let git_dir = if marker.is_dir() {
        marker
    } else {
        let Ok(value) = std::fs::read_to_string(&marker) else {
            return (None, false);
        };
        let Some(gitdir_val) = value.trim().strip_prefix("gitdir:") else {
            return (None, false);
        };
        let path = PathBuf::from(gitdir_val.trim());
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    };
    let Ok(head) = std::fs::read_to_string(git_dir.join("HEAD")) else {
        return (None, false);
    };
    let trimmed = head.trim();
    if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
        (Some(branch.to_string()), false)
    } else if !trimmed.is_empty() {
        let short_sha = if trimmed.len() >= 7 {
            &trimmed[..7]
        } else {
            trimmed
        };
        (Some(format!("Detached ({short_sha})")), true)
    } else {
        (None, false)
    }
}

pub fn candidate_project_roots(
    agent_cwds: &[PathBuf],
    dev_listeners: &[crate::models::DevelopmentListener],
    registered_workspaces: &[PathBuf],
) -> Vec<PathBuf> {
    let mut candidates = HashSet::new();
    for cwd in agent_cwds {
        if let Some((root, _)) = resolve_project(cwd) {
            candidates.insert(root);
        }
    }
    for listener in dev_listeners {
        if let Some(dir) = listener.working_directory.as_deref() {
            if let Some((root, _)) = resolve_project(Path::new(dir)) {
                candidates.insert(root);
            }
        }
    }
    for ws in registered_workspaces {
        if let Some((root, _)) = resolve_project(ws) {
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
    let encoded = format!("{:x}", digest.finalize());
    format!("{namespace}-{}", &encoded[..16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_deepest_repository_and_hides_absolute_path() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo-name");
        let nested = root.join("src/deep");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/feature/test\n").unwrap();

        let (resolved, identity) = resolve_project(&nested).unwrap();
        assert_eq!(resolved, root.canonicalize().unwrap());
        assert_eq!(identity.display_name, "repo-name");
        assert_eq!(identity.branch.as_deref(), Some("feature/test"));
        assert!(!identity.location_hint.starts_with('/'));
        assert!(!identity.id.contains("repo-name"));
    }

    #[test]
    fn same_named_projects_receive_distinct_ids() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("one/project");
        let second = temp.path().join("two/project");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let (_, a) = resolve_project(&first).unwrap();
        let (_, b) = resolve_project(&second).unwrap();
        assert_ne!(a.id, b.id);
        assert_ne!(a.location_hint, b.location_hint);
    }
}
