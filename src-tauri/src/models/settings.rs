use crate::models::{AiControlPreferences, AwakeRule};
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
    AiControl,
    AgentActivity,
}

impl QuickPanelSection {
    pub const DEFAULTS: [Self; 4] = [
        Self::Cleanup,
        Self::Storage,
        Self::Memory,
        Self::AgentActivity,
    ];

    pub const ALL: [Self; 5] = [
        Self::Cleanup,
        Self::Storage,
        Self::Memory,
        Self::Categories,
        Self::AgentActivity,
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
    Projects,
    DevelopmentServers,
    Usage,
    AiControl,
    Awake,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum DashboardRoute {
    #[serde(alias = "disk")]
    Disk,
    Storage,
    Docker,
    Models,
    Memory,
    Projects,
    DevelopmentServers,
    Usage,
    AiControl,
    Awake,
    DeveloperArtifacts,
    LargeFiles,
    Applications,
    Settings,
}

impl From<DashboardTab> for DashboardRoute {
    fn from(tab: DashboardTab) -> Self {
        match tab {
            DashboardTab::Disk => Self::Disk,
            DashboardTab::Storage => Self::Storage,
            DashboardTab::Docker => Self::Docker,
            DashboardTab::Models => Self::Models,
            DashboardTab::Memory => Self::Memory,
            DashboardTab::Projects => Self::Projects,
            DashboardTab::DevelopmentServers => Self::DevelopmentServers,
            DashboardTab::Usage => Self::Usage,
            DashboardTab::AiControl => Self::AiControl,
            DashboardTab::Awake => Self::Awake,
        }
    }
}

impl DashboardTab {
    pub const ALL: [Self; 7] = [
        Self::Storage,
        Self::Docker,
        Self::Models,
        Self::Memory,
        Self::DevelopmentServers,
        Self::Projects,
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
    pub ai_accounts_quota_providers: Vec<String>,
    pub dashboard_tabs: Vec<DashboardTab>,
    #[serde(default = "legacy_dashboard_tabs_revision")]
    pub dashboard_tabs_revision: u8,
    pub sidebar_collapsed: bool,
    pub ai_control: AiControlPreferences,
    #[serde(default)]
    pub agent_notifications: crate::models::AgentNotificationPreferences,
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
            ai_accounts_quota_providers: vec![
                "codex".to_string(),
                "claude".to_string(),
                "opencode".to_string(),
                "openrouter".to_string(),
                "antigravity".to_string(),
            ],
            dashboard_tabs: DashboardTab::ALL.to_vec(),
            dashboard_tabs_revision: 5,
            sidebar_collapsed: false,
            ai_control: AiControlPreferences::default(),
            agent_notifications: crate::models::AgentNotificationPreferences::default(),
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

        // #75 adds Projects once after Memory. The revision guard preserves
        // any later user hide/reorder choice instead of re-inserting it.
        if self.dashboard_tabs_revision < 2 {
            if !self.dashboard_tabs.contains(&DashboardTab::Projects) {
                let insert_at = self
                    .dashboard_tabs
                    .iter()
                    .position(|tab| *tab == DashboardTab::Memory)
                    .map_or(self.dashboard_tabs.len(), |index| index + 1);
                self.dashboard_tabs
                    .insert(insert_at, DashboardTab::Projects);
            }
            self.dashboard_tabs_revision = 2;
        }

        // #74 adds AI Control once after Projects. Preserve user ordering and
        // allow the tab to remain hidden after this migration has run.
        if self.dashboard_tabs_revision < 3 {
            if !self.dashboard_tabs.contains(&DashboardTab::AiControl) {
                let insert_at = self
                    .dashboard_tabs
                    .iter()
                    .position(|tab| *tab == DashboardTab::Projects)
                    .map_or(self.dashboard_tabs.len(), |index| index + 1);
                self.dashboard_tabs
                    .insert(insert_at, DashboardTab::AiControl);
            }
            self.dashboard_tabs_revision = 3;
        }

        // #75 ensures Projects tab exists once, then preserves later user hide/reorder choices.
        if self.dashboard_tabs_revision < 4 {
            if !self.dashboard_tabs.contains(&DashboardTab::Projects) {
                let insert_at = self
                    .dashboard_tabs
                    .iter()
                    .position(|tab| *tab == DashboardTab::Memory)
                    .map_or(self.dashboard_tabs.len(), |index| index + 1);
                self.dashboard_tabs
                    .insert(insert_at, DashboardTab::Projects);
            }
            self.dashboard_tabs_revision = 4;
        }

        // #78 consolidates AI tabs: if user had Usage, AiControl, or Projects,
        // ensure Projects is preserved, and remove Usage and AiControl from dashboard_tabs.
        if self.dashboard_tabs_revision < 5 {
            let had_ai_tab = self.dashboard_tabs.contains(&DashboardTab::Projects)
                || self.dashboard_tabs.contains(&DashboardTab::AiControl)
                || self.dashboard_tabs.contains(&DashboardTab::Usage);
            self.dashboard_tabs
                .retain(|tab| *tab != DashboardTab::AiControl && *tab != DashboardTab::Usage);
            if had_ai_tab && !self.dashboard_tabs.contains(&DashboardTab::Projects) {
                let insert_at = self
                    .dashboard_tabs
                    .iter()
                    .position(|tab| *tab == DashboardTab::DevelopmentServers)
                    .unwrap_or(self.dashboard_tabs.len());
                self.dashboard_tabs
                    .insert(insert_at, DashboardTab::Projects);
            }

            // Consolidate quick panel sections: replace ai_control and ai_usage with agent_activity
            let had_ai_section = self
                .quick_panel_sections
                .contains(&QuickPanelSection::AgentActivity)
                || self
                    .quick_panel_sections
                    .contains(&QuickPanelSection::AiControl)
                || self
                    .quick_panel_sections
                    .contains(&QuickPanelSection::AiUsage);
            self.quick_panel_sections.retain(|sec| {
                *sec != QuickPanelSection::AiControl && *sec != QuickPanelSection::AiUsage
            });
            if had_ai_section
                && !self
                    .quick_panel_sections
                    .contains(&QuickPanelSection::AgentActivity)
            {
                self.quick_panel_sections
                    .push(QuickPanelSection::AgentActivity);
            }

            self.dashboard_tabs_revision = 5;
        }

        // Retired identifiers stay invalid even if a malformed/newer settings
        // payload already claims migration revision 5.
        self.dashboard_tabs
            .retain(|tab| *tab != DashboardTab::AiControl && *tab != DashboardTab::Usage);
        if self.dashboard_tabs.is_empty() {
            self.dashboard_tabs.push(DashboardTab::Storage);
        }
        self.quick_panel_sections.retain(|section| {
            *section != QuickPanelSection::AiControl && *section != QuickPanelSection::AiUsage
        });
        if self.quick_panel_sections.is_empty() {
            self.quick_panel_sections.push(QuickPanelSection::Storage);
        }

        const SUPPORTED_PROVIDERS: [&str; 5] =
            ["codex", "claude", "opencode", "openrouter", "antigravity"];
        let mut providers = HashSet::new();
        self.quick_panel_ai_providers.retain(|provider| {
            SUPPORTED_PROVIDERS.contains(&provider.as_str()) && providers.insert(provider.clone())
        });
        const SUPPORTED_ACCOUNT_PROVIDERS: [&str; 7] = [
            "codex",
            "claude",
            "opencode",
            "openrouter",
            "antigravity",
            "cursor",
            "grok",
        ];
        let mut account_providers = HashSet::new();
        self.ai_accounts_quota_providers.retain(|provider| {
            SUPPORTED_ACCOUNT_PROVIDERS.contains(&provider.as_str())
                && account_providers.insert(provider.clone())
        });
        if self.ai_accounts_quota_providers.is_empty() {
            self.ai_accounts_quota_providers.push("codex".into());
        }
        self.agent_notifications.inactivity_threshold_minutes = self
            .agent_notifications
            .inactivity_threshold_minutes
            .clamp(5, 120);
        self.ai_control = crate::ai_control_center::budgets::sanitize(self.ai_control);
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
            ai_accounts_quota_providers: Vec::new(),
            ..ZenithSettings::default()
        };

        let sanitized = empty.sanitize();
        assert_eq!(
            sanitized.quick_panel_sections,
            vec![QuickPanelSection::Storage]
        );
        assert_eq!(sanitized.dashboard_tabs, vec![DashboardTab::Storage]);
        assert_eq!(sanitized.ai_accounts_quota_providers, vec!["codex"]);
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
            ai_accounts_quota_providers: vec![
                "cursor".into(),
                "unknown".into(),
                "grok".into(),
                "cursor".into(),
            ],
            ..ZenithSettings::default()
        };

        let sanitized = configured.sanitize();
        assert_eq!(
            sanitized.quick_panel_sections,
            vec![QuickPanelSection::Memory]
        );
        assert_eq!(sanitized.dashboard_tabs, vec![DashboardTab::Storage]);
        assert_eq!(sanitized.quick_panel_ai_providers, vec!["codex"]);
        assert_eq!(
            sanitized.ai_accounts_quota_providers,
            vec!["cursor", "grok"]
        );
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
        assert_eq!(
            parsed.ai_accounts_quota_providers,
            vec!["codex", "claude", "opencode", "openrouter", "antigravity"]
        );
        assert!(parsed.launch_at_login);
        assert_eq!(parsed.theme, "dark");
        assert!(!parsed.intensive_cleanup);
        assert!(!parsed.sidebar_collapsed);
    }

    #[test]
    fn sanitize_adds_new_dashboard_tabs_once_for_existing_settings() {
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
                DashboardTab::Projects,
                DashboardTab::DevelopmentServers,
            ]
        );
        assert_eq!(migrated.dashboard_tabs_revision, 5);

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
