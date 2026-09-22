//! Bounded, de-identified cleanup operation history.
//!
//! History is operational evidence, not another inventory. It records only
//! aggregate outcomes; item names, identifiers, paths, provider output, and
//! error messages never enter the file.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::Serialize;
use zenith_platform::PlatformEnvironment;

use crate::models::CleanResult;

const HISTORY_FILE_NAME: &str = "cleanup-history.jsonl";
const MAX_HISTORY_ENTRIES: usize = 200;

#[derive(Debug, Serialize)]
struct CleanupHistoryRecord {
    schema: u8,
    started_at: u64,
    finished_at: u64,
    target_count: usize,
    reclaimed_bytes: u64,
    moved_to_trash_bytes: u64,
    failed_bytes: u64,
    partial_count: u64,
    failed_count: u64,
    skipped_count: u64,
}

impl From<&CleanResult> for CleanupHistoryRecord {
    fn from(result: &CleanResult) -> Self {
        Self {
            schema: 1,
            started_at: result.started_at,
            finished_at: result.finished_at,
            target_count: result.items.len(),
            reclaimed_bytes: result.total_reclaimed_bytes,
            moved_to_trash_bytes: result.total_moved_to_trash_bytes,
            failed_bytes: result.total_failed_bytes,
            partial_count: result.partial_count,
            failed_count: result.failed_count,
            skipped_count: result.skipped_count,
        }
    }
}

fn history_path(environment: &PlatformEnvironment) -> PathBuf {
    crate::diagnostics::log_dir(environment).join(HISTORY_FILE_NAME)
}

pub(crate) fn record(result: &CleanResult, environment: &PlatformEnvironment) -> io::Result<()> {
    let path = history_path(environment);
    let existing = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let mut lines: Vec<&str> = existing.lines().collect();
    let keep = MAX_HISTORY_ENTRIES.saturating_sub(1);
    if lines.len() > keep {
        lines.drain(..lines.len() - keep);
    }

    let record = serde_json::to_string(&CleanupHistoryRecord::from(result))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut contents = lines.join("\n");
    if !contents.is_empty() {
        contents.push('\n');
    }
    contents.push_str(&record);
    contents.push('\n');
    zenith_platform::file_ops::atomic_write(&path, contents.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CleanItemResult, CleanStatus};
    use tempfile::tempdir;
    use uuid::Uuid;
    use zenith_platform::path_algebra::PathFlavor;

    fn result_with_private_item_data() -> CleanResult {
        CleanResult {
            plan_id: Uuid::new_v4(),
            started_at: 10,
            finished_at: 12,
            total_reclaimed_bytes: 42,
            total_moved_to_trash_bytes: 0,
            total_failed_bytes: 0,
            partial_count: 0,
            failed_count: 0,
            skipped_count: 0,
            items: vec![CleanItemResult {
                item_id: "private-item-id".into(),
                name: "Private cache name".into(),
                path: "/Users/example/private/cache".into(),
                status: CleanStatus::Success,
                success: true,
                estimated_bytes: 42,
                bytes_reclaimed: 42,
                moved_to_trash_bytes: 0,
                failure_reason: None,
                error_message: None,
            }],
            actual_disk_free_delta: Some(42),
        }
    }

    #[test]
    fn history_contains_aggregates_but_no_inventory_data() {
        let fixture = tempdir().unwrap();
        let environment = PlatformEnvironment::simulated(PathFlavor::Posix)
            .with_home(fixture.path())
            .with_temp_dir(fixture.path());
        record(&result_with_private_item_data(), &environment).unwrap();

        let contents = fs::read_to_string(history_path(&environment)).unwrap();
        assert!(contents.contains("\"reclaimed_bytes\":42"));
        assert!(contents.contains("\"target_count\":1"));
        assert!(!contents.contains("private-item-id"));
        assert!(!contents.contains("Private cache name"));
        assert!(!contents.contains("/Users/example"));
    }

    #[test]
    fn history_keeps_only_the_latest_bounded_window() {
        let fixture = tempdir().unwrap();
        let environment = PlatformEnvironment::simulated(PathFlavor::Posix)
            .with_home(fixture.path())
            .with_temp_dir(fixture.path());
        for _ in 0..(MAX_HISTORY_ENTRIES + 5) {
            record(&result_with_private_item_data(), &environment).unwrap();
        }

        let contents = fs::read_to_string(history_path(&environment)).unwrap();
        assert_eq!(contents.lines().count(), MAX_HISTORY_ENTRIES);
    }
}
