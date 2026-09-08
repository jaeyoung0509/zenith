use crate::models::ZenithError;
use std::fs;
use std::path::{Path, PathBuf};

pub struct SymlinkGuard;

impl SymlinkGuard {
    /// Checks whether the path is a symbolic link or reparse point (junction, mount point) without following it.
    pub fn is_symlink(path: &Path) -> bool {
        match fs::symlink_metadata(path) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return true;
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return true;
                    }
                }
                false
            }
            Err(_) => false,
        }
    }

    /// Resolves the trusted base anchor for a given target path.
    /// E.g., user home directory (`/Users/username` or `C:\Users\username`), `/tmp`, or temp dir.
    pub fn resolve_trusted_anchor(target: &Path) -> PathBuf {
        let home = crate::platform::NativePlatformPaths::new().home();
        if let Some(home) = home {
            if target.starts_with(&home) {
                return home;
            }
        }
        let temp = std::env::temp_dir();
        if target.starts_with(&temp) {
            return temp;
        }
        if target.starts_with("/private/tmp") {
            return PathBuf::from("/private/tmp");
        }
        if target.starts_with("/tmp") {
            return PathBuf::from("/tmp");
        }
        if target.starts_with("/private/var") {
            return PathBuf::from("/private/var");
        }
        if target.starts_with("/var") {
            return PathBuf::from("/var");
        }
        // A Windows drive/UNC root cannot be represented by the Unix `/` anchor.
        target
            .ancestors()
            .last()
            .unwrap_or(Path::new("/"))
            .to_path_buf()
    }

    /// Validates all path components from `base` down to `target`.
    pub fn validate_components_between(target: &Path, base: &Path) -> Result<(), ZenithError> {
        // Do not erase a link/.. traversal before checking its components.
        if !target.is_absolute()
            || !base.is_absolute()
            || target
                .components()
                .chain(base.components())
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(ZenithError::SymlinkEscape(
                "Expected an absolute path without parent traversal".into(),
            ));
        }
        let normalized_target = crate::safety::Blacklist::normalize_path(target);
        let normalized_base = crate::safety::Blacklist::normalize_path(base);
        let outside_base = || {
            ZenithError::SymlinkEscape(format!(
                "Target {} is not within base {}",
                target.display(),
                base.display()
            ))
        };
        let relative = match normalized_target.strip_prefix(&normalized_base) {
            Ok(relative) => relative.to_path_buf(),
            Err(_) => {
                #[cfg(not(windows))]
                return Err(outside_base());
                #[cfg(windows)]
                {
                    // TEMP can contain an 8.3 account alias (RUNNER~1) while
                    // canonicalize returns its long name. Check both original
                    // paths from their roots BEFORE resolving aliases, so a
                    // junction cannot disappear during canonicalization.
                    for path in [target, base] {
                        let root = path.ancestors().last().ok_or_else(outside_base)?;
                        Self::validate_components_between(path, root)?;
                    }
                    let canonical_target = fs::canonicalize(target).map_err(|_| outside_base())?;
                    let canonical_base = fs::canonicalize(base).map_err(|_| outside_base())?;
                    canonical_target
                        .strip_prefix(&canonical_base)
                        .map_err(|_| outside_base())?
                        .to_path_buf()
                }
            }
        };

        let mut current = base.to_path_buf();
        for component in relative.components() {
            current.push(component);
            if Self::is_symlink(&current) {
                return Err(ZenithError::SymlinkEscape(format!(
                    "Path component is a symlink or reparse escape: {}",
                    current.display()
                )));
            }
        }

        Ok(())
    }

    /// Verifies that no intermediate component between trusted base anchor, trusted_root,
    /// and target (inclusive) is a symlink.
    pub fn validate_no_symlink_ancestors(
        target: &Path,
        trusted_root: &Path,
    ) -> Result<(), ZenithError> {
        let anchor = Self::resolve_trusted_anchor(trusted_root);
        if trusted_root.starts_with(&anchor) && anchor != *trusted_root {
            Self::validate_components_between(trusted_root, &anchor)?;
        }

        Self::validate_components_between(target, trusted_root)?;
        Ok(())
    }

    /// Validates that target has no symlink ancestors from its system anchor (home/temp/root)
    pub fn validate_anchored_path(target: &Path) -> Result<(), ZenithError> {
        let anchor = Self::resolve_trusted_anchor(target);
        Self::validate_components_between(target, &anchor)
    }

    /// Verifies that the path itself is safe. If it is a symlink, ensures its target does not point to a blacklisted destination.
    pub fn validate_symlink_target(path: &Path) -> Result<(), ZenithError> {
        if Self::is_symlink(path) {
            // Read link destination
            if let Ok(target) = fs::read_link(path) {
                let resolved_target = if target.is_relative() {
                    path.parent().unwrap_or(Path::new("")).join(target)
                } else {
                    target
                };

                // Target cannot be blacklisted
                crate::safety::Blacklist::validate(&resolved_target)?;
            }
        }
        Ok(())
    }

    /// Canonicalizes the path and verifies that its canonical location does not violate Blacklist.
    pub fn validate_canonical_blacklist(path: &Path) -> Result<(), ZenithError> {
        if let Ok(canonical) = fs::canonicalize(path) {
            crate::safety::Blacklist::validate(&canonical)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_never_hides_parent_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("link/../cache");
        assert!(SymlinkGuard::validate_components_between(&path, dir.path()).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn selected_plain_path_matches_canonical_profile_without_crossing_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("사용자 하나");
        let workspace = profile.join("프로젝트");
        let other = dir.path().join("사용자 둘");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&other).unwrap();
        let canonical_profile = profile.canonicalize().unwrap();
        let plain_workspace = crate::safety::Blacklist::normalize_path(&workspace);
        SymlinkGuard::validate_components_between(&plain_workspace, &canonical_profile)
            .unwrap_or_else(|error| {
                panic!("target={plain_workspace:?}, base={canonical_profile:?}: {error}")
            });
        assert!(SymlinkGuard::validate_components_between(&other, &canonical_profile).is_err());
        assert!(
            crate::developer_artifacts::validate_workspace_root(&plain_workspace, &profile).is_ok()
        );
        assert!(crate::developer_artifacts::validate_workspace_root(&other, &profile).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn alias_fallback_does_not_hide_a_junction_into_the_trusted_profile() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("사용자 하나");
        let workspace = profile.join("프로젝트");
        let link = dir.path().join("다른 경로");
        fs::create_dir_all(&workspace).unwrap();
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&link)
            .arg(&profile)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(SymlinkGuard::validate_components_between(
            &link.join("프로젝트"),
            &profile.canonicalize().unwrap()
        )
        .is_err());
    }
}
