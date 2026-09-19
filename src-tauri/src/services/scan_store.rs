use std::sync::Mutex;

use crate::models::ScanResult;

/// Backend-owned store for the latest trusted scan result.
///
/// Encapsulates scan retrieval, updating, invalidation, and atomic
/// pre-cleanup validation so concurrent windows and commands cannot race or
/// create plans against invalidated/stale scan states.
pub struct ScanStore {
    last_scan: Mutex<Option<ScanResult>>,
}

impl ScanStore {
    pub fn new() -> Self {
        Self {
            last_scan: Mutex::new(None),
        }
    }

    /// Returns a clone of the latest scan result if one is present.
    pub fn get(&self) -> Option<ScanResult> {
        self.last_scan
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Stores a new completed scan result.
    pub fn set(&self, scan: ScanResult) {
        let mut guard = self
            .last_scan
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = Some(scan);
    }

    /// Invalidates the stored scan, e.g. after a destructive cleanup run.
    #[cfg(test)]
    fn invalidate(&self) {
        let mut guard = self
            .last_scan
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = None;
    }

    /// Atomically validates the current scan against `scan_id` and timestamp `now`,
    /// and invalidates it so subsequent plan creation or execution requires a fresh scan.
    pub fn validate_and_invalidate_for_cleanup(
        &self,
        scan_id: &str,
        now: u64,
    ) -> Result<ScanResult, String> {
        let mut guard = self
            .last_scan
            .lock()
            .map_err(|_| "Scan store lock poisoned".to_string())?;

        let scan = guard.as_ref().ok_or_else(|| {
            "The scan is no longer current. Scan again before cleaning.".to_string()
        })?;

        scan.validate_for_cleanup(scan_id, now)
            .map_err(|error| error.to_string())?;

        let valid_scan = scan.clone();
        *guard = None;
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
            quality: crate::models::ObservationQuality::Fresh,
            skipped_entry_count: 0,
            incomplete_item_count: 0,
            eligibility: Default::default(),
            suppressed_duplicate_count: 0,
            suppressed_duplicate_bytes: 0,
            suppressed_overlap_count: 0,
            suppressed_overlap_bytes: 0,
        }
    }

    #[test]
    fn set_get_and_invalidate() {
        let store = ScanStore::new();
        assert!(store.get().is_none());

        let scan = make_test_scan("s1", 1000);
        store.set(scan.clone());
        assert_eq!(store.get().unwrap().scan_id, "s1");

        store.invalidate();
        assert!(store.get().is_none());
    }

    #[test]
    fn validate_and_invalidate_for_cleanup_atomically_clears() {
        let store = ScanStore::new();
        let scan = make_test_scan("s1", 1000);
        store.set(scan);

        let validated = store
            .validate_and_invalidate_for_cleanup("s1", 1010)
            .unwrap();
        assert_eq!(validated.scan_id, "s1");

        // Stored scan was atomically cleared
        assert!(store.get().is_none());

        // Subsequent cleanup fails because scan was invalidated
        assert!(store
            .validate_and_invalidate_for_cleanup("s1", 1015)
            .is_err());
    }
}
