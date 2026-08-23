use crate::models::AwakeRule;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum QuickPanelSection {
    Storage,
    Cleanup,
    AiUsage,
    Categories,
    Memory,
}

impl QuickPanelSection {
    pub const DEFAULTS: [Self; 4] = [Self::Cleanup, Self::Storage, Self::Memory, Self::AiUsage];
    pub const ALL: [Self; 5] = [
        Self::Cleanup,
        Self::Storage,
        Self::Memory,
        Self::AiUsage,
        Self::Categories,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum DashboardTab {
    #[serde(alias = "disk")]
    Disk,
    Storage,
    Docker,
    Models,
    Memory,
    DevelopmentServers,
    Usage,
    Awake,
}

impl DashboardTab {
    pub const ALL: [Self; 7] = [
        Self::Storage,
        Self::Docker,
        Self::Models,
        Self::Memory,
        Self::DevelopmentServers,
        Self::Usage,
        Self::Awake,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct ZenithSettings {
    pub launch_at_login: bool,
    pub clean_ai_tools: bool,
    pub clean_developer_tools: bool,
    pub clean_docker: bool,
    pub clean_local_models: bool,
    pub include_rebuild_caches: bool,
    pub intensive_cleanup: bool,
    pub theme: String,
    pub excluded_signatures: Vec<String>,
    pub awake_rules: Vec<AwakeRule>,
    pub quick_panel_sections: Vec<QuickPanelSection>,
    pub quick_panel_ai_providers: Vec<String>,
    pub dashboard_tabs: Vec<DashboardTab>,
    #[serde(default = "legacy_dashboard_tabs_revision")]
    pub dashboard_tabs_revision: u8,
    pub sidebar_collapsed: bool,
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
            intensive_cleanup: false,
            theme: "system".to_string(),
            excluded_signatures: Vec::new(),
            awake_rules: vec![
                AwakeRule {
                    id: "rule.codex".to_string(),
                    app_name: "Codex".to_string(),
                    executable_pattern: "codex".to_string(),
                    requires_process_pattern: None,
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
                AwakeRule {
                    id: "rule.claude".to_string(),
                    app_name: "Claude Code / Claude Desktop".to_string(),
                    executable_pattern: "claude".to_string(),
                    requires_process_pattern: None,
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
                AwakeRule {
                    id: "rule.warp".to_string(),
                    app_name: "Warp".to_string(),
                    executable_pattern: "warp".to_string(),
                    requires_process_pattern: None,
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
                AwakeRule {
                    id: "rule.opencode".to_string(),
                    app_name: "Opencode".to_string(),
                    executable_pattern: "opencode".to_string(),
                    requires_process_pattern: None,
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
                AwakeRule {
                    id: "rule.omp".to_string(),
                    app_name: "OMP (Opencode)".to_string(),
                    executable_pattern: "omp".to_string(),
                    requires_process_pattern: None,
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
                AwakeRule {
                    id: "rule.warp-codex".to_string(),
                    app_name: "Warp + Codex/OMP (compound)".to_string(),
                    executable_pattern: "warp".to_string(),
                    requires_process_pattern: Some("codex|opencode|omp|claude".to_string()),
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
                AwakeRule {
                    id: "rule.docker".to_string(),
                    app_name: "Docker Desktop".to_string(),
                    executable_pattern: "com.docker.backend".to_string(),
                    requires_process_pattern: None,
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
                AwakeRule {
                    id: "rule.terminal".to_string(),
                    app_name: "Terminal / iTerm2 / Ghostty".to_string(),
                    executable_pattern: "Terminal|iTerm2|ghostty".to_string(),
                    requires_process_pattern: None,
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
            ],
            quick_panel_sections: QuickPanelSection::DEFAULTS.to_vec(),
            quick_panel_ai_providers: vec![
                "codex".to_string(),
                "claude".to_string(),
                "opencode".to_string(),
                "openrouter".to_string(),
                "antigravity".to_string(),
            ],
            dashboard_tabs: DashboardTab::ALL.to_vec(),
            dashboard_tabs_revision: 1,
            sidebar_collapsed: false,
        }
    }
}

impl ZenithSettings {
    pub fn sanitize(mut self) -> Self {
        let mut sections = HashSet::new();
        self.quick_panel_sections
            .retain(|section| sections.insert(*section));
        if self.quick_panel_sections.is_empty() {
            self.quick_panel_sections.push(QuickPanelSection::Storage);
        }

        // Migrate legacy DashboardTab::Disk to DashboardTab::Storage
        for tab in &mut self.dashboard_tabs {
            if *tab == DashboardTab::Disk {
                *tab = DashboardTab::Storage;
            }
        }

        let mut tabs = HashSet::new();
        self.dashboard_tabs.retain(|tab| tabs.insert(*tab));
        if self.dashboard_tabs.is_empty() {
            self.dashboard_tabs.push(DashboardTab::Storage);
        }

        // Existing settings predate the standalone Development Servers tab.
        // Add it once after Memory, then preserve future user hide/reorder choices.
        if self.dashboard_tabs_revision < 1 {
            if !self
                .dashboard_tabs
                .contains(&DashboardTab::DevelopmentServers)
            {
                let insert_at = self
                    .dashboard_tabs
                    .iter()
                    .position(|tab| *tab == DashboardTab::Memory)
                    .map_or(self.dashboard_tabs.len(), |index| index + 1);
                self.dashboard_tabs
                    .insert(insert_at, DashboardTab::DevelopmentServers);
            }
            self.dashboard_tabs_revision = 1;
        }

        const SUPPORTED_PROVIDERS: [&str; 5] =
            ["codex", "claude", "opencode", "openrouter", "antigravity"];
        let mut providers = HashSet::new();
        self.quick_panel_ai_providers.retain(|provider| {
            SUPPORTED_PROVIDERS.contains(&provider.as_str()) && providers.insert(provider.clone())
        });
        self
    }
}

fn legacy_dashboard_tabs_revision() -> u8 {
    0
}

#[cfg(test)]
mod tests {
    use super::{DashboardTab, QuickPanelSection, ZenithSettings};

    #[test]
    fn sanitize_keeps_at_least_one_section_and_tab() {
        let empty = ZenithSettings {
            quick_panel_sections: Vec::new(),
            dashboard_tabs: Vec::new(),
            quick_panel_ai_providers: Vec::new(),
            ..ZenithSettings::default()
        };

        let sanitized = empty.sanitize();
        assert_eq!(
            sanitized.quick_panel_sections,
            vec![QuickPanelSection::Storage]
        );
        assert_eq!(sanitized.dashboard_tabs, vec![DashboardTab::Storage]);
    }

    #[test]
    fn sanitize_deduplicates_and_rejects_unknown_values() {
        let configured = ZenithSettings {
            quick_panel_sections: vec![
                QuickPanelSection::AiUsage,
                QuickPanelSection::AiUsage,
                QuickPanelSection::Memory,
            ],
            dashboard_tabs: vec![DashboardTab::Usage, DashboardTab::Usage, DashboardTab::Disk],
            quick_panel_ai_providers: vec!["codex".into(), "unknown".into(), "codex".into()],
            ..ZenithSettings::default()
        };

        let sanitized = configured.sanitize();
        assert_eq!(
            sanitized.quick_panel_sections,
            vec![QuickPanelSection::AiUsage, QuickPanelSection::Memory]
        );
        assert_eq!(
            sanitized.dashboard_tabs,
            vec![DashboardTab::Usage, DashboardTab::Storage]
        );
        assert_eq!(sanitized.quick_panel_ai_providers, vec!["codex"]);
    }

    #[test]
    fn old_settings_receive_quick_panel_and_dashboard_defaults() {
        let raw = r#"{
            "launch_at_login": true,
            "theme": "dark"
        }"#;

        let parsed: ZenithSettings = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.quick_panel_sections.len(), 4);
        assert_eq!(parsed.dashboard_tabs.len(), 7);
        assert_eq!(parsed.dashboard_tabs_revision, 0);
        assert!(parsed.launch_at_login);
        assert_eq!(parsed.theme, "dark");
        assert!(!parsed.intensive_cleanup);
        assert!(!parsed.sidebar_collapsed);
    }

    #[test]
    fn sanitize_adds_development_servers_once_for_existing_settings() {
        let raw = r#"{
            "dashboard_tabs": ["storage", "memory", "usage"],
            "theme": "system"
        }"#;

        let parsed: ZenithSettings = serde_json::from_str(raw).unwrap();
        let migrated = parsed.sanitize();
        assert_eq!(
            migrated.dashboard_tabs,
            vec![
                DashboardTab::Storage,
                DashboardTab::Memory,
                DashboardTab::DevelopmentServers,
                DashboardTab::Usage,
            ]
        );
        assert_eq!(migrated.dashboard_tabs_revision, 1);

        let hidden_again = ZenithSettings {
            dashboard_tabs: vec![DashboardTab::Storage, DashboardTab::Memory],
            ..migrated
        }
        .sanitize();
        assert_eq!(
            hidden_again.dashboard_tabs,
            vec![DashboardTab::Storage, DashboardTab::Memory]
        );
    }

    #[test]
    fn sanitize_preserves_default_awake_rules_and_restores_empty_tabs() {
        let defaults = ZenithSettings::default();
        let sanitized = defaults.clone().sanitize();
        assert_eq!(sanitized.awake_rules.len(), 8);
        assert!(sanitized
            .awake_rules
            .iter()
            .any(|r| r.id == "rule.warp-codex" && r.requires_process_pattern.is_some()));
    }

    #[test]
    fn sanitize_restores_empty_tabs_and_sections_to_defaults() {
        let mut settings = ZenithSettings::default();
        settings.dashboard_tabs.clear();
        settings.quick_panel_sections.clear();
        let sanitized = settings.sanitize();
        assert_eq!(sanitized.dashboard_tabs, vec![DashboardTab::Storage]);
        assert_eq!(
            sanitized.quick_panel_sections,
            vec![QuickPanelSection::Storage]
        );
    }
}
