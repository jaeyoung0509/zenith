//! Cancellation for the scans a user can stop.
//!
//! A scan registers the signal it is watching under the id it reports, so a
//! caller can stop it by that id while it runs. The registry exists because the
//! id only becomes known when the scan starts: the interface cannot cancel a
//! scan it has not been told about.
//!
//! Two properties make this a boundary rather than a map:
//!
//! * **bounded life** — an entry expires after a stated window and the registry
//!   evicts the oldest one at its cap, so a scan that never reports back cannot
//!   leak a handle;
//! * **one signal per id** — registering an id again replaces the signal, which
//!   is what a retried scan needs, and the probe a scan holds reads the same
//!   flag the registry stored, so a cancel is never lost to a lookup failure.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::models::CancellationProbe;

/// How long a registered scan may be cancelled after it starts.
pub const DEFAULT_CANCELLATION_TTL_SECS: u64 = 15 * 60;
/// How many scans one workflow may have in flight before the oldest is dropped.
pub const DEFAULT_MAX_ACTIVE_CANCELLATIONS: usize = 64;

#[derive(Debug, Clone)]
struct CancellationEntry {
    signal: Arc<AtomicBool>,
    created_at: u64,
}

/// The cancellation handles of one workflow.
pub struct CancellationRegistry {
    entries: Mutex<HashMap<String, CancellationEntry>>,
    ttl_secs: u64,
    capacity: usize,
}

impl CancellationRegistry {
    pub fn new(ttl_secs: u64, capacity: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl_secs,
            capacity: capacity.max(1),
        }
    }

    /// The registry a user-facing scan uses.
    pub fn for_scans() -> Self {
        Self::new(
            DEFAULT_CANCELLATION_TTL_SECS,
            DEFAULT_MAX_ACTIVE_CANCELLATIONS,
        )
    }

    /// Records the signal a scan watches, replacing any entry for that id.
    pub fn register(&self, scan_id: String, signal: Arc<AtomicBool>) {
        let now = unix_timestamp();
        let mut entries = self.lock();
        entries.retain(|_, entry| {
            zenith_core::domain::is_within_window(entry.created_at, now, self.ttl_secs)
        });
        if entries.len() >= self.capacity {
            // Ordered by age and then by id: two scans registered in the same
            // second must not make eviction depend on map iteration order.
            if let Some(oldest_id) = entries
                .iter()
                .min_by_key(|(id, entry)| (entry.created_at, id.as_str()))
                .map(|(id, _)| id.clone())
            {
                entries.remove(&oldest_id);
            }
        }
        entries.insert(
            scan_id,
            CancellationEntry {
                signal,
                created_at: now,
            },
        );
    }

    /// Forgets a scan's handle. Called on success, on cancellation, and on
    /// error, so a finished scan never leaves a way to cancel something that is
    /// no longer running.
    pub fn remove(&self, scan_id: &str) {
        self.lock().remove(scan_id);
    }

    /// The signal registered for one id, when the entry is still live.
    pub fn signal(&self, scan_id: &str) -> Option<Arc<AtomicBool>> {
        let now = unix_timestamp();
        let mut entries = self.lock();
        entries.retain(|_, entry| {
            zenith_core::domain::is_within_window(entry.created_at, now, self.ttl_secs)
        });
        entries.get(scan_id).map(|entry| entry.signal.clone())
    }

    /// Requests cancellation of one scan.
    ///
    /// An unknown or expired id is not an error: the scan has already finished,
    /// or was never this workflow's. The answer says which happened so a caller
    /// can report it without inventing a failure.
    pub fn request(&self, scan_id: &str) -> bool {
        match self.signal(scan_id) {
            Some(signal) => {
                signal.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// Recovers a poisoned lock: the entries are disposable, and losing them
    /// would silently un-cancel every running scan.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, CancellationEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl std::fmt::Debug for CancellationRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CancellationRegistry")
            .field("entries", &self.lock().len())
            .field("ttl_secs", &self.ttl_secs)
            .field("capacity", &self.capacity)
            .finish()
    }
}

/// The probe one scan holds.
///
/// It reads the same flag the registry stored, so a cancel reaches the scan
/// even if the registry entry is evicted while the scan is still running: a
/// request that arrived must not be forgotten because bookkeeping moved on.
pub struct ScanCancellation {
    signal: Arc<AtomicBool>,
}

impl ScanCancellation {
    pub fn new(signal: Arc<AtomicBool>) -> Self {
        Self { signal }
    }

    /// The flag this probe watches, for a caller that registers it as well.
    pub fn signal(&self) -> Arc<AtomicBool> {
        self.signal.clone()
    }
}

impl CancellationProbe for ScanCancellation {
    fn is_cancelled(&self) -> bool {
        self.signal.load(Ordering::SeqCst)
    }
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::{CancellationRegistry, ScanCancellation};
    use crate::models::CancellationProbe;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    /// A registered scan can be cancelled by id, and the probe the scan holds
    /// observes it.
    #[test]
    fn a_registered_scan_is_cancelled_by_id() {
        let registry = CancellationRegistry::for_scans();
        let signal = Arc::new(AtomicBool::new(false));
        registry.register("scan-1".to_string(), signal.clone());
        let probe = ScanCancellation::new(signal);

        assert!(!probe.is_cancelled());
        assert!(registry.request("scan-1"));
        assert!(probe.is_cancelled());
    }

    /// An id nobody registered, or one whose entry was removed, is not an
    /// error: the scan is simply not running any more.
    #[test]
    fn cancelling_an_unknown_scan_reports_that_nothing_was_running() {
        let registry = CancellationRegistry::for_scans();
        assert!(!registry.request("never-registered"));

        let signal = Arc::new(AtomicBool::new(false));
        registry.register("scan-2".to_string(), signal);
        registry.remove("scan-2");
        assert!(!registry.request("scan-2"));
    }

    /// A scan keeps its signal even after the registry forgets the id: the
    /// request that arrived is not lost to bookkeeping.
    #[test]
    fn a_cancelled_scan_stays_cancelled_after_its_entry_is_removed() {
        let registry = CancellationRegistry::for_scans();
        let signal = Arc::new(AtomicBool::new(false));
        registry.register("scan-3".to_string(), signal.clone());
        let probe = ScanCancellation::new(signal);
        assert!(registry.request("scan-3"));
        registry.remove("scan-3");
        assert!(probe.is_cancelled());
    }

    /// The registry is bounded: past its cap the oldest entry is evicted, so a
    /// workflow cannot accumulate handles for scans that never report back.
    #[test]
    fn the_registry_evicts_the_oldest_entry_at_its_cap() {
        let registry = CancellationRegistry::new(60, 2);
        for id in ["first", "second", "third"] {
            registry.register(id.to_string(), Arc::new(AtomicBool::new(false)));
        }
        assert!(
            registry.signal("first").is_none(),
            "registrations in the same second are ordered by id, so the oldest is still the one evicted"
        );
        assert!(registry.signal("second").is_some());
        assert!(registry.signal("third").is_some());
    }

    /// An entry past its window is treated as gone, so a stale id cannot cancel
    /// a scan that happens to reuse it later.
    #[test]
    fn an_expired_entry_is_no_longer_cancellable() {
        let registry = CancellationRegistry::new(0, 4);
        let signal = Arc::new(AtomicBool::new(false));
        registry.register("scan-4".to_string(), signal.clone());
        assert!(!registry.request("scan-4"));
        assert!(!signal.load(std::sync::atomic::Ordering::SeqCst));
    }
}
