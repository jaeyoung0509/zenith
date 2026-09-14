use std::sync::Arc;
use std::time::SystemTime;
use uuid::Uuid;

use super::plan_store::PlanStore;
use super::scan_service::ScanService;
use super::scan_store::ScanStore;
use crate::cleaner::CleanExecutor;
use crate::execution_budget::ExecutionBudgets;
use crate::models::{
    CleanEvent, CleanResult, CleanStrategy, CleanupEligibility, CleanupProgressSink, DeletePlan,
    ObservationQuality, PlanPreview, PlatformCapabilitiesProvider, PlatformFeature,
    ScanProgressSink, ScanRequest, ScanResult, ZenithSettings,
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

/// The intent for a cleanup operation.
///
/// Main Clean and Quick Clean are intents routed through the same
/// unified security, validation, invalidation, and execution pipeline.
#[derive(Debug)]
enum CleanupIntent {
    ReviewedSelection { plan_id: Uuid },
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
    platform_capabilities: Arc<dyn PlatformCapabilitiesProvider>,
}

impl CleanupService {
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
            platform_capabilities,
        }
    }

    /// Runs a full cleanup scan under the storage operation gate and execution budgets.
    pub async fn start_scan(
        &self,
        request: ScanRequest,
        progress: Arc<dyn ScanProgressSink>,
    ) -> Result<ScanResult, String> {
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

        let permit = self.budgets.acquire_storage_read().await?;

        let scan_service = self.scan_service.clone();
        let scan_store = self.scan_store.clone();
        let operation_gate = self.operation_gate.clone();

        let result = crate::blocking::run_blocking(
            move || {
                let _permit = permit;
                Ok(operation_gate.run_read(|| {
                    let result = scan_service.scan(
                        &request,
                        progress.as_ref(),
                        &crate::models::NeverCancelled,
                    );
                    scan_store.set(result.clone());
                    result
                }))
            },
            "Scan worker panicked",
        )
        .await?;

        Ok(result)
    }

    /// Returns the current cached scan result if available.
    pub fn get_last_scan(&self) -> Option<ScanResult> {
        self.scan_store.get()
    }

    /// Creates and stores a verified DeletePlan from user-reviewed item IDs.
    pub async fn create_delete_plan(
        &self,
        scan_id: String,
        selected_item_ids: Vec<String>,
    ) -> Result<PlanPreview, String> {
        self.platform_capabilities
            .capabilities()
            .require(
                PlatformFeature::Cleanup,
                crate::models::CapabilityAccess::Mutate,
            )
            .map_err(|e| e.to_string())?;

        let scan_store = self.scan_store.clone();
        let plan_store = self.plan_store.clone();
        let registry = self.registry.clone();
        let environment = self.environment.clone();

        crate::blocking::run_blocking(
            move || {
                let scan = scan_store
                    .get()
                    .filter(|s| s.scan_id == scan_id)
                    .ok_or_else(|| {
                        "The scan is no longer current. Scan again before cleaning.".to_string()
                    })?;

                let plan = SafetyPlanner::create_plan_from_scan(
                    &scan,
                    &scan_id,
                    &selected_item_ids,
                    &registry,
                    &environment,
                )
                .map_err(|e| e.to_string())?;

                let ttl = plan_store.ttl_seconds();
                let mut preview = plan.preview(ttl);
                preview.expires_at = preview.expires_at.min(
                    scan.finished_at
                        .saturating_add(u64::from(ScanResult::VALID_FOR_SECONDS)),
                );

                let now = unix_timestamp();
                plan_store.insert(plan, now)?;
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
        progress: Arc<dyn CleanupProgressSink>,
    ) -> Result<CleanResult, String> {
        self.execute_intent(CleanupIntent::ReviewedSelection { plan_id }, None, progress)
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
    ) -> Result<CleanResult, String> {
        self.execute_intent(CleanupIntent::QuickSafe, Some(settings.clone()), progress)
            .await
    }

    /// Internal execution core ensuring identical security and lifecycle semantics for all clean intents.
    async fn execute_intent(
        &self,
        intent: CleanupIntent,
        settings: Option<ZenithSettings>,
        progress: Arc<dyn CleanupProgressSink>,
    ) -> Result<CleanResult, String> {
        self.platform_capabilities
            .capabilities()
            .require(
                PlatformFeature::Cleanup,
                crate::models::CapabilityAccess::Mutate,
            )
            .map_err(|e| e.to_string())?;

        let plan_store = self.plan_store.clone();
        let scan_store = self.scan_store.clone();
        let operation_gate = self.operation_gate.clone();
        let environment = self.environment.clone();
        let registry = self.registry.clone();
        let docker_status_cache = self.docker_status_cache.clone();

        crate::blocking::run_blocking(
            move || -> Result<CleanResult, String> {
                operation_gate.run_write(|| {
                    let now = unix_timestamp();

                    let plan: DeletePlan = match intent {
                        CleanupIntent::ReviewedSelection { plan_id } => {
                            let plan = plan_store.take_valid(plan_id, now)?;
                            // Invalidate scan atomically so pre-cleanup inventory cannot be reused
                            scan_store.validate_and_invalidate_for_cleanup(&plan.scan_id, now)?;
                            plan
                        }
                        CleanupIntent::QuickSafe => {
                            let settings = settings
                                .ok_or_else(|| "Settings required for Quick Clean".to_string())?;
                            let scan = scan_store.get().ok_or_else(|| {
                                "The scan is no longer current. Scan again before cleaning."
                                    .to_string()
                            })?;

                            scan.validate_for_cleanup(&scan.scan_id, now)
                                .map_err(|e| e.to_string())?;

                            // Quick Clean deletes the Safe subset without a
                            // per-item review, so it requires a scan that observed
                            // everything. A partial scan may have missed items and
                            // cannot prove the machine's state; a user-reviewed
                            // selection is different, because the user saw exactly
                            // what was inspected and chose from it.
                            if scan.quality != ObservationQuality::Fresh {
                                return Err(
                                "Quick Clean needs a complete scan. Scan again before cleaning."
                                    .to_string(),
                            );
                            }

                            let eligible_ids = select_quick_clean_safe_candidates(&scan, &settings);
                            if eligible_ids.is_empty() {
                                return Ok(CleanResult {
                                    plan_id: Uuid::new_v4(),
                                    started_at: now,
                                    finished_at: now,
                                    total_reclaimed_bytes: 0,
                                    total_failed_bytes: 0,
                                    partial_count: 0,
                                    failed_count: 0,
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
                            )
                            .map_err(|e| e.to_string())?;

                            // Invalidate scan atomically
                            scan_store.validate_and_invalidate_for_cleanup(&plan.scan_id, now)?;
                            plan
                        }
                    };

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
    use crate::models::{
        Category, CategoryResult, FileSize, ObservationQuality, PlatformCapabilities, RiskTier,
        ScanItem, Signature,
    };
    use zenith_platform::path_algebra::PathFlavor;

    struct TestCapabilitiesProvider(PlatformCapabilities);

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
                items,
            }],
            incomplete_reasons: Vec::new(),
            quality,
            skipped_entry_count: 0,
            incomplete_item_count: 0,
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
    async fn cleanup_service_plan_and_execution_lifecycle() {
        let env = Arc::new(PlatformEnvironment::simulated(PathFlavor::current()));
        let registry = Arc::new(SignatureRegistry::new());
        let scan_service = Arc::new(ScanService::new(registry.clone(), env.clone()));
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
            capabilities,
        );

        // Populate a scan
        let item = make_test_item("item1", RiskTier::Safe, 500);
        let scan = make_test_scan(vec![item]);
        scan_store.set(scan.clone());

        assert_eq!(service.get_last_scan().unwrap().scan_id, "scan_123");

        // Requesting plan with unknown item fails
        let err = service
            .create_delete_plan("scan_123".to_string(), vec!["unknown".to_string()])
            .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn a_consumed_plan_cannot_be_replayed_through_the_service() {
        let fixture = tempfile::tempdir().unwrap();
        let cache = fixture.path().join("fixture-cache");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("payload.bin"), vec![7u8; 512]).unwrap();

        let env = Arc::new(PlatformEnvironment::simulated(PathFlavor::current()));
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
            provider: String::new(),
            management_mode: Default::default(),
            artifact_kind: Default::default(),
            consequence: String::new(),
            reclaimable_is_lower_bound: false,
            exclusions: Vec::new(),
        });
        let registry = Arc::new(registry);

        let scan_service = Arc::new(ScanService::new(registry.clone(), env.clone()));
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

        let first = service.execute_clean(preview.id, progress.clone()).await;
        assert!(
            first.is_ok(),
            "the first execution of a plan runs: {:?}",
            first.err()
        );
        assert!(!cache.exists(), "the plan's target was deleted");

        // One-shot means the same plan ID cannot authorize a second mutation,
        // through this service or any other caller.
        let replay = service.execute_clean(preview.id, progress).await;
        let error = replay.expect_err("a consumed plan must be refused");
        assert!(
            error.contains("not found or already used"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn quick_clean_refuses_a_partial_scan_but_a_reviewed_plan_may_proceed() {
        let fixture = tempfile::tempdir().unwrap();
        let cache = fixture.path().join("fixture-cache");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("payload.bin"), vec![9u8; 256]).unwrap();

        let env = Arc::new(PlatformEnvironment::simulated(PathFlavor::current()));
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
            provider: String::new(),
            management_mode: Default::default(),
            artifact_kind: Default::default(),
            consequence: String::new(),
            reclaimable_is_lower_bound: false,
            exclusions: Vec::new(),
        });
        let registry = Arc::new(registry);

        let scan_service = Arc::new(ScanService::new(registry.clone(), env.clone()));
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
        assert!(error.contains("complete scan"), "unexpected error: {error}");
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
        let env = Arc::new(PlatformEnvironment::simulated(PathFlavor::current()));
        let registry = Arc::new(SignatureRegistry::new());
        let scan_service = Arc::new(ScanService::new(registry.clone(), env.clone()));
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
