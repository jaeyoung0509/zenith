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

    let wide = crate::platform::NativePlatformPaths::to_verbatim_wide(path);

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
