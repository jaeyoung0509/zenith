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
    pub registry: Arc<SignatureRegistry>,
    pub awake_manager: Arc<KeepAwakeManager>,
    pub settings: Arc<Mutex<ZenithSettings>>,
    pub last_scan: Arc<Mutex<Option<ScanResult>>>,
    pub credentials: Arc<dyn crate::ai_providers::CredentialStore>,
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
