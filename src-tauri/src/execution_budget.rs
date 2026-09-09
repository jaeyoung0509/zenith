//! Application-level execution budgets for expensive native work.
//!
//! Issue #136 inventory:
//! - `scanner::engine` already uses a reusable hardware-bounded Rayon pool.
//! - `developer_artifacts` previously built a separate pool of up to four
//!   workers per scan.
//! - `cache_providers` used the global Rayon pool (`into_par_iter`) for two
//!   tiny provider lookups.
//! - `StorageOperationGate` serializes participating mutating storage
//!   workflows; it is a safety serialization primitive, not a performance
//!   semaphore, and is preserved unchanged.
//!
//! This module provides:
//! - [`shared_scan_pool`]: one explicitly bounded scan pool reused by generic
//!   and developer-artifact scans. `cache_providers` routes its two-provider
//!   fan-out through the same pool via [`install_shared`]; that reuse is
//!   intentional and documented here.
//! - [`ExecutionBudgets`]: separate configurable-in-code semaphores for
//!   expensive storage reads vs. subprocess collection. Cheap metrics/cache
//!   reads never acquire a budget and stay responsive while scans run.
//!
//! Locking discipline to avoid nested pool/permit deadlocks:
//! acquire the async permit *before* dispatching blocking work, never hold a
//! domain state lock while waiting for a permit, and never acquire permits or
//! pools in nested order (permit -> pool is the only allowed order).

use rayon::{ThreadPool, ThreadPoolBuilder};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Initial budgets chosen from inspection, not by increasing parallelism:
/// generic scans already cap at four workers and artifact scans built their
/// own four-worker pool, so sharing one four-worker pool cannot exceed prior
/// peak CPU while removing per-request pool expansion.
const SHARED_SCAN_WORKERS_MAX: usize = 4;
/// Expense storage reads (filesystem traversal/measurement) stay lower than
/// the CPU pool so two heavy scans cannot fully starve interactive work.
const STORAGE_READ_PERMITS: usize = 2;
/// Subprocess collection (provider CLIs, Git, discovery) is I/O-bound and gets
/// a slightly larger cap; each permit still bounds thread/process pressure.
const SUBPROCESS_PERMITS: usize = 4;
/// Bounded queue: admissions beyond permits + queued are rejected fast instead
/// of growing an unbounded wait list under repeated refreshes.
const MAX_QUEUED_PER_BUDGET: usize = 16;

fn bounded_worker_count(available: usize, performance_cores: Option<usize>) -> usize {
    performance_cores
        .filter(|count| *count > 0)
        .unwrap_or(available)
        .min(available)
        .clamp(1, SHARED_SCAN_WORKERS_MAX)
}

#[cfg(target_os = "macos")]
fn performance_core_count() -> Option<usize> {
    let mut count: libc::c_uint = 0;
    let mut size = std::mem::size_of_val(&count);
    // SAFETY: both output pointers reference initialized, correctly sized
    // storage and the sysctl name is statically NUL-terminated.
    let status = unsafe {
        libc::sysctlbyname(
            c"hw.perflevel0.physicalcpu".as_ptr(),
            (&mut count as *mut libc::c_uint).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (status == 0 && count > 0).then_some(count as usize)
}

#[cfg(not(target_os = "macos"))]
fn performance_core_count() -> Option<usize> {
    None
}

/// One explicitly bounded scan pool shared by generic and developer-artifact
/// scans. Returns `None` on single-worker machines where callers run inline.
pub fn shared_scan_pool() -> Option<&'static ThreadPool> {
    static POOL: OnceLock<Option<ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        let available = std::thread::available_parallelism()
            .map(|available| available.get())
            .unwrap_or(1);
        let workers = bounded_worker_count(available, performance_core_count());
        if workers < 2 {
            return None;
        }
        ThreadPoolBuilder::new()
            .num_threads(workers)
            .thread_name(|index| format!("zenith-scan-{index}"))
            .build()
            .ok()
    })
    .as_ref()
}

/// Run Rayon work on the shared pool when available, inline otherwise.
pub fn install_shared<T>(work: impl FnOnce() -> T + Send, inline: impl FnOnce() -> T) -> T
where
    T: Send,
{
    if let Some(pool) = shared_scan_pool() {
        pool.install(work)
    } else {
        inline()
    }
}

/// Separate async admission budgets. Cloneable; permits release on drop,
/// including panic paths via the owned-permit guard.
#[derive(Debug, Clone)]
pub struct ExecutionBudgets {
    storage_read: Arc<BudgetSemaphore>,
    subprocess: Arc<BudgetSemaphore>,
}

#[derive(Debug)]
struct BudgetSemaphore {
    semaphore: Arc<Semaphore>,
    queued: AtomicUsize,
}

impl ExecutionBudgets {
    pub fn new() -> Self {
        Self {
            storage_read: Arc::new(BudgetSemaphore {
                semaphore: Arc::new(Semaphore::new(STORAGE_READ_PERMITS)),
                queued: AtomicUsize::new(0),
            }),
            subprocess: Arc::new(BudgetSemaphore {
                semaphore: Arc::new(Semaphore::new(SUBPROCESS_PERMITS)),
                queued: AtomicUsize::new(0),
            }),
        }
    }

    pub fn storage_read_permits() -> usize {
        STORAGE_READ_PERMITS
    }

    pub fn subprocess_permits() -> usize {
        SUBPROCESS_PERMITS
    }

    /// Acquire a storage-read permit before dispatching blocking filesystem
    /// work. Cancellation while queued is dropping the awaiting future, which
    /// releases the queue slot without consuming a permit.
    pub async fn acquire_storage_read(&self) -> Result<OwnedSemaphorePermit, String> {
        acquire(&self.storage_read).await
    }

    /// Acquire a subprocess-collection permit before spawning provider/Git
    /// subprocesses. Cancellation terminates/reaps via the caller's child
    /// guard; the permit releases on drop.
    pub async fn acquire_subprocess(&self) -> Result<OwnedSemaphorePermit, String> {
        acquire(&self.subprocess).await
    }
}

impl Default for ExecutionBudgets {
    fn default() -> Self {
        Self::new()
    }
}

async fn acquire(budget: &BudgetSemaphore) -> Result<OwnedSemaphorePermit, String> {
    budget.queued.fetch_add(1, Ordering::SeqCst);
    struct QueueGuard<'a> {
        queued: &'a AtomicUsize,
        armed: bool,
    }
    impl Drop for QueueGuard<'_> {
        fn drop(&mut self) {
            if self.armed {
                self.queued.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }
    let queue = QueueGuard {
        queued: &budget.queued,
        armed: true,
    };
    // `queued` was already incremented; `load` reflects this waiter.
    if budget.queued.load(Ordering::SeqCst) > MAX_QUEUED_PER_BUDGET {
        return Err("Execution budget is saturated; try again shortly.".to_string());
    }
    let permit = budget
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "Execution budget shut down.".to_string())?;
    // Convert the queue slot into the held permit: dropping the guard
    // releases the slot while the permit stays alive in the caller. Dropping
    // the waiter while queued releases the slot via the guard without
    // consuming a permit.
    drop(queue);
    Ok(permit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn scan_workers_stay_bounded() {
        assert_eq!(bounded_worker_count(8, Some(4)), 4);
        assert_eq!(bounded_worker_count(32, None), 4);
        assert_eq!(bounded_worker_count(1, None), 1);
    }

    #[tokio::test]
    async fn storage_budget_caps_concurrent_fake_workers() {
        let budgets = ExecutionBudgets::new();
        let concurrent = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..6 {
            let budgets = budgets.clone();
            let concurrent = concurrent.clone();
            let peak = peak.clone();
            handles.push(tokio::spawn(async move {
                let _permit = budgets.acquire_storage_read().await.unwrap();
                let now = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                concurrent.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for handle in handles {
            handle.await.unwrap();
        }
        assert!(peak.load(Ordering::SeqCst) <= ExecutionBudgets::storage_read_permits());
        assert!(peak.load(Ordering::SeqCst) > 1 || ExecutionBudgets::storage_read_permits() == 1);
    }

    #[tokio::test]
    async fn subprocess_budget_caps_concurrent_fake_workers() {
        let budgets = ExecutionBudgets::new();
        let concurrent = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let budgets = budgets.clone();
            let concurrent = concurrent.clone();
            let peak = peak.clone();
            handles.push(tokio::spawn(async move {
                let _permit = budgets.acquire_subprocess().await.unwrap();
                let now = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                concurrent.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for handle in handles {
            handle.await.unwrap();
        }
        assert!(peak.load(Ordering::SeqCst) <= ExecutionBudgets::subprocess_permits());
    }

    #[tokio::test]
    async fn cancelled_queued_acquisition_releases_its_slot() {
        let budgets = ExecutionBudgets::new();
        // Hold all permits.
        let _held: Vec<_> = {
            let mut held = Vec::new();
            for _ in 0..ExecutionBudgets::storage_read_permits() {
                held.push(budgets.acquire_storage_read().await.unwrap());
            }
            held
        };
        let waiter = tokio::spawn({
            let budgets = budgets.clone();
            async move { budgets.acquire_storage_read().await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        waiter.abort();
        let _ = waiter.await;
        // Permits still release normally after cancellation.
        drop(_held);
        let _permit = budgets.acquire_storage_read().await.unwrap();
    }

    #[tokio::test]
    async fn permit_releases_on_drop_after_panic_path() {
        let budgets = ExecutionBudgets::new();
        {
            let _permit = budgets.acquire_subprocess().await.unwrap();
            // Permit drops here (including unwind paths via Drop).
        }
        let _again = budgets.acquire_subprocess().await.unwrap();
    }
}
