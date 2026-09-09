use crate::safety::{Blacklist, SymlinkGuard};
use crate::signatures::SignatureLoader;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;

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
#[derive(Debug, Default)]
struct PermissionSnapshot {
    #[cfg(unix)]
    original_mode: Option<u32>,
    #[cfg(unix)]
    directory: Option<fs::File>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl SafeTreeDeleter {
    pub fn delete_contents(root: &Path, exclusions: &[String]) -> TreeDeleteReport {
        let mut report = TreeDeleteReport::default();
        match fs::symlink_metadata(root) {
            Ok(meta) if meta.file_type().is_symlink() => {
                Self::delete_entry(root, root, exclusions, &mut report);
                return report;
            }
            Ok(meta) if !meta.is_dir() => {
                Self::delete_entry(root, root, exclusions, &mut report);
                return report;
            }
            Ok(_) => {}
            Err(error) => {
                // Fail closed: metadata failure aborts instead of being
                // treated as "nothing to delete".
                if !root.exists() {
                    return report;
                }
                report
                    .errors
                    .push(format!("{}: {}", root.display(), error));
                return report;
            }
        }

        if let Err(e) = Blacklist::validate(root) {
            report.errors.push(e.to_string());
            return report;
        }
        if let Err(e) = SymlinkGuard::validate_canonical_blacklist_strict(root) {
            report.errors.push(e.to_string());
            return report;
        }

        let root_metadata = match fs::symlink_metadata(root) {
            Ok(metadata) => metadata,
            Err(error) => {
                report.errors.push(format!("{}: {}", root.display(), error));
                return report;
            }
        };
        let permissions = match Self::prepare_directory(root, &root_metadata) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                report.errors.push(error);
                return report;
            }
        };
        if let Err(error) = Self::verify_directory_identity(root, &permissions) {
            report.errors.push(error);
            Self::restore_directory_permissions(root, permissions, &mut report);
            return report;
        }

        #[cfg(unix)]
        {
            if let Some(ref dir_file) = permissions.directory {
                Self::delete_dir_contents_via_fd(root, dir_file, root, exclusions, &mut report);
                Self::restore_directory_permissions(root, permissions, &mut report);
                return report;
            }
        }

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
        match fs::symlink_metadata(root) {
            Ok(_) => {}
            Err(error) => {
                if !root.exists() && !SymlinkGuard::is_symlink(root) {
                    return report;
                }
                report
                    .errors
                    .push(format!("{}: {}", root.display(), error));
                return report;
            }
        }
        if let Err(e) = Blacklist::validate(root) {
            report.errors.push(e.to_string());
            return report;
        }
        if let Err(e) = SymlinkGuard::validate_canonical_blacklist_strict(root) {
            report.errors.push(e.to_string());
            return report;
        }
        Self::delete_entry(root, root, exclusions, &mut report);
        report
    }

    /// Deletes directory children using the already-verified parent directory
    /// descriptor. The final unlink for every child goes through `unlinkat`
    /// on that descriptor, so replacing or redirecting any parent component
    /// after validation cannot redirect deletion outside the planned scope.
    #[cfg(unix)]
    fn delete_dir_contents_via_fd(
        dir_path: &Path,
        dir_file: &fs::File,
        verified_root: &Path,
        exclusions: &[String],
        report: &mut TreeDeleteReport,
    ) {
        let entries = match fs::read_dir(dir_path) {
            Ok(e) => e.collect::<Vec<_>>(),
            Err(e) => {
                report.errors.push(e.to_string());
                return;
            }
        };

        for entry in entries {
            let ent = match entry {
                Ok(ent) => ent,
                Err(e) => {
                    report.errors.push(e.to_string());
                    continue;
                }
            };
            let child_path = ent.path();
            let Some(file_name) = child_path.file_name() else {
                report.errors.push(format!(
                    "Could not determine file name for {}",
                    child_path.display()
                ));
                continue;
            };

            if Self::is_excluded(&child_path, &exclusions)
                || Blacklist::is_blacklisted(&child_path)
            {
                report.skipped_files += 1;
                continue;
            }

            let metadata = match fs::symlink_metadata(&child_path) {
                Ok(m) => m,
                Err(e) => {
                    report
                        .errors
                        .push(format!("{}: {}", child_path.display(), e));
                    continue;
                }
            };

            if let Err(error) =
                Self::validate_verified_scope(&child_path, verified_root, &metadata)
            {
                report.errors.push(error);
                continue;
            }
            if let Err(error) =
                SymlinkGuard::validate_canonical_blacklist_strict(&child_path)
            {
                report
                    .errors
                    .push(format!("{}: {}", child_path.display(), error));
                continue;
            }
            if let Err(error) = Self::validate_entry_owner(&child_path, &metadata) {
                report.errors.push(error);
                continue;
            }

            if metadata.file_type().is_symlink() || metadata.is_file() {
                if let Err(error) = Self::verify_entry_identity(&child_path, &metadata) {
                    report.errors.push(error);
                    continue;
                }
                let bytes = allocated_bytes(&metadata);
                match Self::unlink_via_parent(dir_file, file_name, false) {
                    Ok(()) => {
                        report.reclaimed_bytes += bytes;
                        report.deleted_files += 1;
                    }
                    Err(e) => {
                        report.errors.push(format!("{}: {}", child_path.display(), e));
                    }
                }
                continue;
            }

            if !metadata.is_dir() {
                continue;
            }

            // Recurse with the child's own verified descriptor.
            let child_permissions = match Self::prepare_directory(&child_path, &metadata) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    report.errors.push(error);
                    continue;
                }
            };
            if let Err(error) = Self::verify_directory_identity(&child_path, &child_permissions)
            {
                report.errors.push(error);
                Self::restore_directory_permissions(&child_path, child_permissions, report);
                continue;
            }
            if let Some(ref child_file) = child_permissions.directory {
                Self::delete_dir_contents_via_fd(
                    &child_path,
                    child_file,
                    verified_root,
                    &exclusions,
                    report,
                );
            }
            if let Err(error) =
                Self::verify_directory_identity(&child_path, &child_permissions)
            {
                report.errors.push(error);
                Self::restore_directory_permissions(&child_path, child_permissions, report);
                continue;
            }
            // Remove the now-empty child directory relative to the verified
            // parent descriptor. The child's own permissions are restored
            // implicitly by removing it; only restore on failure.
            match Self::unlink_via_parent(dir_file, file_name, true) {
                Ok(()) => {}
                Err(error)
                    if error.kind() == io::ErrorKind::DirectoryNotEmpty =>
                {
                    Self::restore_directory_permissions(&child_path, child_permissions, report);
                }
                Err(error) => {
                    report.errors.push(format!("{}: {}", child_path.display(), error));
                    Self::restore_directory_permissions(&child_path, child_permissions, report);
                }
            }
        }
    }

    /// Unlinks a child name relative to a verified parent directory
    /// descriptor. Never re-resolves the full path for the final mutation.
    #[cfg(unix)]
    fn unlink_via_parent(
        parent: &fs::File,
        name: &OsStr,
        is_dir: bool,
    ) -> io::Result<()> {
        use std::os::unix::ffi::OsStrExt;

        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes.contains(&b'/') || bytes == b"." || bytes == b".." {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to unlink unsafe child name",
            ));
        }
        let mut buf = Vec::with_capacity(bytes.len() + 1);
        buf.extend_from_slice(bytes);
        buf.push(0);
        let flags = if is_dir { libc::AT_REMOVEDIR } else { 0 };
        // SAFETY: `buf` is a NUL-terminated non-empty basename without `/`,
        // and `parent` is a verified directory fd opened with O_NOFOLLOW.
        let res = unsafe {
            libc::unlinkat(
                parent.as_raw_fd(),
                buf.as_ptr() as *const libc::c_char,
                flags,
            )
        };
        if res == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
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
        // links, never traversed. Canonicalization failure fails closed.
        if let Err(error) = SymlinkGuard::validate_canonical_blacklist_strict(path) {
            report.errors.push(format!("{}: {}", path.display(), error));
            return;
        }

        if let Err(error) = Self::validate_entry_owner(path, &metadata) {
            report.errors.push(error);
            return;
        }

        if metadata.file_type().is_symlink() || metadata.is_file() {
            if let Err(error) = Self::verify_entry_identity(path, &metadata) {
                report.errors.push(error);
                return;
            }
            let bytes = allocated_bytes(&metadata);
            #[cfg(unix)]
            {
                // Prefer descriptor-relative unlink via the verified parent.
                // Falls back to path removal only when the parent cannot be
                // opened safely, in which case the error fails closed.
                match Self::remove_file_via_verified_parent(path) {
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
            #[cfg(not(unix))]
            {
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
        }

        if !metadata.is_dir() {
            return;
        }

        let permissions = match Self::prepare_directory(path, &metadata) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                report.errors.push(error);
                return;
            }
        };
        if let Err(error) = Self::verify_directory_identity(path, &permissions) {
            report.errors.push(error);
            Self::restore_directory_permissions(path, permissions, report);
            return;
        }

        #[cfg(unix)]
        if let Some(ref _dir_file) = permissions.directory {
            Self::delete_dir_contents_via_fd(path, _dir_file, verified_root, exclusions, report);
            if let Err(error) = Self::verify_directory_identity(path, &permissions) {
                report.errors.push(error);
                Self::restore_directory_permissions(path, permissions, report);
                return;
            }
            // The top-level directory itself is removed relative to its own
            // verified parent so a replaced parent cannot redirect it.
            // `delete_contents` never reaches here for its root (it returns
            // after `delete_dir_contents_via_fd`); `delete_path` removes it.
            let remove_result = Self::remove_dir_via_verified_parent(path);
            match remove_result {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => {
                    Self::restore_directory_permissions(path, permissions, report);
                }
                Err(error) => {
                    report.errors.push(format!("{}: {}", path.display(), error));
                    Self::restore_directory_permissions(path, permissions, report);
                }
            }
            return;
        }

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

        if let Err(error) = Self::verify_directory_identity(path, &permissions) {
            report.errors.push(error);
            Self::restore_directory_permissions(path, permissions, report);
            return;
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

    /// Removes a single file/symlink relative to its verified parent
    /// directory descriptor opened with O_DIRECTORY | O_NOFOLLOW.
    #[cfg(unix)]
    fn remove_file_via_verified_parent(path: &Path) -> io::Result<()> {
        let (parent, name) = Self::open_parent_nofollow(path)?;
        Self::unlink_via_parent(&parent, &name, false)
    }

    /// Removes an empty directory relative to its verified parent descriptor.
    #[cfg(unix)]
    fn remove_dir_via_verified_parent(path: &Path) -> io::Result<()> {
        let (parent, name) = Self::open_parent_nofollow(path)?;
        Self::unlink_via_parent(&parent, &name, true)
    }

    /// Opens the parent directory without following symlinks and verifies it
    /// is still a directory. Returns the parent handle plus the child basename.
    #[cfg(unix)]
    fn open_parent_nofollow(path: &Path) -> io::Result<(fs::File, std::ffi::OsString)> {
        let parent_path = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "path has no parent directory")
        })?;
        let file_name = path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "path has no file name")
        })?;
        if file_name == "." || file_name == ".." {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to unlink dot path",
            ));
        }
        let parent = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(parent_path)?;
        let meta = parent.metadata()?;
        if !meta.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "parent is no longer a directory",
            ));
        }
        Ok((parent, file_name.to_os_string()))
    }

    /// Make a user-owned directory traversable and writable for the duration
    /// of recursive deletion. Only missing owner bits are added; group/other
    /// permissions and special bits are preserved.
    fn prepare_directory(
        path: &Path,
        expected_metadata: &fs::Metadata,
    ) -> Result<PermissionSnapshot, String> {
        #[cfg(unix)]
        {
            let directory = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)
                .map_err(|error| {
                    format!(
                        "Permission denied: could not safely open cleanup directory ({}): {}",
                        path.display(),
                        error
                    )
                })?;
            let metadata = directory.metadata().map_err(|error| {
                format!(
                    "Could not verify cleanup directory {}: {}",
                    path.display(),
                    error
                )
            })?;
            if metadata.dev() != expected_metadata.dev()
                || metadata.ino() != expected_metadata.ino()
                || !metadata.is_dir()
            {
                return Err(format!(
                    "Directory changed during cleanup: {}",
                    path.display()
                ));
            }

            // fchmod is intentionally limited to a no-follow descriptor for a
            // directory owned by the effective user. Zenith never escalates
            // privileges or chmods a replacement symlink.
            let effective_uid = unsafe { libc::geteuid() } as u32;
            if metadata.uid() != effective_uid {
                return Err(format!(
                    "Permission denied: cleanup directory is not owned by the current user: {}",
                    path.display()
                ));
            }

            let original_mode = metadata.mode() & 0o7777;
            let required_mode = original_mode | 0o700;
            if required_mode != original_mode {
                directory
                    .set_permissions(fs::Permissions::from_mode(required_mode))
                    .map_err(|error| {
                        format!(
                            "Permission denied: could not make cleanup directory writable ({}): {}",
                            path.display(),
                            error
                        )
                    })?;
            }

            let snapshot = PermissionSnapshot {
                original_mode: (required_mode != original_mode).then_some(original_mode),
                directory: Some(directory),
                device: metadata.dev(),
                inode: metadata.ino(),
            };

            Ok(snapshot)
        }

        #[cfg(not(unix))]
        {
            // Windows prepares no permission snapshot; identity is captured
            // per entry via no-follow handles in ToctouGuard. Fail closed if
            // the directory cannot be metadata-verified as a real directory.
            let current = fs::symlink_metadata(path).map_err(|error| {
                format!("Could not verify cleanup directory {}: {}", path.display(), error)
            })?;
            if current.file_type().is_symlink() || !current.is_dir() {
                return Err(format!(
                    "Directory changed during cleanup: {}",
                    path.display()
                ));
            }
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

            let Some(directory) = snapshot.directory.as_ref() else {
                return;
            };
            if let Err(error) = directory.set_permissions(fs::Permissions::from_mode(original_mode))
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

    fn validate_entry_owner(path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
        #[cfg(unix)]
        {
            let effective_uid = unsafe { libc::geteuid() } as u32;
            if metadata.uid() != effective_uid {
                return Err(format!(
                    "Permission denied: cleanup entry is not owned by the current user: {}",
                    path.display()
                ));
            }
        }

        #[cfg(not(unix))]
        let _ = (path, metadata);

        Ok(())
    }

    fn verify_entry_identity(path: &Path, expected: &fs::Metadata) -> Result<(), String> {
        let current = fs::symlink_metadata(path).map_err(|error| {
            format!(
                "Could not re-verify cleanup entry {}: {}",
                path.display(),
                error
            )
        })?;

        #[cfg(unix)]
        if current.dev() != expected.dev() || current.ino() != expected.ino() {
            return Err(format!("Entry changed during cleanup: {}", path.display()));
        }

        if current.file_type() != expected.file_type() {
            return Err(format!("Entry changed during cleanup: {}", path.display()));
        }
        Ok(())
    }

    fn verify_directory_identity(path: &Path, snapshot: &PermissionSnapshot) -> Result<(), String> {
        #[cfg(unix)]
        {
            let current = fs::symlink_metadata(path).map_err(|error| {
                format!(
                    "Could not re-verify cleanup directory {}: {}",
                    path.display(),
                    error
                )
            })?;
            if current.file_type().is_symlink()
                || !current.is_dir()
                || current.dev() != snapshot.device
                || current.ino() != snapshot.inode
            {
                return Err(format!(
                    "Directory changed during cleanup: {}",
                    path.display()
                ));
            }
        }

        #[cfg(not(unix))]
        {
            // Windows has no retained descriptor; re-verify symlink state and
            // fail closed on metadata errors.
            let is_link = SymlinkGuard::is_symlink_strict(path).map_err(|e| e.to_string())?;
            if is_link {
                return Err(format!(
                    "Directory changed during cleanup: {}",
                    path.display()
                ));
            }
            let current = fs::symlink_metadata(path).map_err(|error| {
                format!(
                    "Could not re-verify cleanup directory {}: {}",
                    path.display(),
                    error
                )
            })?;
            if !current.is_dir() {
                return Err(format!(
                    "Directory changed during cleanup: {}",
                    path.display()
                ));
            }
        }

        Ok(())
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
        // Canonicalization failure fails closed on mutation paths.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[cfg(unix)]
    #[test]
    fn parent_replacement_between_verification_and_unlink_leaves_outside_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let parent = root.join("parent");
        std::fs::create_dir_all(&parent).unwrap();
        let child = parent.join("child.bin");
        std::fs::write(&child, b"inside").unwrap();

        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let outside_target = outside.join("child.bin");
        std::fs::write(&outside_target, b"outside").unwrap();

        // Open the verified parent descriptor before replacement.
        let parent_file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&parent)
            .unwrap();

        // Simulate a parent-component race: replace `parent` with a symlink to
        // the outside directory after validation.
        std::fs::remove_file(&child).unwrap();
        std::fs::remove_dir(&parent).unwrap();
        std::os::unix::fs::symlink(&outside, &parent).unwrap();

        // The verified descriptor still points at the original directory, so
        // unlinking a stale basename through it must fail without touching
        // the outside target.
        let result =
            SafeTreeDeleter::unlink_via_parent(&parent_file, std::ffi::OsStr::new("child.bin"), false);
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(&outside_target).unwrap(),
            b"outside",
            "outside target must remain untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_relative_file_removal_deletes_inside_target() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let payload = root.join("payload.bin");
        std::fs::write(&payload, b"payload").unwrap();

        let report = SafeTreeDeleter::delete_contents(&root, &[]);
        assert!(report.is_success(), "errors: {:?}", report.errors);
        assert!(!payload.exists());
        assert!(root.is_dir());
    }

    #[test]
    fn symlink_metadata_failure_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist-link");
        assert!(SymlinkGuard::is_symlink_strict(&missing).is_err());
        assert!(SymlinkGuard::validate_canonical_blacklist_strict(&missing).is_err());
    }
}
