use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ListenerProtocol {
    Tcp,
}

impl ListenerProtocol {
    pub fn display_name(&self) -> &'static str {
        match self {
            ListenerProtocol::Tcp => "TCP",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ListenerExposure {
    Loopback,
    Network,
    AllInterfaces,
}

impl ListenerExposure {
    pub fn display_name(&self) -> &'static str {
        match self {
            ListenerExposure::Loopback => "Loopback",
            ListenerExposure::Network => "Network",
            ListenerExposure::AllInterfaces => "All Interfaces",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DevelopmentListener {
    pub id: String,
    pub port: u16,
    pub protocol: ListenerProtocol,
    pub bind_address: String,
    pub exposure: ListenerExposure,
    pub pid: u32,
    pub server_name: String,
    pub project_name: Option<String>,
    pub working_directory: Option<String>,
    pub started_at: Option<u64>,
    pub can_release: bool,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseMode {
    Graceful,
    Force,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseOutcome {
    Released,
    StillListening,
    OwnershipChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ReleaseDevelopmentListenerResult {
    pub port: u16,
    pub outcome: ReleaseOutcome,
    pub listener: Option<DevelopmentListener>,
}
