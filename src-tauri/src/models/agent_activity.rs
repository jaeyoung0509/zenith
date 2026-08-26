use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AgentEvidence {
    VendorConfirmed,
    ProcessObserved,
    Heuristic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivityStatus {
    Active,
    Waiting,
    Finished,
    Exited,
    Unknown,
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
    pub evidence: AgentEvidence,
    pub observed_at: u64,
    pub started_at: u64,
    pub elapsed_seconds: u64,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub project_id: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProjectIdentity {
    /// Opaque hash of the canonical project/worktree root.
    pub id: String,
    pub display_name: String,
    /// A compact parent/name hint, never a full absolute path.
    pub location_hint: String,
    pub repository_id: Option<String>,
    pub is_worktree: bool,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ProjectContext {
    pub identity: ProjectIdentity,
    pub sessions: Vec<AgentSession>,
    pub last_seen_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AgentAdapterHealth {
    pub tool_id: String,
    pub display_name: String,
    pub state: AgentAdapterState,
    pub evidence: Option<AgentEvidence>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct AgentActivitySnapshot {
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
