use crate::models::ZenithError;
use std::fs;
use std::path::Path;

pub struct SymlinkGuard;

impl SymlinkGuard {
    /// Checks whether the path is a symbolic link without following it.
    pub fn is_symlink(path: &Path) -> bool {
        match fs::symlink_metadata(path) {
            Ok(meta) => meta.file_type().is_symlink(),
            Err(_) => false,
        }
    }

    /// Verifies that no intermediate component between trusted_root and target (inclusive) is a symlink.
    pub fn validate_no_symlink_ancestors(
        target: &Path,
        trusted_root: &Path,
    ) -> Result<(), ZenithError> {
        let relative = target.strip_prefix(trusted_root).map_err(|_| {
            ZenithError::SymlinkEscape(format!(
                "Target {} is not within trusted root {}",
                target.display(),
                trusted_root.display()
            ))
        })?;

        let mut current = trusted_root.to_path_buf();
        for component in relative.components() {
            current.push(component);
            if let Ok(meta) = fs::symlink_metadata(&current) {
                if meta.file_type().is_symlink() {
                    return Err(ZenithError::SymlinkEscape(format!(
                        "Path component is a symlink escape: {}",
                        current.display()
                    )));
                }
            }
        }
        Ok(())
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
}
