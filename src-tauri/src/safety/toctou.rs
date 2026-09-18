use crate::models::{CleanupIdentity, FileIdentity, ModifiedStamp, ZenithError};
use std::fs;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub struct ToctouGuard;

impl ToctouGuard {
    /// Captures the filesystem identity (device ID, inode, file type, size, and modification timestamp).
    ///
    /// The platform-specific capture is unchanged; the result is wrapped in
    /// [`CleanupIdentity`] so the executor can only compare it against another
    /// capture from this same path, and never against a reviewed storage
    /// target's identity.
    pub fn capture(path: &Path) -> Option<CleanupIdentity> {
        let meta = fs::symlink_metadata(path).ok()?;

        #[cfg(unix)]
        {
            Some(CleanupIdentity::new(
                FileIdentity::new(meta.dev(), meta.ino()),
                meta.is_dir(),
                meta.len(),
                ModifiedStamp::new(meta.mtime().max(0) as u64, meta.mtime_nsec().max(0) as u32),
            ))
        }

        #[cfg(windows)]
        {
            let (mtime_secs, mtime_nanos) = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| (d.as_secs(), d.subsec_nanos()))
                .unwrap_or((0, 0));

            let (device, inode) = windows_file_identity(path)?;

            Some(CleanupIdentity::new(
                FileIdentity::new(device, inode),
                meta.is_dir(),
                meta.len(),
                ModifiedStamp::new(mtime_secs, mtime_nanos),
            ))
        }

        #[cfg(not(any(unix, windows)))]
        {
            let (mtime_secs, mtime_nanos) = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| (d.as_secs(), d.subsec_nanos()))
                .unwrap_or((0, 0));

            Some(CleanupIdentity::new(
                FileIdentity::new(0, 0),
                meta.is_dir(),
                meta.len(),
                ModifiedStamp::new(mtime_secs, mtime_nanos),
            ))
        }
    }

    fn capture_or_err(path: &Path) -> Result<CleanupIdentity, ZenithError> {
        match Self::capture(path) {
            Some(id) => Ok(id),
            None => match fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Err(ZenithError::Missing(path.display().to_string()))
                }
                Err(error) => Err(ZenithError::ChangedSinceScan(format!(
                    "Could not read metadata for {}: {}",
                    path.display(),
                    error
                ))),
                Ok(_) => {
                    // The path still exists, but `capture` could not derive a
                    // complete identity (for example, Windows could not open
                    // the handle). That is never equivalent to absence.
                    Err(ZenithError::ChangedSinceScan(format!(
                        "Could not verify filesystem identity for {}",
                        path.display()
                    )))
                }
            },
        }
    }

    fn verify_entity_identity(
        current: &CleanupIdentity,
        expected: &CleanupIdentity,
        path: &Path,
    ) -> Result<(), ZenithError> {
        // A missing or zero identity is never accepted as verified when
        // identity comparison is required. Previous Windows captures that
        // could not open a directory recorded (0, 0) and skipped the check;
        // that fail-open is now a verification failure.
        #[cfg(any(unix, windows))]
        {
            if expected.entity().is_unknown() {
                return Err(ZenithError::ChangedSinceScan(format!(
                    "Filesystem identity unavailable for {}; refusing to mutate",
                    path.display()
                )));
            }
            if current.entity().is_unknown() {
                return Err(ZenithError::ChangedSinceScan(format!(
                    "Could not verify filesystem identity for {}; refusing to mutate",
                    path.display()
                )));
            }
            if current.entity() != expected.entity() {
                return Err(ZenithError::ChangedSinceScan(format!(
                    "Identity mismatch for {}: expected ({}), found ({})",
                    path.display(),
                    expected.entity(),
                    current.entity()
                )));
            }
        }

        // File type (directory vs file) must strictly match
        if current.is_dir() != expected.is_dir() {
            return Err(ZenithError::ChangedSinceScan(format!(
                "File type changed from is_dir={} to is_dir={} for {}",
                expected.is_dir(),
                current.is_dir(),
                path.display()
            )));
        }

        Ok(())
    }

    /// Verifies the filesystem entity (device/inode or volume/file ID and kind)
    /// without requiring modification timestamp or size equality.
    ///
    /// Used by stale-content cleanup where child files change and modify the
    /// directory's mtime, but the target must remain the same filesystem object
    /// approved by the scan.
    pub fn verify_entity(path: &Path, expected: &CleanupIdentity) -> Result<(), ZenithError> {
        let current = Self::capture_or_err(path)?;
        Self::verify_entity_identity(&current, expected, path)
    }

    /// Verifies that the filesystem identity matches what was recorded during scanning.
    pub fn verify(path: &Path, expected: &CleanupIdentity) -> Result<(), ZenithError> {
        let current = Self::capture_or_err(path)?;
        Self::verify_entity_identity(&current, expected, path)?;

        // Modification timestamp and size must match for files. Directories
        // are also freshness-checked by default so a target directory changed
        // after scanning fails closed.
        if current.modified() != expected.modified() {
            return Err(ZenithError::ChangedSinceScan(format!(
                "{} was modified after scanning (mtime mismatch)",
                path.display()
            )));
        }
        if !current.is_dir() && current.size() != expected.size() {
            return Err(ZenithError::ChangedSinceScan(format!(
                "File {} size changed from {} to {} bytes after scanning",
                path.display(),
                expected.size(),
                current.size()
            )));
        }

        Ok(())
    }
}

/// Opens a path without following reparse points and returns its stable
/// volume/file identity with ordered fallback (ReFS FileIdInfo -> BY_HANDLE_FILE_INFORMATION -> weaker verification).
#[cfg(windows)]
pub(crate) fn windows_identity_from_handle(
    handle: windows_sys::Win32::Foundation::HANDLE,
) -> Option<(u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        BY_HANDLE_FILE_INFORMATION, FILE_ID_INFO,
    };

    // 1. ReFS/NTFS 128-bit file ID via FileIdInfo
    unsafe {
        let mut file_id_info: FILE_ID_INFO = std::mem::zeroed();
        let ok = GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            &mut file_id_info as *mut _ as *mut std::ffi::c_void,
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        );
        if ok != 0 {
            let volume = file_id_info.VolumeSerialNumber;
            let id_bytes = file_id_info.FileId.Identifier;
            let low = u64::from_le_bytes(id_bytes[0..8].try_into().unwrap());
            let high = u64::from_le_bytes(id_bytes[8..16].try_into().unwrap());
            let inode = low ^ high;
            if volume != 0 || inode != 0 {
                return Some((volume, inode));
            }
        }
    }

    // 2. Fall back to standard 64-bit file index via GetFileInformationByHandle
    unsafe {
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        let ok = GetFileInformationByHandle(handle, &mut info);
        if ok != 0 {
            let device = info.dwVolumeSerialNumber as u64;
            let inode = ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64);
            if device != 0 || inode != 0 {
                return Some((device, inode));
            }

            // 3. Fallback for FAT32 / exFAT / network shares that do not supply a file index.
            // Degrade to documented weaker verification combining volume serial with creation time and size.
            let ctime = ((info.ftCreationTime.dwHighDateTime as u64) << 32)
                | (info.ftCreationTime.dwLowDateTime as u64);
            let size = ((info.nFileSizeHigh as u64) << 32) | (info.nFileSizeLow as u64);
            let weak_inode = (ctime ^ size).max(1);
            let weak_device = if device == 0 { 1 } else { device };
            return Some((weak_device, weak_inode));
        }
    }

    None
}

#[cfg(windows)]
pub fn windows_file_identity(path: &Path) -> Option<(u64, u64)> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let wide = zenith_platform::NativePlatformPaths::to_verbatim_wide(path);

    unsafe {
        // Request minimum access (0) so locked/open files (node.exe logs, docker vhdx) can still be measured
        let handle = CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return None;
        }
        let identity = windows_identity_from_handle(handle);
        CloseHandle(handle);
        identity
    }
}

#[cfg(test)]
mod tests {
    use super::ToctouGuard;
    use crate::models::ZenithError;

    /// An identity check that races with a deletion reports absence rather than
    /// a generic I/O error, so the caller can classify it as already-absent.
    #[test]
    fn verify_on_a_missing_path_reports_missing() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("gone.dat");
        std::fs::write(&target, b"x").unwrap();
        let identity = ToctouGuard::capture(&target).expect("capture identity");
        std::fs::remove_file(&target).unwrap();

        match ToctouGuard::verify(&target, &identity) {
            Err(ZenithError::Missing(_)) => {}
            other => panic!("expected ZenithError::Missing, got {other:?}"),
        }
    }

    /// A present path whose identity changed is still a change, never absence.
    #[test]
    fn verify_on_a_replaced_path_still_reports_changed() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("payload.bin");
        std::fs::write(&target, b"v1").unwrap();
        let identity = ToctouGuard::capture(&target).expect("capture identity");
        std::fs::remove_file(&target).unwrap();
        std::fs::write(&target, b"v2").unwrap();

        assert!(matches!(
            ToctouGuard::verify(&target, &identity),
            Err(ZenithError::ChangedSinceScan(_))
        ));
    }

    /// `verify_entity` allows mtime changes on a directory while strictly verifying entity.
    #[test]
    fn verify_entity_allows_mtime_change_on_same_directory() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("cache");
        std::fs::create_dir_all(&target).unwrap();
        let identity = ToctouGuard::capture(&target).expect("capture identity");

        // Adding a file updates the directory's mtime
        std::thread::sleep(std::time::Duration::from_millis(15));
        std::fs::write(target.join("new_entry.tmp"), b"data").unwrap();

        assert!(
            ToctouGuard::verify_entity(&target, &identity).is_ok(),
            "verify_entity must succeed despite directory mtime change"
        );
    }

    /// `verify_entity` refuses a directory that was replaced with a new one at the same path.
    #[test]
    fn verify_entity_refuses_replaced_directory() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("cache");
        std::fs::create_dir_all(&target).unwrap();
        let identity = ToctouGuard::capture(&target).expect("capture identity");

        std::fs::remove_dir_all(&target).unwrap();
        std::fs::create_dir_all(&target).unwrap();

        // On filesystems with unique inodes/file IDs, recreation has a different ID
        #[cfg(any(unix, windows))]
        assert!(matches!(
            ToctouGuard::verify_entity(&target, &identity),
            Err(ZenithError::ChangedSinceScan(_))
        ));
    }
}
