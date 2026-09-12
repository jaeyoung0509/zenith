use crate::collection::SingleFlight;
use crate::execution_budget::ExecutionBudgets;
use crate::models::{AiProviderUsage, AiUsageSnapshot, DeletePlan, ScanResult, ZenithSettings};
use crate::operation_gate::StorageOperationGate;
use crate::power::KeepAwakeManager;
use crate::runtime_metrics::RuntimeMetrics;
use crate::signatures::SignatureRegistry;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

pub struct AppState {
    /// Platform facts the backend may depend on. Tests inject a simulated
    /// environment here instead of reading the host's.
    pub environment: Arc<crate::platform::PlatformEnvironment>,
    /// Container host observed at startup, so no adapter reads the process
    /// environment on its own.
    pub container_host: crate::docker::adapter::ContainerHost,
    pub registry: Arc<SignatureRegistry>,
    /// Why the embedded signature catalog never loaded, when it did not.
    ///
    /// A catalog that failed is empty, and an empty catalog scans clean: the
    /// scan refuses while this is set instead of reporting a healthy machine.
    pub registry_load_error: Option<String>,
    pub awake_manager: Arc<KeepAwakeManager>,
    pub settings: Arc<Mutex<ZenithSettings>>,
    pub last_scan: Arc<Mutex<Option<ScanResult>>>,
    pub credentials: Arc<dyn crate::ai_providers::CredentialStore>,
    pub ai_collection_service: Arc<crate::ai_providers::ProviderCollectionService>,
    pub ai_usage_cache: Arc<Mutex<Option<AiUsageSnapshot>>>,
    pub usage_singleflight: Arc<SingleFlight<AiUsageSnapshot, AiProviderUsage>>,
    pub usage_generation: Arc<AtomicU64>,
    pub delete_plans: Arc<Mutex<HashMap<uuid::Uuid, DeletePlan>>>,
    pub storage_operation_gate: StorageOperationGate,
    pub storage_state: Arc<crate::storage_commands::StorageWorkflowState>,
    pub memory_sampler: Arc<crate::metrics::MemorySampler>,
    pub memory_termination_store: Arc<Mutex<crate::metrics::MemoryTerminationStore>>,
    pub dev_port_store: Arc<Mutex<crate::dev_ports::DevelopmentPortStore>>,
    pub agent_activity_cache: Arc<Mutex<Option<crate::agent_activity::AgentActivityRegistry>>>,
    pub activity_singleflight: Arc<SingleFlight<crate::agent_activity::AgentActivityRegistry, ()>>,
    pub activity_generation: Arc<AtomicU64>,
    pub ai_control_state: Arc<Mutex<crate::ai_control_center::state::AiControlCenterState>>,
    pub ai_control_refresh_lock: Arc<Mutex<()>>,
    pub ai_control_runtime: Arc<crate::ai_control_center::runtime::AiControlRuntime>,
    pub platform_capabilities: Arc<dyn crate::platform::PlatformCapabilitiesProvider>,
    pub runtime_metrics: Arc<RuntimeMetrics>,
    pub execution_budgets: Arc<ExecutionBudgets>,
    pub docker_status_cache: Arc<Mutex<Option<(crate::models::DockerStatus, std::time::Instant)>>>,
}

impl AppState {
    /// Builds the process-wide state from the two facts the composition root
    /// observes: the environment every component is described by and the
    /// container host.
    ///
    /// Construction lives here rather than inline in `run()` so a test can
    /// build the same state from a simulated environment and exercise the
    /// commands' dependencies (the environment self-check, the metrics handles,
    /// the shared caches) without a Tauri app handle.
    pub fn new(
        environment: Arc<crate::platform::PlatformEnvironment>,
        container_host: crate::docker::adapter::ContainerHost,
    ) -> Self {
        // The catalog is loaded against the same description every other
        // component receives, instead of building a second, unrelated native
        // environment.
        let (registry, registry_load_error) =
            SignatureRegistry::load_or_default(&environment, SignatureRegistry::load_embedded_with);
        Self::with_catalog(environment, container_host, registry, registry_load_error)
    }

    /// Builds the state around a catalog the caller already loaded.
    ///
    /// [`AppState::new`] delegates here after loading the embedded catalog; the
    /// seam exists so a test can assert that a catalog which failed to load
    /// refuses a scan instead of reporting an empty, healthy-looking one.
    pub fn with_catalog(
        environment: Arc<crate::platform::PlatformEnvironment>,
        container_host: crate::docker::adapter::ContainerHost,
        registry: SignatureRegistry,
        registry_load_error: Option<String>,
    ) -> Self {
        let registry = Arc::new(registry);
        let awake_manager = Arc::new(KeepAwakeManager::new());
        awake_manager.set_session_validator(crate::agent_activity::has_active_verified_session);
        let settings = Arc::new(Mutex::new(ZenithSettings::default()));
        let last_scan = Arc::new(Mutex::new(None));
        let credentials: Arc<dyn crate::ai_providers::CredentialStore> =
            Arc::new(crate::ai_providers::OsCredentialStore::default());
        let ai_collection_service =
            Arc::new(crate::ai_providers::ProviderCollectionService::default());
        let ai_usage_cache = Arc::new(Mutex::new(None));
        let runtime_metrics = Arc::new(RuntimeMetrics::new());
        let usage_singleflight = Arc::new(SingleFlight::with_metrics(runtime_metrics.clone()));
        let usage_generation = Arc::new(AtomicU64::new(1));
        let delete_plans = Arc::new(Mutex::new(HashMap::new()));
        let storage_operation_gate = StorageOperationGate::default();
        let storage_state = Arc::new(crate::storage_commands::StorageWorkflowState::new());
        let memory_sampler = Arc::new(crate::metrics::MemorySampler::new());
        let memory_termination_store =
            Arc::new(Mutex::new(crate::metrics::MemoryTerminationStore::default()));
        let dev_port_store =
            Arc::new(Mutex::new(crate::dev_ports::DevelopmentPortStore::default()));
        let agent_activity_cache = Arc::new(Mutex::new(None));
        let activity_singleflight = Arc::new(SingleFlight::with_metrics(runtime_metrics.clone()));
        let activity_generation = Arc::new(AtomicU64::new(1));
        let execution_budgets = Arc::new(ExecutionBudgets::new());
        let ai_control_state = Arc::new(Mutex::new(
            crate::ai_control_center::state::AiControlCenterState::default(),
        ));
        let ai_control_refresh_lock = Arc::new(Mutex::new(()));
        let ai_control_runtime =
            Arc::new(crate::ai_control_center::runtime::AiControlRuntime::new(
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
        let platform_capabilities: Arc<dyn crate::platform::PlatformCapabilitiesProvider> =
            Arc::new(crate::platform::NativePlatformCapabilities::new(
                environment.clone(),
            ));

        Self {
            environment,
            container_host,
            registry,
            registry_load_error,
            awake_manager,
            settings,
            last_scan,
            credentials,
            ai_collection_service,
            ai_usage_cache,
            usage_singleflight,
            usage_generation,
            delete_plans,
            storage_operation_gate,
            storage_state,
            memory_sampler,
            memory_termination_store,
            dev_port_store,
            agent_activity_cache,
            activity_singleflight,
            activity_generation,
            ai_control_state,
            ai_control_refresh_lock,
            ai_control_runtime,
            platform_capabilities,
            runtime_metrics,
            execution_budgets,
            docker_status_cache: Arc::new(Mutex::new(None)),
        }
    }

    /// The reason a scan cannot run, when the signature catalog never loaded.
    ///
    /// An empty catalog scans nothing and reports `Fresh`, which is exactly
    /// what a clean machine reports; refusing the scan is what keeps a startup
    /// failure from being presented as one.
    pub fn catalog_failure(&self) -> Option<String> {
        self.registry_load_error.as_deref().map(|error| {
            format!("{error}. Scan and cleanup are unavailable until the signature catalog loads.")
        })
    }
}
