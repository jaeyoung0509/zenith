use crate::models::{AiControlPreferences, AwakeRule, ProviderId};
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
    Cpu,
    Battery,
    Awake,
}

impl QuickPanelSection {
    /// Default compact-panel order (#280): the next action first, then the
    /// system summary pair, the disk fact, observed AI/services activity and
    /// the keep-awake state.
    pub const DEFAULTS: [Self; 7] = [
        Self::Cleanup,
        Self::Cpu,
        Self::Memory,
        Self::Battery,
        Self::Storage,
        Self::AgentActivity,
        Self::Awake,
    ];

    /// Sections #280 introduced. They are added to an existing saved layout
    /// once, at their default position, without disturbing its relative order.
    pub const ADDED_IN_REVISION_6: [Self; 3] = [Self::Cpu, Self::Battery, Self::Awake];
}

/// Adds `added` to `existing` at the position each item holds in `order`,
/// preserving the relative order of everything the user already had. An item
/// the payload already contains is left untouched, so a user's own placement —
/// including deliberately hiding a new section — survives every later load.
fn merge_ordered_additions(
    existing: &[QuickPanelSection],
    order: &[QuickPanelSection],
    added: &[QuickPanelSection],
) -> Vec<QuickPanelSection> {
    let mut merged = existing.to_vec();
    for candidate in added {
        if merged.contains(candidate) {
            continue;
        }
        let position = order.iter().position(|item| item == candidate);
        let insert_at = position.map_or(merged.len(), |index| {
            merged
                .iter()
                .position(|item| {
                    order
                        .iter()
                        .position(|entry| entry == item)
                        .is_some_and(|existing_index| existing_index > index)
                })
                .unwrap_or(merged.len())
        });
        merged.insert(insert_at.min(merged.len()), *candidate);
    }
    merged
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum DashboardTab {
    Overview,
    #[serde(alias = "disk")]
    Disk,
    Storage,
    /// #280: the former Memory tab is the Memory detail of the Performance
    /// page. `memory` stays a valid persisted value so an upgrade never drops
    /// the user's placement.
    #[serde(alias = "memory")]
    Performance,
    Docker,
    Models,
    Projects,
    DevelopmentServers,
    Usage,
    AiControl,
    Awake,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum DashboardRoute {
    Overview,
    #[serde(alias = "disk")]
    Disk,
    Storage,
    /// The Performance page, Memory detail — the destination the former
    /// `memory` route and its deep links resolve to.
    #[serde(alias = "memory")]
    Memory,
    Performance,
    /// The Performance page's other local details. They are routes rather than
    /// tabs because a summary click must land on the exact destination.
    Cpu,
    Battery,
    Docker,
    Models,
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
            DashboardTab::Overview => Self::Overview,
            DashboardTab::Disk => Self::Disk,
            DashboardTab::Storage => Self::Storage,
            DashboardTab::Performance => Self::Performance,
            DashboardTab::Docker => Self::Docker,
            DashboardTab::Models => Self::Models,
            DashboardTab::Projects => Self::Projects,
            DashboardTab::DevelopmentServers => Self::DevelopmentServers,
            DashboardTab::Usage => Self::Usage,
            DashboardTab::AiControl => Self::AiControl,
            DashboardTab::Awake => Self::Awake,
        }
    }
}

impl DashboardTab {
    pub const ALL: [Self; 8] = [
        Self::Overview,
        Self::Storage,
        Self::Performance,
        Self::Projects,
        Self::Docker,
        Self::Models,
        Self::DevelopmentServers,
        Self::Awake,
    ];
}

fn default_quick_panel_ai_providers() -> Vec<ProviderId> {
    crate::ai_providers::ProviderRegistry::default_quick_panel_providers()
}

fn default_ai_accounts_quota_providers() -> Vec<ProviderId> {
    crate::ai_providers::ProviderRegistry::default_account_providers()
}

fn deserialize_tolerant_provider_ids<'de, D>(deserializer: D) -> Result<Vec<ProviderId>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Vec::<String>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .filter_map(|id| ProviderId::parse_legacy(&id))
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct ZenithSettings {
    pub launch_at_login: bool,
    pub clean_ai_tools: bool,
    pub clean_developer_tools: bool,
    pub clean_docker: bool,
    pub include_rebuild_caches: bool,
    pub intensive_cleanup: bool,
    pub theme: String,
    pub excluded_signatures: Vec<String>,
    pub awake_rules: Vec<AwakeRule>,
    pub quick_panel_sections: Vec<QuickPanelSection>,
    #[serde(
        default = "default_quick_panel_ai_providers",
        deserialize_with = "deserialize_tolerant_provider_ids"
    )]
    #[specta(type = Vec<ProviderId>)]
    pub quick_panel_ai_providers: Vec<ProviderId>,
    #[serde(
        default = "default_ai_accounts_quota_providers",
        deserialize_with = "deserialize_tolerant_provider_ids"
    )]
    #[specta(type = Vec<ProviderId>)]
    pub ai_accounts_quota_providers: Vec<ProviderId>,
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
            include_rebuild_caches: false,
            intensive_cleanup: false,
            theme: "light".to_string(),
            excluded_signatures: Vec::new(),
            awake_rules: vec![
                AwakeRule {
                    id: "rule.codex".to_string(),
                    app_name: "Codex".to_string(),
                    executable_pattern: "codex".to_string(),
                    requires_process_pattern: None,
                    application: None,
                    agent_ids: Vec::new(),
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
                AwakeRule {
                    id: "rule.claude".to_string(),
                    app_name: "Claude Code / Claude Desktop".to_string(),
                    executable_pattern: "claude".to_string(),
                    requires_process_pattern: None,
                    application: None,
                    agent_ids: Vec::new(),
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
                AwakeRule {
                    id: "rule.warp".to_string(),
                    app_name: "Warp".to_string(),
                    executable_pattern: "warp".to_string(),
                    requires_process_pattern: None,
                    application: None,
                    agent_ids: Vec::new(),
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
                AwakeRule {
                    id: "rule.opencode".to_string(),
                    app_name: "Opencode".to_string(),
                    executable_pattern: "opencode".to_string(),
                    requires_process_pattern: None,
                    application: None,
                    agent_ids: Vec::new(),
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
                AwakeRule {
                    id: "rule.omp".to_string(),
                    app_name: "OMP (Opencode)".to_string(),
                    executable_pattern: "omp".to_string(),
                    requires_process_pattern: None,
                    application: None,
                    agent_ids: Vec::new(),
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
                AwakeRule {
                    id: "rule.warp-codex".to_string(),
                    app_name: "Warp + Codex/OMP (compound)".to_string(),
                    executable_pattern: "warp".to_string(),
                    requires_process_pattern: Some("codex|opencode|omp|claude".to_string()),
                    application: None,
                    agent_ids: Vec::new(),
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
                AwakeRule {
                    id: "rule.docker".to_string(),
                    app_name: "Docker Desktop".to_string(),
                    executable_pattern: "com.docker.backend".to_string(),
                    requires_process_pattern: None,
                    application: None,
                    agent_ids: Vec::new(),
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
                AwakeRule {
                    id: "rule.terminal".to_string(),
                    app_name: "Terminal / iTerm2 / Ghostty".to_string(),
                    executable_pattern: "Terminal|iTerm2|ghostty".to_string(),
                    requires_process_pattern: None,
                    application: None,
                    agent_ids: Vec::new(),
                    behavior: crate::models::AwakeBehavior::PreventSystemSleep,
                    power_condition: crate::models::PowerCondition::AcPowerOnly,
                    enabled: false,
                },
            ],
            quick_panel_sections: QuickPanelSection::DEFAULTS.to_vec(),
            quick_panel_ai_providers:
                crate::ai_providers::ProviderRegistry::default_quick_panel_providers(),
            ai_accounts_quota_providers:
                crate::ai_providers::ProviderRegistry::default_account_providers(),
            dashboard_tabs: DashboardTab::ALL.to_vec(),
            dashboard_tabs_revision: 6,
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
        // Add it once after Performance (the former Memory tab), then preserve
        // future user hide/reorder choices.
        if self.dashboard_tabs_revision < 1 {
            if !self
                .dashboard_tabs
                .contains(&DashboardTab::DevelopmentServers)
            {
                let insert_at = self
                    .dashboard_tabs
                    .iter()
                    .position(|tab| *tab == DashboardTab::Performance)
                    .map_or(self.dashboard_tabs.len(), |index| index + 1);
                self.dashboard_tabs
                    .insert(insert_at, DashboardTab::DevelopmentServers);
            }
            self.dashboard_tabs_revision = 1;
        }

        // #75 adds Projects once after Performance. The revision guard preserves
        // any later user hide/reorder choice instead of re-inserting it.
        if self.dashboard_tabs_revision < 2 {
            if !self.dashboard_tabs.contains(&DashboardTab::Projects) {
                let insert_at = self
                    .dashboard_tabs
                    .iter()
                    .position(|tab| *tab == DashboardTab::Performance)
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
                    .position(|tab| *tab == DashboardTab::Performance)
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

        // #280 adds the Overview control tower and turns the Memory tab into
        // the Performance page's Memory detail. Overview joins directly after
        // the tab that currently opens first, so an upgraded user's start page
        // never changes, and the new compact-panel sections join at their
        // default position. Hiding or reordering them afterwards stays
        // authoritative, exactly like the earlier tab migrations.
        if self.dashboard_tabs_revision < 6 {
            if !self.dashboard_tabs.contains(&DashboardTab::Overview) {
                let insert_at = if self.dashboard_tabs.is_empty() { 0 } else { 1 };
                self.dashboard_tabs
                    .insert(insert_at, DashboardTab::Overview);
            }
            self.quick_panel_sections = merge_ordered_additions(
                &self.quick_panel_sections,
                &QuickPanelSection::DEFAULTS,
                &QuickPanelSection::ADDED_IN_REVISION_6,
            );
            self.dashboard_tabs_revision = 6;
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

        // Typed Keep Awake rules use a bounded allowlist and explicit any-of
        // semantics. Preserve pattern strings exactly for genuine legacy
        // rules, and make compatibility fields canonical for typed rules so
        // the two persisted representations cannot drift.
        for rule in &mut self.awake_rules {
            let mut agents = HashSet::new();
            rule.agent_ids.retain(|agent| agents.insert(*agent));
            if let Some(application) = &rule.application {
                rule.app_name = application.display_name.clone();
                rule.executable_pattern = application.executable_name.clone();
                rule.requires_process_pattern = None;
            }
        }

        let mut providers = HashSet::new();
        self.quick_panel_ai_providers.retain(|provider| {
            crate::ai_providers::ProviderRegistry::supports_quick_panel(*provider)
                && providers.insert(*provider)
        });

        let mut account_providers = HashSet::new();
        self.ai_accounts_quota_providers.retain(|provider| {
            crate::ai_providers::ProviderRegistry::is_supported(*provider)
                && account_providers.insert(*provider)
        });
        if self.ai_accounts_quota_providers.is_empty() {
            self.ai_accounts_quota_providers.push(ProviderId::Codex);
        }
        self.agent_notifications.inactivity_threshold_minutes = self
            .agent_notifications
            .inactivity_threshold_minutes
            .clamp(5, 120);
        self.ai_control = crate::ai_control_center::budgets::sanitize(self.ai_control);
        self
    }

    /// Returns whether the specified category is enabled for cleanup under current settings.
    pub fn is_category_clean_enabled(&self, category: crate::models::Category) -> bool {
        match category {
            crate::models::Category::Ai => self.clean_ai_tools,
            crate::models::Category::Developer => self.clean_developer_tools,
            crate::models::Category::Container => self.clean_docker,
            crate::models::Category::Model => false,
            crate::models::Category::System => true,
        }
    }
}

fn legacy_dashboard_tabs_revision() -> u8 {
    0
}

#[cfg(test)]
mod tests {
    use crate::models::{ApplicationIdentity, AwakeAgentId};

    use super::{DashboardTab, ProviderId, QuickPanelSection, ZenithSettings};

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
        assert_eq!(
            sanitized.ai_accounts_quota_providers,
            vec![ProviderId::Codex]
        );
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
            quick_panel_ai_providers: vec![ProviderId::Codex, ProviderId::Codex],
            ai_accounts_quota_providers: vec![
                ProviderId::Cursor,
                ProviderId::GrokBuild,
                ProviderId::Cursor,
            ],
            ..ZenithSettings::default()
        };

        let sanitized = configured.sanitize();
        assert_eq!(
            sanitized.quick_panel_sections,
            vec![QuickPanelSection::Memory]
        );
        assert_eq!(sanitized.dashboard_tabs, vec![DashboardTab::Storage]);
        assert_eq!(sanitized.quick_panel_ai_providers, vec![ProviderId::Codex]);
        assert_eq!(
            sanitized.ai_accounts_quota_providers,
            vec![ProviderId::Cursor, ProviderId::GrokBuild]
        );
    }

    #[test]
    fn sanitize_migrates_legacy_grok_to_grok_build() {
        let raw = r#"{
            "ai_accounts_quota_providers": ["grok"],
            "quick_panel_ai_providers": ["grok"]
        }"#;
        let parsed: ZenithSettings = serde_json::from_str(raw).unwrap();
        let sanitized = parsed.sanitize();
        assert_eq!(
            sanitized.ai_accounts_quota_providers,
            vec![ProviderId::GrokBuild]
        );
        assert!(sanitized.quick_panel_ai_providers.is_empty());
    }

    #[test]
    fn settings_deserializes_tolerantly_with_unknown_providers() {
        let raw = r#"{
            "ai_accounts_quota_providers": ["codex", "future-ai-superprovider", "claude"],
            "quick_panel_ai_providers": ["unknown-ai", "cursor"]
        }"#;
        let parsed: ZenithSettings =
            serde_json::from_str(raw).expect("unknown providers should be safely skipped");
        assert_eq!(
            parsed.ai_accounts_quota_providers,
            vec![ProviderId::Codex, ProviderId::Claude]
        );
        assert_eq!(parsed.quick_panel_ai_providers, vec![ProviderId::Cursor]);
    }

    #[test]
    fn old_settings_receive_quick_panel_and_dashboard_defaults() {
        let raw = r#"{
            "launch_at_login": true,
            "theme": "dark"
        }"#;

        let parsed: ZenithSettings = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.quick_panel_sections.len(), 7);
        assert_eq!(parsed.dashboard_tabs.len(), 8);
        assert_eq!(parsed.dashboard_tabs_revision, 0);
        assert_eq!(
            parsed.ai_accounts_quota_providers,
            vec![
                ProviderId::Codex,
                ProviderId::Claude,
                ProviderId::OpenCode,
                ProviderId::OpenRouter,
                ProviderId::Antigravity,
            ]
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
                DashboardTab::Overview,
                DashboardTab::Performance,
                DashboardTab::Projects,
                DashboardTab::DevelopmentServers,
            ]
        );
        // The legacy "memory" id keeps its slot as the Performance page, and
        // Storage still opens first: an upgrade must not move the start page.
        assert_eq!(
            migrated.dashboard_tabs.first(),
            Some(&DashboardTab::Storage)
        );
        assert_eq!(migrated.dashboard_tabs_revision, 6);

        let hidden_again = ZenithSettings {
            dashboard_tabs: vec![DashboardTab::Storage, DashboardTab::Performance],
            ..migrated
        }
        .sanitize();
        assert_eq!(
            hidden_again.dashboard_tabs,
            vec![DashboardTab::Storage, DashboardTab::Performance]
        );
    }

    #[test]
    fn sanitize_places_overview_after_the_current_start_page_and_adds_new_panel_sections() {
        let raw = r#"{
            "dashboard_tabs": ["docker", "storage"],
            "quick_panel_sections": ["storage", "cleanup", "memory"],
            "dashboard_tabs_revision": 5,
            "theme": "system"
        }"#;

        let migrated: ZenithSettings = serde_json::from_str::<ZenithSettings>(raw)
            .unwrap()
            .sanitize();

        // The customized order survives, and the tab that opened first still
        // opens first with Overview directly behind it.
        assert_eq!(
            migrated.dashboard_tabs,
            vec![
                DashboardTab::Docker,
                DashboardTab::Overview,
                DashboardTab::Storage,
            ]
        );
        // New sections land at their default position relative to the sections
        // the user already had, and the user's own relative order is preserved.
        assert_eq!(
            migrated.quick_panel_sections,
            vec![
                QuickPanelSection::Cpu,
                QuickPanelSection::Battery,
                QuickPanelSection::Storage,
                QuickPanelSection::Cleanup,
                QuickPanelSection::Memory,
                QuickPanelSection::Awake,
            ]
        );

        // A second load must not re-insert anything.
        let again = migrated.clone().sanitize();
        assert_eq!(again.dashboard_tabs, migrated.dashboard_tabs);
        assert_eq!(again.quick_panel_sections, migrated.quick_panel_sections);
    }

    #[test]
    fn sanitize_keeps_a_deliberately_hidden_new_panel_section_hidden() {
        let raw = r#"{
            "dashboard_tabs": ["overview", "storage"],
            "quick_panel_sections": ["cleanup", "memory"],
            "dashboard_tabs_revision": 6,
            "theme": "system"
        }"#;

        let sanitized: ZenithSettings = serde_json::from_str::<ZenithSettings>(raw)
            .unwrap()
            .sanitize();

        assert_eq!(
            sanitized.quick_panel_sections,
            vec![QuickPanelSection::Cleanup, QuickPanelSection::Memory]
        );
        assert_eq!(
            sanitized.dashboard_tabs,
            vec![DashboardTab::Overview, DashboardTab::Storage]
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
    fn sanitize_canonicalizes_typed_awake_rules_without_touching_identity() {
        let mut settings = ZenithSettings::default();
        let rule = settings.awake_rules.first_mut().unwrap();
        rule.application = Some(ApplicationIdentity {
            display_name: "Warp".into(),
            executable_name: "stable".into(),
            path: "/Applications/Warp.app".into(),
        });
        rule.agent_ids = vec![
            AwakeAgentId::Codex,
            AwakeAgentId::OpenCode,
            AwakeAgentId::Codex,
        ];
        rule.app_name = "stale display name".into();
        rule.executable_pattern = "stale|raw|pattern".into();
        rule.requires_process_pattern = Some("must-not-survive".into());

        let sanitized = settings.sanitize();
        let rule = &sanitized.awake_rules[0];
        assert_eq!(
            rule.agent_ids,
            vec![AwakeAgentId::Codex, AwakeAgentId::OpenCode]
        );
        assert_eq!(
            rule.application.as_ref().unwrap().path,
            "/Applications/Warp.app"
        );
        assert_eq!(rule.app_name, "Warp");
        assert_eq!(rule.executable_pattern, "stable");
        assert_eq!(rule.requires_process_pattern, None);
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
