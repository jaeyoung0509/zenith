use crate::models::AwakeRule;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZenithSettings {
    pub launch_at_login: bool,
    pub clean_ai_tools: bool,
    pub clean_developer_tools: bool,
    pub clean_docker: bool,
    pub clean_local_models: bool,
    pub include_rebuild_caches: bool,
    pub theme: String,
    pub excluded_signatures: Vec<String>,
    pub awake_rules: Vec<AwakeRule>,
}

impl Default for ZenithSettings {
    fn default() -> Self {
        Self {
            launch_at_login: false,
            clean_ai_tools: true,
            clean_developer_tools: true,
            clean_docker: true,
            clean_local_models: false,
            include_rebuild_caches: false,
            theme: "system".to_string(),
            excluded_signatures: Vec::new(),
            awake_rules: vec![
                AwakeRule {
                    id: "rule.claude".to_string(),
                    app_name: "Claude Code / Claude Desktop".to_string(),
                    executable_pattern: "claude".to_string(),
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    enabled: true,
                },
                AwakeRule {
                    id: "rule.docker".to_string(),
                    app_name: "Docker Desktop".to_string(),
                    executable_pattern: "com.docker.backend".to_string(),
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    enabled: false,
                },
                AwakeRule {
                    id: "rule.terminal".to_string(),
                    app_name: "Terminal / iTerm2 / Ghostty".to_string(),
                    executable_pattern: "Terminal|iTerm2|ghostty".to_string(),
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    enabled: false,
                },
            ],
        }
    }
}
