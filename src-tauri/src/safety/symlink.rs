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
        if let Some(home) = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
        {
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
        PathBuf::from("/")
    }

    /// Validates all path components from `base` down to `target`.
    pub fn validate_components_between(target: &Path, base: &Path) -> Result<(), ZenithError> {
        let relative = target.strip_prefix(base).map_err(|_| {
            ZenithError::SymlinkEscape(format!(
                "Target {} is not within base {}",
                target.display(),
                base.display()
            ))
        })?;

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
