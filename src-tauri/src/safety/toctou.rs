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
                mtime_secs: meta.mtime() as u64,
            })
        }

        #[cfg(not(unix))]
        {
            let mtime_secs = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            Some(FileIdentity {
                device: 0,
                inode: 0,
                is_dir: meta.is_dir(),
                size: meta.len(),
                mtime_secs,
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

        // Inode and device must match on Unix
        #[cfg(unix)]
        {
            if current.device != expected.device || current.inode != expected.inode {
                return Err(ZenithError::ChangedSinceScan(format!(
                    "Inode or device mismatch for {}: expected (dev={}, ino={}), found (dev={}, ino={})",
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

        Ok(())
    }
}
