use crate::models::FileSize;
use crate::platform::PlatformEnvironment;
use crate::safety::{Blacklist, SymlinkGuard};
use rayon::{Scope, ThreadPool};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathMeasurement {
    pub size: FileSize,
    pub file_count: usize,
    pub complete: bool,
    pub incomplete_reason: Option<String>,
    /// Entries the walk did not account for: excluded, blacklisted, protected,
    /// unreadable, or beyond the depth limit. A size that skipped entries must
    /// never be presented as a complete measurement.
    pub skipped_entries: u64,
}

impl PathMeasurement {
    pub fn new(
        size: FileSize,
        file_count: usize,
        complete: bool,
        incomplete_reason: Option<String>,
    ) -> Self {
        Self {
            size,
            file_count,
            complete,
            incomplete_reason,
            skipped_entries: 0,
        }
    }

    pub fn complete(size: FileSize, file_count: usize) -> Self {
        Self {
            size,
            file_count,
            complete: true,
            incomplete_reason: None,
            skipped_entries: 0,
        }
    }

    pub fn incomplete(size: FileSize, file_count: usize, reason: impl Into<String>) -> Self {
        Self {
            size,
            file_count,
            complete: false,
            incomplete_reason: Some(reason.into()),
            skipped_entries: 0,
        }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            size: FileSize::default(),
            file_count: 0,
            complete: false,
            incomplete_reason: Some(reason.into()),
            skipped_entries: 0,
        }
    }

    /// Records how many entries the walk did not measure.
    pub fn with_skipped_entries(mut self, skipped_entries: u64) -> Self {
        self.skipped_entries = skipped_entries;
        self
    }
}

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
    /// Measures a file or directory, reporting how complete the observation is.
    ///
    /// Exclusions are expanded through the described environment, so a
    /// path-shaped exclusion resolves exactly as the environment states. The
    /// caller receives [`PathMeasurement`] rather than a size pair because a
    /// measurement that skipped entries must be logged, not added up silently.
    pub fn measure_path_full<P: AsRef<Path>>(
        path: P,
        exclusions: &[String],
        environment: &PlatformEnvironment,
    ) -> PathMeasurement {
        Self::measure_path_with_pool(path, exclusions, None, environment)
    }

    /// Measures a path and records an incomplete observation in the log.
    ///
    /// Callers that report the number to the user need both the measurement and
    /// the knowledge that it is a lower bound: a walk that could not read part
    /// of the tree used to arrive as an ordinary size. `measure_path_full`
    /// stays available for callers that handle incompleteness themselves.
    pub fn measure_path_logged<P: AsRef<Path>>(
        path: P,
        exclusions: &[String],
        environment: &PlatformEnvironment,
    ) -> PathMeasurement {
        let path = path.as_ref();
        let measurement = Self::measure_path_full(path, exclusions, environment);
        if !measurement.complete {
            crate::diagnostics::log_error(
                "measurement",
                &format!(
                    "Incomplete measurement for {}: {}",
                    path.display(),
                    measurement
                        .incomplete_reason
                        .as_deref()
                        .unwrap_or("unknown boundary")
                ),
            );
        }
        measurement
    }

    pub(crate) fn measure_path_with_pool<P: AsRef<Path>>(
        path: P,
        exclusions: &[String],
        pool: Option<&ThreadPool>,
        environment: &PlatformEnvironment,
    ) -> PathMeasurement {
        let path = path.as_ref();
        if !path.exists() && !SymlinkGuard::is_symlink(path) {
            return PathMeasurement::complete(FileSize::default(), 0);
        }

        // Check if path is in blacklist
        if Blacklist::is_blacklisted_with(path, environment) {
            return PathMeasurement::incomplete(
                FileSize::default(),
                0,
                "Protected by system safety blacklist",
            )
            .with_skipped_entries(1);
        }

        // If path is a symlink, only measure the symlink itself
        if SymlinkGuard::is_symlink(path) {
            let logical = match fs::symlink_metadata(path) {
                Ok(m) => m.len(),
                Err(err) => {
                    return PathMeasurement::incomplete(
                        FileSize::default(),
                        0,
                        format!(
                            "Could not read symlink metadata for {}: {}",
                            path.display(),
                            err
                        ),
                    )
                    .with_skipped_entries(1);
                }
            };
            return PathMeasurement::complete(FileSize::new(logical, Some(logical)), 1);
        }

        let meta = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(err) => {
                return PathMeasurement::unavailable(format!(
                    "Could not read metadata for {}: {}",
                    path.display(),
                    err
                ))
                .with_skipped_entries(1);
            }
        };

        if meta.is_file() {
            let logical = meta.len();
            #[cfg(unix)]
            let allocated = Some(meta.blocks() * 512);
            #[cfg(windows)]
            let allocated = get_allocated_size(path);
            #[cfg(not(any(unix, windows)))]
            let allocated = Some(logical);

            return PathMeasurement::complete(FileSize::new(logical, allocated), 1);
        }

        if meta.is_dir() {
            if let Some(pool) = pool {
                return Self::measure_dir_parallel(path, exclusions, pool, environment);
            }
            return Self::measure_dir_recursive(path, exclusions, 0, 32, environment);
        }

        PathMeasurement::complete(FileSize::default(), 0)
    }

    fn measure_dir_parallel(
        path: &Path,
        exclusions: &[String],
        pool: &ThreadPool,
        environment: &PlatformEnvironment,
    ) -> PathMeasurement {
        let logical = AtomicU64::new(0);
        let allocated = AtomicU64::new(0);
        let file_count = AtomicUsize::new(0);
        let complete = AtomicBool::new(true);
        let skipped = AtomicU64::new(0);
        let reason = Mutex::new(None);

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
                &complete,
                &skipped,
                &reason,
                environment,
            );
        });

        let is_complete = complete.load(Ordering::Relaxed);
        let incomplete_reason = reason.into_inner().unwrap_or_default();
        PathMeasurement {
            size: FileSize::new(
                logical.load(Ordering::Relaxed),
                Some(allocated.load(Ordering::Relaxed)),
            ),
            file_count: file_count.load(Ordering::Relaxed),
            complete: is_complete,
            incomplete_reason,
            skipped_entries: skipped.load(Ordering::Relaxed),
        }
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
        complete: &'scope AtomicBool,
        skipped: &'scope AtomicU64,
        reason: &'scope Mutex<Option<String>>,
        environment: &'scope PlatformEnvironment,
    ) {
        scope.spawn(move |scope| {
            if current_depth > max_depth {
                complete.store(false, Ordering::Relaxed);
                skipped.fetch_add(1, Ordering::Relaxed);
                let mut r = reason.lock().unwrap_or_else(|p| p.into_inner());
                if r.is_none() {
                    *r = Some(format!(
                        "Directory depth limit of {} exceeded at {}",
                        max_depth,
                        dir.display()
                    ));
                }
                return;
            }
            let entries = match fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(err) => {
                    complete.store(false, Ordering::Relaxed);
                    skipped.fetch_add(1, Ordering::Relaxed);
                    let mut r = reason.lock().unwrap_or_else(|p| p.into_inner());
                    if r.is_none() {
                        *r = Some(format!(
                            "Failed to read directory {}: {}",
                            dir.display(),
                            err
                        ));
                    }
                    return;
                }
            };

            // Aggregate files locally and synchronize only once per directory.
            let mut local_logical = 0u64;
            let mut local_allocated = 0u64;
            let mut local_file_count = 0usize;
            let mut local_skipped = 0u64;

            for entry in entries {
                let ent = match entry {
                    Ok(e) => e,
                    Err(err) => {
                        complete.store(false, Ordering::Relaxed);
                        local_skipped += 1;
                        let mut r = reason.lock().unwrap_or_else(|p| p.into_inner());
                        if r.is_none() {
                            *r = Some(format!(
                                "Failed to read directory entry in {}: {}",
                                dir.display(),
                                err
                            ));
                        }
                        continue;
                    }
                };
                let child_path = ent.path();
                if Self::is_excluded(&child_path, exclusions, environment)
                    || Blacklist::is_blacklisted_with(&child_path, environment)
                {
                    local_skipped += 1;
                    continue;
                }

                // Never follow symlinked directories; account only for the link.
                if SymlinkGuard::is_symlink(&child_path) {
                    match fs::symlink_metadata(&child_path) {
                        Ok(meta) => {
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
                        Err(err) => {
                            complete.store(false, Ordering::Relaxed);
                            local_skipped += 1;
                            let mut r = reason.lock().unwrap_or_else(|p| p.into_inner());
                            if r.is_none() {
                                *r = Some(format!(
                                    "Failed to read symlink metadata for {}: {}",
                                    child_path.display(),
                                    err
                                ));
                            }
                        }
                    }
                    continue;
                }

                match fs::symlink_metadata(&child_path) {
                    Ok(meta) => {
                        if meta.is_dir()
                            && child_path
                                .extension()
                                .and_then(|ext| ext.to_str())
                                .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
                        {
                            complete.store(false, Ordering::Relaxed);
                            local_skipped += 1;
                            let mut r = reason.lock().unwrap_or_else(|p| p.into_inner());
                            if r.is_none() {
                                *r = Some(format!(
                                    "Protected application bundle encountered in {}",
                                    child_path.display()
                                ));
                            }
                            continue;
                        }

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
                        } else if meta.is_dir() {
                            if current_depth < max_depth {
                                Self::spawn_dir_measurement(
                                    scope,
                                    child_path,
                                    exclusions,
                                    current_depth + 1,
                                    max_depth,
                                    logical,
                                    allocated,
                                    file_count,
                                    complete,
                                    skipped,
                                    reason,
                                    environment,
                                );
                            } else {
                                complete.store(false, Ordering::Relaxed);
                                local_skipped += 1;
                                let mut r = reason.lock().unwrap_or_else(|p| p.into_inner());
                                if r.is_none() {
                                    *r = Some(format!(
                                        "Directory depth limit of {} exceeded at {}",
                                        max_depth,
                                        child_path.display()
                                    ));
                                }
                            }
                        }
                    }
                    Err(err) => {
                        complete.store(false, Ordering::Relaxed);
                        local_skipped += 1;
                        let mut r = reason.lock().unwrap_or_else(|p| p.into_inner());
                        if r.is_none() {
                            *r = Some(format!(
                                "Failed to read metadata for {}: {}",
                                child_path.display(),
                                err
                            ));
                        }
                    }
                }
            }

            logical.fetch_add(local_logical, Ordering::Relaxed);
            allocated.fetch_add(local_allocated, Ordering::Relaxed);
            file_count.fetch_add(local_file_count, Ordering::Relaxed);
            skipped.fetch_add(local_skipped, Ordering::Relaxed);
        });
    }

    fn is_excluded(
        child_path: &Path,
        exclusions: &[String],
        environment: &PlatformEnvironment,
    ) -> bool {
        let child_str = child_path.to_string_lossy();
        exclusions.iter().any(|exclusion| {
            if crate::signatures::SignatureLoader::expand_exclusion(exclusion, environment)
                .is_some_and(|expanded| child_path == expanded || child_path.starts_with(expanded))
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
    ) -> PathMeasurement {
        if current_depth > max_depth {
            return PathMeasurement::incomplete(
                FileSize::default(),
                0,
                format!(
                    "Directory depth limit of {} exceeded at {}",
                    max_depth,
                    dir.display()
                ),
            )
            .with_skipped_entries(1);
        }

        let mut total_logical = 0u64;
        let mut total_allocated = 0u64;
        let mut file_count = 0usize;
        let mut complete = true;
        let mut incomplete_reason: Option<String> = None;
        let mut skipped_entries = 0u64;

        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(err) => {
                return PathMeasurement::incomplete(
                    FileSize::default(),
                    0,
                    format!("Failed to read directory {}: {}", dir.display(), err),
                )
                .with_skipped_entries(1);
            }
        };

        for entry in entries {
            let ent = match entry {
                Ok(e) => e,
                Err(err) => {
                    complete = false;
                    skipped_entries += 1;
                    if incomplete_reason.is_none() {
                        incomplete_reason = Some(format!(
                            "Failed to read directory entry in {}: {}",
                            dir.display(),
                            err
                        ));
                    }
                    continue;
                }
            };
            let child_path = ent.path();

            if Self::is_excluded(&child_path, exclusions, environment) {
                skipped_entries += 1;
                continue;
            }

            // Check blacklist
            if Blacklist::is_blacklisted_with(&child_path, environment) {
                skipped_entries += 1;
                continue;
            }

            // Symlink check: DO NOT traverse into symlinked directories
            if SymlinkGuard::is_symlink(&child_path) {
                match fs::symlink_metadata(&child_path) {
                    Ok(m) => {
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
                    Err(err) => {
                        complete = false;
                        skipped_entries += 1;
                        if incomplete_reason.is_none() {
                            incomplete_reason = Some(format!(
                                "Failed to read symlink metadata for {}: {}",
                                child_path.display(),
                                err
                            ));
                        }
                    }
                }
                continue;
            }

            match fs::symlink_metadata(&child_path) {
                Ok(meta) => {
                    if meta.is_dir()
                        && child_path
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
                    {
                        complete = false;
                        skipped_entries += 1;
                        if incomplete_reason.is_none() {
                            incomplete_reason = Some(format!(
                                "Protected application bundle encountered in {}",
                                child_path.display()
                            ));
                        }
                        continue;
                    }

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
                        let sub = Self::measure_dir_recursive(
                            &child_path,
                            exclusions,
                            current_depth + 1,
                            max_depth,
                            environment,
                        );
                        total_logical += sub.size.logical;
                        total_allocated += sub.size.allocated.unwrap_or(sub.size.logical);
                        file_count += sub.file_count;
                        skipped_entries += sub.skipped_entries;
                        if !sub.complete {
                            complete = false;
                            if incomplete_reason.is_none() {
                                incomplete_reason = sub.incomplete_reason;
                            }
                        }
                    }
                }
                Err(err) => {
                    complete = false;
                    skipped_entries += 1;
                    if incomplete_reason.is_none() {
                        incomplete_reason = Some(format!(
                            "Failed to read metadata for {}: {}",
                            child_path.display(),
                            err
                        ));
                    }
                }
            }
        }

        PathMeasurement {
            size: FileSize::new(total_logical, Some(total_allocated)),
            file_count,
            complete,
            incomplete_reason,
            skipped_entries,
        }
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
        let sequential = SizeCalculator::measure_path_full(root.path(), &exclusions, &environment);
        let pool = ThreadPoolBuilder::new().num_threads(4).build().unwrap();
        let parallel = SizeCalculator::measure_path_with_pool(
            root.path(),
            &exclusions,
            Some(&pool),
            &environment,
        );

        assert_eq!(parallel, sequential);
        assert!(parallel.complete);
        let expected_files = 2 + usize::from(cfg!(unix));
        assert_eq!(
            parallel.file_count, expected_files,
            "two files plus (on unix) one untraversed symlink"
        );
        assert_eq!(
            parallel.skipped_entries, 2,
            "the named exclusion and the `.git` tree are deliberately not measured"
        );
    }

    /// The prune and model-inventory paths measure a cache around a mutation; a
    /// walk that could not read part of the tree has to report that boundary
    /// instead of returning a smaller number that looks complete.
    #[test]
    fn an_incomplete_provider_cache_measurement_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("_cacache");
        std::fs::create_dir_all(cache.join("Tool.app/Contents")).unwrap();
        std::fs::write(cache.join("content.bin"), vec![1u8; 8_192]).unwrap();
        std::fs::write(cache.join("Tool.app/Contents/payload"), vec![2u8; 4_096]).unwrap();

        let clean = root.path().join("clean-cache");
        std::fs::create_dir_all(clean.join(".git")).unwrap();
        std::fs::write(clean.join("content.bin"), vec![3u8; 1_024]).unwrap();

        let environment = PlatformEnvironment::simulated(PathFlavor::current());
        let bounded = SizeCalculator::measure_path_logged(&cache, &[], &environment);
        assert!(
            !bounded.complete,
            "the protected bundle is a boundary the walk cannot cross"
        );
        assert_eq!(bounded.skipped_entries, 1);
        assert!(bounded
            .incomplete_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("Protected application bundle")));
        assert_eq!(bounded.size.logical, 8_192);
        assert_eq!(bounded.file_count, 1);

        // A skipped `.git` tree is a deliberate exclusion, not a boundary: the
        // measurement is complete and says how many entries it left out.
        let complete = SizeCalculator::measure_path_logged(&clean, &[], &environment);
        assert!(complete.complete);
        assert_eq!(complete.skipped_entries, 1);
        assert_eq!(complete.size.logical, 1_024);
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
        let redirected =
            SizeCalculator::measure_path_full(&scanned, &exclusions, &redirected_environment);
        assert_eq!(
            redirected.file_count, 2,
            "the redirect moves the exclusion out of this tree, so both files are measured"
        );

        // Without a stated redirect the literal profile spelling is excluded.
        let literal_environment = simulated(false);
        let literal =
            SizeCalculator::measure_path_full(&scanned, &exclusions, &literal_environment);
        assert_eq!(
            literal.file_count, 1,
            "the literal profile spelling excludes `~/Downloads/keep`"
        );
    }
}
