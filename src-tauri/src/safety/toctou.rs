use crate::models::{FileIdentity, ZenithError};
use std::fs;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub struct ToctouGuard;

impl ToctouGuard {
    /// Captures the filesystem identity (device ID, inode, file type, size, and modification timestamp).
    pub fn capture(path: &Path) -> Option<FileIdentity> {
        let meta = fs::symlink_metadata(path).ok()?;

        #[cfg(unix)]
        {
            Some(FileIdentity {
                device: meta.dev(),
                inode: meta.ino(),
                is_dir: meta.is_dir(),
                size: meta.len(),
                mtime_secs: meta.mtime().max(0) as u64,
                mtime_nanos: meta.mtime_nsec().max(0) as u32,
            })
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

            Some(FileIdentity {
                device,
                inode,
                is_dir: meta.is_dir(),
                size: meta.len(),
                mtime_secs,
                mtime_nanos,
            })
        }

        #[cfg(not(any(unix, windows)))]
        {
            let (mtime_secs, mtime_nanos) = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| (d.as_secs(), d.subsec_nanos()))
                .unwrap_or((0, 0));

            Some(FileIdentity {
                device: 0,
                inode: 0,
                is_dir: meta.is_dir(),
                size: meta.len(),
                mtime_secs,
                mtime_nanos,
            })
        }
    }

    /// Verifies that the filesystem identity matches what was recorded during scanning.
    pub fn verify(path: &Path, expected: &FileIdentity) -> Result<(), ZenithError> {
        if !path.exists() && !crate::safety::SymlinkGuard::is_symlink(path) {
            return Err(ZenithError::Io(format!(
                "Path {} does not exist",
                path.display()
            )));
        }

        let current = match Self::capture(path) {
            Some(id) => id,
            None => {
                return Err(ZenithError::ChangedSinceScan(format!(
                    "Could not read metadata for {}",
                    path.display()
                )))
            }
        };

        // A missing or zero identity is never accepted as verified when
        // identity comparison is required. Previous Windows captures that
        // could not open a directory recorded (0, 0) and skipped the check;
        // that fail-open is now a verification failure.
        #[cfg(any(unix, windows))]
        {
            if expected.device == 0 && expected.inode == 0 {
                return Err(ZenithError::ChangedSinceScan(format!(
                    "Filesystem identity unavailable for {}; refusing to mutate",
                    path.display()
                )));
            }
            if current.device == 0 && current.inode == 0 {
                return Err(ZenithError::ChangedSinceScan(format!(
                    "Could not verify filesystem identity for {}; refusing to mutate",
                    path.display()
                )));
            }
            if current.device != expected.device || current.inode != expected.inode {
                return Err(ZenithError::ChangedSinceScan(format!(
                    "Identity mismatch for {}: expected (dev={}, ino={}), found (dev={}, ino={})",
                    path.display(),
                    expected.device,
                    expected.inode,
                    current.device,
                    current.inode
                )));
            }
        }

        // File type (directory vs file) must strictly match
        if current.is_dir != expected.is_dir {
            return Err(ZenithError::ChangedSinceScan(format!(
                "File type changed from is_dir={} to is_dir={} for {}",
                expected.is_dir,
                current.is_dir,
                path.display()
            )));
        }

        // Modification timestamp and size must match for files. Directories
        // are also freshness-checked by default so a target directory changed
        // after scanning fails closed. The narrowly documented exception is
        // stale-temp signatures (`min_age_days`): the executor re-measures the
        // full tree newest-mtime immediately before deletion instead of
        // relying on the single directory mtime captured at plan time.
        if current.mtime_secs != expected.mtime_secs || current.mtime_nanos != expected.mtime_nanos
        {
            return Err(ZenithError::ChangedSinceScan(format!(
                "{} was modified after scanning (mtime mismatch)",
                path.display()
            )));
        }
        if !current.is_dir && current.size != expected.size {
            return Err(ZenithError::ChangedSinceScan(format!(
                "File {} size changed from {} to {} bytes after scanning",
                path.display(),
                expected.size,
                current.size
            )));
        }

        Ok(())
    }
}

/// Opens a path without following reparse points and returns its stable
/// volume/file identity. Used on Windows so directory identity capture never
/// traverses a junction or symlink and never accepts `(0, 0)` as verified.
#[cfg(windows)]
fn windows_file_identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // `\\?\` prefix allows long paths beyond MAX_PATH.
    let wide_prefixed: Vec<u16> =
        if path.as_os_str().len() > 240 && !path.to_string_lossy().starts_with(r"\\?\") {
            format!(r"\\?\{}", path.display())
                .encode_utf16()
                .chain([0])
                .collect()
        } else {
            wide
        };

    unsafe {
        let handle = CreateFileW(
            wide_prefixed.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            0,
        );
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return None;
        }
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        let ok = GetFileInformationByHandle(handle, &mut info);
        CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        let device = info.dwVolumeSerialNumber as u64;
        let inode = ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64);
        if device == 0 && inode == 0 {
            return None;
        }
        Some((device, inode))
    }
}
