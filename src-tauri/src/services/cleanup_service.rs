use std::sync::Arc;
use std::time::SystemTime;
use uuid::Uuid;

use super::cancellation::{CancellationRegistry, ScanCancellation};
use super::plan_store::{PlanStore, PlanStoreError};
use super::scan_service::ScanService;
use super::scan_store::{ScanCheckpoint, ScanStore};
use crate::cleaner::{CleanExecutor, LifecycleProviderRegistry, OwnerProviderRegistry};
use crate::execution_budget::ExecutionBudgets;
use crate::models::{
    CleanEvent, CleanFailureReason, CleanResult, CleanStrategy, CleanupEligibility, CleanupFailure,
    CleanupFailureScope, CleanupProgressSink, DeletePlan, ObservationQuality, PlanPreview,
    PlanRefusalPreview, PlatformCapabilitiesProvider, PlatformFeature, PublishedScan,
    ResumeScanRequest, ScanEvent, ScanProgressSink, ScanRequest, ScanResult, ZenithError,
    ZenithSettings,
};
use crate::operation_gate::StorageOperationGate;
use crate::safety::SafetyPlanner;
use crate::services::system_service::DockerStatusCache;
use crate::signatures::SignatureRegistry;
use zenith_platform::PlatformEnvironment;

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Derives backend-owned Safe-only candidates for Quick Clean.
///
/// Enforces:
/// - disposition.eligibility == AutoCleanable
/// - cleanable_bytes > 0
/// - category enabled in settings
///
/// Never includes Rebuild, Manual, Blocked, or Advisory items. The scan-level
/// rule is the caller's: Quick Clean is the one path that deletes without a
/// per-item review, so it runs only on a scan that observed everything
/// (`CleanupIntent::QuickSafe`), while a user-reviewed selection may proceed
/// from a partial scan whose items the user looked at.
pub fn select_quick_clean_safe_candidates(
    scan: &ScanResult,
    settings: &ZenithSettings,
) -> Vec<String> {
    let mut eligible_ids = Vec::new();
    for category in &scan.categories {
        if !settings.is_category_clean_enabled(category.category) {
            continue;
        }
        for item in &category.items {
            if item.has_current_disposition()
                && item.disposition.eligibility == CleanupEligibility::AutoCleanable
                && item.cleanable_bytes() > 0
            {
                eligible_ids.push(item.id.clone());
            }
        }
    }
    eligible_ids
}

/// The projection of one planner refusal onto the interface contract.
fn refusal_preview(refusal: &crate::models::PlanItemRefusal) -> PlanRefusalPreview {
    PlanRefusalPreview {
        item_id: refusal.item_id.clone(),
        name: refusal.item_name.clone(),
        reason: refusal.reason,
        message: refusal.message.clone(),
    }
}

/// Maps a planning failure onto the scope the interface reacts to.
///
/// The distinction the interface cannot make for itself: a refusal that names
/// items leaves the inventory and every other selection usable, while a scan
/// that is no longer current makes the inventory a description of a machine
/// that has changed. Reading both as the same failure is what made a correct
/// refusal look like a broken selection.
fn plan_failure(error: ZenithError) -> CleanupFailure {
    match error {
        ZenithError::RefusedSelection(refusals) => CleanupFailure::items(
            "Nothing in the selection can be cleaned right now.",
            refusals.iter().map(refusal_preview).collect(),
        ),
        ZenithError::ChangedSinceScan(message) => CleanupFailure::inventory_stale(message),
        ZenithError::UnsupportedManualOperation(name) => CleanupFailure::items(
            format!(
                "`{name}` is reported for information only; this build has no reviewed operation that removes it"
            ),
            Vec::new(),
        ),
        other => CleanupFailure::new(
            CleanupFailureScope::Internal,
            CleanFailureReason::Unknown,
            other.to_string(),
        ),
    }
}

/// Maps a plan-store refusal onto the scope the interface reacts to.
fn store_failure(error: PlanStoreError) -> CleanupFailure {
    match error {
        PlanStoreError::Unavailable(message) => CleanupFailure::inventory_stale(message),
        PlanStoreError::Unusable(message) => CleanupFailure::new(
            CleanupFailureScope::Internal,
            CleanFailureReason::Unknown,
            message,
        ),
    }
}

/// The intent for a cleanup operation.
///
/// Main Clean and Quick Clean are intents routed through the same
/// unified security, validation, invalidation, and execution pipeline.
#[derive(Debug)]
enum CleanupIntent {
    ReviewedSelection { plan_id: Uuid, confirmed: bool },
    QuickSafe,
}

/// Application service coordinating scanning, safety planning, and execution.
///
/// Manages operation serialization, execution budgets, plan TTLs, and scan invalidation
/// centrally so command handlers remain thin IPC adapters.
pub struct CleanupService {
    scan_service: Arc<ScanService>,
    plan_store: Arc<PlanStore<DeletePlan>>,
    scan_store: Arc<ScanStore>,
    operation_gate: StorageOperationGate,
    budgets: Arc<ExecutionBudgets>,
    environment: Arc<PlatformEnvironment>,
    registry: Arc<SignatureRegistry>,
    docker_status_cache: Arc<DockerStatusCache>,
    lifecycle_providers: Arc<LifecycleProviderRegistry>,
    owner_providers: Arc<OwnerProviderRegistry>,
    /// Native Trash / Recycle Bin adapter used only after the cleanup safety
    /// guard minted a validated non-Safe filesystem target.
    trash_backend: Arc<dyn zenith_platform::TrashBackend>,
    /// The cancellation handles of the scans this service is running, keyed by
    /// the id each scan reports so `cancel_scan` can reach one in flight.
    scan_cancellations: Arc<CancellationRegistry>,
    platform_capabilities: Arc<dyn PlatformCapabilitiesProvider>,
}

impl CleanupService {
    const SCAN_CATEGORY_SLICE_LIMIT: usize = 2;
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        scan_service: Arc<ScanService>,
        plan_store: Arc<PlanStore<DeletePlan>>,
        scan_store: Arc<ScanStore>,
        operation_gate: StorageOperationGate,
        budgets: Arc<ExecutionBudgets>,
        environment: Arc<PlatformEnvironment>,
        registry: Arc<SignatureRegistry>,
        docker_status_cache: Arc<DockerStatusCache>,
        lifecycle_providers: Arc<LifecycleProviderRegistry>,
        owner_providers: Arc<OwnerProviderRegistry>,
        trash_backend: Arc<dyn zenith_platform::TrashBackend>,
        platform_capabilities: Arc<dyn PlatformCapabilitiesProvider>,
    ) -> Self {
        Self {
            scan_service,
            plan_store,
            scan_store,
            operation_gate,
            budgets,
            environment,
            registry,
            docker_status_cache,
            lifecycle_providers,
            owner_providers,
            trash_backend,
            scan_cancellations: Arc::new(CancellationRegistry::for_scans()),
            platform_capabilities,
        }
    }

    /// Runs a full cleanup scan under the storage operation gate and execution budgets.
    pub async fn start_scan(
        &self,
        request: ScanRequest,
        progress: Arc<dyn ScanProgressSink>,
    ) -> Result<PublishedScan, String> {
        self.start_scan_with_limit(request, progress, Self::SCAN_CATEGORY_SLICE_LIMIT)
            .await
    }

    /// Runs to exhaustion for a surface that is intentionally not granted the
    /// resume command, such as the Quick Panel.
    pub async fn start_scan_complete(
        &self,
        request: ScanRequest,
        progress: Arc<dyn ScanProgressSink>,
    ) -> Result<PublishedScan, String> {
        self.start_scan_with_limit(request, progress, usize::MAX)
            .await
    }

    async fn start_scan_with_limit(
        &self,
        request: ScanRequest,
        progress: Arc<dyn ScanProgressSink>,
        slice_limit: usize,
    ) -> Result<PublishedScan, String> {
        self.platform_capabilities
            .capabilities()
            .require(
                PlatformFeature::Cleanup,
                crate::models::CapabilityAccess::Inspect,
            )
            .map_err(|e| e.to_string())?;

        if request.intensive_cleanup {
            self.platform_capabilities
                .capabilities()
                .require(
                    PlatformFeature::IntensiveCleanup,
                    crate::models::CapabilityAccess::Inspect,
                )
                .map_err(|e| e.to_string())?;
        }

        self.scan_cancellations.request_all();
        let lease = self.scan_store.begin();
        let mut categories = request.categories.clone().unwrap_or_else(|| {
            vec![
                crate::models::Category::Ai,
                crate::models::Category::Developer,
                crate::models::Category::Container,
                crate::models::Category::System,
            ]
        });
        let remaining = if categories.len() > slice_limit {
            categories.split_off(slice_limit)
        } else {
            Vec::new()
        };
        let mut pass_request = request.clone();
        pass_request.categories = Some(categories);
        let result = match self.run_scan_pass(pass_request, progress).await {
            Ok(result) => result,
            Err(error) => {
                self.scan_store.stop(lease, error.clone());
                return Err(error);
            }
        };
        let checkpoint = (!result.cancelled && !remaining.is_empty()).then(|| ScanCheckpoint {
            request,
            remaining_categories: remaining,
            slices: vec![result.clone()],
            freshness_anchor: result.finished_at,
        });
        if result.cancelled {
            self.scan_store
                .publish_stopped(lease, result, "Scan was cancelled before completion.")
        } else {
            self.scan_store.publish(lease, result, checkpoint)
        }
    }

    /// Continues exactly the backend checkpoint named by the current scan.
    pub async fn resume_scan(
        &self,
        request: ResumeScanRequest,
        progress: Arc<dyn ScanProgressSink>,
    ) -> Result<PublishedScan, String> {
        self.platform_capabilities
            .capabilities()
            .require(
                PlatformFeature::Cleanup,
                crate::models::CapabilityAccess::Inspect,
            )
            .map_err(|error| error.to_string())?;
        let claimed = self.scan_store.claim(&request, unix_timestamp())?;
        let mut checkpoint = claimed.checkpoint;
        let mut categories = std::mem::take(&mut checkpoint.remaining_categories);
        let remaining = if categories.len() > Self::SCAN_CATEGORY_SLICE_LIMIT {
            categories.split_off(Self::SCAN_CATEGORY_SLICE_LIMIT)
        } else {
            Vec::new()
        };
        let mut pass_request = checkpoint.request.clone();
        pass_request.categories = Some(categories);
        let result = match self.run_scan_pass(pass_request, progress).await {
            Ok(result) => result,
            Err(error) => {
                self.scan_store.stop(claimed.lease, error.clone());
                return Err(error);
            }
        };
        checkpoint.slices.push(result.clone());
        let Some(mut merged) = self.scan_service.merge_slices(&checkpoint.slices) else {
            let error = "The retained scan contained no observations.".to_string();
            self.scan_store.stop(claimed.lease, error.clone());
            return Err(error);
        };
        merged.finished_at = checkpoint.freshness_anchor;
        checkpoint.remaining_categories = remaining;
        let next = (!result.cancelled && !checkpoint.remaining_categories.is_empty())
            .then_some(checkpoint);
        if result.cancelled {
            self.scan_store.publish_stopped(
                claimed.lease,
                merged,
                "Scan was cancelled before completion.",
            )
        } else {
            self.scan_store.publish(claimed.lease, merged, next)
        }
    }

    async fn run_scan_pass(
        &self,
        request: ScanRequest,
        progress: Arc<dyn ScanProgressSink>,
    ) -> Result<ScanResult, String> {
        let permit = self.budgets.acquire_storage_read().await?;
        let scan_service = self.scan_service.clone();
        let operation_gate = self.operation_gate.clone();
        let cancellations = self.scan_cancellations.clone();
        crate::blocking::run_blocking(
            move || -> Result<_, String> {
                let _permit = permit;
                Ok(operation_gate.run_read(|| {
                    let signal = Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let registered: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
                    let probe = ScanCancellation::new(signal.clone());
                    let sink = |event: ScanEvent| {
                        if let ScanEvent::Started { scan_id } = &event {
                            *registered
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                                Some(scan_id.clone());
                            cancellations.register(scan_id.clone(), signal.clone());
                        }
                        progress.emit(event);
                    };
                    let result = scan_service.scan(&request, &sink, &probe);
                    let finished = registered
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take();
                    if let Some(scan_id) = finished.as_deref() {
                        cancellations.remove(scan_id);
                    }
                    result
                }))
            },
            "Scan worker panicked",
        )
        .await
    }

    /// Returns the current cached scan result if available.
    pub fn get_last_scan(&self) -> Option<PublishedScan> {
        self.scan_store.get_published()
    }

    /// Requests cancellation of the scan reporting `scan_id`.
    ///
    /// A scan that already finished is not an error: there is nothing to stop,
    /// and saying so as a failure would report a normal race as a broken
    /// command. The caller sees the same answer either way, and the scan's own
    /// result is what states whether it was cancelled.
    pub fn cancel_scan(&self, scan_id: &str) -> Result<(), String> {
        self.scan_cancellations.request(scan_id);
        Ok(())
    }

    /// Creates and stores a verified cleanup plan from user-reviewed item IDs.
    ///
    /// The refusal scope is the contract's point: an item a current policy
    /// declines is reported per item, and only a scan that is no longer current
    /// invalidates the inventory the interface holds.
    pub async fn create_delete_plan(
        &self,
        scan_id: String,
        selected_item_ids: Vec<String>,
    ) -> Result<PlanPreview, CleanupFailure> {
        self.platform_capabilities
            .capabilities()
            .require(
                PlatformFeature::Cleanup,
                crate::models::CapabilityAccess::Mutate,
            )
            .map_err(|error| {
                CleanupFailure::new(
                    CleanupFailureScope::Permission,
                    CleanFailureReason::PermissionDenied,
                    error.to_string(),
                )
            })?;

        let scan_store = self.scan_store.clone();
        let plan_store = self.plan_store.clone();
        let registry = self.registry.clone();
        let environment = self.environment.clone();
        let owner_providers = self.owner_providers.clone();

        crate::blocking::run_blocking(
            move || -> Result<PlanPreview, CleanupFailure> {
                let scan = scan_store
                    .get()
                    .filter(|scan| scan.scan_id == scan_id)
                    .ok_or_else(|| {
                        CleanupFailure::inventory_stale(
                            "The scan is no longer current. Scan again before cleaning.",
                        )
                    })?;

                let plan = SafetyPlanner::create_plan_from_scan(
                    &scan,
                    &scan_id,
                    &selected_item_ids,
                    &registry,
                    &environment,
                    &owner_providers,
                )
                .map_err(plan_failure)?;

                let ttl = plan_store.ttl_seconds();
                let mut preview = plan.preview(ttl);
                preview.expires_at = preview.expires_at.min(
                    scan.finished_at
                        .saturating_add(u64::from(ScanResult::VALID_FOR_SECONDS)),
                );

                let now = unix_timestamp();
                plan_store.insert(plan, now).map_err(store_failure)?;
                Ok(preview)
            },
            "Delete plan worker panicked",
        )
        .await
    }

    /// Executes a reviewed DeletePlan by plan_id.
    pub async fn execute_clean(
        &self,
        plan_id: Uuid,
        confirmed: bool,
        progress: Arc<dyn CleanupProgressSink>,
    ) -> Result<CleanResult, CleanupFailure> {
        self.execute_intent(
            CleanupIntent::ReviewedSelection { plan_id, confirmed },
            None,
            progress,
        )
        .await
    }

    /// Executes Quick Clean for the Safe subset of a complete, current scan.
    ///
    /// The scan is required to be `Fresh`: this path deletes without a per-item
    /// review, so a scan that may have missed items cannot authorize it. A
    /// user-reviewed selection is the path that accepts a partial scan.
    pub async fn quick_clean_safe(
        &self,
        settings: &ZenithSettings,
        progress: Arc<dyn CleanupProgressSink>,
    ) -> Result<CleanResult, CleanupFailure> {
        self.execute_intent(CleanupIntent::QuickSafe, Some(settings.clone()), progress)
            .await
    }

    /// Internal execution core ensuring identical security and lifecycle semantics for all clean intents.
    async fn execute_intent(
        &self,
        intent: CleanupIntent,
        settings: Option<ZenithSettings>,
        progress: Arc<dyn CleanupProgressSink>,
    ) -> Result<CleanResult, CleanupFailure> {
        self.platform_capabilities
            .capabilities()
            .require(
                PlatformFeature::Cleanup,
                crate::models::CapabilityAccess::Mutate,
            )
            .map_err(|error| {
                CleanupFailure::new(
                    CleanupFailureScope::Permission,
                    CleanFailureReason::PermissionDenied,
                    error.to_string(),
                )
            })?;

        let plan_store = self.plan_store.clone();
        let scan_store = self.scan_store.clone();
        let operation_gate = self.operation_gate.clone();
        let environment = self.environment.clone();
        let registry = self.registry.clone();
        let docker_status_cache = self.docker_status_cache.clone();
        let lifecycle_providers = self.lifecycle_providers.clone();
        let owner_providers = self.owner_providers.clone();
        let trash_backend = self.trash_backend.clone();

        crate::blocking::run_blocking(
            move || -> Result<CleanResult, CleanupFailure> {
                operation_gate.run_write(|| -> Result<CleanResult, CleanupFailure> {
                    let now = unix_timestamp();

                    let plan: DeletePlan = match intent {
                        CleanupIntent::ReviewedSelection { plan_id, confirmed } => {
                            let plan = plan_store.take_valid(plan_id, now).map_err(store_failure)?;
                            if plan.requires_confirmation() && !confirmed {
                                return Err(CleanupFailure::new(
                                    CleanupFailureScope::Internal,
                                    CleanFailureReason::Unknown,
                                    "This cleanup includes an action that requires explicit confirmation. Review the plan and confirm it before cleaning.",
                                ));
                            }
                            // Invalidate scan atomically so pre-cleanup inventory cannot be reused
                            scan_store.validate_and_invalidate_for_cleanup(&plan.scan_id, now)?;
                            plan
                        }
                        CleanupIntent::QuickSafe => {
                            let settings = settings
                                .ok_or_else(|| "Settings required for Quick Clean".to_string())?;
                            let scan = scan_store.get().ok_or_else(|| {
                                CleanupFailure::inventory_stale(
                                    "The scan is no longer current. Scan again before cleaning.",
                                )
                            })?;

                            scan.validate_for_cleanup(&scan.scan_id, now)
                                .map_err(|error| {
                                    CleanupFailure::inventory_stale(error.to_string())
                                })?;

                            // Quick Clean deletes the Safe subset without a
                            // per-item review, so it requires a scan that observed
                            // everything. A partial scan may have missed items and
                            // cannot prove the machine's state; a user-reviewed
                            // selection is different, because the user saw exactly
                            // what was inspected and chose from it.
                            if scan.quality != ObservationQuality::Fresh {
                                return Err(CleanupFailure::inventory_stale(
                                    "Quick Clean needs a complete scan. Scan again before cleaning.",
                                ));
                            }

                            let eligible_ids = select_quick_clean_safe_candidates(&scan, &settings);
                            if eligible_ids.is_empty() {
                                return Ok(CleanResult {
                                    plan_id: Uuid::new_v4(),
                                    started_at: now,
                                    finished_at: now,
                                    total_reclaimed_bytes: 0,
                                    total_moved_to_trash_bytes: 0,
                                    total_failed_bytes: 0,
                                    partial_count: 0,
                                    failed_count: 0,
                                    skipped_count: 0,
                                    items: vec![],
                                    actual_disk_free_delta: Some(0),
                                });
                            }

                            let plan = SafetyPlanner::create_plan_from_scan(
                                &scan,
                                &scan.scan_id,
                                &eligible_ids,
                                &registry,
                                &environment,
                                &owner_providers,
                            )
                            .map_err(plan_failure)?;

                            if plan.requires_confirmation() {
                                return Err(CleanupFailure::new(
                                    CleanupFailureScope::Internal,
                                    CleanFailureReason::Unknown,
                                    "Quick Clean cannot execute actions that require explicit confirmation.",
                                ));
                            }

                            // Invalidate scan atomically
                            scan_store
                                .validate_and_invalidate_for_cleanup(&plan.scan_id, now)
                                .map_err(CleanupFailure::inventory_stale)?;
                            plan
                        }
                    };

                    // Only a plan that authorizes a mutation may reach the
                    // executor. A projection the user reviewed and a deletion
                    // are different claims, and the plan states which one it is.
                    if !plan.mode.is_mutating() {
                        return Err(CleanupFailure::new(
                            CleanupFailureScope::Internal,
                            CleanFailureReason::Unknown,
                            format!(
                                "This plan is {} and cannot be executed",
                                plan.mode.display_name()
                            ),
                        ));
                    }

                    // A Docker prune may have changed the runtime's state, so the
                    // shared observation is dropped rather than reported stale.
                    if plan
                        .targets
                        .iter()
                        .any(|t| t.strategy == CleanStrategy::DockerPrune)
                    {
                        docker_status_cache.invalidate();
                    }

                    Ok(CleanExecutor::execute(
                        plan,
                        &environment,
                        &lifecycle_providers,
                        &owner_providers,
                        trash_backend.as_ref(),
                        move |event: CleanEvent| {
                            progress.emit(event);
                        },
                    ))
                })
            },
            "Cleanup worker panicked",
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache_providers::{CacheProviderScan, CacheProviderScanner};
    use crate::models::{
        Category, CategoryResult, FileSize, ObservationQuality, PlatformCapabilities, RiskTier,
        ScanDiscovery, ScanItem, Signature,
    };
    use crate::scanner::{ScanLimits, TraversalCounters};
    use std::sync::Mutex;
    use zenith_platform::path_algebra::PathFlavor;

    struct TestCapabilitiesProvider(PlatformCapabilities);

    #[derive(Default)]
    struct RecordingCacheProviders {
        excluded: Mutex<Vec<Vec<String>>>,
    }

    impl CacheProviderScanner for RecordingCacheProviders {
        fn scan_items(
            &self,
            _registry: &SignatureRegistry,
            excluded_signatures: &[String],
            _environment: &PlatformEnvironment,
            _cancellation: &dyn crate::models::CancellationProbe,
            _limits: ScanLimits,
            _counters: &TraversalCounters,
            _progress: &dyn crate::scanner::RootProgressSink,
        ) -> CacheProviderScan {
            self.excluded
                .lock()
                .unwrap()
                .push(excluded_signatures.to_vec());
            CacheProviderScan::default()
        }
    }

    impl PlatformCapabilitiesProvider for TestCapabilitiesProvider {
        fn capabilities(&self) -> PlatformCapabilities {
            self.0.clone()
        }
    }

    fn make_test_item(id: &str, risk: RiskTier, bytes: u64) -> ScanItem {
        let mut item = ScanItem::mock(
            id,
            "test_sig",
            "Test Item",
            Category::System,
            risk,
            "/tmp/test",
            FileSize::new(bytes, Some(bytes)),
            1,
        );
        item.disposition = item.derive_disposition();
        item
    }

    fn make_test_scan(items: Vec<ScanItem>) -> ScanResult {
        make_test_scan_with_quality(items, ObservationQuality::Fresh)
    }

    /// A scan whose overall quality is stated, so a caller can present the
    /// partial observations a cancelled or bounded scan produces.
    fn make_test_scan_with_quality(
        items: Vec<ScanItem>,
        quality: ObservationQuality,
    ) -> ScanResult {
        let cleanable = items.iter().map(|i| i.cleanable_bytes()).sum();
        let now = unix_timestamp();
        ScanResult {
            cancelled: false,
            metrics: Default::default(),
            scan_id: "scan_123".to_string(),
            valid_for_seconds: ScanResult::VALID_FOR_SECONDS,
            started_at: now.saturating_sub(5),
            finished_at: now,
            total_bytes: cleanable,
            cleanable_bytes: cleanable,
            safe_bytes: cleanable,
            rebuild_bytes: 0,
            manual_bytes: 0,
            categories: vec![CategoryResult {
                category: Category::System,
                display_name: Category::System.display_name().to_string(),
                total_bytes: cleanable,
                cleanable_bytes: cleanable,
                safe_bytes: cleanable,
                rebuild_bytes: 0,
                manual_bytes: 0,
                quality,
                skipped_entry_count: 0,
                incomplete_item_count: 0,
                eligibility: Default::default(),
                suppressed_duplicate_count: 0,
                suppressed_duplicate_bytes: 0,
                suppressed_overlap_count: 0,
                suppressed_overlap_bytes: 0,
                ambiguous_overlap_count: 0,
                ambiguous_overlap_bytes: 0,
                items,
            }],
            incomplete_reasons: Vec::new(),
            gaps: Vec::new(),
            quality,
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

    #[test]
    fn select_quick_clean_filters_categories_and_risk() {
        let safe_item = make_test_item("item_safe", RiskTier::Safe, 100);
        let rebuild_item = make_test_item("item_rebuild", RiskTier::Rebuild, 200);
        let scan = make_test_scan(vec![safe_item, rebuild_item]);

        let settings = ZenithSettings::default();
        let selected = select_quick_clean_safe_candidates(&scan, &settings);
        assert_eq!(selected, vec!["item_safe"]);
    }

    #[tokio::test]
    async fn resumed_developer_scan_keeps_the_original_provider_exclusions() {
        let environment = Arc::new(
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(
                if PathFlavor::current().is_windows() {
                    r"Z:\ZenithFixtureHome"
                } else {
                    "/zenith-fixture-home"
                },
            ),
        );
        let registry = Arc::new(SignatureRegistry::new());
        let lifecycle = Arc::new(crate::cleaner::LifecycleProviderRegistry::new(Vec::new()));
        let owners = Arc::new(crate::cleaner::OwnerProviderRegistry::new(Vec::new()));
        let cache_providers = Arc::new(RecordingCacheProviders::default());
        let scan_service = Arc::new(ScanService::new_with_cache_providers(
            registry.clone(),
            lifecycle.clone(),
            owners.clone(),
            cache_providers.clone(),
            environment.clone(),
        ));
        let service = CleanupService::new(
            scan_service,
            Arc::new(PlanStore::new(crate::services::PlanLifecycle::cleanup())),
            Arc::new(ScanStore::new()),
            StorageOperationGate::default(),
            Arc::new(ExecutionBudgets::new()),
            environment,
            registry,
            Arc::new(DockerStatusCache::new()),
            lifecycle,
            owners,
            Arc::new(zenith_platform::MockTrashBackend::new()),
            Arc::new(TestCapabilitiesProvider(PlatformCapabilities::current())),
        );
        let progress: Arc<dyn ScanProgressSink> = Arc::new(|_: ScanEvent| {});
        let excluded = vec!["dev.uv.cache".to_string()];

        let first = service
            .start_scan(
                ScanRequest {
                    categories: Some(vec![Category::System, Category::Ai, Category::Developer]),
                    excluded_signatures: excluded.clone(),
                    intensive_cleanup: false,
                },
                progress.clone(),
            )
            .await
            .unwrap();
        assert!(cache_providers.excluded.lock().unwrap().is_empty());
        let ScanDiscovery::Paused { continuation_id } = first.discovery else {
            panic!("the developer category should remain for the resumed slice")
        };

        let completed = service
            .resume_scan(
                ResumeScanRequest {
                    scan_id: first.result.scan_id,
                    continuation_id,
                },
                progress,
            )
            .await
            .unwrap();

        assert_eq!(completed.discovery, ScanDiscovery::Exhausted);
        assert_eq!(
            cache_providers.excluded.lock().unwrap().as_slice(),
            &[excluded]
        );
    }

    /// A cancel requested while a scan runs stops it, and the scan says so:
    /// the result is partial with a cancellation reason, the categories that
    /// had not started are absent, and the flag distinguishes it from any other
    /// incomplete scan.
    #[tokio::test]
    async fn cancelling_a_running_scan_stops_it_and_the_result_says_so() {
        struct CancelOnFirstItem {
            service: Arc<CleanupService>,
        }

        impl ScanProgressSink for CancelOnFirstItem {
            fn emit(&self, event: crate::models::ScanEvent) {
                if let crate::models::ScanEvent::Started { scan_id } = &event {
                    // The cancel is requested as soon as the scan states which
                    // scan it is, exactly as the command does from the UI.
                    self.service
                        .cancel_scan(scan_id)
                        .expect("a cancel request is accepted");
                }
            }
        }

        let fixture = tempfile::tempdir().unwrap();
        let first_root = fixture.path().join("first-cache");
        let second_root = fixture.path().join("second-cache");
        std::fs::create_dir_all(&first_root).unwrap();
        std::fs::create_dir_all(&second_root).unwrap();
        std::fs::write(first_root.join("data.bin"), vec![1u8; 512]).unwrap();
        std::fs::write(second_root.join("data.bin"), vec![2u8; 512]).unwrap();

        let env = Arc::new(
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(
                if PathFlavor::current().is_windows() {
                    r"Z:\ZenithFixtureHome"
                } else {
                    "/zenith-fixture-home"
                },
            ),
        );
        let mut registry = SignatureRegistry::new();
        for (id, name, path) in [
            ("test.cancel.first", "First cache", first_root),
            ("test.cancel.second", "Second cache", second_root),
        ] {
            registry.register(Signature {
                id: id.to_string(),
                name: name.to_string(),
                category: Category::Developer,
                risk: RiskTier::Safe,
                strategy: CleanStrategy::DeleteDirectory,
                paths: vec![path.to_string_lossy().into_owned()],
                exclusions: Vec::new(),
                description: "test-only signature".to_string(),
                min_age_days: None,
                include_prefixes: Vec::new(),
                exclude_prefixes: Vec::new(),
                intensive_only: false,
                platforms: Vec::new(),
                discovery: Default::default(),
                unit: None,
                owner: String::new(),
                priority: 0,
                fail_if_running: Vec::new(),
                provider: String::new(),
                provider_id: None,
                artifact_kind: Default::default(),
                consequence: String::new(),
            });
        }
        let registry = Arc::new(registry);
        let providers = Arc::new(crate::cleaner::LifecycleProviderRegistry::new(Vec::new()));
        let owner_providers = Arc::new(crate::cleaner::OwnerProviderRegistry::new(Vec::new()));
        let scan_service = Arc::new(ScanService::new(
            registry.clone(),
            providers.clone(),
            owner_providers.clone(),
            env.clone(),
        ));
        let service = Arc::new(CleanupService::new(
            scan_service,
            Arc::new(PlanStore::new(crate::services::PlanLifecycle::cleanup())),
            Arc::new(ScanStore::new()),
            StorageOperationGate::default(),
            Arc::new(ExecutionBudgets::new()),
            env.clone(),
            registry,
            Arc::new(DockerStatusCache::new()),
            providers,
            owner_providers,
            Arc::new(zenith_platform::MockTrashBackend::new()),
            Arc::new(TestCapabilitiesProvider(PlatformCapabilities::current())),
        ));

        let result = service
            .start_scan(
                ScanRequest {
                    categories: Some(vec![Category::Developer]),
                    excluded_signatures: Vec::new(),
                    intensive_cleanup: false,
                },
                Arc::new(CancelOnFirstItem {
                    service: service.clone(),
                }),
            )
            .await
            .expect("a cancelled scan still returns its partial result");

        assert!(
            result.result.cancelled,
            "the result states that it was cancelled: {result:?}"
        );
        assert_eq!(result.result.quality, ObservationQuality::Partial);
        assert!(result
            .result
            .incomplete_reasons
            .iter()
            .any(|reason| reason.contains("cancelled")));
        assert!(
            result.result.categories.is_empty(),
            "the cancelled scan stopped before finishing any category: {result:?}"
        );

        // The finished scan keeps no cancellation handle: requesting it again
        // is a no-op rather than a second cancel of something else.
        assert!(service.cancel_scan(&result.result.scan_id).is_ok());
    }

    #[tokio::test]
    async fn a_bounded_scan_resumes_forward_and_publishes_one_merged_snapshot() {
        let environment = Arc::new(
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(
                if PathFlavor::current().is_windows() {
                    r"Z:\ZenithFixtureHome"
                } else {
                    "/zenith-fixture-home"
                },
            ),
        );
        let registry = Arc::new(SignatureRegistry::new());
        let lifecycle = Arc::new(crate::cleaner::LifecycleProviderRegistry::new(Vec::new()));
        let owners = Arc::new(crate::cleaner::OwnerProviderRegistry::new(Vec::new()));
        let scan_service = Arc::new(ScanService::new(
            registry.clone(),
            lifecycle.clone(),
            owners.clone(),
            environment.clone(),
        ));
        let service = CleanupService::new(
            scan_service,
            Arc::new(PlanStore::new(crate::services::PlanLifecycle::cleanup())),
            Arc::new(ScanStore::new()),
            StorageOperationGate::default(),
            Arc::new(ExecutionBudgets::new()),
            environment,
            registry,
            Arc::new(DockerStatusCache::new()),
            lifecycle,
            owners,
            Arc::new(zenith_platform::MockTrashBackend::new()),
            Arc::new(TestCapabilitiesProvider(PlatformCapabilities::current())),
        );
        let progress: Arc<dyn ScanProgressSink> = Arc::new(|_: ScanEvent| {});

        let first = service
            .start_scan(ScanRequest::default(), progress.clone())
            .await
            .expect("the first bounded pass publishes");
        let ScanDiscovery::Paused { continuation_id } = first.discovery else {
            panic!("the default four-category scan should pause after two categories")
        };
        assert_eq!(first.result.categories.len(), 2);
        let freshness_anchor = first.result.finished_at;

        let completed = service
            .resume_scan(
                ResumeScanRequest {
                    scan_id: first.result.scan_id,
                    continuation_id,
                },
                progress,
            )
            .await
            .expect("the retained pass resumes");

        assert_eq!(completed.discovery, ScanDiscovery::Exhausted);
        assert_eq!(completed.result.categories.len(), 4);
        assert_eq!(completed.result.finished_at, freshness_anchor);
        assert_eq!(
            completed
                .result
                .categories
                .iter()
                .map(|category| category.category)
                .collect::<Vec<_>>(),
            vec![
                Category::Ai,
                Category::Developer,
                Category::Container,
                Category::System,
            ]
        );
    }

    #[tokio::test]
    async fn cleanup_service_plan_and_execution_lifecycle() {
        let env = Arc::new(
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(
                if PathFlavor::current().is_windows() {
                    r"Z:\ZenithFixtureHome"
                } else {
                    "/zenith-fixture-home"
                },
            ),
        );
        let registry = Arc::new(SignatureRegistry::new());
        let owner_providers = Arc::new(crate::cleaner::OwnerProviderRegistry::new(Vec::new()));
        let scan_service = Arc::new(ScanService::new(
            registry.clone(),
            Arc::new(crate::cleaner::LifecycleProviderRegistry::new(Vec::new())),
            owner_providers.clone(),
            env.clone(),
        ));
        let plan_store = Arc::new(PlanStore::new(crate::services::PlanLifecycle::cleanup()));
        let scan_store = Arc::new(ScanStore::new());
        let operation_gate = StorageOperationGate::default();
        let budgets = Arc::new(ExecutionBudgets::new());
        let docker_cache = Arc::new(DockerStatusCache::new());
        let capabilities = Arc::new(TestCapabilitiesProvider(PlatformCapabilities::current()));

        let service = CleanupService::new(
            scan_service,
            plan_store.clone(),
            scan_store.clone(),
            operation_gate,
            budgets,
            env,
            registry,
            docker_cache,
            Arc::new(crate::cleaner::LifecycleProviderRegistry::new(Vec::new())),
            owner_providers,
            Arc::new(zenith_platform::MockTrashBackend::new()),
            capabilities,
        );

        // Populate a scan
        let item = make_test_item("item1", RiskTier::Safe, 500);
        let scan = make_test_scan(vec![item]);
        scan_store.set(scan.clone());

        assert_eq!(service.get_last_scan().unwrap().result.scan_id, "scan_123");

        // Requesting plan with unknown item fails
        let err = service
            .create_delete_plan("scan_123".to_string(), vec!["unknown".to_string()])
            .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn a_confirmation_required_provider_cannot_execute_without_confirmation() {
        use crate::cleaner::providers::test_support::StatedProvider;

        let env = Arc::new(
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(
                if PathFlavor::current().is_windows() {
                    r"Z:\ZenithFixtureHome"
                } else {
                    "/zenith-fixture-home"
                },
            ),
        );
        let mut registry = SignatureRegistry::new();
        registry.register(Signature {
            id: "test.stated.store".to_string(),
            name: "Stated Store".to_string(),
            category: Category::System,
            risk: RiskTier::Manual,
            strategy: CleanStrategy::LifecycleProvider,
            paths: Vec::new(),
            exclusions: Vec::new(),
            description: "A provider-owned store.".to_string(),
            min_age_days: None,
            include_prefixes: Vec::new(),
            exclude_prefixes: Vec::new(),
            intensive_only: false,
            platforms: vec![crate::models::PlatformKind::current()],
            discovery: Default::default(),
            unit: None,
            owner: String::new(),
            priority: 0,
            fail_if_running: Vec::new(),
            provider: "Stated Owner".to_string(),
            provider_id: Some("test.stated".to_string()),
            artifact_kind: Default::default(),
            consequence: String::new(),
        });
        let registry = Arc::new(registry);
        let providers = Arc::new(crate::cleaner::LifecycleProviderRegistry::new(vec![
            StatedProvider::holding(2_048, 1).shared(),
        ]));
        let owner_providers = Arc::new(crate::cleaner::OwnerProviderRegistry::new(Vec::new()));
        let scan_service = Arc::new(ScanService::new(
            registry.clone(),
            providers.clone(),
            owner_providers.clone(),
            env.clone(),
        ));
        let plan_store = Arc::new(PlanStore::new(crate::services::PlanLifecycle::cleanup()));
        let scan_store = Arc::new(ScanStore::new());
        let service = CleanupService::new(
            scan_service,
            plan_store,
            scan_store.clone(),
            StorageOperationGate::default(),
            Arc::new(ExecutionBudgets::new()),
            env.clone(),
            registry.clone(),
            Arc::new(DockerStatusCache::new()),
            providers.clone(),
            owner_providers,
            Arc::new(zenith_platform::MockTrashBackend::new()),
            Arc::new(TestCapabilitiesProvider(PlatformCapabilities::current())),
        );

        let items = providers.scan_items(&registry, Category::System, false, &[], &env);
        assert_eq!(items.len(), 1);
        assert!(items[0].requires_confirmation);
        assert_eq!(
            items[0].disposition.eligibility,
            crate::models::CleanupEligibility::Reviewable
        );
        scan_store.set(make_test_scan(items));

        let preview = service
            .create_delete_plan(
                "scan_123".to_string(),
                vec!["test.stated.store".to_string()],
            )
            .await
            .expect("provider selection creates a plan");
        assert!(preview.requires_confirmation);
        assert!(preview.targets[0].requires_confirmation);

        let progress: Arc<dyn CleanupProgressSink> = Arc::new(|_| {});
        let refused = service
            .execute_clean(preview.id, false, progress.clone())
            .await
            .expect_err("confirmation-required plan must fail closed");
        assert!(
            refused.message.contains("explicit confirmation"),
            "{}",
            refused.message
        );

        let confirmed_preview = service
            .create_delete_plan(
                "scan_123".to_string(),
                vec!["test.stated.store".to_string()],
            )
            .await
            .expect("the unconfirmed refusal leaves the scan available for review");
        let result = service
            .execute_clean(confirmed_preview.id, true, progress)
            .await
            .expect("a confirmed provider action executes");
        assert_eq!(result.failed_count, 0);
        assert_eq!(result.total_reclaimed_bytes, 2_048);
    }

    #[tokio::test]
    async fn a_consumed_plan_cannot_be_replayed_through_the_service() {
        let fixture = tempfile::tempdir().unwrap();
        let cache = fixture.path().join("fixture-cache");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("payload.bin"), vec![7u8; 512]).unwrap();

        let env = Arc::new(
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(
                if PathFlavor::current().is_windows() {
                    r"Z:\ZenithFixtureHome"
                } else {
                    "/zenith-fixture-home"
                },
            ),
        );
        let mut registry = SignatureRegistry::new();
        registry.register(Signature {
            id: "test_sig".to_string(),
            name: "Fixture cache".to_string(),
            category: Category::System,
            risk: RiskTier::Safe,
            strategy: CleanStrategy::DeleteDirectory,
            paths: vec![cache.to_string_lossy().to_string()],
            description: "test-only signature".to_string(),
            min_age_days: None,
            include_prefixes: Vec::new(),
            exclude_prefixes: Vec::new(),
            intensive_only: false,
            platforms: Vec::new(),
            discovery: Default::default(),
            unit: None,
            owner: String::new(),
            priority: 0,
            fail_if_running: Vec::new(),
            provider: String::new(),
            provider_id: None,
            artifact_kind: Default::default(),
            consequence: String::new(),
            exclusions: Vec::new(),
        });
        let registry = Arc::new(registry);

        let owner_providers = Arc::new(crate::cleaner::OwnerProviderRegistry::new(Vec::new()));
        let scan_service = Arc::new(ScanService::new(
            registry.clone(),
            Arc::new(crate::cleaner::LifecycleProviderRegistry::new(Vec::new())),
            owner_providers.clone(),
            env.clone(),
        ));
        let plan_store = Arc::new(PlanStore::new(crate::services::PlanLifecycle::cleanup()));
        let scan_store = Arc::new(ScanStore::new());
        let service = CleanupService::new(
            scan_service,
            plan_store,
            scan_store.clone(),
            StorageOperationGate::default(),
            Arc::new(ExecutionBudgets::new()),
            env,
            registry,
            Arc::new(DockerStatusCache::new()),
            Arc::new(crate::cleaner::LifecycleProviderRegistry::new(Vec::new())),
            owner_providers,
            Arc::new(zenith_platform::MockTrashBackend::new()),
            Arc::new(TestCapabilitiesProvider(PlatformCapabilities::current())),
        );

        let mut item = ScanItem::mock(
            "item1",
            "test_sig",
            "Fixture cache",
            Category::System,
            RiskTier::Safe,
            cache.to_string_lossy().to_string(),
            FileSize::new(512, Some(512)),
            1,
        );
        item.disposition = item.derive_disposition();
        scan_store.set(make_test_scan(vec![item]));

        let preview = service
            .create_delete_plan("scan_123".to_string(), vec!["item1".to_string()])
            .await
            .expect("a reviewed item creates a plan");
        let progress: Arc<dyn CleanupProgressSink> = Arc::new(|_| {});

        let first = service
            .execute_clean(preview.id, false, progress.clone())
            .await;
        assert!(
            first.is_ok(),
            "the first execution of a plan runs: {:?}",
            first.err()
        );
        assert!(!cache.exists(), "the plan's target was deleted");

        // One-shot means the same plan ID cannot authorize a second mutation,
        // through this service or any other caller.
        let replay = service.execute_clean(preview.id, false, progress).await;
        let error = replay.expect_err("a consumed plan must be refused");
        assert!(
            error.message.contains("not found or already used"),
            "unexpected error: {}",
            error.message
        );
    }

    #[tokio::test]
    async fn quick_clean_refuses_a_partial_scan_but_a_reviewed_plan_may_proceed() {
        let fixture = tempfile::tempdir().unwrap();
        let cache = fixture.path().join("fixture-cache");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("payload.bin"), vec![9u8; 256]).unwrap();

        let env = Arc::new(
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(
                if PathFlavor::current().is_windows() {
                    r"Z:\ZenithFixtureHome"
                } else {
                    "/zenith-fixture-home"
                },
            ),
        );
        let mut registry = SignatureRegistry::new();
        registry.register(Signature {
            id: "test_sig".to_string(),
            name: "Fixture cache".to_string(),
            category: Category::System,
            risk: RiskTier::Safe,
            strategy: CleanStrategy::DeleteDirectory,
            paths: vec![cache.to_string_lossy().to_string()],
            description: "test-only signature".to_string(),
            min_age_days: None,
            include_prefixes: Vec::new(),
            exclude_prefixes: Vec::new(),
            intensive_only: false,
            platforms: Vec::new(),
            discovery: Default::default(),
            unit: None,
            owner: String::new(),
            priority: 0,
            fail_if_running: Vec::new(),
            provider: String::new(),
            provider_id: None,
            artifact_kind: Default::default(),
            consequence: String::new(),
            exclusions: Vec::new(),
        });
        let registry = Arc::new(registry);

        let owner_providers = Arc::new(crate::cleaner::OwnerProviderRegistry::new(Vec::new()));
        let scan_service = Arc::new(ScanService::new(
            registry.clone(),
            Arc::new(crate::cleaner::LifecycleProviderRegistry::new(Vec::new())),
            owner_providers.clone(),
            env.clone(),
        ));
        let plan_store = Arc::new(PlanStore::new(crate::services::PlanLifecycle::cleanup()));
        let scan_store = Arc::new(ScanStore::new());
        let service = CleanupService::new(
            scan_service,
            plan_store,
            scan_store.clone(),
            StorageOperationGate::default(),
            Arc::new(ExecutionBudgets::new()),
            env,
            registry,
            Arc::new(DockerStatusCache::new()),
            Arc::new(crate::cleaner::LifecycleProviderRegistry::new(Vec::new())),
            owner_providers,
            Arc::new(zenith_platform::MockTrashBackend::new()),
            Arc::new(TestCapabilitiesProvider(PlatformCapabilities::current())),
        );

        // A partial scan is the state a cancelled or bounded scan leaves behind:
        // some items were observed, and the machine as a whole was not.
        let mut item = ScanItem::mock(
            "item1",
            "test_sig",
            "Fixture cache",
            Category::System,
            RiskTier::Safe,
            cache.to_string_lossy().to_string(),
            FileSize::new(256, Some(256)),
            1,
        );
        item.disposition = item.derive_disposition();
        scan_store.set(make_test_scan_with_quality(
            vec![item],
            ObservationQuality::Partial,
        ));

        let progress: Arc<dyn CleanupProgressSink> = Arc::new(|_| {});
        let settings = ZenithSettings::default();
        let refused = service.quick_clean_safe(&settings, progress).await;
        let error = refused.expect_err("Quick Clean needs a scan that observed everything");
        assert!(
            error.message.contains("complete scan"),
            "unexpected error: {}",
            error.message
        );
        assert!(cache.join("payload.bin").is_file(), "nothing was deleted");

        // The same partial scan still supports a reviewed selection: the user
        // saw which items were inspected and chose one of them.
        let preview = service
            .create_delete_plan("scan_123".to_string(), vec!["item1".to_string()])
            .await
            .expect("a reviewed selection may proceed from a partial scan");
        assert_eq!(preview.targets.len(), 1);
    }

    #[tokio::test]
    async fn quick_clean_safe_handles_empty_candidates_gracefully() {
        let env = Arc::new(
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(
                if PathFlavor::current().is_windows() {
                    r"Z:\ZenithFixtureHome"
                } else {
                    "/zenith-fixture-home"
                },
            ),
        );
        let registry = Arc::new(SignatureRegistry::new());
        let owner_providers = Arc::new(crate::cleaner::OwnerProviderRegistry::new(Vec::new()));
        let scan_service = Arc::new(ScanService::new(
            registry.clone(),
            Arc::new(crate::cleaner::LifecycleProviderRegistry::new(Vec::new())),
            owner_providers.clone(),
            env.clone(),
        ));
        let plan_store = Arc::new(PlanStore::new(crate::services::PlanLifecycle::cleanup()));
        let scan_store = Arc::new(ScanStore::new());
        let operation_gate = StorageOperationGate::default();
        let budgets = Arc::new(ExecutionBudgets::new());
        let docker_cache = Arc::new(DockerStatusCache::new());
        let capabilities = Arc::new(TestCapabilitiesProvider(PlatformCapabilities::current()));

        let service = CleanupService::new(
            scan_service,
            plan_store,
            scan_store.clone(),
            operation_gate,
            budgets,
            env,
            registry,
            docker_cache,
            Arc::new(crate::cleaner::LifecycleProviderRegistry::new(Vec::new())),
            owner_providers,
            Arc::new(zenith_platform::MockTrashBackend::new()),
            capabilities,
        );

        // Scan with only Rebuild item (no Safe auto-cleanable candidates)
        let rebuild_item = make_test_item("item_rebuild", RiskTier::Rebuild, 300);
        let scan = make_test_scan(vec![rebuild_item]);
        scan_store.set(scan);

        let progress = Arc::new(|_| {});
        let settings = ZenithSettings::default();
        let result = service.quick_clean_safe(&settings, progress).await.unwrap();

        assert_eq!(result.total_reclaimed_bytes, 0);
        assert_eq!(result.items.len(), 0);
    }
}
