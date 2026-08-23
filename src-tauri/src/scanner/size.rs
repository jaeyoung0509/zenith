use crate::models::FileSize;
use crate::safety::{Blacklist, SymlinkGuard};
use rayon::{Scope, ThreadPool};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub struct SizeCalculator;

impl SizeCalculator {
    /// Calculates the FileSize (logical size and allocated size on disk) for a single file or directory.
    pub fn measure_path<P: AsRef<Path>>(path: P, exclusions: &[String]) -> (FileSize, usize) {
        Self::measure_path_with_pool(path, exclusions, None)
    }

    pub(crate) fn measure_path_with_pool<P: AsRef<Path>>(
        path: P,
        exclusions: &[String],
        pool: Option<&ThreadPool>,
    ) -> (FileSize, usize) {
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
            if let Some(pool) = pool {
                return Self::measure_dir_parallel(path, exclusions, pool);
            }
            return Self::measure_dir_recursive(path, exclusions, 0, 32);
        }

        (FileSize::default(), 0)
    }

    fn measure_dir_parallel(
        path: &Path,
        exclusions: &[String],
        pool: &ThreadPool,
    ) -> (FileSize, usize) {
        let logical = AtomicU64::new(0);
        let allocated = AtomicU64::new(0);
        let file_count = AtomicUsize::new(0);
        pool.scope(|scope| {
            Self::spawn_dir_measurement(
                scope,
                path.to_path_buf(),
                exclusions,
                0,
                32,
                &logical,
                &allocated,
                &file_count,
            );
        });

        (
            FileSize::new(
                logical.load(Ordering::Relaxed),
                Some(allocated.load(Ordering::Relaxed)),
            ),
            file_count.load(Ordering::Relaxed),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_dir_measurement<'scope>(
        scope: &Scope<'scope>,
        dir: PathBuf,
        exclusions: &'scope [String],
        current_depth: usize,
        max_depth: usize,
        logical: &'scope AtomicU64,
        allocated: &'scope AtomicU64,
        file_count: &'scope AtomicUsize,
    ) {
        scope.spawn(move |scope| {
            if current_depth > max_depth {
                return;
            }
            let entries = match fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(_) => return,
            };

            // Aggregate files locally and synchronize only once per directory.
            let mut local_logical = 0u64;
            let mut local_allocated = 0u64;
            let mut local_file_count = 0usize;

            for entry in entries.flatten() {
                let child_path = entry.path();
                if Self::is_excluded(&child_path, exclusions)
                    || Blacklist::is_blacklisted(&child_path)
                {
                    continue;
                }

                // Never follow symlinked directories; account only for the link.
                if SymlinkGuard::is_symlink(&child_path) {
                    if let Ok(meta) = fs::symlink_metadata(&child_path) {
                        let len = meta.len();
                        local_logical += len;
                        #[cfg(unix)]
                        {
                            local_allocated += meta.blocks() * 512;
                        }
                        #[cfg(not(unix))]
                        {
                            local_allocated += len;
                        }
                        local_file_count += 1;
                    }
                    continue;
                }

                if let Ok(meta) = fs::symlink_metadata(&child_path) {
                    if meta.is_file() {
                        let len = meta.len();
                        local_logical += len;
                        #[cfg(unix)]
                        {
                            local_allocated += meta.blocks() * 512;
                        }
                        #[cfg(not(unix))]
                        {
                            local_allocated += len;
                        }
                        local_file_count += 1;
                    } else if meta.is_dir() && current_depth < max_depth {
                        Self::spawn_dir_measurement(
                            scope,
                            child_path,
                            exclusions,
                            current_depth + 1,
                            max_depth,
                            logical,
                            allocated,
                            file_count,
                        );
                    }
                }
            }

            logical.fetch_add(local_logical, Ordering::Relaxed);
            allocated.fetch_add(local_allocated, Ordering::Relaxed);
            file_count.fetch_add(local_file_count, Ordering::Relaxed);
        });
    }

    fn is_excluded(child_path: &Path, exclusions: &[String]) -> bool {
        let child_str = child_path.to_string_lossy();
        exclusions.iter().any(|exclusion| {
            if (exclusion.starts_with('~') || exclusion.starts_with('/'))
                && crate::signatures::SignatureLoader::expand_path(exclusion).is_some_and(
                    |expanded| child_path == expanded || child_path.starts_with(expanded),
                )
            {
                return true;
            }
            child_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == exclusion)
                || child_str.contains(exclusion)
        })
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

            if Self::is_excluded(&child_path, exclusions) {
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

#[cfg(all(test, unix))]
mod tests {
    use super::SizeCalculator;
    use rayon::ThreadPoolBuilder;
    use std::os::unix::fs::symlink;

    #[test]
    fn parallel_measurement_matches_sequential_safety_semantics() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("nested");
        let excluded = root.path().join("excluded");
        let git = root.path().join(".git");
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(nested.join("child")).unwrap();
        std::fs::create_dir(&excluded).unwrap();
        std::fs::create_dir(&git).unwrap();
        std::fs::write(nested.join("one.bin"), vec![1u8; 8_192]).unwrap();
        std::fs::write(nested.join("child/two.bin"), vec![2u8; 4_096]).unwrap();
        std::fs::write(excluded.join("ignored.bin"), vec![3u8; 16_384]).unwrap();
        std::fs::write(git.join("protected.bin"), vec![4u8; 32_768]).unwrap();
        std::fs::write(outside.path().join("escape.bin"), vec![5u8; 65_536]).unwrap();
        symlink(outside.path(), root.path().join("outside-link")).unwrap();

        let exclusions = vec!["excluded".to_string()];
        let sequential = SizeCalculator::measure_path(root.path(), &exclusions);
        let pool = ThreadPoolBuilder::new().num_threads(4).build().unwrap();
        let parallel =
            SizeCalculator::measure_path_with_pool(root.path(), &exclusions, Some(&pool));

        assert_eq!(parallel, sequential);
        assert_eq!(parallel.1, 3, "two files plus one untraversed symlink");
    }
}
