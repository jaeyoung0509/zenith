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
