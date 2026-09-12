use crate::cache_providers::CacheProviderRegistry;
use crate::docker::DockerAdapter;
use crate::metrics::DiskMetricsCollector;
use crate::models::{
    CleanEvent, CleanFailureReason, CleanItemResult, CleanResult, CleanStatus, CleanStrategy,
    DeletePlan, DeleteTarget,
};
use crate::platform::PlatformEnvironment;
use crate::safety::{Blacklist, SafeTreeDeleter, SymlinkGuard, ToctouGuard};
use std::time::SystemTime;

pub struct CleanExecutor;

/// Counts the items that did not fully clean, as `(partial, failed)`.
///
/// A `Partial` item reclaimed some bytes and therefore reports `success`, but
/// it is not a success: without this count a run whose targets were mostly left
/// behind is indistinguishable from a clean one at the summary level.
fn count_incomplete_items(items: &[CleanItemResult]) -> (u64, u64) {
    items
        .iter()
        .fold((0u64, 0u64), |(partial, failed), item| match item.status {
            CleanStatus::Success => (partial, failed),
            CleanStatus::Partial => (partial + 1, failed),
            CleanStatus::Failed => (partial, failed + 1),
        })
}

/// The diagnostics line for one target that did not fully clean.
fn incomplete_item_message(result: &CleanItemResult) -> String {
    let reason = result
        .error_message
        .as_deref()
        .unwrap_or("no reason reported");
    if result.status == CleanStatus::Partial {
        format!(
            "Target `{}` ({}) did not fully clean: {reason}",
            result.name, result.path
        )
    } else {
        format!(
            "Target `{}` ({}) failed: {reason}",
            result.name, result.path
        )
    }
}

impl CleanExecutor {
    /// Executes a verified DeletePlan safely and securely, emitting streaming CleanEvents.
    ///
    /// The environment is threaded to the deletion and external-command
    /// boundaries so exclusion expansion and provider resolution follow the
    /// same environment the plan was built against.
    pub fn execute<F>(
        plan: DeletePlan,
        environment: &PlatformEnvironment,
        mut on_event: F,
    ) -> CleanResult
    where
        F: FnMut(CleanEvent),
    {
        let started_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let initial_disk = DiskMetricsCollector::get_primary_disk(environment).ok();

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

            let result = Self::clean_target(target, environment);

            if result.success {
                total_reclaimed_bytes += result.bytes_reclaimed;
            } else {
                total_failed_bytes += target.expected_bytes;
            }
            if result.status != CleanStatus::Success {
                crate::diagnostics::log_error("cleanup", &incomplete_item_message(&result));
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

        let (partial_count, failed_count) = count_incomplete_items(&item_results);

        let finished_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let final_disk = DiskMetricsCollector::get_primary_disk(environment).ok();
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
            partial_count,
            failed_count,
            items: item_results,
            actual_disk_free_delta,
        };

        on_event(CleanEvent::Finished {
            result: clean_result.clone(),
        });

        clean_result
    }

    fn clean_target(target: &DeleteTarget, environment: &PlatformEnvironment) -> CleanItemResult {
        // Special case: DockerPrune strategy doesn't operate on standard filesystem paths
        if target.strategy == CleanStrategy::DockerPrune {
            return match DockerAdapter::prune_category(environment, &target.signature_id) {
                Ok(reclaimed) => CleanItemResult {
                    item_id: target.item_id.clone(),
                    name: target.name.clone(),
                    path: target.path.to_string_lossy().to_string(),
                    status: CleanStatus::Success,
                    success: true,
                    bytes_reclaimed: reclaimed,
                    failure_reason: None,
                    error_message: None,
                },
                Err(e) => CleanItemResult {
                    item_id: target.item_id.clone(),
                    name: target.name.clone(),
                    path: target.path.to_string_lossy().to_string(),
                    status: CleanStatus::Failed,
                    success: false,
                    bytes_reclaimed: 0,
                    failure_reason: Some(CleanFailureReason::ExternalCommandFailed),
                    error_message: Some(e.to_string()),
                },
            };
        }

        let path = &target.path;

        if crate::cache_providers::mutation_blocked_by_active_runtime(&target.signature_id) {
            return CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().into_owned(),
                status: CleanStatus::Failed,
                success: false,
                bytes_reclaimed: 0,
                failure_reason: Some(CleanFailureReason::ExternalCommandFailed),
                error_message: Some(
                    "A matching AI compiler or Python runtime is active. Close it and scan again."
                        .to_string(),
                ),
            };
        }

        // 1. Blacklist check (lexical & canonical, fail closed on mutation)
        if let Err(e) = Blacklist::validate_with(path, environment) {
            return CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                status: CleanStatus::Failed,
                success: false,
                bytes_reclaimed: 0,
                failure_reason: Some(CleanFailureReason::Blacklisted),
                error_message: Some(e.to_string()),
            };
        }
        if let Err(e) = SymlinkGuard::validate_canonical_blacklist_strict(path, environment) {
            return CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                status: CleanStatus::Failed,
                success: false,
                bytes_reclaimed: 0,
                failure_reason: Some(CleanFailureReason::Blacklisted),
                error_message: Some(e.to_string()),
            };
        }

        // 1b. Symlink metadata must be readable; failure fails closed.
        if let Err(e) = SymlinkGuard::is_symlink_strict(path) {
            return CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                status: CleanStatus::Failed,
                success: false,
                bytes_reclaimed: 0,
                failure_reason: Some(CleanFailureReason::ChangedSinceScan),
                error_message: Some(e.to_string()),
            };
        }

        // 2. Check path existence
        if !path.exists() && !SymlinkGuard::is_symlink(path) {
            return CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                status: CleanStatus::Success,
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
                    status: CleanStatus::Failed,
                    success: false,
                    bytes_reclaimed: 0,
                    failure_reason: Some(CleanFailureReason::ChangedSinceScan),
                    error_message: Some(e.to_string()),
                };
            }
        }

        // 3b. Stale Temp Directory TOCTOU: re-verify freshness invariant if target has min_age_days
        if let Some(days) = target.min_age_days {
            if path.is_dir() {
                let stats = crate::scanner::DirectoryScanner::measure_tree_stats(
                    environment,
                    path,
                    &target.exclusions,
                    0,
                    32,
                );
                if !stats.complete {
                    return CleanItemResult {
                        item_id: target.item_id.clone(),
                        name: target.name.clone(),
                        path: path.to_string_lossy().to_string(),
                        status: CleanStatus::Failed,
                        success: false,
                        bytes_reclaimed: 0,
                        failure_reason: Some(CleanFailureReason::ChangedSinceScan),
                        error_message: Some(
                            "Directory structure could not be fully verified; aborted to protect active files"
                                .to_string(),
                        ),
                    };
                }
                let Some(newest) = stats.newest_mtime else {
                    return CleanItemResult {
                        item_id: target.item_id.clone(),
                        name: target.name.clone(),
                        path: path.to_string_lossy().to_string(),
                        status: CleanStatus::Failed,
                        success: false,
                        bytes_reclaimed: 0,
                        failure_reason: Some(CleanFailureReason::ChangedSinceScan),
                        error_message: Some(
                            "Directory modification timestamp unavailable; aborted to protect active files"
                                .to_string(),
                        ),
                    };
                };
                let minimum_age = std::time::Duration::from_secs(days as u64 * 86_400);
                if std::time::SystemTime::now()
                    .duration_since(newest)
                    .unwrap_or_default()
                    < minimum_age
                {
                    return CleanItemResult {
                        item_id: target.item_id.clone(),
                        name: target.name.clone(),
                        path: path.to_string_lossy().to_string(),
                        status: CleanStatus::Failed,
                        success: false,
                        bytes_reclaimed: 0,
                        failure_reason: Some(CleanFailureReason::ChangedSinceScan),
                        error_message: Some(format!(
                            "Directory contains files modified within the last {} day(s); aborted to protect active files",
                            days
                        )),
                    };
                }
            }
        }

        // 4. Perform non-destructive deletion according to strategy
        let report = match target.strategy {
            CleanStrategy::DeleteContents => {
                SafeTreeDeleter::delete_contents(path, &target.exclusions, environment)
            }
            CleanStrategy::DeleteDirectory => {
                SafeTreeDeleter::delete_path(path, &target.exclusions, environment)
            }
            CleanStrategy::Manual => {
                return CleanItemResult {
                    item_id: target.item_id.clone(),
                    name: target.name.clone(),
                    path: path.to_string_lossy().to_string(),
                    status: CleanStatus::Failed,
                    success: false,
                    bytes_reclaimed: 0,
                    failure_reason: Some(CleanFailureReason::Unknown),
                    error_message: Some("Manual cleanup requires a dedicated adapter".to_string()),
                };
            }
            CleanStrategy::ExternalCommand => {
                return match CacheProviderRegistry::prune(&target.signature_id, path, environment) {
                    // The provider pruned its own cache but the amount could not
                    // be measured completely: the target is partial, not clean,
                    // and no byte count is claimed for it.
                    Ok(None) => CleanItemResult {
                        item_id: target.item_id.clone(),
                        name: target.name.clone(),
                        path: path.to_string_lossy().into_owned(),
                        status: CleanStatus::Partial,
                        success: true,
                        bytes_reclaimed: 0,
                        failure_reason: None,
                        error_message: Some(
                            "The provider pruned its cache, but the reclaimed amount could not be measured completely"
                                .to_string(),
                        ),
                    },
                    Ok(Some(reclaimed)) => CleanItemResult {
                        item_id: target.item_id.clone(),
                        name: target.name.clone(),
                        path: path.to_string_lossy().into_owned(),
                        status: CleanStatus::Success,
                        success: true,
                        bytes_reclaimed: reclaimed,
                        failure_reason: None,
                        error_message: None,
                    },
                    Err(error) => CleanItemResult {
                        item_id: target.item_id.clone(),
                        name: target.name.clone(),
                        path: path.to_string_lossy().into_owned(),
                        status: CleanStatus::Failed,
                        success: false,
                        bytes_reclaimed: 0,
                        failure_reason: Some(CleanFailureReason::ExternalCommandFailed),
                        error_message: Some(error),
                    },
                };
            }
            CleanStrategy::DockerPrune => unreachable!(),
        };

        if report.is_success() {
            CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                status: CleanStatus::Success,
                success: true,
                bytes_reclaimed: report.reclaimed_bytes,
                failure_reason: None,
                error_message: None,
            }
        } else if report.reclaimed_bytes > 0 {
            // Partial success: accurately record partial status and reclaimed bytes
            let error_msg = if report.errors.is_empty() {
                "Some file(s) could not be removed (e.g. locked or permission denied)".to_string()
            } else {
                format!(
                    "{} file(s) could not be removed: {}",
                    report.errors.len(),
                    report.errors.join("; ")
                )
            };
            CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                status: CleanStatus::Partial,
                success: true,
                bytes_reclaimed: report.reclaimed_bytes,
                failure_reason: None,
                error_message: Some(error_msg),
            }
        } else {
            let error_str = report.errors.join("; ");
            let failure_reason =
                classify_cleanup_failure_with_codes(&report.os_error_codes, &error_str);
            // A refusal under Controlled Folder Access is not something the user
            // can find from the raw OS error, so the message names the setting.
            let error_message = if failure_reason == CleanFailureReason::PermissionDenied {
                crate::platform::environment::describe_access_refusal(environment, path, &error_str)
            } else {
                error_str
            };
            CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                status: CleanStatus::Failed,
                success: false,
                bytes_reclaimed: 0,
                failure_reason: Some(failure_reason),
                error_message: Some(error_message),
            }
        }
    }
}

pub fn classify_cleanup_failure(error_str: &str) -> CleanFailureReason {
    classify_cleanup_failure_with_codes(&[], error_str)
}

/// Classifies a cleanup failure. Raw OS error codes take precedence over
/// message text so localized Windows errors are still classified correctly;
/// the message fallback covers guard errors that Zenith produces itself.
pub fn classify_cleanup_failure_with_codes(
    os_error_codes: &[i32],
    error_str: &str,
) -> CleanFailureReason {
    for code in os_error_codes {
        match *code {
            // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION
            32 | 33 => return CleanFailureReason::InUse,
            // ERROR_ACCESS_DENIED
            5 => return CleanFailureReason::PermissionDenied,
            // ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND
            2 | 3 => return CleanFailureReason::NotFound,
            _ => {}
        }
    }

    let lower = error_str.to_ascii_lowercase();
    if lower.contains("sharing violation")
        || lower.contains("lock violation")
        || lower.contains("used by another process")
        || lower.contains("(os error 32)")
        || lower.contains("os error 32:")
        || lower.contains("(os error 33)")
        || lower.contains("os error 33:")
        || lower.contains("is in use")
    {
        CleanFailureReason::InUse
    } else if lower.contains("permission denied")
        || lower.contains("access denied")
        || lower.contains("access is denied")
        || lower.contains("(os error 5)")
        || lower.contains("os error 5:")
    {
        CleanFailureReason::PermissionDenied
    } else if lower.contains("changed during cleanup") {
        CleanFailureReason::ChangedSinceScan
    } else if lower.contains("no such file")
        || lower.contains("(os error 2)")
        || lower.contains("os error 2:")
        || lower.contains("(os error 3)")
        || lower.contains("os error 3:")
        || lower.contains("cannot find the file specified")
        || lower.contains("cannot find the path specified")
    {
        CleanFailureReason::NotFound
    } else {
        CleanFailureReason::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_with_status(status: CleanStatus) -> CleanItemResult {
        CleanItemResult {
            item_id: "item".to_string(),
            name: "Cache".to_string(),
            path: "/tmp/cache".to_string(),
            status,
            success: status != CleanStatus::Failed,
            bytes_reclaimed: if status == CleanStatus::Failed {
                0
            } else {
                512
            },
            failure_reason: None,
            error_message: None,
        }
    }

    #[test]
    fn clean_result_counts_partial_and_failed_items() {
        let items = vec![
            item_with_status(CleanStatus::Success),
            item_with_status(CleanStatus::Partial),
            item_with_status(CleanStatus::Partial),
            item_with_status(CleanStatus::Failed),
        ];

        assert_eq!(count_incomplete_items(&items), (2, 1));
        assert_eq!(count_incomplete_items(&[]), (0, 0));
    }

    /// Both incomplete shapes reach the log with the target named; a partial
    /// item is the one the old summary line could not report at all.
    #[test]
    fn incomplete_item_message_names_partial_and_failed_targets() {
        let mut partial = item_with_status(CleanStatus::Partial);
        partial.error_message = Some("2 file(s) could not be removed: busy".to_string());
        assert_eq!(
            incomplete_item_message(&partial),
            "Target `Cache` (/tmp/cache) did not fully clean: 2 file(s) could not be removed: busy"
        );

        assert_eq!(
            incomplete_item_message(&item_with_status(CleanStatus::Failed)),
            "Target `Cache` (/tmp/cache) failed: no reason reported"
        );
    }

    #[cfg(unix)]
    #[test]
    fn partial_cleanup_keeps_item_success_but_reports_the_count() {
        use std::os::unix::fs::symlink;

        let fixture = tempfile::tempdir().unwrap();
        let cache_root = fixture.path().join("partial-cache");
        std::fs::create_dir(&cache_root).unwrap();
        let removable = cache_root.join("removable.bin");
        std::fs::write(&removable, b"payload").unwrap();
        // A link that resolves into the user profile is refused by the
        // canonical blacklist, so its sibling is deleted while this child is
        // recorded as an error: bytes were reclaimed without finishing.
        let home = crate::platform::NativePlatformPaths::new()
            .home()
            .expect("a POSIX host exposes a home directory");
        let refused = cache_root.join("linked-profile");
        symlink(&home, &refused).unwrap();

        let plan = DeletePlan {
            id: uuid::Uuid::new_v4(),
            scan_id: "scan-partial".to_string(),
            targets: vec![DeleteTarget {
                item_id: "partial-target".to_string(),
                signature_id: "test.partial".to_string(),
                name: "Partial cache".to_string(),
                path: cache_root.clone(),
                strategy: CleanStrategy::DeleteContents,
                expected_bytes: 4096,
                risk: crate::models::RiskTier::Safe,
                identity: None,
                exclusions: vec![],
                min_age_days: None,
            }],
            expected_reclaim_bytes: 4096,
            risk: crate::models::RiskSummary::default(),
            created_at: 0,
        };

        let result = CleanExecutor::execute(plan, &PlatformEnvironment::native(), |_| {});

        assert_eq!(result.items.len(), 1);
        assert!(!removable.exists(), "the deletable sibling must be gone");
        assert!(refused.exists(), "the refused link must be left in place");
        let item = &result.items[0];
        assert_eq!(item.status, CleanStatus::Partial);
        assert!(item.success, "a partial item still reclaimed bytes");
        assert!(item.bytes_reclaimed > 0);
        assert!(item.error_message.is_some());
        assert_eq!(result.partial_count, 1);
        assert_eq!(result.failed_count, 0);
    }

    #[test]
    fn classifies_sharing_violation_as_in_use() {
        assert_eq!(
            classify_cleanup_failure(
                "C:\\file.bin: Sharing violation (file in use by another process): os error 32"
            ),
            CleanFailureReason::InUse
        );
        assert_eq!(
            classify_cleanup_failure("Sharing violation (os error 32): file is locked"),
            CleanFailureReason::InUse
        );
        assert_eq!(
            classify_cleanup_failure("file is used by another process"),
            CleanFailureReason::InUse
        );
        assert_eq!(
            classify_cleanup_failure("The process cannot access the file because another process has locked a portion of the file. (os error 33)"),
            CleanFailureReason::InUse
        );
        assert_eq!(
            classify_cleanup_failure("Lock violation: resource busy"),
            CleanFailureReason::InUse
        );
    }

    #[test]
    fn classifies_access_denied_as_permission_denied() {
        assert_eq!(
            classify_cleanup_failure(
                "C:\\file.bin: Access denied (permission denied): (os error 5)"
            ),
            CleanFailureReason::PermissionDenied
        );
        assert_eq!(
            classify_cleanup_failure("Permission denied: /var/log/syslog"),
            CleanFailureReason::PermissionDenied
        );
        assert_eq!(
            classify_cleanup_failure("Access is denied (os error 5)"),
            CleanFailureReason::PermissionDenied
        );
    }

    #[test]
    fn does_not_misclassify_os_error_50_as_permission_denied() {
        assert_eq!(
            classify_cleanup_failure("The network request is not supported (os error 50)"),
            CleanFailureReason::Unknown
        );
    }

    #[test]
    fn classifies_failures_from_the_os_error_code_not_the_message() {
        assert_eq!(
            classify_cleanup_failure_with_codes(
                &[32],
                "Der Prozess kann nicht auf die Datei zugreifen."
            ),
            CleanFailureReason::InUse
        );
        assert_eq!(
            classify_cleanup_failure_with_codes(&[33], "Die Datei ist gesperrt."),
            CleanFailureReason::InUse
        );
        assert_eq!(
            classify_cleanup_failure_with_codes(&[5], "Zugriff verweigert."),
            CleanFailureReason::PermissionDenied
        );
        assert_eq!(
            classify_cleanup_failure_with_codes(&[2], "Die Datei wurde nicht gefunden."),
            CleanFailureReason::NotFound
        );
        assert_eq!(
            classify_cleanup_failure_with_codes(&[87], "Ungültiger Parameter."),
            CleanFailureReason::Unknown
        );
    }

    #[test]
    fn classifies_changed_since_scan_and_not_found() {
        assert_eq!(
            classify_cleanup_failure("Directory changed during cleanup: /tmp/app"),
            CleanFailureReason::ChangedSinceScan
        );
        assert_eq!(
            classify_cleanup_failure("No such file or directory: /tmp/missing"),
            CleanFailureReason::NotFound
        );
        assert_eq!(
            classify_cleanup_failure("The system cannot find the file specified. (os error 2)"),
            CleanFailureReason::NotFound
        );
        assert_eq!(
            classify_cleanup_failure("The system cannot find the path specified. (os error 3)"),
            CleanFailureReason::NotFound
        );
        assert_eq!(
            classify_cleanup_failure("unknown disk failure"),
            CleanFailureReason::Unknown
        );
    }
}
