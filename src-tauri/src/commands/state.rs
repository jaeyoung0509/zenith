use crate::models::{AiUsageSnapshot, DeletePlan, ScanResult, ZenithSettings};
use crate::operation_gate::StorageOperationGate;
use crate::power::KeepAwakeManager;
use crate::signatures::SignatureRegistry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub registry: Arc<SignatureRegistry>,
    pub awake_manager: Arc<KeepAwakeManager>,
    pub settings: Arc<Mutex<ZenithSettings>>,
    pub last_scan: Arc<Mutex<Option<ScanResult>>>,
    pub openrouter_key: Arc<Mutex<Option<String>>>,
    pub ai_usage_cache: Arc<Mutex<Option<AiUsageSnapshot>>>,
    pub ai_usage_refresh_lock: Arc<Mutex<()>>,
    pub delete_plans: Arc<Mutex<HashMap<uuid::Uuid, DeletePlan>>>,
    pub storage_operation_gate: StorageOperationGate,
    pub storage_state: Arc<crate::storage_commands::StorageWorkflowState>,
    pub memory_sampler: Arc<crate::metrics::MemorySampler>,
    pub dev_port_store: Arc<Mutex<crate::dev_ports::DevelopmentPortStore>>,
    pub agent_activity_cache: Arc<Mutex<Option<crate::agent_activity::AgentActivityRegistry>>>,
    pub ai_control_state: Arc<Mutex<crate::ai_control_center::state::AiControlCenterState>>,
    pub ai_control_refresh_lock: Arc<Mutex<()>>,
    pub ai_control_runtime: Arc<crate::ai_control_center::runtime::AiControlRuntime>,
    pub platform_capabilities: Arc<dyn crate::platform::PlatformCapabilitiesProvider>,
}
