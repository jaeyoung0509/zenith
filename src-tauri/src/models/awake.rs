use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AwakeBehavior {
    PreventSystemSleep,
    KeepDisplayAwake,
}

impl AwakeBehavior {
    pub fn display_name(&self) -> &'static str {
        match self {
            AwakeBehavior::PreventSystemSleep => "Prevent System Sleep",
            AwakeBehavior::KeepDisplayAwake => "Keep Display Awake",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwakeRule {
    pub id: String,
    pub app_name: String,
    pub executable_pattern: String,
    pub behavior: AwakeBehavior,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedApplication {
    pub name: String,
    pub executable_pattern: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwakeState {
    pub is_active: bool,
    pub behavior: Option<AwakeBehavior>,
    pub trigger_source: Option<String>,
    pub active_process_name: Option<String>,
    pub manual_expires_at: Option<u64>,
    pub active_rules_count: usize,
}
