//! Wiring the desktop object graph.

use std::sync::{Arc, Mutex};

use zenith_platform::PlatformCapabilitiesProvider;

use crate::ai_control_center::state::AiControlCenterState;
use crate::ai_providers::CredentialStore;
use crate::commands::DesktopState;
use crate::docker::adapter::ContainerHost;
use crate::execution_budget::ExecutionBudgets;
use crate::models::ZenithSettings;
use crate::operation_gate::StorageOperationGate;
use crate::power::KeepAwakeManager;
use crate::services::{
    AiService, CleanupService, DockerStatusCache, SettingsAuthority, StorageService, SystemService,
};
use crate::signatures::SignatureRegistry;

/// Builds the process-wide state from the two facts the shell observes: the
/// environment every component is described by and the container host.
///
/// Construction lives here rather than inline in `run()` so a test can build
/// the same state from a simulated environment and exercise the commands'
/// dependencies (the environment self-check, the metrics handles, the shared
/// caches) without a Tauri app handle.
pub fn desktop_state(
    environment: Arc<zenith_platform::PlatformEnvironment>,
    container_host: ContainerHost,
) -> DesktopState {
    // The catalog is loaded against the same description every other component
    // receives, instead of building a second, unrelated native environment.
    let (registry, registry_load_error) =
        SignatureRegistry::load_or_default(&environment, SignatureRegistry::load_embedded_with);
    desktop_state_with_catalog(environment, container_host, registry, registry_load_error)
}

/// Builds the state around a catalog the caller already loaded.
///
/// [`desktop_state`] delegates here after loading the embedded catalog; the
/// seam exists so a test can assert that a catalog which failed to load
/// refuses a scan instead of reporting an empty, healthy-looking one.
pub fn desktop_state_with_catalog(
    environment: Arc<zenith_platform::PlatformEnvironment>,
    container_host: ContainerHost,
    registry: SignatureRegistry,
    registry_load_error: Option<String>,
) -> DesktopState {
    let registry = Arc::new(registry);
    let settings = Arc::new(SettingsAuthority::new(ZenithSettings::default()));

    let awake_manager = Arc::new(KeepAwakeManager::new());
    awake_manager.set_session_validator(crate::agent_activity::has_active_verified_session);

    let runtime_metrics = Arc::new(crate::runtime_metrics::RuntimeMetrics::new());
    let operation_gate = StorageOperationGate::default();
    let budgets = Arc::new(ExecutionBudgets::new());

    let memory_sampler = Arc::new(crate::metrics::MemorySampler::new());
    let memory_termination_store =
        Arc::new(Mutex::new(crate::metrics::MemoryTerminationStore::default()));
    let dev_port_store = Arc::new(Mutex::new(crate::dev_ports::DevelopmentPortStore::default()));
    let docker_status = Arc::new(DockerStatusCache::new());

    let platform_capabilities: Arc<dyn PlatformCapabilitiesProvider> =
        Arc::new(zenith_platform::NativePlatformCapabilities::new(
            environment.clone(),
            // The container answer is asked exactly once, where the CLI
            // resolution lives, and the capability snapshot reports it.
            Arc::new(crate::docker::container_cli_detected),
        ));

    // The reviewed-storage workflows own their gate, budgets, inventories,
    // plan store, and the Trash executor, so a handler never orchestrates
    // them. The executor is not kept separately: the raw port stays inside the
    // service.
    let trash_executor = Arc::new(crate::trash_manager::TrashExecutor::new(Arc::new(
        zenith_platform::NativeTrashBackend,
    )));
    let storage_service = Arc::new(StorageService::new(
        operation_gate.clone(),
        budgets.clone(),
        environment.clone(),
        trash_executor,
        Arc::new(zenith_platform::NativeSystemActions::new()),
        platform_capabilities.clone(),
    ));

    let scan_service = Arc::new(crate::services::ScanService::new(
        registry.clone(),
        environment.clone(),
    ));
    let plan_store = Arc::new(crate::services::PlanStore::new(
        crate::services::PlanLifecycle::cleanup(),
    ));
    let scan_store = Arc::new(crate::services::ScanStore::new());
    let cleanup_service = Arc::new(CleanupService::new(
        scan_service,
        plan_store,
        scan_store,
        operation_gate.clone(),
        budgets.clone(),
        environment.clone(),
        registry.clone(),
        docker_status.clone(),
        platform_capabilities.clone(),
    ));

    // The AI surfaces share one single-flight, cache, and generation set with
    // the background runtime, so a foreground refresh and a background tick
    // cannot publish differently configured results.
    let credentials: Arc<dyn CredentialStore> =
        Arc::new(crate::ai_providers::OsCredentialStore::default());
    let ai_collection_service = Arc::new(crate::ai_providers::ProviderCollectionService::default());
    let ai_usage_cache = Arc::new(Mutex::new(None));
    let usage_singleflight = Arc::new(crate::collection::SingleFlight::with_metrics(
        runtime_metrics.clone(),
    ));
    let usage_generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let agent_activity_cache = Arc::new(Mutex::new(None));
    let activity_singleflight = Arc::new(crate::collection::SingleFlight::with_metrics(
        runtime_metrics.clone(),
    ));
    let activity_generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let ai_control_state = Arc::new(Mutex::new(AiControlCenterState::default()));
    let ai_control_refresh_lock = Arc::new(Mutex::new(()));
    let ai_control_runtime = Arc::new(crate::ai_control_center::runtime::AiControlRuntime::new(
        memory_sampler.clone(),
        dev_port_store.clone(),
        environment.clone(),
        agent_activity_cache.clone(),
        activity_singleflight.clone(),
        activity_generation.clone(),
        runtime_metrics.clone(),
        ai_control_state.clone(),
        awake_manager.clone(),
        settings.clone(),
    ));

    let ai_service = Arc::new(AiService::new(
        environment.clone(),
        platform_capabilities.clone(),
        settings.clone(),
        credentials,
        ai_collection_service,
        ai_usage_cache,
        usage_singleflight,
        usage_generation,
        agent_activity_cache,
        activity_singleflight,
        activity_generation,
        ai_control_state,
        ai_control_refresh_lock,
        ai_control_runtime,
        runtime_metrics,
        budgets.clone(),
        memory_sampler.clone(),
        dev_port_store.clone(),
        awake_manager.clone(),
        storage_service.clone(),
    ));

    // A settings save is one use case that spans domains: the AI caches are
    // keyed to preferences the system service persists, so the AI service is
    // the reaction the persistence path reports to.
    let system_service = Arc::new(SystemService::new(
        environment.clone(),
        container_host,
        platform_capabilities,
        operation_gate.clone(),
        budgets,
        memory_sampler,
        memory_termination_store,
        awake_manager,
        docker_status,
        dev_port_store,
        settings.clone(),
        ai_service.clone(),
    ));

    DesktopState::new(
        environment,
        registry,
        registry_load_error,
        settings,
        cleanup_service,
        storage_service,
        ai_service,
        system_service,
    )
}
