//! What one scan states about its own traversal: its bounds, its progress, and
//! what it observed while walking.
//!
//! Three things belong together here because they answer the same question —
//! *what is this traversal allowed to do, and what did it actually do* — and
//! all three have to travel down the same call chain:
//!
//! * [`ScanLimits`] states the depth and the number of directory reads a scan
//!   may keep in flight. The numbers live in one place instead of at each call
//!   site, so a limit is a decision rather than a literal.
//! * [`RootProgressSink`] reports which root is being read. It is a side
//!   channel: results never depend on whether anything is listening, so a walk
//!   with no sink behaves exactly as one with a sink.
//! * [`TraversalCounters`] records what the walk visited, read, and how many
//!   reads were in flight at once. The peak is a measurement of the bound being
//!   honoured, not a claim about it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::models::{CancellationProbe, ScanItem, Signature};
use zenith_platform::PlatformEnvironment;

/// The bounds one scan runs under.
///
/// The values are the scan's policy, not the machine's capacity: the pool that
/// executes the work is bounded separately (`execution_budget`), and these say
/// how deep the walk may go and how many directory reads it may keep
/// outstanding. A fixture that wants a smaller limit states one; production
/// uses [`ScanLimits::default`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanLimits {
    /// How many directory levels below a root the walk may descend. A deeper
    /// entry is not silently skipped: it is counted and reported as the reason
    /// the measurement is incomplete.
    pub max_depth: usize,
    /// How many directory reads one walk may keep in flight at once.
    ///
    /// A walk that reaches this bound descends inline instead of queueing
    /// another task, so the traversal's outstanding work is bounded by the
    /// limit rather than by the size of the tree.
    pub max_concurrent_directory_reads: usize,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            // Deep enough for any real cache tree (a package manager's store
            // nests well under twenty levels), shallow enough that a symlink
            // loop or a pathological tree cannot walk forever.
            max_depth: 32,
            // The shared scan pool runs at most four workers; allowing each of
            // them a few outstanding reads keeps the pool fed without letting
            // an enormous tree turn into an enormous task queue.
            max_concurrent_directory_reads: 16,
        }
    }
}

impl ScanLimits {
    /// The limits a fixture states for a small, deterministic walk.
    pub fn bounded(max_depth: usize, max_concurrent_directory_reads: usize) -> Self {
        Self {
            max_depth,
            max_concurrent_directory_reads: max_concurrent_directory_reads.max(1),
        }
    }
}

/// Where a walk reports the root it is about to read.
///
/// The sink is written from the scan's own thread — progress is reported in the
/// sequential root loop, never from a pool worker — so it deliberately carries
/// no `Send + Sync` bound: the scan's event channel is a single ordered writer,
/// and pretending otherwise would let two workers interleave the stream.
pub trait RootProgressSink {
    /// One root of one signature is about to be read.
    fn root_started(&self, signature: &Signature, root: &Path);
}

/// A sink that reports nothing: the walk still works, it just says less.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoRootProgress;

impl RootProgressSink for NoRootProgress {
    fn root_started(&self, _signature: &Signature, _root: &Path) {}
}

/// What a traversal counted while it ran.
///
/// Atomics because the walk is parallel; the peak is updated with a
/// compare-exchange loop rather than a load-then-store, so two readers that
/// overlap cannot lose a high-water mark.
#[derive(Debug, Default)]
pub struct TraversalCounters {
    visited_entries: AtomicU64,
    directories_read: AtomicU64,
    peak_outstanding_directory_tasks: AtomicU64,
}

impl TraversalCounters {
    pub const fn new() -> Self {
        Self {
            visited_entries: AtomicU64::new(0),
            directories_read: AtomicU64::new(0),
            peak_outstanding_directory_tasks: AtomicU64::new(0),
        }
    }

    /// Records that an entry was looked at: a file measured, a directory read,
    /// or an entry refused without being read.
    pub fn visit_entry(&self) {
        self.visited_entries.fetch_add(1, Ordering::Relaxed);
    }

    /// Records that `count` entries were looked at in one batch.
    pub fn visit_entries(&self, count: u64) {
        if count > 0 {
            self.visited_entries.fetch_add(count, Ordering::Relaxed);
        }
    }

    /// Records that a directory's contents are being read.
    pub fn directory_read(&self) {
        self.directories_read.fetch_add(1, Ordering::Relaxed);
    }

    /// Records how many directory tasks are outstanding, keeping the highest.
    pub fn observe_outstanding_directory_tasks(&self, outstanding: u64) {
        let mut peak = self
            .peak_outstanding_directory_tasks
            .load(Ordering::Relaxed);
        while outstanding > peak {
            match self.peak_outstanding_directory_tasks.compare_exchange_weak(
                peak,
                outstanding,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(observed) => peak = observed,
            }
        }
    }

    pub fn visited_entries(&self) -> u64 {
        self.visited_entries.load(Ordering::Relaxed)
    }

    pub fn directories_read(&self) -> u64 {
        self.directories_read.load(Ordering::Relaxed)
    }

    pub fn peak_outstanding_directory_tasks(&self) -> u64 {
        self.peak_outstanding_directory_tasks
            .load(Ordering::Relaxed)
    }

    /// Folds another counter set into this one, for a walk that ran its own.
    pub fn merge(&self, other: &TraversalCounters) {
        self.visit_entries(other.visited_entries());
        let directories = other.directories_read();
        if directories > 0 {
            self.directories_read
                .fetch_add(directories, Ordering::Relaxed);
        }
        self.observe_outstanding_directory_tasks(other.peak_outstanding_directory_tasks());
    }
}

/// The scan's own runtime context, threaded through one signature walk.
///
/// One value instead of four parameters: what the walk may do ([`ScanLimits`]),
/// what it reports while doing it ([`RootProgressSink`]), what it counted
/// ([`TraversalCounters`]), and the two facts every walker entry point already
/// needed — the stated environment and the caller's cancellation contract.
/// Bundling them is what keeps a new bound from reaching one walk path and
/// being forgotten in another.
pub struct WalkContext<'a> {
    pub environment: &'a PlatformEnvironment,
    pub cancellation: &'a dyn CancellationProbe,
    pub limits: ScanLimits,
    pub counters: &'a TraversalCounters,
    pub progress: &'a dyn RootProgressSink,
}

impl<'a> WalkContext<'a> {
    pub fn new(
        environment: &'a PlatformEnvironment,
        cancellation: &'a dyn CancellationProbe,
        limits: ScanLimits,
        counters: &'a TraversalCounters,
        progress: &'a dyn RootProgressSink,
    ) -> Self {
        Self {
            environment,
            cancellation,
            limits,
            counters,
            progress,
        }
    }
}

/// What walking one signature produced: the items it found, and what the walk
/// observed about itself while finding them.
#[derive(Debug, Default)]
pub struct SignatureScan {
    pub items: Vec<ScanItem>,
    /// Roots the walk reported before reading them, in visit order.
    pub roots: Vec<PathBuf>,
    /// Number of selector patterns whose bounded expansion had more matches
    /// than the scanner could inspect.
    pub selector_truncated_count: u64,
}

impl SignatureScan {
    pub fn new(items: Vec<ScanItem>) -> Self {
        Self {
            items,
            roots: Vec::new(),
            selector_truncated_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ScanLimits, TraversalCounters};

    /// The peak keeps the highest value under concurrent updates: a
    /// load-then-store would lose a high-water mark whenever two readers
    /// overlapped.
    #[test]
    fn the_peak_survives_concurrent_updates() {
        let counters = TraversalCounters::new();
        std::thread::scope(|scope| {
            for worker in 1..=8u64 {
                let counters = &counters;
                scope.spawn(move || {
                    for step in 1..=64u64 {
                        counters.observe_outstanding_directory_tasks(worker * step);
                    }
                });
            }
        });
        assert_eq!(
            counters.peak_outstanding_directory_tasks(),
            8 * 64,
            "the highest value any thread observed is the one that is kept"
        );
    }

    /// Visits accumulate, and merging folds an inner walk's counts into the
    /// outer one without ever lowering the peak.
    #[test]
    fn counters_merge_without_losing_the_peak() {
        let outer = TraversalCounters::new();
        outer.visit_entries(3);
        outer.directory_read();
        outer.observe_outstanding_directory_tasks(2);

        let inner = TraversalCounters::new();
        inner.visit_entries(4);
        inner.directory_read();
        inner.observe_outstanding_directory_tasks(5);

        outer.merge(&inner);
        assert_eq!(outer.visited_entries(), 7);
        assert_eq!(outer.directories_read(), 2);
        assert_eq!(outer.peak_outstanding_directory_tasks(), 5);
    }

    /// The limits are stated once and are meaningful: a walk always allows at
    /// least one directory read, whatever a caller asks for.
    #[test]
    fn limits_state_a_usable_bound() {
        let default = ScanLimits::default();
        assert_eq!(default.max_depth, 32);
        assert!(default.max_concurrent_directory_reads >= 4);

        let cramped = ScanLimits::bounded(1, 0);
        assert_eq!(cramped.max_depth, 1);
        assert_eq!(
            cramped.max_concurrent_directory_reads, 1,
            "a limit of zero would deadlock the walk, so it is raised to one"
        );
    }
}
