use crate::safety::{Blacklist, SymlinkGuard};
use crate::signatures::SignatureLoader;
use std::fs;
use std::io;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeDeleteReport {
    pub reclaimed_bytes: u64,
    pub deleted_files: usize,
    pub skipped_files: usize,
    pub errors: Vec<String>,
}

impl TreeDeleteReport {
    pub fn is_success(&self) -> bool {
        self.errors.is_empty()
    }
}

pub struct SafeTreeDeleter;

/// Records the directory identity and mode before cleanup temporarily adds the
/// owner permissions needed to remove a read-only tree.
#[derive(Debug, Clone, Copy, Default)]
struct PermissionSnapshot {
    #[cfg(unix)]
    original_mode: Option<u32>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl SafeTreeDeleter {
    pub fn delete_contents(root: &Path, exclusions: &[String]) -> TreeDeleteReport {
        let mut report = TreeDeleteReport::default();
        if !root.exists() && !SymlinkGuard::is_symlink(root) {
            return report;
        }
        if !root.is_dir() || SymlinkGuard::is_symlink(root) {
            Self::delete_entry(root, root, exclusions, &mut report);
            return report;
        }

        if let Err(e) = Blacklist::validate(root) {
            report.errors.push(e.to_string());
            return report;
        }
        if let Err(e) = SymlinkGuard::validate_canonical_blacklist(root) {
            report.errors.push(e.to_string());
            return report;
        }

        let permissions = match Self::prepare_directory(root) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                report.errors.push(error);
                return report;
            }
        };

        let entries = match fs::read_dir(root) {
            Ok(e) => e,
            Err(e) => {
                report.errors.push(e.to_string());
                Self::restore_directory_permissions(root, permissions, &mut report);
                return report;
            }
        };

        for entry in entries {
            match entry {
                Ok(ent) => Self::delete_entry(&ent.path(), root, exclusions, &mut report),
                Err(e) => report.errors.push(e.to_string()),
            }
        }
        Self::restore_directory_permissions(root, permissions, &mut report);
        report
    }

    pub fn delete_path(root: &Path, exclusions: &[String]) -> TreeDeleteReport {
        let mut report = TreeDeleteReport::default();
        if !root.exists() && !SymlinkGuard::is_symlink(root) {
            return report;
        }
        if let Err(e) = Blacklist::validate(root) {
            report.errors.push(e.to_string());
            return report;
        }
        if let Err(e) = SymlinkGuard::validate_canonical_blacklist(root) {
            report.errors.push(e.to_string());
            return report;
        }
        Self::delete_entry(root, root, exclusions, &mut report);
        report
    }

    fn delete_entry(
        path: &Path,
        verified_root: &Path,
        exclusions: &[String],
        report: &mut TreeDeleteReport,
    ) {
        if Self::is_excluded(path, exclusions) || Blacklist::is_blacklisted(path) {
            report.skipped_files += 1;
            return;
        }

        let metadata = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                report.errors.push(format!("{}: {}", path.display(), e));
                return;
            }
        };

        if let Err(error) = Self::validate_verified_scope(path, verified_root, &metadata) {
            report.errors.push(error);
            return;
        }

        // Re-check the canonical location at every recursive entry before any
        // permission change or deletion. Symlink entries are still removed as
        // links, never traversed.
        if let Err(error) = SymlinkGuard::validate_canonical_blacklist(path) {
            report.errors.push(format!("{}: {}", path.display(), error));
            return;
        }

        if metadata.file_type().is_symlink() || metadata.is_file() {
            let bytes = allocated_bytes(&metadata);
            match fs::remove_file(path) {
                Ok(()) => {
                    report.reclaimed_bytes += bytes;
                    report.deleted_files += 1;
                }
                Err(e) => {
                    report.errors.push(format!("{}: {}", path.display(), e));
                }
            }
            return;
        }

        if !metadata.is_dir() {
            return;
        }

        let permissions = match Self::prepare_directory(path) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                report.errors.push(error);
                return;
            }
        };

        let entries = match fs::read_dir(path) {
            Ok(e) => e,
            Err(e) => {
                report.errors.push(format!("{}: {}", path.display(), e));
                Self::restore_directory_permissions(path, permissions, report);
                return;
            }
        };

        for entry in entries {
            match entry {
                Ok(ent) => Self::delete_entry(&ent.path(), verified_root, exclusions, report),
                Err(e) => report.errors.push(format!("{}: {}", path.display(), e)),
            }
        }

        match fs::remove_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => {
                Self::restore_directory_permissions(path, permissions, report);
            }
            Err(error) => {
                report.errors.push(format!("{}: {}", path.display(), error));
                Self::restore_directory_permissions(path, permissions, report);
            }
        }
    }

    /// Make a user-owned directory traversable and writable for the duration
    /// of recursive deletion. Only missing owner bits are added; group/other
    /// permissions and special bits are preserved.
    fn prepare_directory(path: &Path) -> Result<PermissionSnapshot, String> {
        #[cfg(unix)]
        {
            let metadata = fs::symlink_metadata(path)
                .map_err(|error| format!("{}: {}", path.display(), error))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Ok(PermissionSnapshot::default());
            }

            // chmod is intentionally limited to directories owned by the
            // effective user. Zenith never escalates privileges for cleanup.
            let effective_uid = unsafe { libc::geteuid() } as u32;
            if metadata.uid() != effective_uid {
                return Err(format!(
                    "Permission denied: cleanup directory is not owned by the current user: {}",
                    path.display()
                ));
            }

            let original_mode = metadata.mode() & 0o7777;
            let required_mode = original_mode | 0o700;
            let snapshot = PermissionSnapshot {
                original_mode: (required_mode != original_mode).then_some(original_mode),
                device: metadata.dev(),
                inode: metadata.ino(),
            };

            if required_mode != original_mode {
                fs::set_permissions(path, fs::Permissions::from_mode(required_mode)).map_err(
                    |error| {
                        format!(
                            "Permission denied: could not make cleanup directory writable ({}): {}",
                            path.display(),
                            error
                        )
                    },
                )?;
            }

            Ok(snapshot)
        }

        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(PermissionSnapshot::default())
        }
    }

    fn restore_directory_permissions(
        path: &Path,
        snapshot: PermissionSnapshot,
        report: &mut TreeDeleteReport,
    ) {
        #[cfg(unix)]
        if let Some(original_mode) = snapshot.original_mode {
            // Never restore through a replacement or symlink. If the entry
            // changed while cleanup was running, leave it untouched and let
            // the next scan surface the change.
            let Ok(metadata) = fs::symlink_metadata(path) else {
                return;
            };
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || metadata.dev() != snapshot.device
                || metadata.ino() != snapshot.inode
            {
                return;
            }

            if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(original_mode))
            {
                report.errors.push(format!(
                    "Permission denied: could not restore directory permissions ({}): {}",
                    path.display(),
                    error
                ));
            }
        }

        #[cfg(not(unix))]
        {
            let _ = (path, snapshot, report);
        }
    }

    fn validate_verified_scope(
        path: &Path,
        verified_root: &Path,
        metadata: &fs::Metadata,
    ) -> Result<(), String> {
        let normalized_path = Blacklist::normalize_path(path);
        let normalized_root = Blacklist::normalize_path(verified_root);
        if normalized_path != normalized_root && !normalized_path.starts_with(&normalized_root) {
            return Err(format!(
                "Path escaped the verified cleanup target: {}",
                path.display()
            ));
        }

        // A final symlink is removed as a link and is never traversed. For all
        // real files/directories, also compare canonical paths so a replaced
        // parent symlink cannot redirect cleanup outside the planned root.
        if metadata.file_type().is_symlink() {
            return Ok(());
        }
        let canonical_root = fs::canonicalize(verified_root).map_err(|error| {
            format!(
                "Could not verify cleanup root {}: {}",
                verified_root.display(),
                error
            )
        })?;
        let canonical_path = fs::canonicalize(path).map_err(|error| {
            format!(
                "Could not verify cleanup path {}: {}",
                path.display(),
                error
            )
        })?;
        if canonical_path != canonical_root && !canonical_path.starts_with(&canonical_root) {
            return Err(format!(
                "Path escaped the verified cleanup target: {}",
                path.display()
            ));
        }
        Ok(())
    }

    fn is_excluded(path: &Path, exclusions: &[String]) -> bool {
        exclusions.iter().any(|exclusion| {
            if (exclusion.starts_with('~') || exclusion.starts_with('/'))
                && SignatureLoader::expand_path(exclusion)
                    .is_some_and(|expanded| path == expanded || path.starts_with(expanded))
            {
                return true;
            }
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == exclusion)
        })
    }
}

fn allocated_bytes(metadata: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        metadata.blocks().saturating_mul(512)
    }
    #[cfg(not(unix))]
    {
        metadata.len()
    }
}
