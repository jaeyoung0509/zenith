use std::sync::Mutex;

use uuid::Uuid;

use crate::models::{
    Category, PublishedScan, ResumeScanRequest, ScanDiscovery, ScanRequest, ScanResult,
};

/// Private discovery state retained between bounded scan passes.
#[derive(Debug, Clone)]
pub(crate) struct ScanCheckpoint {
    pub request: ScanRequest,
    pub remaining_categories: Vec<Category>,
    pub slices: Vec<ScanResult>,
    /// The first published result owns freshness for the whole session.
    pub freshness_anchor: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScanLease {
    generation: u64,
    revision: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ClaimedScan {
    pub lease: ScanLease,
    pub checkpoint: ScanCheckpoint,
}

#[derive(Debug, Clone)]
struct RetainedContinuation {
    id: String,
    scan_id: String,
    generation: u64,
    revision: u64,
    expires_at: u64,
    checkpoint: ScanCheckpoint,
}

#[derive(Debug, Default)]
struct ScanState {
    generation: u64,
    revision: u64,
    published: Option<PublishedScan>,
    continuation: Option<RetainedContinuation>,
    running: Option<ScanLease>,
}

/// Backend-owned authority for the latest scan and its one-shot continuation.
pub struct ScanStore {
    inner: Mutex<ScanState>,
}

impl ScanStore {
    const MAX_RETAINED_ITEMS: usize = 50_000;
    const MAX_RETAINED_PATH_BYTES: usize = 8 * 1024 * 1024;

    pub fn new() -> Self {
        Self {
            inner: Mutex::new(ScanState::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ScanState> {
        self.inner.lock().unwrap_or_else(|poisoned| {
            // Continuations are disposable authority. If a transition panics,
            // discard every retained fact rather than trusting half-written state.
            let mut state = poisoned.into_inner();
            *state = ScanState::default();
            self.inner.clear_poison();
            state
        })
    }

    /// Starts a new generation and atomically supersedes every prior snapshot.
    pub(crate) fn begin(&self) -> ScanLease {
        let mut state = self.lock();
        state.generation = state.generation.wrapping_add(1).max(1);
        state.revision = 0;
        state.published = None;
        state.continuation = None;
        let lease = ScanLease {
            generation: state.generation,
            revision: 0,
        };
        state.running = Some(lease);
        lease
    }

    /// Atomically consumes a matching continuation. A mismatched request
    /// consumes nothing, while a racing caller sees that the token is gone.
    pub(crate) fn claim(
        &self,
        request: &ResumeScanRequest,
        now: u64,
    ) -> Result<ClaimedScan, String> {
        let mut state = self.lock();
        let retained = state
            .continuation
            .as_ref()
            .ok_or_else(|| "This scan can no longer be continued. Start a new scan.".to_string())?;
        if retained.scan_id != request.scan_id || retained.id != request.continuation_id {
            return Err("The continuation does not belong to the current scan.".to_string());
        }
        if now >= retained.expires_at {
            state.continuation = None;
            if let Some(published) = state.published.as_mut() {
                published.discovery = ScanDiscovery::Stopped {
                    reason: "The retained continuation expired. Start a new scan.".into(),
                };
            }
            return Err("The retained continuation expired. Start a new scan.".to_string());
        }

        let retained = state.continuation.take().expect("checked above");
        let lease = ScanLease {
            generation: retained.generation,
            revision: retained.revision,
        };
        state.running = Some(lease);
        if let Some(published) = state.published.as_mut() {
            published.discovery = ScanDiscovery::Stopped {
                reason: "Continuation is in progress.".into(),
            };
        }
        Ok(ClaimedScan {
            lease,
            checkpoint: retained.checkpoint,
        })
    }

    /// Publishes only for the worker that still owns the current generation and
    /// revision. A superseded worker cannot restore an older inventory.
    pub(crate) fn publish(
        &self,
        lease: ScanLease,
        mut result: ScanResult,
        checkpoint: Option<ScanCheckpoint>,
    ) -> Result<PublishedScan, String> {
        let mut state = self.lock();
        if state.running != Some(lease)
            || state.generation != lease.generation
            || state.revision != lease.revision
        {
            return Err("This scan was superseded before it could publish.".to_string());
        }

        state.revision = state.revision.saturating_add(1);
        state.running = None;
        let discovery = if let Some(checkpoint) = checkpoint {
            let retained_items = checkpoint
                .slices
                .iter()
                .flat_map(|slice| &slice.categories)
                .map(|category| category.items.len())
                .sum::<usize>();
            let retained_path_bytes = checkpoint
                .slices
                .iter()
                .flat_map(|slice| &slice.categories)
                .flat_map(|category| &category.items)
                .map(|item| item.path.len() + item.name.len() + item.id.len())
                .sum::<usize>();
            if retained_items > Self::MAX_RETAINED_ITEMS
                || retained_path_bytes > Self::MAX_RETAINED_PATH_BYTES
            {
                state.continuation = None;
                let published = PublishedScan {
                    result,
                    discovery: ScanDiscovery::Stopped {
                        reason: "The partial scan is too large to retain safely. Start a new scan."
                            .into(),
                    },
                };
                state.published = Some(published.clone());
                return Ok(published);
            }
            let continuation_id = Uuid::new_v4().to_string();
            let expires_at = checkpoint
                .freshness_anchor
                .saturating_add(u64::from(ScanResult::VALID_FOR_SECONDS));
            result.finished_at = checkpoint.freshness_anchor;
            state.continuation = Some(RetainedContinuation {
                id: continuation_id.clone(),
                scan_id: result.scan_id.clone(),
                generation: state.generation,
                revision: state.revision,
                expires_at,
                checkpoint,
            });
            ScanDiscovery::Paused { continuation_id }
        } else {
            state.continuation = None;
            ScanDiscovery::Exhausted
        };
        let published = PublishedScan { result, discovery };
        state.published = Some(published.clone());
        Ok(published)
    }

    pub(crate) fn publish_stopped(
        &self,
        lease: ScanLease,
        result: ScanResult,
        reason: impl Into<String>,
    ) -> Result<PublishedScan, String> {
        let mut published = self.publish(lease, result, None)?;
        published.discovery = ScanDiscovery::Stopped {
            reason: reason.into(),
        };
        let mut state = self.lock();
        if state
            .published
            .as_ref()
            .is_some_and(|current| current.result.scan_id == published.result.scan_id)
        {
            state.published = Some(published.clone());
        }
        Ok(published)
    }

    /// Retires a claimed pass after cancellation or failure without restoring
    /// its consumed token.
    pub(crate) fn stop(&self, lease: ScanLease, reason: impl Into<String>) {
        let mut state = self.lock();
        if state.running == Some(lease) {
            state.running = None;
            state.continuation = None;
            if let Some(published) = state.published.as_mut() {
                published.discovery = ScanDiscovery::Stopped {
                    reason: reason.into(),
                };
            }
        }
    }

    pub fn get_published(&self) -> Option<PublishedScan> {
        self.lock().published.clone()
    }

    /// Returns cleanup authority only after discovery is exhausted.
    pub fn get(&self) -> Option<ScanResult> {
        let state = self.lock();
        if state.running.is_some() {
            return None;
        }
        state.published.as_ref().and_then(|scan| {
            matches!(scan.discovery, ScanDiscovery::Exhausted).then(|| scan.result.clone())
        })
    }

    /// Compatibility helper for tests and callers that publish an exhausted scan.
    #[cfg(test)]
    pub fn set(&self, scan: ScanResult) {
        let lease = self.begin();
        let _ = self.publish(lease, scan, None);
    }

    #[cfg(test)]
    fn invalidate(&self) {
        let mut state = self.lock();
        state.generation = state.generation.wrapping_add(1).max(1);
        state.revision = 0;
        state.published = None;
        state.continuation = None;
        state.running = None;
    }

    pub fn validate_and_invalidate_for_cleanup(
        &self,
        scan_id: &str,
        now: u64,
    ) -> Result<ScanResult, String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "Scan store lock poisoned".to_string())?;
        if state.running.is_some() {
            return Err("The scan is still being continued. Finish it before cleaning.".into());
        }
        if !matches!(
            state.published.as_ref().map(|scan| &scan.discovery),
            Some(ScanDiscovery::Exhausted)
        ) {
            return Err(
                "Discovery is incomplete. Continue or restart the scan before cleaning.".into(),
            );
        }
        let scan = state
            .published
            .as_ref()
            .map(|published| &published.result)
            .ok_or_else(|| {
                "The scan is no longer current. Scan again before cleaning.".to_string()
            })?;
        scan.validate_for_cleanup(scan_id, now)
            .map_err(|error| error.to_string())?;
        let valid_scan = scan.clone();
        state.generation = state.generation.wrapping_add(1).max(1);
        state.revision = 0;
        state.published = None;
        state.continuation = None;
        state.running = None;
        Ok(valid_scan)
    }
}

impl Default for ScanStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_scan(scan_id: &str, finished_at: u64) -> ScanResult {
        ScanResult {
            cancelled: false,
            metrics: Default::default(),
            scan_id: scan_id.to_string(),
            valid_for_seconds: ScanResult::VALID_FOR_SECONDS,
            started_at: finished_at.saturating_sub(5),
            finished_at,
            total_bytes: 500,
            cleanable_bytes: 500,
            safe_bytes: 500,
            rebuild_bytes: 0,
            manual_bytes: 0,
            categories: vec![],
            incomplete_reasons: vec![],
            gaps: vec![],
            quality: crate::models::ObservationQuality::Fresh,
            skipped_entry_count: 0,
            incomplete_item_count: 0,
            eligibility: Default::default(),
            suppressed_duplicate_count: 0,
            suppressed_duplicate_bytes: 0,
            suppressed_overlap_count: 0,
            suppressed_overlap_bytes: 0,
            ambiguous_overlap_count: 0,
            ambiguous_overlap_bytes: 0,
        }
    }

    fn checkpoint(anchor: u64) -> ScanCheckpoint {
        ScanCheckpoint {
            request: ScanRequest::default(),
            remaining_categories: vec![Category::System],
            slices: vec![],
            freshness_anchor: anchor,
        }
    }

    #[test]
    fn set_get_and_invalidate() {
        let store = ScanStore::new();
        assert!(store.get().is_none());
        store.set(make_test_scan("s1", 1000));
        assert_eq!(store.get().unwrap().scan_id, "s1");
        store.invalidate();
        assert!(store.get().is_none());
    }

    #[test]
    fn continuation_is_one_shot_and_mismatch_does_not_consume_it() {
        let store = ScanStore::new();
        let lease = store.begin();
        let published = store
            .publish(lease, make_test_scan("s1", 1000), Some(checkpoint(1000)))
            .unwrap();
        let ScanDiscovery::Paused { continuation_id } = published.discovery else {
            panic!("expected paused scan")
        };
        assert!(store
            .claim(
                &ResumeScanRequest {
                    scan_id: "s1".into(),
                    continuation_id: "wrong".into(),
                },
                1001,
            )
            .is_err());
        let request = ResumeScanRequest {
            scan_id: "s1".into(),
            continuation_id,
        };
        assert!(store.claim(&request, 1001).is_ok());
        assert!(store.claim(&request, 1001).is_err());
    }

    #[test]
    fn late_worker_cannot_publish_after_new_generation() {
        let store = ScanStore::new();
        let old = store.begin();
        let current = store.begin();
        assert!(store
            .publish(old, make_test_scan("old", 1000), None)
            .is_err());
        assert!(store
            .publish(current, make_test_scan("new", 1000), None)
            .is_ok());
        assert_eq!(store.get().unwrap().scan_id, "new");
    }

    #[test]
    fn resume_preserves_original_freshness_anchor() {
        let store = ScanStore::new();
        let lease = store.begin();
        let published = store
            .publish(lease, make_test_scan("s1", 1000), Some(checkpoint(1000)))
            .unwrap();
        let ScanDiscovery::Paused { continuation_id } = published.discovery else {
            panic!("expected paused scan")
        };
        let claimed = store
            .claim(
                &ResumeScanRequest {
                    scan_id: "s1".into(),
                    continuation_id,
                },
                1001,
            )
            .unwrap();
        let resumed = store
            .publish(
                claimed.lease,
                make_test_scan("s2", 1100),
                Some(checkpoint(1000)),
            )
            .unwrap();
        assert_eq!(resumed.result.finished_at, 1000);
    }

    #[test]
    fn continuation_expires_at_the_absolute_boundary_and_never_authorizes_cleanup() {
        let store = ScanStore::new();
        let lease = store.begin();
        let published = store
            .publish(lease, make_test_scan("s1", 1000), Some(checkpoint(1000)))
            .unwrap();
        let ScanDiscovery::Paused { continuation_id } = published.discovery else {
            panic!("expected paused scan")
        };
        assert!(
            store.get().is_none(),
            "paused discovery cannot authorize cleanup"
        );
        assert!(store
            .claim(
                &ResumeScanRequest {
                    scan_id: "s1".into(),
                    continuation_id,
                },
                1300,
            )
            .is_err());
        assert!(matches!(
            store.get_published().unwrap().discovery,
            ScanDiscovery::Stopped { .. }
        ));
    }

    #[test]
    fn validate_and_invalidate_for_cleanup_atomically_clears() {
        let store = ScanStore::new();
        store.set(make_test_scan("s1", 1000));
        let validated = store
            .validate_and_invalidate_for_cleanup("s1", 1010)
            .unwrap();
        assert_eq!(validated.scan_id, "s1");
        assert!(store.get().is_none());
        assert!(store
            .validate_and_invalidate_for_cleanup("s1", 1015)
            .is_err());
    }
}
