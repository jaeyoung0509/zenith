use crate::models::FileSize;
use crate::platform::PlatformEnvironment;
use crate::safety::{Blacklist, SymlinkGuard};
use rayon::{Scope, ThreadPool};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[cfg(windows)]
pub fn get_allocated_size(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{GetLastError, NO_ERROR};
    use windows_sys::Win32::Storage::FileSystem::{GetCompressedFileSizeW, INVALID_FILE_SIZE};

    let path_text = path.to_string_lossy();
    let wide: Vec<u16> = if path_text.starts_with(r"\\?\") {
        path.as_os_str().encode_wide().chain([0]).collect()
    } else if let Some(unc_path) = path_text.strip_prefix(r"\\") {
        format!(r"\\?\UNC\{}", unc_path)
            .encode_utf16()
            .chain([0])
            .collect()
    } else if path.as_os_str().encode_wide().count() > 240 {
        format!(r"\\?\{}", path.display())
            .encode_utf16()
            .chain([0])
            .collect()
    } else {
        path.as_os_str().encode_wide().chain([0]).collect()
    };

    let mut high: u32 = 0;
    let low = unsafe { GetCompressedFileSizeW(wide.as_ptr(), &mut high) };
    if low == INVALID_FILE_SIZE {
        let err = unsafe { GetLastError() };
        if err != NO_ERROR {
            return None;
        }
    }
    Some(((high as u64) << 32) | (low as u64))
}

#[cfg(not(windows))]
pub fn get_allocated_size(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        fs::symlink_metadata(path).ok().map(|m| m.blocks() * 512)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

pub struct SizeCalculator;

impl SizeCalculator {
    /// Calculates the FileSize (logical size and allocated size on disk) for a single file or directory.
    ///
    /// Exclusions are expanded through the described environment, so a
    /// path-shaped exclusion resolves exactly as the environment states.
    pub fn measure_path<P: AsRef<Path>>(
        path: P,
        exclusions: &[String],
        environment: &PlatformEnvironment,
    ) -> (FileSize, usize) {
        Self::measure_path_with_pool(path, exclusions, None, environment)
    }

    pub(crate) fn measure_path_with_pool<P: AsRef<Path>>(
        path: P,
        exclusions: &[String],
        pool: Option<&ThreadPool>,
        environment: &PlatformEnvironment,
    ) -> (FileSize, usize) {
        let path = path.as_ref();
        if !path.exists() && !SymlinkGuard::is_symlink(path) {
            return (FileSize::default(), 0);
        }

        // Check if path is in blacklist
        if Blacklist::is_blacklisted_with(path, environment) {
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
            #[cfg(windows)]
            let allocated = get_allocated_size(path);
            #[cfg(not(any(unix, windows)))]
            let allocated = Some(logical);

            return (FileSize::new(logical, allocated), 1);
        }

        if meta.is_dir() {
            if let Some(pool) = pool {
                return Self::measure_dir_parallel(path, exclusions, pool, environment);
            }
            return Self::measure_dir_recursive(path, exclusions, 0, 32, environment);
        }

        (FileSize::default(), 0)
    }

    fn measure_dir_parallel(
        path: &Path,
        exclusions: &[String],
        pool: &ThreadPool,
        environment: &PlatformEnvironment,
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
                environment,
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
        environment: &'scope PlatformEnvironment,
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
                if Self::is_excluded(&child_path, exclusions, environment)
                    || Blacklist::is_blacklisted_with(&child_path, environment)
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
                        #[cfg(windows)]
                        {
                            local_allocated += get_allocated_size(&child_path).unwrap_or(len);
                        }
                        #[cfg(not(any(unix, windows)))]
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
                        #[cfg(windows)]
                        {
                            local_allocated += get_allocated_size(&child_path).unwrap_or(len);
                        }
                        #[cfg(not(any(unix, windows)))]
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
                            environment,
                        );
                    }
                }
            }

            logical.fetch_add(local_logical, Ordering::Relaxed);
            allocated.fetch_add(local_allocated, Ordering::Relaxed);
            file_count.fetch_add(local_file_count, Ordering::Relaxed);
        });
    }

    fn is_excluded(
        child_path: &Path,
        exclusions: &[String],
        environment: &PlatformEnvironment,
    ) -> bool {
        let child_str = child_path.to_string_lossy();
        exclusions.iter().any(|exclusion| {
            if (exclusion.starts_with('~') || exclusion.starts_with('/'))
                && crate::signatures::SignatureLoader::expand_path(exclusion, environment)
                    .is_some_and(|expanded| {
                        child_path == expanded || child_path.starts_with(expanded)
                    })
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
        environment: &PlatformEnvironment,
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

            if Self::is_excluded(&child_path, exclusions, environment) {
                continue;
            }

            // Check blacklist
            if Blacklist::is_blacklisted_with(&child_path, environment) {
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
                    #[cfg(windows)]
                    {
                        total_allocated += get_allocated_size(&child_path).unwrap_or(len);
                    }
                    #[cfg(not(any(unix, windows)))]
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
                    #[cfg(windows)]
                    {
                        total_allocated += get_allocated_size(&child_path).unwrap_or(len);
                    }
                    #[cfg(not(any(unix, windows)))]
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
                        environment,
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

#[cfg(test)]
mod tests {
    use super::SizeCalculator;
    use crate::platform::path_algebra::PathFlavor;
    use crate::platform::PlatformEnvironment;
    use rayon::ThreadPoolBuilder;

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

        // Only a POSIX host can create the untraversed symlink; the rest of the
        // safety semantics (exclusion, blacklist) hold on every platform.
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(outside.path(), root.path().join("outside-link")).unwrap();
        }

        let environment = PlatformEnvironment::simulated(PathFlavor::current());
        let exclusions = vec!["excluded".to_string()];
        let sequential = SizeCalculator::measure_path(root.path(), &exclusions, &environment);
        let pool = ThreadPoolBuilder::new().num_threads(4).build().unwrap();
        let parallel = SizeCalculator::measure_path_with_pool(
            root.path(),
            &exclusions,
            Some(&pool),
            &environment,
        );

        assert_eq!(parallel, sequential);
        let expected_files = 2 + usize::from(cfg!(unix));
        assert_eq!(
            parallel.1, expected_files,
            "two files plus (on unix) one untraversed symlink"
        );
    }

    #[test]
    fn exclusions_resolve_through_the_stated_environment() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        // The scanned tree is the literal profile `Downloads` folder. Scanning
        // the profile root itself is refused by the blacklist, and the point
        // here is which location a known-folder exclusion resolves to, so the
        // tree sits one level below the stated home.
        let scanned = home.join("Downloads");
        let kept = scanned.join("keep");
        std::fs::create_dir_all(&kept).unwrap();
        std::fs::write(kept.join("keep.bin"), vec![1u8; 4_096]).unwrap();
        std::fs::write(scanned.join("other.bin"), vec![2u8; 8_192]).unwrap();

        // When the environment states a redirected Downloads folder outside the
        // literal profile, `~/Downloads/keep` must resolve there and stop
        // excluding the identically named folder inside this tree.
        let redirected = root.path().join("redirected/Downloads");
        std::fs::create_dir_all(&redirected).unwrap();
        let simulated = |redirect: bool| {
            let environment = PlatformEnvironment::simulated(PathFlavor::current()).with_roots(
                std::sync::Arc::new(
                    crate::platform::paths::SimulatedPaths::new()
                        .with_flavor(PathFlavor::current())
                        .with_home(&home),
                ),
            );
            if redirect {
                environment.with_known_folder(crate::platform::KnownFolder::Downloads, &redirected)
            } else {
                environment
            }
        };

        let exclusions = vec!["~/Downloads/keep".to_string()];

        let redirected_environment = simulated(true);
        let (_, redirected_files) =
            SizeCalculator::measure_path(&scanned, &exclusions, &redirected_environment);
        assert_eq!(
            redirected_files, 2,
            "the redirect moves the exclusion out of this tree, so both files are measured"
        );

        // Without a stated redirect the literal profile spelling is excluded.
        let literal_environment = simulated(false);
        let (_, literal_files) =
            SizeCalculator::measure_path(&scanned, &exclusions, &literal_environment);
        assert_eq!(
            literal_files, 1,
            "the literal profile spelling excludes `~/Downloads/keep`"
        );
    }
}
