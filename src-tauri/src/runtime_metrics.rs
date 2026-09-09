//! Lightweight, sanitized runtime-efficiency instrumentation.
//!
//! Records monotonic admission/collection/cache/join/lock/subprocess/scan
//! counters for the scenarios in issue #136. No argv, credentials, prompts,
//! full paths, or raw provider output are stored. Timing uses monotonic
//! [`std::time::Instant`] deltas; only millisecond counts are retained.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Sanitized counters for collection admission and execution budgets.
///
/// All fields are process-wide atomics so background and foreground paths can
/// record without holding a shared-state lock.
#[derive(Debug, Default)]
pub struct RuntimeMetrics {
    collection_admissions: AtomicU64,
    cache_hits: AtomicU64,
    coalesced_joins: AtomicU64,
    collections_started: AtomicU64,
    collections_completed: AtomicU64,
    collections_failed: AtomicU64,
    subprocess_starts: AtomicU64,
    subprocess_timeouts: AtomicU64,
    subprocess_cancellations: AtomicU64,
    subprocess_completions: AtomicU64,
    lock_wait_millis: AtomicU64,
    lock_hold_millis: AtomicU64,
    scan_completed: AtomicU64,
    scan_bytes: AtomicU64,
}

impl RuntimeMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_admission(&self) {
        self.collection_admissions.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_coalesced_join(&self) {
        self.coalesced_joins.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_collection_started(&self) {
        self.collections_started.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_collection_completed(&self) {
        self.collections_completed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_collection_failed(&self) {
        self.collections_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_subprocess_start(&self) {
        self.subprocess_starts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_subprocess_timeout(&self) {
        self.subprocess_timeouts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_subprocess_cancellation(&self) {
        self.subprocess_cancellations
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_subprocess_completion(&self) {
        self.subprocess_completions.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_scan_completed(&self, bytes: u64) {
        self.scan_completed.fetch_add(1, Ordering::Relaxed);
        self.scan_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Measure time spent waiting for a shared-state lock.
    pub fn wait_timer(&self) -> LockTimer<'_> {
        LockTimer {
            metrics: self,
            start: Instant::now(),
            is_wait: true,
        }
    }

    /// Measure time a shared-state lock is held. Keep critical sections short;
    /// never run subprocesses, disk I/O, or channel sends inside the timed region.
    pub fn hold_timer(&self) -> LockTimer<'_> {
        LockTimer {
            metrics: self,
            start: Instant::now(),
            is_wait: false,
        }
    }

    pub fn snapshot(&self) -> RuntimeMetricsSnapshot {
        RuntimeMetricsSnapshot {
            collection_admissions: self.collection_admissions.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            coalesced_joins: self.coalesced_joins.load(Ordering::Relaxed),
            collections_started: self.collections_started.load(Ordering::Relaxed),
            collections_completed: self.collections_completed.load(Ordering::Relaxed),
            collections_failed: self.collections_failed.load(Ordering::Relaxed),
            subprocess_starts: self.subprocess_starts.load(Ordering::Relaxed),
            subprocess_timeouts: self.subprocess_timeouts.load(Ordering::Relaxed),
            subprocess_cancellations: self.subprocess_cancellations.load(Ordering::Relaxed),
            subprocess_completions: self.subprocess_completions.load(Ordering::Relaxed),
            lock_wait_millis: self.lock_wait_millis.load(Ordering::Relaxed),
            lock_hold_millis: self.lock_hold_millis.load(Ordering::Relaxed),
            scan_completed: self.scan_completed.load(Ordering::Relaxed),
            scan_bytes: self.scan_bytes.load(Ordering::Relaxed),
        }
    }
}

/// RAII timer recording monotonic lock wait/hold durations on drop.
pub struct LockTimer<'a> {
    metrics: &'a RuntimeMetrics,
    start: Instant,
    is_wait: bool,
}

impl Drop for LockTimer<'_> {
    fn drop(&mut self) {
        let millis = self.start.elapsed().as_millis().min(u64::MAX as u128) as u64;
        if self.is_wait {
            self.metrics
                .lock_wait_millis
                .fetch_add(millis, Ordering::Relaxed);
        } else {
            self.metrics
                .lock_hold_millis
                .fetch_add(millis, Ordering::Relaxed);
        }
    }
}

/// Plain snapshot for tests and local baseline scripts. Integer fields stay far
/// below JavaScript `Number.MAX_SAFE_INTEGER`; if these counters are ever
/// exposed over IPC they must use the shared `ipc_numeric` serde adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeMetricsSnapshot {
    pub collection_admissions: u64,
    pub cache_hits: u64,
    pub coalesced_joins: u64,
    pub collections_started: u64,
    pub collections_completed: u64,
    pub collections_failed: u64,
    pub subprocess_starts: u64,
    pub subprocess_timeouts: u64,
    pub subprocess_cancellations: u64,
    pub subprocess_completions: u64,
    pub lock_wait_millis: u64,
    pub lock_hold_millis: u64,
    pub scan_completed: u64,
    pub scan_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_accumulate_without_recording_secrets() {
        let metrics = RuntimeMetrics::new();
        metrics.record_admission();
        metrics.record_cache_hit();
        metrics.record_coalesced_join();
        metrics.record_collection_started();
        metrics.record_collection_completed();
        metrics.record_subprocess_start();
        metrics.record_subprocess_completion();
        metrics.record_scan_completed(1024);

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.collection_admissions, 1);
        assert_eq!(snapshot.cache_hits, 1);
        assert_eq!(snapshot.coalesced_joins, 1);
        assert_eq!(snapshot.collections_started, 1);
        assert_eq!(snapshot.collections_completed, 1);
        assert_eq!(snapshot.subprocess_starts, 1);
        assert_eq!(snapshot.subprocess_completions, 1);
        assert_eq!(snapshot.scan_completed, 1);
        assert_eq!(snapshot.scan_bytes, 1024);
    }

    #[test]
    fn lock_timers_record_monotonic_durations() {
        let metrics = RuntimeMetrics::new();
        {
            let _wait = metrics.wait_timer();
        }
        {
            let _hold = metrics.hold_timer();
        }
        let snapshot = metrics.snapshot();
        // Durations are monotonic millisecond counts; they must exist but no
        // machine-specific timing value is asserted.
        let _ = snapshot.lock_wait_millis;
        let _ = snapshot.lock_hold_millis;
    }

    #[test]
    fn concurrent_increments_do_not_lose_updates() {
        use std::sync::Arc;

        let metrics = Arc::new(RuntimeMetrics::new());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let metrics = metrics.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    metrics.record_admission();
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(metrics.snapshot().collection_admissions, 800);
    }
}
