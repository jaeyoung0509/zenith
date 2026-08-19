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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerCondition {
    #[default]
    Always,
    AcPowerOnly,
}

impl PowerCondition {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Always => "Always",
            Self::AcPowerOnly => "Only while plugged in",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerSourceType {
    Ac,
    Battery,
    #[default]
    Unknown,
}

impl PowerSourceType {
    pub fn is_ac(&self) -> bool {
        matches!(self, Self::Ac)
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Ac => "Plugged In (AC)",
            Self::Battery => "Battery Power",
            Self::Unknown => "Unknown Power Source",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AwakeRuleStatus {
    Active,
    WaitingProcess,
    WaitingPower,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwakeRule {
    pub id: String,
    pub app_name: String,
    pub executable_pattern: String,
    pub behavior: AwakeBehavior,
    #[serde(default)]
    pub power_condition: PowerCondition,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedApplication {
    pub name: String,
    pub executable_pattern: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwakeRuleEvaluation {
    pub rule_id: String,
    pub status: AwakeRuleStatus,
    pub is_process_running: bool,
    pub is_power_eligible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwakeState {
    pub is_active: bool,
    pub behavior: Option<AwakeBehavior>,
    pub trigger_source: Option<String>,
    pub active_process_name: Option<String>,
    pub active_rule_id: Option<String>,
    pub manual_expires_at: Option<u64>,
    pub active_rules_count: usize,
    pub power_source: PowerSourceType,
    pub last_error: Option<String>,
    pub rule_evaluations: Vec<AwakeRuleEvaluation>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backward_compatible_deserialization_defaults_to_always() {
        let legacy_json = r#"{
            "id": "legacy.rule",
            "app_name": "Legacy App",
            "executable_pattern": "legacy_bin",
            "behavior": "prevent_system_sleep",
            "enabled": true
        }"#;

        let rule: AwakeRule = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(rule.power_condition, PowerCondition::Always);
    }
}
