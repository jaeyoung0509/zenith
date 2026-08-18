use crate::docker::DockerAdapter;
use crate::metrics::DiskMetricsCollector;
use crate::models::{
    CleanEvent, CleanFailureReason, CleanItemResult, CleanResult, CleanStrategy, DeletePlan,
    DeleteTarget,
};
use crate::safety::{Blacklist, SafeTreeDeleter, SymlinkGuard, ToctouGuard};
use std::time::SystemTime;

pub struct CleanExecutor;

impl CleanExecutor {
    /// Executes a verified DeletePlan safely and securely, emitting streaming CleanEvents.
    pub fn execute<F>(plan: DeletePlan, mut on_event: F) -> CleanResult
    where
        F: FnMut(CleanEvent),
    {
        let started_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let initial_disk = DiskMetricsCollector::get_primary_disk().ok();

        on_event(CleanEvent::Started {
            plan_id: plan.id,
            total_targets: plan.targets.len(),
            expected_bytes: plan.expected_reclaim_bytes,
        });

        let mut item_results = Vec::new();
        let mut total_reclaimed_bytes = 0u64;
        let mut total_failed_bytes = 0u64;
        let total_count = plan.targets.len();

        for (index, target) in plan.targets.iter().enumerate() {
            on_event(CleanEvent::ItemStarted {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                index: index + 1,
                total: total_count,
            });

            let result = Self::clean_target(target);

            if result.success {
                total_reclaimed_bytes += result.bytes_reclaimed;
            } else {
                total_failed_bytes += target.expected_bytes;
            }

            on_event(CleanEvent::ItemFinished {
                item_id: result.item_id.clone(),
                name: result.name.clone(),
                success: result.success,
                reclaimed_bytes: result.bytes_reclaimed,
                error: result.error_message.clone(),
            });

            item_results.push(result);
        }

        let finished_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let final_disk = DiskMetricsCollector::get_primary_disk().ok();
        let actual_disk_free_delta = match (initial_disk, final_disk) {
            (Some(init), Some(fin)) => Some(fin.free_bytes as i64 - init.free_bytes as i64),
            _ => None,
        };

        let clean_result = CleanResult {
            plan_id: plan.id,
            started_at,
            finished_at,
            total_reclaimed_bytes,
            total_failed_bytes,
            items: item_results,
            actual_disk_free_delta,
        };

        on_event(CleanEvent::Finished {
            result: clean_result.clone(),
        });

        clean_result
    }

    fn clean_target(target: &DeleteTarget) -> CleanItemResult {
        // Special case: DockerPrune strategy doesn't operate on standard filesystem paths
        if target.strategy == CleanStrategy::DockerPrune {
            return match DockerAdapter::prune_category(&target.signature_id) {
                Ok(reclaimed) => CleanItemResult {
                    item_id: target.item_id.clone(),
                    name: target.name.clone(),
                    path: target.path.to_string_lossy().to_string(),
                    success: true,
                    bytes_reclaimed: reclaimed,
                    failure_reason: None,
                    error_message: None,
                },
                Err(e) => CleanItemResult {
                    item_id: target.item_id.clone(),
                    name: target.name.clone(),
                    path: target.path.to_string_lossy().to_string(),
                    success: false,
                    bytes_reclaimed: 0,
                    failure_reason: Some(CleanFailureReason::ExternalCommandFailed),
                    error_message: Some(e.to_string()),
                },
            };
        }

        let path = &target.path;

        // 1. Blacklist check
        if let Err(e) = Blacklist::validate(path) {
            return CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                success: false,
                bytes_reclaimed: 0,
                failure_reason: Some(CleanFailureReason::Blacklisted),
                error_message: Some(e.to_string()),
            };
        }

        // 2. Check path existence
        if !path.exists() && !SymlinkGuard::is_symlink(path) {
            return CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                success: true,
                bytes_reclaimed: 0,
                failure_reason: None,
                error_message: None,
            };
        }

        // 3. TOCTOU identity verification
        if let Some(ref expected_identity) = target.identity {
            if let Err(e) = ToctouGuard::verify(path, expected_identity) {
                return CleanItemResult {
                    item_id: target.item_id.clone(),
                    name: target.name.clone(),
                    path: path.to_string_lossy().to_string(),
                    success: false,
                    bytes_reclaimed: 0,
                    failure_reason: Some(CleanFailureReason::ChangedSinceScan),
                    error_message: Some(e.to_string()),
                };
            }
        }

        // 4. Perform non-destructive deletion according to strategy
        let report = match target.strategy {
            CleanStrategy::DeleteContents => {
                SafeTreeDeleter::delete_contents(path, &target.exclusions)
            }
            CleanStrategy::DeleteDirectory => {
                SafeTreeDeleter::delete_path(path, &target.exclusions)
            }
            CleanStrategy::Manual => {
                return CleanItemResult {
                    item_id: target.item_id.clone(),
                    name: target.name.clone(),
                    path: path.to_string_lossy().to_string(),
                    success: false,
                    bytes_reclaimed: 0,
                    failure_reason: Some(CleanFailureReason::Unknown),
                    error_message: Some("Manual cleanup requires a dedicated adapter".to_string()),
                };
            }
            CleanStrategy::ExternalCommand => {
                SafeTreeDeleter::delete_contents(path, &target.exclusions)
            }
            CleanStrategy::DockerPrune => unreachable!(),
        };

        if report.is_success() {
            CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                success: true,
                bytes_reclaimed: report.reclaimed_bytes,
                failure_reason: None,
                error_message: None,
            }
        } else if report.reclaimed_bytes > 0 {
            // Partial success: accurately record reclaimed bytes even if some files failed
            CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                success: true,
                bytes_reclaimed: report.reclaimed_bytes,
                failure_reason: None,
                error_message: Some(format!(
                    "Partially cleaned ({} errors): {}",
                    report.errors.len(),
                    report.errors.join("; ")
                )),
            }
        } else {
            let error_str = report.errors.join("; ");
            let failure_reason = if error_str.contains("Permission denied") {
                CleanFailureReason::PermissionDenied
            } else if error_str.contains("No such file") {
                CleanFailureReason::NotFound
            } else {
                CleanFailureReason::Unknown
            };
            CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                success: false,
                bytes_reclaimed: 0,
                failure_reason: Some(failure_reason),
                error_message: Some(error_str),
            }
        }
    }
}
