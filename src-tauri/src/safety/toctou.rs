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

            let (device, inode) = if let Ok(file) = std::fs::File::open(path) {
                use std::os::windows::io::AsRawHandle;
                use windows_sys::Win32::Storage::FileSystem::{
                    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
                };
                unsafe {
                    let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
                    if GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) != 0 {
                        let dev = info.dwVolumeSerialNumber as u64;
                        let ino =
                            ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64);
                        (dev, ino)
                    } else {
                        (0, 0)
                    }
                }
            } else {
                (0, 0)
            };

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

        // Inode/FileId and device/volume must match on Unix and Windows
        #[cfg(any(unix, windows))]
        {
            if (expected.device != 0 || expected.inode != 0)
                && (current.device != expected.device || current.inode != expected.inode)
            {
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

        // For individual files, modification timestamp and size must match
        if !current.is_dir {
            if current.mtime_secs != expected.mtime_secs
                || current.mtime_nanos != expected.mtime_nanos
            {
                return Err(ZenithError::ChangedSinceScan(format!(
                    "File {} was modified after scanning (mtime mismatch)",
                    path.display()
                )));
            }
            if current.size != expected.size {
                return Err(ZenithError::ChangedSinceScan(format!(
                    "File {} size changed from {} to {} bytes after scanning",
                    path.display(),
                    expected.size,
                    current.size
                )));
            }
        }

        Ok(())
    }
}
