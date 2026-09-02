use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AgentEvidence {
    #[serde(alias = "vendor_event")]
    VendorEvent,
    #[serde(alias = "vendor_confirmed")]
    VendorConfirmed,
    VendorProtocol,
    ProcessObserved,
    Heuristic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivityStatus {
    Starting,
    Working,
    WaitingForUser,
    Idle,
    PossiblyInactive,
    Exited,
    Unknown,
    #[serde(alias = "active")]
    Active,
    #[serde(alias = "waiting")]
    Waiting,
    #[serde(alias = "finished")]
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AttentionReason {
    Approval,
    Input,
    TurnComplete,
    Inactivity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AgentAdapterState {
    NotInstalled,
    ProcessOnly,
    IntegrationAvailable,
    Connected,
    VersionUnsupported,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotQuality {
    Fresh,
    Stale,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct AgentSession {
    /// Opaque, snapshot-stable identity. Never a PID.
    pub id: String,
    pub tool_id: String,
    pub tool_name: String,
    pub status: AgentActivityStatus,
    pub attention_reason: Option<AttentionReason>,
    pub evidence: AgentEvidence,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub observed_at: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub started_at: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub elapsed_seconds: u64,
    pub cpu_percent: f32,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub memory_bytes: u64,
    pub project_id: Option<String>,
    pub worktree_id: Option<String>,
    pub detail: String,
    pub can_stop: bool,
    pub stop_lease_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProjectIdentity {
    /// Opaque hash of the canonical project/worktree root.
    pub id: String,
    pub display_name: String,
    /// A compact parent/name hint, never a full absolute path in public payloads.
    pub location_hint: String,
    /// Main-window only display path (e.g. ~/Myproject/clean1).
    pub display_path: String,
    pub repository_id: Option<String>,
    pub worktree_id: Option<String>,
    pub is_worktree: bool,
    pub branch: Option<String>,
    pub is_dirty: bool,
    pub is_detached: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ProjectContext {
    pub identity: ProjectIdentity,
    pub sessions: Vec<AgentSession>,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub last_seen_at: u64,
    pub dev_ports: Vec<u16>,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub artifact_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AgentAdapterHealth {
    pub tool_id: String,
    pub display_name: String,
    pub state: AgentAdapterState,
    pub evidence: Option<AgentEvidence>,
    pub message: String,
    pub installed_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct AgentActivitySnapshot {
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub observed_at: u64,
    pub quality: SnapshotQuality,
    pub projects: Vec<ProjectContext>,
    pub unassigned_sessions: Vec<AgentSession>,
    pub adapters: Vec<AgentAdapterHealth>,
    pub partial_errors: Vec<String>,
}

/// #75's canonical source consumed by the Project Cockpit and the paired
/// control-center work. Keeping one alias prevents a competing project model.
pub type ProjectContextSnapshot = AgentActivitySnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AgentIntegrationInfo {
    pub tool_id: String,
    pub display_name: String,
    pub supported: bool,
    pub installed: bool,
    pub integration_active: bool,
    pub config_path: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AgentIntegrationResult {
    pub tool_id: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct AgentNotificationPreferences {
    pub enabled: bool,
    pub notify_on_turn_completed: bool,
    pub notify_on_approval_or_input: bool,
    pub notify_on_possibly_inactive: bool,
    pub hide_project_basename: bool,
    pub inactivity_threshold_minutes: u32,
}

impl Default for AgentNotificationPreferences {
    fn default() -> Self {
        Self {
            enabled: false,
            notify_on_turn_completed: true,
            notify_on_approval_or_input: true,
            notify_on_possibly_inactive: false,
            hide_project_basename: false,
            inactivity_threshold_minutes: 15,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AgentQuickSessionRow {
    pub session_id: String,
    pub tool_name: String,
    pub project_name: String,
    pub status: AgentActivityStatus,
    pub evidence: AgentEvidence,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub elapsed_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AgentQuickSummary {
    pub active_count: u32,
    pub attention_count: u32,
    pub sessions: Vec<AgentQuickSessionRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AgentLifecycleEvent {
    SessionStart,
    Working,
    WaitingForUser,
    Idle,
    TurnComplete,
    SessionEnd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct IngestedAgentEvent {
    pub tool_id: String,
    pub vendor_session_id: String,
    pub cwd: Option<String>,
    pub lifecycle: AgentLifecycleEvent,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub timestamp: u64,
    pub turn_id: Option<String>,
    pub attention_reason: Option<AttentionReason>,
}
