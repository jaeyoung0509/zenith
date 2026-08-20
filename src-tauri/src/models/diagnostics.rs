use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DiagnosticsSnapshot {
    pub app_version: String,
    pub os_version: String,
    pub arch: String,
    pub log_path: String,
    pub enabled_features: Vec<String>,
    pub recent_errors: Vec<String>,
    pub settings_corrupt_recovered: bool,
}
