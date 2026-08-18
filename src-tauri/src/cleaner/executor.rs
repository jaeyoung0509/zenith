use crate::docker::DockerAdapter;
use crate::metrics::DiskMetricsCollector;
use crate::models::{
    CleanEvent, CleanFailureReason, CleanItemResult, CleanResult, CleanStrategy, DeletePlan,
    DeleteTarget,
};
use crate::safety::{Blacklist, SymlinkGuard, ToctouGuard};
use std::fs;
use std::path::Path;
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
        let clean_op = match target.strategy {
            CleanStrategy::DeleteContents => Self::delete_contents_only(path),
            CleanStrategy::DeleteDirectory => Self::delete_path_itself(path),
            CleanStrategy::Manual => Self::delete_path_itself(path),
            CleanStrategy::ExternalCommand => Self::delete_contents_only(path),
            CleanStrategy::DockerPrune => unreachable!(),
        };

        match clean_op {
            Ok(bytes) => CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                success: true,
                bytes_reclaimed: if bytes > 0 {
                    bytes
                } else {
                    target.expected_bytes
                },
                failure_reason: None,
                error_message: None,
            },
            Err(e) => {
                let failure_reason = match e.kind() {
                    std::io::ErrorKind::PermissionDenied => CleanFailureReason::PermissionDenied,
                    std::io::ErrorKind::NotFound => CleanFailureReason::NotFound,
                    _ => CleanFailureReason::Unknown,
                };
                let user_msg = failure_reason.user_message(&target.name);
                CleanItemResult {
                    item_id: target.item_id.clone(),
                    name: target.name.clone(),
                    path: path.to_string_lossy().to_string(),
                    success: false,
                    bytes_reclaimed: 0,
                    failure_reason: Some(failure_reason),
                    error_message: Some(user_msg),
                }
            }
        }
    }

    /// Deletes all contents inside a directory, leaving the top-level directory intact.
    fn delete_contents_only(dir: &Path) -> Result<u64, std::io::Error> {
        if !dir.exists() {
            return Ok(0);
        }

        if !dir.is_dir() {
            let len = fs::metadata(dir).map(|m| m.len()).unwrap_or(0);
            fs::remove_file(dir)?;
            return Ok(len);
        }

        let mut total_freed = 0u64;
        let entries = fs::read_dir(dir)?;

        for entry in entries.flatten() {
            let child = entry.path();
            // Blacklist check for safety
            if Blacklist::is_blacklisted(&child) {
                continue;
            }

            if SymlinkGuard::is_symlink(&child) {
                let len = fs::symlink_metadata(&child).map(|m| m.len()).unwrap_or(0);
                fs::remove_file(&child)?;
                total_freed += len;
            } else if child.is_dir() {
                let len = crate::scanner::SizeCalculator::measure_path(&child, &[])
                    .0
                    .reclaimable();
                fs::remove_dir_all(&child)?;
                total_freed += len;
            } else {
                let len = fs::metadata(&child).map(|m| m.len()).unwrap_or(0);
                fs::remove_file(&child)?;
                total_freed += len;
            }
        }

        Ok(total_freed)
    }

    /// Deletes the file or entire directory tree.
    fn delete_path_itself(path: &Path) -> Result<u64, std::io::Error> {
        if !path.exists() && !SymlinkGuard::is_symlink(path) {
            return Ok(0);
        }

        // Symlink safety: if path is a symlink, only remove the link file
        if SymlinkGuard::is_symlink(path) {
            let len = fs::symlink_metadata(path).map(|m| m.len()).unwrap_or(0);
            fs::remove_file(path)?;
            return Ok(len);
        }

        if path.is_dir() {
            let len = crate::scanner::SizeCalculator::measure_path(path, &[])
                .0
                .reclaimable();
            fs::remove_dir_all(path)?;
            Ok(len)
        } else {
            let len = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            fs::remove_file(path)?;
            Ok(len)
        }
    }
}
