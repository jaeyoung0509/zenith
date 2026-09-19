use crate::cache_providers::CacheProviderRegistry;
use crate::docker::DockerAdapter;
use crate::metrics::DiskMetricsCollector;
use crate::models::{
    CleanEvent, CleanFailureReason, CleanItemResult, CleanResult, CleanStatus, CleanStrategy,
    CleanupMode, CleanupOperation, DeletePlan, DeleteTarget,
};
use crate::safety::SafeTreeDeleter;
use std::time::SystemTime;
use zenith_core::domain::cleanup::{ProviderOutcome, ProviderStatus};
use zenith_platform::PlatformEnvironment;

pub struct CleanExecutor;

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

/// Builds the result for one target.
///
/// Every result states what the plan expected and what the run observed as
/// separate fields, and every construction site goes through here so the two
/// can never be conflated by a new arm.
///
/// `success` means "this run removed what it could from this target"; a skip
/// removed nothing, so it is never reported as success even though nothing
/// went wrong. The status and the reason are what tell the two apart.
fn item_result(
    target: &DeleteTarget,
    status: CleanStatus,
    reason: Option<CleanFailureReason>,
    bytes_reclaimed: u64,
    message: Option<String>,
) -> CleanItemResult {
    CleanItemResult {
        item_id: target.item_id.clone(),
        name: target.name.clone(),
        path: target.path.to_string_lossy().to_string(),
        status,
        success: matches!(status, CleanStatus::Success | CleanStatus::Partial),
        estimated_bytes: target.expected_bytes,
        bytes_reclaimed,
        failure_reason: reason,
        error_message: message,
    }
}

/// Counts targets that did not do what the plan expected, as
/// `(partial, failed, skipped)`.
///
/// A skip is its own answer: the target is no longer the object the plan
/// authorized, or was already gone. Counting it as a failure would report a
/// protected outcome as a broken run.
fn count_incomplete_items(items: &[CleanItemResult]) -> (u64, u64, u64) {
    items.iter().fold(
        (0u64, 0u64, 0u64),
        |(partial, failed, skipped), item| match item.status {
            CleanStatus::Success => (partial, failed, skipped),
            CleanStatus::Partial => (partial + 1, failed, skipped),
            CleanStatus::Failed => (partial, failed + 1, skipped),
            CleanStatus::Skipped => (partial, failed, skipped + 1),
        },
    )
}

/// Maps a provider's verified outcome onto the result of one target.
///
/// The provider's own status decides the shape, and nothing here can reach a
/// deletion primitive: a provider action that failed is reported as failed.
/// The reclaimed amount is the provider's measurement, never the plan's
/// estimate, and a refusal states zero rather than the bytes it could not
/// remove.
fn provider_result(target: &DeleteTarget, outcome: ProviderOutcome) -> CleanItemResult {
    let detail = outcome.detail.clone();
    match outcome.status {
        ProviderStatus::Cleaned => item_result(
            target,
            CleanStatus::Success,
            None,
            outcome.reclaimed_bytes,
            detail,
        ),
        ProviderStatus::PartiallyCleaned => item_result(
            target,
            CleanStatus::Partial,
            None,
            outcome.reclaimed_bytes,
            detail,
        ),
        // The build has no adapter for the action the catalog named, so this
        // is not a failure the user can retry into existence.
        ProviderStatus::Unsupported => item_result(
            target,
            CleanStatus::Failed,
            Some(CleanFailureReason::ProviderUnavailable),
            0,
            Some(detail.unwrap_or_else(|| {
                "No provider adapter for this action is available on this platform".to_string()
            })),
        ),
        // A prerequisite that does not hold, a store that cannot be read, and a
        // failed action are one answer to the user: the action did not
        // complete, and the provider's own words say why.
        ProviderStatus::PrerequisiteNotMet | ProviderStatus::Blocked | ProviderStatus::Failed => {
            item_result(
                target,
                CleanStatus::Failed,
                Some(CleanFailureReason::ProviderRefused),
                0,
                Some(detail.unwrap_or_else(|| {
                    format!(
                        "The provider reported its action {}",
                        outcome.status.display_name()
                    )
                })),
            )
        }
        // An outcome that is still a probe state means the provider never
        // answered for the action; reporting it as anything but a refusal
        // would claim a result nobody observed.
        ProviderStatus::Ready => item_result(
            target,
            CleanStatus::Failed,
            Some(CleanFailureReason::ProviderRefused),
            0,
            Some(
                "The provider did not report an outcome for the action it was asked to perform"
                    .to_string(),
            ),
        ),
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
        providers: &crate::cleaner::LifecycleProviderRegistry,
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

            // The executor is the destructive boundary. A future caller may
            // build a Preview or Trash plan without passing through the current
            // service, so only the implemented deletion mode may reach an adapter.
            let result = if plan.mode == CleanupMode::PermanentDelete {
                Self::clean_target(target, environment, providers)
            } else {
                item_result(
                    target,
                    CleanStatus::Failed,
                    Some(CleanFailureReason::Unknown),
                    0,
                    Some(format!(
                        "{} is not implemented by this executor; refusing to mutate",
                        plan.mode.display_name()
                    )),
                )
            };

            match result.status {
                CleanStatus::Success | CleanStatus::Partial => {
                    total_reclaimed_bytes += result.bytes_reclaimed;
                }
                // A skipped target reclaimed nothing because there was nothing
                // left to remove: its estimate is not a failed amount.
                CleanStatus::Skipped => {}
                CleanStatus::Failed => total_failed_bytes += target.expected_bytes,
            }
            if result.status != CleanStatus::Success {
                crate::diagnostics::log_error("cleanup", &incomplete_item_message(&result));
            }

            on_event(CleanEvent::ItemFinished {
                item_id: result.item_id.clone(),
                name: result.name.clone(),
                status: result.status,
                success: result.success,
                reclaimed_bytes: result.bytes_reclaimed,
                error: result.error_message.clone(),
            });

            item_results.push(result);
        }

        let (partial_count, failed_count, skipped_count) = count_incomplete_items(&item_results);

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
            skipped_count,
            items: item_results,
            actual_disk_free_delta,
        };

        on_event(CleanEvent::Finished {
            result: clean_result.clone(),
        });

        clean_result
    }

    fn clean_target(
        target: &DeleteTarget,
        environment: &PlatformEnvironment,
        providers: &crate::cleaner::LifecycleProviderRegistry,
    ) -> CleanItemResult {
        // The classification decides what the target authorizes: a container
        // prune runs through the runtime's own CLI, a provider prune runs
        // through the tool's own fixed arguments, a lifecycle provider action
        // runs through the provider the catalog named, and only a filesystem
        // operation can reach a deletion primitive with `target.path`.
        let Some(operation) = CleanupOperation::of(target) else {
            return item_result(
                target,
                CleanStatus::Failed,
                Some(CleanFailureReason::Unknown),
                0,
                Some(
                    "The planned target authorizes no operation this executor implements; refusing to mutate"
                        .to_string(),
                ),
            );
        };

        // A runtime that owns the cache is a reason to refuse a mutation that
        // would corrupt it. The policy travels with the plan, so the guard
        // judges the authorized target rather than re-reading the catalog. A
        // container prune asks the runtime itself, which is running by
        // definition when it answers.
        if !matches!(operation, CleanupOperation::Container(_)) {
            let running = crate::cleaner::running_executables(&target.process_guard);
            if !running.is_empty() {
                let owner = if target.owner.is_known() {
                    format!(" ({})", target.owner.owner)
                } else {
                    String::new()
                };
                return item_result(
                    target,
                    CleanStatus::Failed,
                    Some(CleanFailureReason::InUse),
                    0,
                    Some(format!(
                        "A process that owns this cache{} is running ({}). Close it and scan again.",
                        owner,
                        running.join(", ")
                    )),
                );
            }
        }

        match operation {
            CleanupOperation::Container(cleanup) => {
                match DockerAdapter::prune_category(environment, cleanup.signature_id()) {
                    Ok(reclaimed) => {
                        item_result(target, CleanStatus::Success, None, reclaimed, None)
                    }
                    Err(e) => item_result(
                        target,
                        CleanStatus::Failed,
                        Some(CleanFailureReason::ExternalCommandFailed),
                        0,
                        Some(e.to_string()),
                    ),
                }
            }
            CleanupOperation::Provider(cleanup) => {
                match CacheProviderRegistry::prune(
                    cleanup.signature_id(),
                    cleanup.expected_location(),
                    environment,
                ) {
                    // The provider pruned its own cache but the amount could not
                    // be measured completely: the target is partial, not clean,
                    // and no byte count is claimed for it.
                    Ok(None) => item_result(
                        target,
                        CleanStatus::Partial,
                        None,
                        0,
                        Some(
                            "The provider pruned its cache, but the reclaimed amount could not be measured completely"
                                .to_string(),
                        ),
                    ),
                    Ok(Some(reclaimed)) => {
                        item_result(target, CleanStatus::Success, None, reclaimed, None)
                    }
                    Err(error) => item_result(
                        target,
                        CleanStatus::Failed,
                        Some(CleanFailureReason::ExternalCommandFailed),
                        0,
                        Some(error),
                    ),
                }
            }
            CleanupOperation::LifecycleProvider(action) => {
                provider_result(target, providers.execute(&action, environment))
            }
            CleanupOperation::Filesystem(_) => Self::clean_filesystem_target(target, environment),
        }
    }

    /// Executes one signature-scoped filesystem mutation.
    ///
    /// The target is revalidated through the safety layer to produce a
    /// `ValidatedTarget`: presence, lexical and canonical blacklist, symlink
    /// integrity, TOCTOU identity, and intensive-cleanup age constraints are
    /// all checked here, and the mutation primitive accepts only the validated
    /// authority that comes back.
    fn clean_filesystem_target(
        target: &DeleteTarget,
        environment: &PlatformEnvironment,
    ) -> CleanItemResult {
        let path = &target.path;

        let validated_target = match crate::safety::SafetyValidator::revalidate(target, environment)
        {
            crate::safety::RevalidationOutcome::Validated(validated) => validated,
            // A skip is the guard's answer, and it is reported as one: the plan
            // authorized an object that is no longer there, or is no longer the
            // same object.
            crate::safety::RevalidationOutcome::Skipped(result) => return result,
            crate::safety::RevalidationOutcome::Failed(result) => return result,
        };

        let report = match validated_target.strategy() {
            CleanStrategy::DeleteContents => {
                SafeTreeDeleter::delete_contents_validated(&validated_target, environment)
            }
            CleanStrategy::DeleteDirectory => {
                SafeTreeDeleter::delete_path_validated(&validated_target, environment)
            }
            CleanStrategy::DeleteStaleContents => {
                SafeTreeDeleter::prune_stale_contents_validated(&validated_target, environment)
            }
            // The classification above produced a filesystem operation from one
            // of these strategies, and `ValidatedTarget` carries the planned
            // strategy unchanged, so no other arm is reachable. Refusing
            // explicitly keeps a future strategy from silently deleting.
            CleanStrategy::ExternalCommand
            | CleanStrategy::DockerPrune
            | CleanStrategy::LifecycleProvider
            | CleanStrategy::Manual => {
                return item_result(
                    target,
                    CleanStatus::Failed,
                    Some(CleanFailureReason::Unknown),
                    0,
                    Some("Target strategy does not authorize a filesystem mutation".to_string()),
                );
            }
        };

        if report.is_success() {
            item_result(
                target,
                CleanStatus::Success,
                None,
                report.reclaimed_bytes,
                None,
            )
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
            item_result(
                target,
                CleanStatus::Partial,
                None,
                report.reclaimed_bytes,
                Some(error_msg),
            )
        } else {
            let error_str = report.errors.join("; ");
            let failure_reason =
                classify_cleanup_failure_with_codes(&report.os_error_codes, &error_str);
            // A refusal under Controlled Folder Access is not something the user
            // can find from the raw OS error, so the message names the setting.
            let error_message = if failure_reason == CleanFailureReason::PermissionDenied {
                zenith_platform::environment::describe_access_refusal(environment, path, &error_str)
            } else {
                error_str
            };
            item_result(
                target,
                CleanStatus::Failed,
                Some(failure_reason),
                0,
                Some(error_message),
            )
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
    use crate::models::CleanupMode;

    use crate::safety::ToctouGuard;

    fn item_with_status(status: CleanStatus) -> CleanItemResult {
        CleanItemResult {
            item_id: "item".to_string(),
            name: "Cache".to_string(),
            path: "/tmp/cache".to_string(),
            status,
            success: status != CleanStatus::Failed,
            estimated_bytes: 0,
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

        assert_eq!(count_incomplete_items(&items), (2, 1, 0));
        assert_eq!(count_incomplete_items(&[]), (0, 0, 0));
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

    /// The plan's expectation is never the run's measurement: a target whose
    /// estimate overstates the tree reclaims what the run removed, and the two
    /// numbers stay separate fields.
    #[test]
    fn reclaimed_bytes_are_measured_not_copied_from_the_estimate() {
        let fixture = tempfile::tempdir().unwrap();
        let cache_root = fixture.path().join("stale-estimate");
        std::fs::create_dir(&cache_root).unwrap();
        std::fs::write(cache_root.join("data.bin"), vec![1u8; 8_192]).unwrap();

        // An estimate from an earlier, larger tree: a plan's expectation is a
        // property of the scan it came from, not of what this run finds.
        let plan = DeletePlan {
            id: uuid::Uuid::new_v4(),
            scan_id: "scan-estimate".to_string(),
            targets: vec![DeleteTarget {
                item_id: "estimated-target".to_string(),
                signature_id: "test.estimate".to_string(),
                name: "Stale estimate".to_string(),
                path: cache_root.clone(),
                strategy: CleanStrategy::DeleteContents,
                expected_bytes: 1_048_576,
                risk: crate::models::RiskTier::Safe,
                identity: ToctouGuard::capture(&cache_root),
                exclusions: vec![],
                min_age_days: None,
                unit: crate::models::CleanupUnit::fixed_path(
                    cache_root.to_string_lossy().to_string(),
                ),
                target_kind: crate::models::EntryKind::Directory,
                owner: crate::models::CleanupOwnership::unknown(),
                process_guard: crate::models::RunningProcessPolicy::none(),
                provider_id: None,
            }],
            expected_reclaim_bytes: 1_048_576,
            risk: crate::models::RiskSummary::default(),
            created_at: 0,
            mode: CleanupMode::PermanentDelete,
        };

        let result = CleanExecutor::execute(
            plan,
            &PlatformEnvironment::native(),
            &crate::cleaner::LifecycleProviderRegistry::new(Vec::new()),
            |_| {},
        );

        let item = &result.items[0];
        assert_eq!(item.status, CleanStatus::Success);
        assert_eq!(item.estimated_bytes, 1_048_576);
        assert!(
            item.bytes_reclaimed > 0 && item.bytes_reclaimed < item.estimated_bytes,
            "the reclaim states what the run removed, not what the plan expected: {item:?}"
        );
        assert_eq!(result.total_reclaimed_bytes, item.bytes_reclaimed);
        assert_eq!(result.failed_count, 0);
        assert!(
            !cache_root.join("data.bin").exists(),
            "delete_contents removes the entries and leaves the root"
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
        let home = zenith_platform::NativePlatformPaths::new()
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
                identity: ToctouGuard::capture(&cache_root),
                exclusions: vec![],
                min_age_days: None,
                unit: crate::models::CleanupUnit::fixed_path(
                    cache_root.to_string_lossy().to_string(),
                ),
                target_kind: crate::models::EntryKind::Directory,
                owner: crate::models::CleanupOwnership::unknown(),
                process_guard: crate::models::RunningProcessPolicy::none(),
                provider_id: None,
            }],
            expected_reclaim_bytes: 4096,
            risk: crate::models::RiskSummary::default(),
            created_at: 0,
            mode: CleanupMode::PermanentDelete,
        };

        let result = CleanExecutor::execute(
            plan,
            &PlatformEnvironment::native(),
            &crate::cleaner::LifecycleProviderRegistry::new(Vec::new()),
            |_| {},
        );

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

    /// A plan whose single target is the given path, with the identity captured
    /// while the path still existed.
    fn volatile_plan(path: &std::path::Path, is_dir: bool) -> DeletePlan {
        let identity = ToctouGuard::capture(path).expect("capture identity while present");
        assert_eq!(identity.is_dir(), is_dir);
        DeletePlan {
            id: uuid::Uuid::new_v4(),
            scan_id: "scan-volatile".to_string(),
            targets: vec![DeleteTarget {
                item_id: "volatile-target".to_string(),
                signature_id: "test.volatile".to_string(),
                name: "Volatile temp".to_string(),
                path: path.to_path_buf(),
                strategy: CleanStrategy::DeleteDirectory,
                expected_bytes: 4096,
                risk: crate::models::RiskTier::Safe,
                identity: Some(identity),
                exclusions: vec![],
                min_age_days: None,
                unit: crate::models::CleanupUnit::fixed_path(path.to_string_lossy().to_string()),
                target_kind: crate::models::EntryKind::Directory,
                owner: crate::models::CleanupOwnership::unknown(),
                process_guard: crate::models::RunningProcessPolicy::none(),
                provider_id: None,
            }],
            expected_reclaim_bytes: 4096,
            risk: crate::models::RiskSummary::default(),
            created_at: 0,
            mode: CleanupMode::PermanentDelete,
        }
    }

    /// A plan whose target is a provider-owned cache, identified by signature.
    fn provider_plan(path: &std::path::Path, signature_id: &str) -> DeletePlan {
        DeletePlan {
            id: uuid::Uuid::new_v4(),
            scan_id: "scan-provider".to_string(),
            targets: vec![DeleteTarget {
                item_id: "provider-target".to_string(),
                signature_id: signature_id.to_string(),
                name: "Provider cache".to_string(),
                path: path.to_path_buf(),
                strategy: CleanStrategy::ExternalCommand,
                expected_bytes: 4096,
                risk: crate::models::RiskTier::Safe,
                identity: None,
                exclusions: vec![],
                min_age_days: None,
                unit: crate::models::CleanupUnit::fixed_path(path.to_string_lossy().to_string()),
                target_kind: crate::models::EntryKind::Directory,
                owner: crate::models::CleanupOwnership::unknown(),
                process_guard: crate::models::RunningProcessPolicy::none(),
                provider_id: None,
            }],
            expected_reclaim_bytes: 4096,
            risk: crate::models::RiskSummary::default(),
            created_at: 0,
            mode: CleanupMode::PermanentDelete,
        }
    }

    #[test]
    fn a_provider_cleanup_failure_never_falls_through_to_filesystem_deletion() {
        let fixture = tempfile::tempdir().unwrap();
        let cache = fixture.path().join("provider-cache");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("payload.bin"), vec![3u8; 128]).unwrap();

        // The signature names no provider Zenith knows, so the prune cannot be
        // performed. The target still carries a real path, which is exactly the
        // shape that must never become filesystem authority.
        let plan = provider_plan(&cache, "test.unknown.provider");
        let environment =
            PlatformEnvironment::simulated(zenith_platform::path_algebra::PathFlavor::current());

        let result = CleanExecutor::execute(
            plan,
            &environment,
            &crate::cleaner::LifecycleProviderRegistry::new(Vec::new()),
            |_| {},
        );

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].status, CleanStatus::Failed);
        assert_eq!(
            result.items[0].failure_reason,
            Some(CleanFailureReason::ExternalCommandFailed)
        );
        assert_eq!(result.total_reclaimed_bytes, 0);
        assert!(
            cache.join("payload.bin").is_file(),
            "a refused provider prune must leave the reviewed path untouched"
        );
    }

    /// A directory that vanished after the scan is skipped, never a failed item
    /// reporting `Could not verify canonical location`.
    #[test]
    fn vanished_directory_target_is_a_skip() {
        let fixture = tempfile::tempdir().unwrap();
        let target_dir = fixture.path().join("codex-clipboard-temp");
        std::fs::create_dir(&target_dir).unwrap();
        std::fs::write(target_dir.join("clipboard.png"), b"volatile").unwrap();

        let plan = volatile_plan(&target_dir, true);
        std::fs::remove_dir_all(&target_dir).unwrap();

        let result = CleanExecutor::execute(
            plan,
            &PlatformEnvironment::native(),
            &crate::cleaner::LifecycleProviderRegistry::new(Vec::new()),
            |_| {},
        );
        assert_eq!(result.failed_count, 0);
        assert_eq!(result.partial_count, 0);
        assert_eq!(result.skipped_count, 1);
        let item = &result.items[0];
        assert_eq!(item.status, CleanStatus::Skipped);
        assert!(
            !item.success,
            "a skipped target removed nothing, so it is not reported as cleaned"
        );
        assert_eq!(item.bytes_reclaimed, 0);
        assert_eq!(item.failure_reason, Some(CleanFailureReason::NotFound));
    }

    /// The same for a regular file: the run reports a skip, and the estimate the
    /// plan carried stays separate from the zero this run measured.
    #[test]
    fn vanished_file_target_is_a_skip() {
        let fixture = tempfile::tempdir().unwrap();
        let target_file = fixture.path().join("tauri-stop-dev-processes.sh");
        std::fs::write(&target_file, b"#!/bin/sh\nexit 0\n").unwrap();

        let plan = volatile_plan(&target_file, false);
        std::fs::remove_file(&target_file).unwrap();

        let result = CleanExecutor::execute(
            plan,
            &PlatformEnvironment::native(),
            &crate::cleaner::LifecycleProviderRegistry::new(Vec::new()),
            |_| {},
        );
        assert_eq!(result.failed_count, 0);
        assert_eq!(result.skipped_count, 1);
        let item = &result.items[0];
        assert_eq!(item.status, CleanStatus::Skipped);
        assert!(!item.success);
        assert_eq!(item.bytes_reclaimed, 0);
        assert_eq!(item.failure_reason, Some(CleanFailureReason::NotFound));
        assert!(
            item.estimated_bytes > 0,
            "the plan-time estimate is reported even when the run reclaimed nothing"
        );
    }

    /// A present target whose identity changed must still fail closed: the run
    /// refuses to delete an object the plan did not authorize.
    #[test]
    fn identity_mismatch_on_a_present_target_is_skipped() {
        let fixture = tempfile::tempdir().unwrap();
        let target = fixture.path().join("cache.dat");
        std::fs::write(&target, b"v1").unwrap();

        let plan = volatile_plan(&target, false);
        std::fs::remove_file(&target).unwrap();
        std::fs::write(&target, b"v2 replaced contents").unwrap();

        let result = CleanExecutor::execute(
            plan,
            &PlatformEnvironment::native(),
            &crate::cleaner::LifecycleProviderRegistry::new(Vec::new()),
            |_| {},
        );
        assert_eq!(result.failed_count, 0);
        assert_eq!(result.skipped_count, 1);
        let item = &result.items[0];
        assert_eq!(item.status, CleanStatus::Skipped);
        assert!(!item.success);
        assert_eq!(
            item.failure_reason,
            Some(CleanFailureReason::ChangedSinceScan)
        );
        assert!(target.exists(), "the replacement must not be deleted");
    }

    /// A plan whose single target is a lifecycle provider action, naming the
    /// provider the catalog declared.
    fn lifecycle_plan(provider_id: &str, pseudo_path: &str, expected_bytes: u64) -> DeletePlan {
        DeletePlan {
            id: uuid::Uuid::new_v4(),
            scan_id: "scan-lifecycle".to_string(),
            targets: vec![DeleteTarget {
                item_id: "lifecycle-target".to_string(),
                signature_id: "test.stated.store".to_string(),
                name: "Stated store".to_string(),
                path: std::path::PathBuf::from(pseudo_path),
                strategy: CleanStrategy::LifecycleProvider,
                expected_bytes,
                risk: crate::models::RiskTier::Manual,
                identity: None,
                exclusions: Vec::new(),
                min_age_days: None,
                unit: crate::models::CleanupUnit::new(
                    crate::models::CleanupUnitKind::ProviderAction,
                    pseudo_path,
                    pseudo_path,
                ),
                target_kind: crate::models::EntryKind::Other,
                owner: crate::models::CleanupOwnership::unknown(),
                process_guard: crate::models::RunningProcessPolicy::none(),
                provider_id: Some(provider_id.to_string()),
            }],
            expected_reclaim_bytes: expected_bytes,
            risk: crate::models::RiskSummary::default(),
            created_at: 0,
            mode: CleanupMode::PermanentDelete,
        }
    }

    /// The provider's measurement is what the run reports, and the plan's
    /// estimate stays a separate field — the same contract a filesystem target
    /// answers to.
    #[test]
    fn a_lifecycle_action_reports_the_providers_measurement_not_the_estimate() {
        use crate::cleaner::providers::test_support::StatedProvider;

        let provider = StatedProvider::holding(3_000, 4).with_outcome(
            zenith_core::domain::cleanup::ProviderOutcome::cleaned(2_500, Some(500)),
        );
        let providers = crate::cleaner::LifecycleProviderRegistry::new(vec![provider.shared()]);

        let result = CleanExecutor::execute(
            lifecycle_plan("test.stated", "stated-store://all-volumes", 9_999),
            &PlatformEnvironment::native(),
            &providers,
            |_| {},
        );

        let item = &result.items[0];
        assert_eq!(item.status, CleanStatus::Success);
        assert_eq!(
            item.estimated_bytes, 9_999,
            "the estimate is what the plan authorized"
        );
        assert_eq!(
            item.bytes_reclaimed, 2_500,
            "the reclaimed amount is what the provider verified"
        );
        assert_eq!(result.total_reclaimed_bytes, 2_500);
        assert_eq!(result.failed_count, 0);
    }

    /// A provider that refuses leaves the reviewed path untouched: the
    /// pseudo-location a provider target carries is never deletion authority,
    /// so a refused action cannot degrade into a filesystem delete.
    #[test]
    fn a_refused_lifecycle_action_never_falls_through_to_filesystem_deletion() {
        use crate::cleaner::providers::test_support::{refusing_provider, StatedProvider};
        use zenith_core::domain::cleanup::{ProviderOutcome, ProviderStatus};

        let fixture = tempfile::tempdir().unwrap();
        let reviewed = fixture.path().join("stated-store");
        std::fs::create_dir_all(&reviewed).unwrap();
        std::fs::write(reviewed.join("payload.bin"), vec![3u8; 128]).unwrap();

        // The plan carries a real directory as its pseudo location, which is
        // exactly the shape that must never become deletion authority.
        let real_path = reviewed.to_string_lossy().to_string();
        let providers = crate::cleaner::LifecycleProviderRegistry::new(vec![refusing_provider(
            ProviderStatus::PrerequisiteNotMet,
            "the owning service is still running",
        )]);

        let result = CleanExecutor::execute(
            lifecycle_plan("test.stated", &real_path, 1_024),
            &PlatformEnvironment::native(),
            &providers,
            |_| {},
        );

        let item = &result.items[0];
        assert_eq!(item.status, CleanStatus::Failed);
        assert_eq!(
            item.failure_reason,
            Some(CleanFailureReason::ProviderRefused)
        );
        assert_eq!(item.bytes_reclaimed, 0);
        assert_eq!(result.total_reclaimed_bytes, 0);
        assert!(
            reviewed.join("payload.bin").is_file(),
            "a refused provider action must leave the reviewed path untouched"
        );

        // An action nothing implements is its own answer: the user cannot
        // retry it into existence, and it says so.
        let empty = crate::cleaner::LifecycleProviderRegistry::new(Vec::new());
        let result = CleanExecutor::execute(
            lifecycle_plan("test.missing", &real_path, 1_024),
            &PlatformEnvironment::native(),
            &empty,
            |_| {},
        );
        let item = &result.items[0];
        assert_eq!(item.status, CleanStatus::Failed);
        assert_eq!(
            item.failure_reason,
            Some(CleanFailureReason::ProviderUnavailable)
        );
        assert!(
            reviewed.join("payload.bin").is_file(),
            "an unimplemented provider action deletes nothing"
        );

        // A partial outcome keeps `success` and reports the remainder.
        let partial = StatedProvider::holding(4_000, 2).with_outcome(
            ProviderOutcome::partially_cleaned(3_000, Some(1_000), "1000 bytes remain"),
        );
        let providers = crate::cleaner::LifecycleProviderRegistry::new(vec![partial.shared()]);
        let result = CleanExecutor::execute(
            lifecycle_plan("test.stated", &real_path, 4_000),
            &PlatformEnvironment::native(),
            &providers,
            |_| {},
        );
        let item = &result.items[0];
        assert_eq!(item.status, CleanStatus::Partial);
        assert!(item.success);
        assert_eq!(item.bytes_reclaimed, 3_000);
        assert_eq!(result.partial_count, 1);
    }
}
