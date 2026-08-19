use crate::models::FileSize;
use crate::safety::{Blacklist, SymlinkGuard};
use std::fs;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub struct SizeCalculator;

impl SizeCalculator {
    /// Calculates the FileSize (logical size and allocated size on disk) for a single file or directory.
    pub fn measure_path<P: AsRef<Path>>(path: P, exclusions: &[String]) -> (FileSize, usize) {
        let path = path.as_ref();
        if !path.exists() && !SymlinkGuard::is_symlink(path) {
            return (FileSize::default(), 0);
        }

        // Check if path is in blacklist
        if Blacklist::is_blacklisted(path) {
            return (FileSize::default(), 0);
        }

        // If path is a symlink, only measure the symlink itself
        if SymlinkGuard::is_symlink(path) {
            let logical = fs::symlink_metadata(path).map(|m| m.len()).unwrap_or(0);
            return (FileSize::new(logical, Some(logical)), 1);
        }

        let meta = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(_) => return (FileSize::default(), 0),
        };

        if meta.is_file() {
            let logical = meta.len();
            #[cfg(unix)]
            let allocated = Some(meta.blocks() * 512);
            #[cfg(not(unix))]
            let allocated = Some(logical);

            return (FileSize::new(logical, allocated), 1);
        }

        if meta.is_dir() {
            return Self::measure_dir_recursive(path, exclusions, 0, 32);
        }

        (FileSize::default(), 0)
    }

    fn measure_dir_recursive(
        dir: &Path,
        exclusions: &[String],
        current_depth: usize,
        max_depth: usize,
    ) -> (FileSize, usize) {
        if current_depth > max_depth {
            return (FileSize::default(), 0);
        }

        let mut total_logical = 0u64;
        let mut total_allocated = 0u64;
        let mut file_count = 0usize;

        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return (FileSize::default(), 0),
        };

        for entry in entries.flatten() {
            let child_path = entry.path();

            // Check exclusions
            let child_str = child_path.to_string_lossy();
            if exclusions.iter().any(|ex| {
                if ex.starts_with('~') || ex.starts_with('/') {
                    if let Some(expanded) = crate::signatures::SignatureLoader::expand_path(ex) {
                        if child_path == expanded || child_path.starts_with(&expanded) {
                            return true;
                        }
                    }
                }
                child_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n == ex)
                    .unwrap_or(false)
                    || child_str.contains(ex)
            }) {
                continue;
            }

            // Check blacklist
            if Blacklist::is_blacklisted(&child_path) {
                continue;
            }

            // Symlink check: DO NOT traverse into symlinked directories
            if SymlinkGuard::is_symlink(&child_path) {
                if let Ok(m) = fs::symlink_metadata(&child_path) {
                    let len = m.len();
                    total_logical += len;
                    #[cfg(unix)]
                    {
                        total_allocated += m.blocks() * 512;
                    }
                    #[cfg(not(unix))]
                    {
                        total_allocated += len;
                    }
                    file_count += 1;
                }
                continue;
            }

            if let Ok(meta) = fs::symlink_metadata(&child_path) {
                if meta.is_file() {
                    let len = meta.len();
                    total_logical += len;
                    #[cfg(unix)]
                    {
                        total_allocated += meta.blocks() * 512;
                    }
                    #[cfg(not(unix))]
                    {
                        total_allocated += len;
                    }
                    file_count += 1;
                } else if meta.is_dir() {
                    let (sub_size, sub_count) = Self::measure_dir_recursive(
                        &child_path,
                        exclusions,
                        current_depth + 1,
                        max_depth,
                    );
                    total_logical += sub_size.logical;
                    total_allocated += sub_size.allocated.unwrap_or(sub_size.logical);
                    file_count += sub_count;
                }
            }
        }

        (
            FileSize::new(total_logical, Some(total_allocated)),
            file_count,
        )
    }
}
