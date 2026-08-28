use crate::models::ProjectIdentity;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub fn resolve_project(cwd: &Path) -> Option<(PathBuf, ProjectIdentity)> {
    let canonical_cwd = cwd.canonicalize().ok()?;
    if !canonical_cwd.is_dir() {
        return None;
    }

    let root = canonical_cwd
        .ancestors()
        .find(|candidate| candidate.join(".git").exists())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| canonical_cwd.clone());
    let git_marker = root.join(".git");
    let is_repository = git_marker.exists();
    let is_worktree = git_marker.is_file();
    let display_name = root.file_name()?.to_string_lossy().to_string();
    let parent_name = root
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().to_string());
    let location_hint = parent_name
        .map(|parent| format!("{parent}/{display_name}"))
        .unwrap_or_else(|| display_name.clone());

    let id = opaque_id("project", &root);
    let repository_id =
        is_repository.then(|| opaque_id("repository", &repository_identity_path(&root)));
    let branch = is_repository.then(|| read_head_name(&root)).flatten();

    Some((
        root,
        ProjectIdentity {
            id,
            display_name,
            location_hint,
            repository_id,
            is_worktree,
            branch,
        },
    ))
}

fn repository_identity_path(root: &Path) -> PathBuf {
    let marker = root.join(".git");
    if marker.is_dir() {
        return marker;
    }
    let Ok(contents) = std::fs::read_to_string(marker) else {
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

fn read_head_name(root: &Path) -> Option<String> {
    let marker = root.join(".git");
    let git_dir = if marker.is_dir() {
        marker
    } else {
        let value = std::fs::read_to_string(marker).ok()?;
        let value = value.trim().strip_prefix("gitdir:")?.trim();
        let path = PathBuf::from(value);
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    };
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    head.trim()
        .strip_prefix("ref: refs/heads/")
        .map(str::to_string)
        .or_else(|| Some("Detached HEAD".to_string()))
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
