use super::observation::{ScanLimits, TraversalCounters};
use crate::models::{CancellationProbe, FileSize, NeverCancelled};
use crate::safety::{Blacklist, SymlinkGuard};
use rayon::{Scope, ThreadPool};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use zenith_platform::PlatformEnvironment;

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
            // The requested root itself could not be measured. Callers may
            // replace this when they have a more precise subtree count.
            skipped_entries: 1,
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

/// The bytes a before/after measurement pair proves were reclaimed.
///
/// Both sides have to be complete. A measurement that skipped entries cannot be
/// subtracted from another one and reported as an exact amount: the difference
/// between two incomplete numbers looks precise and is not, so the caller is
/// told the amount is unknown instead.
pub fn reclaimed_between(before: &PathMeasurement, after: &PathMeasurement) -> Option<u64> {
    if !before.complete || !after.complete {
        return None;
    }
    Some(
        before
            .size
            .reclaimable()
            .saturating_sub(after.size.reclaimable()),
    )
}

fn measurement_for_metadata_error(path: &Path, error: &std::io::Error) -> PathMeasurement {
    if error.kind() == std::io::ErrorKind::NotFound {
        PathMeasurement::complete(FileSize::default(), 0)
    } else {
        PathMeasurement::unavailable(format!(
            "Could not read metadata for {}: {}",
            path.display(),
            error
        ))
    }
}

/// Why a measurement stopped early when the scan was cancelled.
///
/// The text is the same everywhere a cancelled walk can end, so a partial
/// result is attributable to cancellation rather than to a read failure.
fn cancelled_measurement_reason(dir: &Path) -> String {
    format!("Scan cancelled while measuring {}", dir.display())
}

/// One reason a walk did not account for an entry.
///
/// The parallel walk can hit several failures at once, so the reason a scan
/// reports is chosen by the walk's own order — shallowest first, then path,
/// then message — instead of by whichever worker happened to finish first. Two
/// scans of one tree therefore state the same reason, which is what makes a
/// repeat-scan comparison meaningful.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FailureRecord {
    depth: usize,
    path: String,
    message: String,
}

impl FailureRecord {
    fn new(depth: usize, path: &Path, message: String) -> Self {
        Self {
            depth,
            path: path.to_string_lossy().into_owned(),
            message,
        }
    }

    fn ranked(&self) -> (usize, &str, &str) {
        (self.depth, self.path.as_str(), self.message.as_str())
    }

    fn earlier_than(&self, other: &Self) -> bool {
        self.ranked() < other.ranked()
    }
}

/// The shared accumulators of one parallel measurement.
///
/// Bundled so the task body takes one reference per concern rather than eleven
/// positional arguments, and so a counter cannot be wired into the queued path
/// and forgotten in the inline one.
struct MeasurementState<'scope> {
    logical: &'scope AtomicU64,
    allocated: &'scope AtomicU64,
    file_count: &'scope AtomicUsize,
    complete: &'scope AtomicBool,
    skipped: &'scope AtomicU64,
    reason: &'scope Mutex<Option<FailureRecord>>,
    counters: &'scope TraversalCounters,
    in_flight: &'scope AtomicUsize,
}

impl MeasurementState<'_> {
    /// Records a failure, keeping the walk's own ordering.
    fn note_reason(&self, record: FailureRecord) {
        let mut slot = self
            .reason
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let replace = match slot.as_ref() {
            None => true,
            Some(current) => record.earlier_than(current),
        };
        if replace {
            *slot = Some(record);
        }
    }

    /// Records one entry the walk could not account for, and why.
    ///
    /// Failures are counted here rather than in the per-directory locals: a
    /// failure can end a directory before its local counters are folded, so the
    /// shared counter is the one place where every failure is counted exactly
    /// once. Deliberate skips (an exclusion, a blacklist match) are different:
    /// they are not failures, and they are counted locally by the directory
    /// that observed them.
    fn fail(&self, record: FailureRecord) {
        self.complete.store(false, Ordering::Relaxed);
        self.skipped.fetch_add(1, Ordering::Relaxed);
        self.note_reason(record);
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
        Self::measure_path_full_with_cancellation(path, exclusions, environment, &NeverCancelled)
    }

    /// [`Self::measure_path_full`] under stated bounds, with the caller's
    /// traversal counters.
    ///
    /// A caller that measures a path outside a scan passes
    /// `&TraversalCounters::default()`: the walk behaves identically, and the
    /// counts land somewhere nobody reads instead of in a scan's metrics.
    pub fn measure_path_bounded<P: AsRef<Path>>(
        path: P,
        exclusions: &[String],
        environment: &PlatformEnvironment,
        cancellation: &dyn CancellationProbe,
        limits: ScanLimits,
        counters: &TraversalCounters,
    ) -> PathMeasurement {
        Self::measure_path_with_pool(
            path,
            exclusions,
            None,
            environment,
            cancellation,
            limits,
            counters,
        )
    }

    /// [`Self::measure_path_full`] with the caller's cancellation contract.
    ///
    /// A scan that was cancelled mid-tree returns an incomplete measurement:
    /// the walk stops at the next directory boundary, the bytes already
    /// observed are kept, and the reason says why. Callers that are not part of
    /// a cancellable scan use [`Self::measure_path_full`].
    pub fn measure_path_full_with_cancellation<P: AsRef<Path>>(
        path: P,
        exclusions: &[String],
        environment: &PlatformEnvironment,
        cancellation: &dyn CancellationProbe,
    ) -> PathMeasurement {
        Self::measure_path_with_pool(
            path,
            exclusions,
            None,
            environment,
            cancellation,
            ScanLimits::default(),
            &TraversalCounters::default(),
        )
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn measure_path_with_pool<P: AsRef<Path>>(
        path: P,
        exclusions: &[String],
        pool: Option<&ThreadPool>,
        environment: &PlatformEnvironment,
        cancellation: &dyn CancellationProbe,
        limits: ScanLimits,
        counters: &TraversalCounters,
    ) -> PathMeasurement {
        let path = path.as_ref();
        counters.visit_entry();
        // Check if path is in blacklist
        if Blacklist::is_blacklisted_with(path, environment) {
            return PathMeasurement::incomplete(
                FileSize::default(),
                0,
                "Protected by system safety blacklist",
            )
            .with_skipped_entries(1);
        }

        let meta = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(err) => return measurement_for_metadata_error(path, &err),
        };

        // If path is a symlink, only measure the link itself. Reusing the
        // metadata above avoids a second TOCTOU window between classification
        // and accounting, and dangling links remain visible.
        if meta.file_type().is_symlink() {
            let logical = meta.len();
            return PathMeasurement::complete(FileSize::new(logical, Some(logical)), 1);
        }

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
                return Self::measure_dir_parallel(
                    path,
                    exclusions,
                    pool,
                    environment,
                    cancellation,
                    limits,
                    counters,
                );
            }
            return Self::measure_dir_recursive(
                path,
                exclusions,
                0,
                limits.max_depth,
                environment,
                cancellation,
                counters,
            );
        }

        PathMeasurement::complete(FileSize::default(), 0)
    }

    /// Measures a directory tree on the shared pool.
    ///
    /// The walk is bounded twice over: by `limits.max_depth` (how deep it may
    /// descend) and by `limits.max_concurrent_directory_reads` (how many
    /// directory tasks may be outstanding at once). Past the second bound the
    /// walk descends inline on the worker that is already running, so a tree of
    /// a million directories becomes work for four threads rather than a queue
    /// of a million tasks.
    #[allow(clippy::too_many_arguments)]
    fn measure_dir_parallel(
        path: &Path,
        exclusions: &[String],
        pool: &ThreadPool,
        environment: &PlatformEnvironment,
        cancellation: &dyn CancellationProbe,
        limits: ScanLimits,
        counters: &TraversalCounters,
    ) -> PathMeasurement {
        let logical = AtomicU64::new(0);
        let allocated = AtomicU64::new(0);
        let file_count = AtomicUsize::new(0);
        let complete = AtomicBool::new(true);
        let skipped = AtomicU64::new(0);
        let reason = Mutex::new(None);
        let in_flight = AtomicUsize::new(0);

        let state = MeasurementState {
            logical: &logical,
            allocated: &allocated,
            file_count: &file_count,
            complete: &complete,
            skipped: &skipped,
            reason: &reason,
            counters,
            in_flight: &in_flight,
        };

        pool.scope(|scope| {
            Self::dispatch_directory(
                scope,
                path.to_path_buf(),
                0,
                exclusions,
                limits,
                &state,
                environment,
                cancellation,
            );
        });

        let is_complete = complete.load(Ordering::Relaxed);
        let incomplete_reason = reason
            .into_inner()
            .unwrap_or_default()
            .map(|recorded: FailureRecord| recorded.message);
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

    /// Runs a directory measurement now, or queues it when the walk is below its
    /// stated number of outstanding tasks.
    ///
    /// The permit is released when the task finishes, so the bound counts work
    /// that has been accepted and not yet completed rather than work that was
    /// merely requested.
    #[allow(clippy::too_many_arguments)]
    fn dispatch_directory<'scope>(
        scope: &Scope<'scope>,
        dir: PathBuf,
        depth: usize,
        exclusions: &'scope [String],
        limits: ScanLimits,
        state: &'scope MeasurementState<'scope>,
        environment: &'scope PlatformEnvironment,
        cancellation: &'scope dyn CancellationProbe,
    ) {
        let outstanding = state.in_flight.fetch_add(1, Ordering::AcqRel) + 1;
        if outstanding <= limits.max_concurrent_directory_reads {
            // Only accepted work is counted: an inline descent is not work
            // waiting for a worker, so the peak states how much queued work the
            // walk held rather than how deep it recursed.
            state
                .counters
                .observe_outstanding_directory_tasks(outstanding as u64);
            scope.spawn(move |scope| {
                Self::measure_directory(
                    scope,
                    &dir,
                    depth,
                    exclusions,
                    limits,
                    state,
                    environment,
                    cancellation,
                );
                state.in_flight.fetch_sub(1, Ordering::AcqRel);
            });
            return;
        }
        // At the bound: descend on the worker that is already running instead of
        // growing the queue. The depth-first fallback is what keeps retained
        // work proportional to the bound rather than to the tree.
        state.in_flight.fetch_sub(1, Ordering::AcqRel);
        Self::measure_directory(
            scope,
            &dir,
            depth,
            exclusions,
            limits,
            state,
            environment,
            cancellation,
        );
    }

    /// Reads one directory and accounts for everything in it.
    #[allow(clippy::too_many_arguments)]
    fn measure_directory<'scope>(
        scope: &Scope<'scope>,
        dir: &Path,
        depth: usize,
        exclusions: &'scope [String],
        limits: ScanLimits,
        state: &'scope MeasurementState<'scope>,
        environment: &'scope PlatformEnvironment,
        cancellation: &'scope dyn CancellationProbe,
    ) {
        // A cancelled scan stops at the next directory boundary: the bytes
        // already observed are kept and reported as an incomplete measurement
        // rather than silently becoming a smaller total.
        if cancellation.is_cancelled() {
            state.fail(FailureRecord::new(
                depth,
                dir,
                cancelled_measurement_reason(dir),
            ));
            return;
        }
        if depth > limits.max_depth {
            state.fail(FailureRecord::new(
                depth,
                dir,
                format!(
                    "Directory depth limit of {} exceeded at {}",
                    limits.max_depth,
                    dir.display()
                ),
            ));
            return;
        }

        state.counters.directory_read();
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) => {
                state.fail(FailureRecord::new(
                    depth,
                    dir,
                    format!("Failed to read directory {}: {}", dir.display(), err),
                ));
                return;
            }
        };

        // Aggregate files locally and synchronize only once per directory.
        let mut local_logical = 0u64;
        let mut local_allocated = 0u64;
        let mut local_file_count = 0usize;
        let mut local_skipped = 0u64;
        let mut local_visits = 0u64;

        for entry in entries {
            if cancellation.is_cancelled() {
                state.fail(FailureRecord::new(
                    depth,
                    dir,
                    cancelled_measurement_reason(dir),
                ));
                break;
            }
            local_visits += 1;
            let ent = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    state.fail(FailureRecord::new(
                        depth,
                        dir,
                        format!(
                            "Failed to read directory entry in {}: {}",
                            dir.display(),
                            err
                        ),
                    ));
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

            // Never follow a link or a reparse point; account only for the link
            // itself, including a dangling one.
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
                        state.fail(FailureRecord::new(
                            depth,
                            &child_path,
                            format!(
                                "Failed to read symlink metadata for {}: {}",
                                child_path.display(),
                                err
                            ),
                        ));
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
                        state.fail(FailureRecord::new(
                            depth,
                            &child_path,
                            format!(
                                "Protected application bundle encountered in {}",
                                child_path.display()
                            ),
                        ));
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
                        if depth < limits.max_depth {
                            Self::dispatch_directory(
                                scope,
                                child_path,
                                depth + 1,
                                exclusions,
                                limits,
                                state,
                                environment,
                                cancellation,
                            );
                        } else {
                            state.fail(FailureRecord::new(
                                depth,
                                &child_path,
                                format!(
                                    "Directory depth limit of {} exceeded at {}",
                                    limits.max_depth,
                                    child_path.display()
                                ),
                            ));
                        }
                    }
                }
                Err(err) => {
                    state.fail(FailureRecord::new(
                        depth,
                        &child_path,
                        format!(
                            "Failed to read metadata for {}: {}",
                            child_path.display(),
                            err
                        ),
                    ));
                }
            }
        }

        state.counters.visit_entries(local_visits);
        state.logical.fetch_add(local_logical, Ordering::Relaxed);
        state
            .allocated
            .fetch_add(local_allocated, Ordering::Relaxed);
        state
            .file_count
            .fetch_add(local_file_count, Ordering::Relaxed);
        state.skipped.fetch_add(local_skipped, Ordering::Relaxed);
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
        cancellation: &dyn CancellationProbe,
        counters: &TraversalCounters,
    ) -> PathMeasurement {
        // The root is counted by measure_path_with_pool, and every recursive
        // child directory is counted by its parent's entry loop before descent.
        // Counting again here makes visited_entries depend on whether the
        // scheduler used the pooled or inline walk.
        if cancellation.is_cancelled() {
            return PathMeasurement::incomplete(
                FileSize::default(),
                0,
                cancelled_measurement_reason(dir),
            )
            .with_skipped_entries(1);
        }
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

        counters.directory_read();
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
            // The entry loop is the innermost shared point of both walks, so a
            // cancelled scan stops here rather than after the tree is finished.
            if cancellation.is_cancelled() {
                complete = false;
                skipped_entries += 1;
                if incomplete_reason.is_none() {
                    incomplete_reason = Some(cancelled_measurement_reason(dir));
                }
                break;
            }
            counters.visit_entry();
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
                            cancellation,
                            counters,
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
    use crate::models::NeverCancelled;

    use super::{measurement_for_metadata_error, SizeCalculator};
    use rayon::ThreadPoolBuilder;
    use zenith_platform::path_algebra::PathFlavor;
    use zenith_platform::PlatformEnvironment;

    /// A probe that cancels once it has been consulted `after` times.
    struct CancelAfter {
        calls: std::sync::atomic::AtomicUsize,
        after: usize,
    }

    impl crate::models::CancellationProbe for CancelAfter {
        fn is_cancelled(&self) -> bool {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= self.after
        }
    }

    #[test]
    fn a_cancelled_scan_stops_the_walk_and_reports_an_incomplete_measurement() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("cache");
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("first.bin"), vec![1u8; 100]).unwrap();
        std::fs::write(root.join("nested/second.bin"), vec![2u8; 200]).unwrap();
        let environment = PlatformEnvironment::simulated(PathFlavor::current());

        // The same tree measures completely when nothing cancels, which is what
        // makes the cancelled result below attributable to the probe.
        let complete = SizeCalculator::measure_path_full(&root, &[], &environment);
        assert!(complete.complete);
        assert_eq!(complete.size.logical, 300);

        // The walk stops at the next entry boundary and keeps what it observed.
        let cancelled = SizeCalculator::measure_path_full_with_cancellation(
            &root,
            &[],
            &environment,
            &CancelAfter {
                calls: std::sync::atomic::AtomicUsize::new(0),
                after: 1,
            },
        );
        assert!(!cancelled.complete, "a cancelled walk is never complete");
        assert_eq!(
            cancelled.skipped_entries, 1,
            "the interrupted entry is accounted for, not dropped"
        );
        let reason = cancelled
            .incomplete_reason
            .as_deref()
            .expect("a cancelled measurement explains itself");
        assert!(
            reason.contains("Scan cancelled"),
            "unexpected reason: {reason}"
        );
    }

    #[test]
    fn metadata_errors_are_not_mistaken_for_missing_zero_byte_paths() {
        let root = tempfile::tempdir().unwrap();
        let environment = PlatformEnvironment::simulated(PathFlavor::current());

        let denied = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let measurement = measurement_for_metadata_error(root.path(), &denied);

        assert!(!measurement.complete);
        assert_eq!(measurement.skipped_entries, 1);
        assert!(measurement.incomplete_reason.is_some());

        let missing =
            SizeCalculator::measure_path_full(root.path().join("missing"), &[], &environment);
        assert!(missing.complete);
        assert_eq!(missing.skipped_entries, 0);
    }

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
        let counters = crate::scanner::TraversalCounters::default();
        let parallel = SizeCalculator::measure_path_with_pool(
            root.path(),
            &exclusions,
            Some(&pool),
            &environment,
            &NeverCancelled,
            crate::scanner::ScanLimits::default(),
            &counters,
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

    /// Windows rules run on every runner: the stated flavor decides which
    /// entries are protected, so the same fixture reports a different boundary
    /// count for a Windows machine than for a POSIX one.
    #[test]
    fn windows_flavor_measurement_applies_the_stated_exclusions() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("_cacache");
        std::fs::create_dir_all(cache.join(".git")).unwrap();
        std::fs::write(cache.join(".git/objects"), vec![1u8; 1_024]).unwrap();
        // A reserved device name is a Windows boundary, but Windows itself
        // cannot create every one of them, so the fixture states it only where
        // the host can hold it and the expectation follows.
        let _ = std::fs::create_dir_all(cache.join("con"));
        // Windows reports success for a reserved name without creating it, so
        // the fixture is what the filesystem actually holds.
        let reserved_created = std::fs::symlink_metadata(cache.join("con")).is_ok();
        if reserved_created {
            std::fs::write(cache.join("con/payload"), vec![2u8; 2_048]).unwrap();
        }
        std::fs::create_dir_all(cache.join("keep")).unwrap();
        std::fs::write(cache.join("keep/payload"), vec![3u8; 512]).unwrap();
        std::fs::write(cache.join("content.bin"), vec![4u8; 4_096]).unwrap();

        let exclusions = vec!["keep".to_string()];
        let windows = PlatformEnvironment::simulated(PathFlavor::Windows);
        let measured = SizeCalculator::measure_path_full(&cache, &exclusions, &windows);

        assert_eq!(measured.file_count, 1, "only the readable file is counted");
        assert_eq!(measured.size.logical, 4_096);
        assert!(
            measured.complete,
            "a protected name is a deliberate boundary, not a read failure"
        );
        assert_eq!(
            measured.skipped_entries,
            2 + u64::from(reserved_created),
            "the exclusion, `.git`, and, where the host holds one, the reserved device name are not measured"
        );

        // The same tree on a POSIX machine has no reserved-device rule, so the
        // stated flavor is what decided the third boundary.
        let posix = PlatformEnvironment::simulated(PathFlavor::Posix);
        let measured = SizeCalculator::measure_path_full(&cache, &exclusions, &posix);
        assert_eq!(
            measured.skipped_entries, 2,
            "POSIX protects the exclusion and `.git` only"
        );
        assert_eq!(
            measured.size.logical,
            4_096 + if reserved_created { 2_048 } else { 0 }
        );
    }

    /// A partial measurement never becomes an exact reclaim amount: the pair
    /// reports `None` instead of a number the caller cannot defend.
    #[test]
    fn a_partial_measurement_never_becomes_an_exact_reclaim_amount() {
        use super::reclaimed_between;

        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("_cacache");
        std::fs::create_dir_all(cache.join("Tool.app/Contents")).unwrap();
        std::fs::write(cache.join("content.bin"), vec![1u8; 4_096]).unwrap();
        std::fs::write(cache.join("Tool.app/Contents/payload"), vec![2u8; 2_048]).unwrap();

        let clean = root.path().join("clean");
        std::fs::create_dir(&clean).unwrap();
        std::fs::write(clean.join("content.bin"), vec![3u8; 1_024]).unwrap();

        let environment = PlatformEnvironment::simulated(PathFlavor::current());
        let bounded = SizeCalculator::measure_path_full(&cache, &[], &environment);
        let complete = SizeCalculator::measure_path_full(&clean, &[], &environment);
        assert!(!bounded.complete);
        assert!(complete.complete);

        assert_eq!(reclaimed_between(&bounded, &complete), None);
        assert_eq!(reclaimed_between(&complete, &bounded), None);

        // Two complete measurements do produce the difference the caller
        // reports, on the same accounting the sizes use.
        let wiped = SizeCalculator::measure_path_full(&clean, &[], &environment);
        std::fs::remove_file(clean.join("content.bin")).unwrap();
        let emptied = SizeCalculator::measure_path_full(&clean, &[], &environment);
        assert!(emptied.complete);
        assert_eq!(
            reclaimed_between(&wiped, &emptied),
            Some(wiped.size.reclaimable())
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
                    zenith_platform::paths::SimulatedPaths::new()
                        .with_flavor(PathFlavor::current())
                        .with_home(&home),
                ),
            );
            if redirect {
                environment.with_known_folder(zenith_platform::KnownFolder::Downloads, &redirected)
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

    /// A wide tree is measured within the traversal's stated bound, and the
    /// bound changes only how the work is scheduled, never what it counts.
    #[test]
    fn a_wide_tree_stays_within_the_stated_directory_task_bound() {
        const DIRECTORIES: usize = 48;
        const FILES_PER_DIRECTORY: usize = 3;

        let root = tempfile::tempdir().unwrap();
        for directory in 0..DIRECTORIES {
            let path = root.path().join(format!("cache-{directory:03}"));
            std::fs::create_dir(&path).unwrap();
            for file in 0..FILES_PER_DIRECTORY {
                std::fs::write(path.join(format!("entry-{file}.bin")), vec![1u8; 1_024]).unwrap();
            }
        }
        let environment = PlatformEnvironment::simulated(PathFlavor::current());
        let pool = ThreadPoolBuilder::new().num_threads(4).build().unwrap();

        // One outstanding directory task is the smallest bound that still makes
        // progress, so the peak it reports is exact rather than a race.
        let single = crate::scanner::TraversalCounters::default();
        let serial = SizeCalculator::measure_path_with_pool(
            root.path(),
            &[],
            Some(&pool),
            &environment,
            &NeverCancelled,
            crate::scanner::ScanLimits::bounded(8, 1),
            &single,
        );
        assert!(serial.complete);
        assert_eq!(single.peak_outstanding_directory_tasks(), 1);
        assert_eq!(
            single.directories_read(),
            (DIRECTORIES + 1) as u64,
            "the root and every child directory are read exactly once"
        );
        assert_eq!(
            single.visited_entries(),
            (DIRECTORIES + DIRECTORIES * FILES_PER_DIRECTORY + 1) as u64,
            "each directory entry is visited once, plus the root itself"
        );

        // A wider bound may fan out, and it never exceeds the number it states.
        let fanned = crate::scanner::TraversalCounters::default();
        let parallel = SizeCalculator::measure_path_with_pool(
            root.path(),
            &[],
            Some(&pool),
            &environment,
            &NeverCancelled,
            crate::scanner::ScanLimits::bounded(8, 4),
            &fanned,
        );
        assert!(
            fanned.peak_outstanding_directory_tasks() <= 4,
            "the walk never keeps more directory tasks outstanding than it states: {}",
            fanned.peak_outstanding_directory_tasks()
        );

        assert_eq!(serial.size, parallel.size);
        assert_eq!(serial.file_count, parallel.file_count);
        assert_eq!(serial.skipped_entries, parallel.skipped_entries);
        assert_eq!(serial.complete, parallel.complete);

        // A host without a shared pool must report the same traversal facts.
        // Scheduling may change; the meaning of visited_entries may not.
        let inline_counters = crate::scanner::TraversalCounters::default();
        let inline = SizeCalculator::measure_path_with_pool(
            root.path(),
            &[],
            None,
            &environment,
            &NeverCancelled,
            crate::scanner::ScanLimits::bounded(8, 4),
            &inline_counters,
        );
        assert_eq!(inline, serial);
        assert_eq!(
            inline_counters.visited_entries(),
            single.visited_entries(),
            "visited entries are filesystem facts, not a consequence of whether a pool exists"
        );
        assert_eq!(
            inline_counters.directories_read(),
            single.directories_read(),
            "directory reads are identical whichever scheduler executes the walk"
        );
    }

    /// A tree deeper than the stated limit reports what it did not take rather
    /// than quietly measuring less, and it reports the same reason whichever
    /// walk runs it.
    #[test]
    fn a_tree_deeper_than_the_limit_is_reported_not_silently_skipped() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("shallow.bin"), vec![1u8; 1_024]).unwrap();
        let mut nested = root.path().to_path_buf();
        for level in 0..5 {
            nested.push(format!("level-{level}"));
        }
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("deep.bin"), vec![2u8; 2_048]).unwrap();

        let environment = PlatformEnvironment::simulated(PathFlavor::current());
        for pool in [
            None,
            Some(ThreadPoolBuilder::new().num_threads(4).build().unwrap()),
        ] {
            let counters = crate::scanner::TraversalCounters::default();
            let measurement = SizeCalculator::measure_path_with_pool(
                root.path(),
                &[],
                pool.as_ref(),
                &environment,
                &NeverCancelled,
                crate::scanner::ScanLimits::bounded(2, 4),
                &counters,
            );
            assert!(!measurement.complete);
            assert_eq!(
                measurement.file_count, 1,
                "only the file inside the stated depth is measured"
            );
            assert!(measurement.skipped_entries >= 1);
            assert!(
                measurement
                    .incomplete_reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("depth limit")),
                "the reason names the limit: {:?}",
                measurement.incomplete_reason
            );
        }
    }
}
