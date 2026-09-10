use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AwakeBehavior {
    PreventSystemSleep,
    KeepDisplayAwake,
}

impl AwakeBehavior {
    pub fn display_name(&self) -> &'static str {
        match self {
            AwakeBehavior::PreventSystemSleep => "Keep Computer Awake",
            AwakeBehavior::KeepDisplayAwake => "Keep Computer + Display Awake",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AwakeRuleStatus {
    Active,
    WaitingApplication,
    WaitingAgent,
    WaitingProcess,
    WaitingPower,
    InvalidApplication,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AwakeAgentId {
    Codex,
    Claude,
    Antigravity,
    #[serde(rename = "opencode")]
    OpenCode,
}

impl AwakeAgentId {
    pub const ALL: [Self; 4] = [Self::Codex, Self::Claude, Self::Antigravity, Self::OpenCode];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
            Self::Antigravity => "Antigravity",
            Self::OpenCode => "OpenCode / OMP",
        }
    }

    /// The adapter id is shared with Agent Activity so a typed Keep Awake rule
    /// cannot invent its own process-signature table.
    pub const fn adapter_id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Antigravity => "antigravity",
            Self::OpenCode => "opencode",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ApplicationIdentity {
    pub display_name: String,
    pub executable_name: String,
    pub path: String,
}

impl ApplicationIdentity {
    pub fn is_structurally_valid(&self) -> bool {
        !self.display_name.trim().is_empty()
            && !self.executable_name.trim().is_empty()
            && std::path::Path::new(&self.path).is_absolute()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AwakeRule {
    pub id: String,
    pub app_name: String,
    pub executable_pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_process_pattern: Option<String>,
    /// Native picker identity for the primary application. When present, this
    /// is a typed rule and raw pattern fields are retained only for legacy
    /// compatibility/debugging; they are not used for matching.
    #[serde(default)]
    pub application: Option<ApplicationIdentity>,
    /// Allowlisted agent adapters. Semantics are explicit any-of (OR).
    #[serde(default)]
    pub agent_ids: Vec<AwakeAgentId>,
    pub behavior: AwakeBehavior,
    #[serde(default)]
    pub power_condition: PowerCondition,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SelectedApplication {
    pub name: String,
    pub executable_pattern: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AwakeRuleEvaluation {
    pub rule_id: String,
    pub status: AwakeRuleStatus,
    pub is_process_running: bool,
    pub is_power_eligible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AwakeState {
    pub is_active: bool,
    pub behavior: Option<AwakeBehavior>,
    pub trigger_source: Option<String>,
    pub active_process_name: Option<String>,
    pub active_rule_id: Option<String>,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
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
        assert_eq!(rule.requires_process_pattern, None);
        assert_eq!(rule.application, None);
        assert!(rule.agent_ids.is_empty());
    }

    #[test]
    fn typed_rule_round_trips_application_identity_and_known_agents() {
        let rule = AwakeRule {
            id: "rule.warp-agents".into(),
            app_name: "Warp".into(),
            executable_pattern: "Warp".into(),
            requires_process_pattern: None,
            application: Some(ApplicationIdentity {
                display_name: "Warp".into(),
                executable_name: "stable".into(),
                path: "/Applications/Warp.app".into(),
            }),
            agent_ids: vec![AwakeAgentId::Codex, AwakeAgentId::OpenCode],
            behavior: AwakeBehavior::PreventSystemSleep,
            power_condition: PowerCondition::AcPowerOnly,
            enabled: false,
        };

        let encoded = serde_json::to_string(&rule).unwrap();
        let decoded: AwakeRule = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, rule);
        assert!(decoded.application.unwrap().is_structurally_valid());
    }
}
