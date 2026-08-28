use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSourceKind {
    LiveAuthoritative,
    LiveQuota,
    LocalEstimate,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ObservationScope {
    Subscription,
    ApiKey,
    Project,
    Organization,
    LocalSessions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ObservationQuality {
    Fresh,
    Stale,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct MoneyMicros {
    pub micros: i64,
    pub currency: String,
}

impl MoneyMicros {
    pub fn usd(micros: i64) -> Self {
        Self {
            micros: micros.max(0),
            currency: "USD".into(),
        }
    }
    pub fn percent_of(&self, limit: &Self) -> Option<u16> {
        if limit.micros <= 0 || self.currency != limit.currency {
            return None;
        }
        let basis_points =
            (i128::from(self.micros) * 10_000 / i128::from(limit.micros)).clamp(0, 65_535);
        Some(basis_points as u16)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ObservationPeriod {
    pub starts_at: Option<u64>,
    pub ends_at: Option<u64>,
    pub resets_at: Option<u64>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderMetric {
    pub label: String,
    pub tokens: Option<u64>,
    pub cost: Option<MoneyMicros>,
    pub used_basis_points: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderObservation {
    pub provider_id: String,
    pub display_name: String,
    pub source_kind: ObservationSourceKind,
    pub source_id: String,
    pub scope: ObservationScope,
    pub observed_at: u64,
    pub period: ObservationPeriod,
    pub fresh_for_seconds: u64,
    pub quality: ObservationQuality,
    pub installed: bool,
    pub connected: bool,
    pub status_message: String,
    pub metrics: Vec<ProviderMetric>,
    pub action_url: Option<String>,
    pub partial_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum BudgetPeriod {
    Weekly,
    Monthly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct LocalAlertBudget {
    pub id: String,
    pub provider_id: Option<String>,
    pub period: BudgetPeriod,
    pub limit: MoneyMicros,
    pub threshold_percents: Vec<u8>,
    pub enabled: bool,
}

impl Default for LocalAlertBudget {
    fn default() -> Self {
        Self {
            id: "budget.monthly".into(),
            provider_id: None,
            period: BudgetPeriod::Monthly,
            limit: MoneyMicros::usd(50_000_000),
            threshold_percents: vec![50, 80, 100],
            enabled: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct BudgetStatus {
    pub budget_id: String,
    pub spent: MoneyMicros,
    pub limit: MoneyMicros,
    pub used_basis_points: u16,
    pub crossed_thresholds: Vec<u8>,
    pub source_label: String,
    pub mixed_sources: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct ManualProviderUsage {
    pub provider_id: String,
    pub spent: MoneyMicros,
    pub limit: Option<MoneyMicros>,
    pub resets_at: Option<u64>,
    pub entered_at: u64,
}
impl Default for ManualProviderUsage {
    fn default() -> Self {
        Self {
            provider_id: String::new(),
            spent: MoneyMicros::usd(0),
            limit: None,
            resets_at: None,
            entered_at: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct AutopilotPreferences {
    pub keep_awake_for_verified_sessions: bool,
    pub keep_awake_ac_only: bool,
    pub notify_on_battery: bool,
    pub notify_on_memory_pressure: bool,
    pub notify_on_session_completion: bool,
    pub recommendation_cooldown_seconds: u64,
}
impl Default for AutopilotPreferences {
    fn default() -> Self {
        Self {
            keep_awake_for_verified_sessions: false,
            keep_awake_ac_only: true,
            notify_on_battery: false,
            notify_on_memory_pressure: false,
            notify_on_session_completion: false,
            recommendation_cooldown_seconds: 900,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct AiControlPreferences {
    pub budgets: Vec<LocalAlertBudget>,
    pub manual_usage: Vec<ManualProviderUsage>,
    pub autopilot: AutopilotPreferences,
    pub dismissed_findings: Vec<String>,
    pub audit_retention_days: u16,
}
impl Default for AiControlPreferences {
    fn default() -> Self {
        Self {
            budgets: vec![LocalAlertBudget::default()],
            manual_usage: vec![],
            autopilot: AutopilotPreferences::default(),
            dismissed_findings: vec![],
            audit_retention_days: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ResourceAttribution {
    pub session_id: String,
    pub project_id: Option<String>,
    pub tool_name: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub process_count: u32,
    pub duration_seconds: u64,
    pub open_dev_ports: u32,
    pub power_eligible: bool,
    pub confidence: String,
    pub reason: String,
    pub mutable_actions_allowed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationKind {
    Battery,
    Memory,
    SessionCompleted,
    OrphanProcess,
    DevelopmentPort,
    CleanupReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Recommendation {
    pub id: String,
    pub kind: RecommendationKind,
    pub title: String,
    pub message: String,
    pub created_at: u64,
    pub cooldown_until: u64,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub action_label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SafetyFindingKind {
    SecretsExposure,
    ToolPermissions,
    McpServers,
    ProtectedPaths,
    GitChanges,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SafetyFinding {
    pub id: String,
    pub project_id: String,
    pub kind: SafetyFindingKind,
    pub severity: FindingSeverity,
    pub evidence_type: String,
    pub adapter: String,
    pub relative_path: Option<String>,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
    pub observed_at: u64,
    pub remediation: String,
    pub dismissed: bool,
    pub normalized_evidence: Option<NormalizedSafetyEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct NormalizedSafetyEvidence {
    pub server_name: Option<String>,
    pub scope: Option<String>,
    pub transport: Option<String>,
    pub permission_mode: Option<String>,
    pub sandbox_mode: Option<String>,
    pub command_basename: Option<String>,
    pub domain: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SafetySnapshot {
    pub observed_at: u64,
    pub quality: ObservationQuality,
    pub findings: Vec<SafetyFinding>,
    pub scanned_files: u32,
    pub skipped_files: u32,
    pub status_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct GitChangeSummary {
    pub project_id: String,
    pub baseline_head: Option<String>,
    pub current_head: Option<String>,
    pub baseline_at: u64,
    pub added: u32,
    pub modified: u32,
    pub deleted: u32,
    pub renamed: u32,
    pub untracked: u32,
    pub changed_paths: Vec<String>,
    pub available: bool,
    pub status_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: u64,
    pub event_kind: String,
    pub outcome: String,
    pub project_ref: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ControlCenterQuickSummary {
    pub observed_at: u64,
    pub active_sessions: u32,
    pub budget_alerts: u32,
    pub safety_findings: u32,
    pub quality: ObservationQuality,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct AiControlCenterSnapshot {
    pub observed_at: u64,
    pub providers: Vec<ProviderObservation>,
    pub budget_statuses: Vec<BudgetStatus>,
    pub resources: Vec<ResourceAttribution>,
    pub recommendations: Vec<Recommendation>,
    pub safety: SafetySnapshot,
    pub git_summaries: Vec<GitChangeSummary>,
    pub audit: Vec<AuditEntry>,
    pub quick_summary: ControlCenterQuickSummary,
    pub keep_awake_active: bool,
    pub partial_errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RecommendationPreview {
    pub id: String,
    pub recommendation_id: String,
    pub title: String,
    pub explanation: String,
    pub destination: String,
    pub expires_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn money_thresholds_are_integer_and_deterministic() {
        assert_eq!(
            MoneyMicros::usd(25_000_000).percent_of(&MoneyMicros::usd(50_000_000)),
            Some(5_000)
        );
        assert_eq!(
            MoneyMicros::usd(40_000_000).percent_of(&MoneyMicros::usd(50_000_000)),
            Some(8_000)
        );
        assert_eq!(
            MoneyMicros::usd(50_000_000).percent_of(&MoneyMicros::usd(50_000_000)),
            Some(10_000)
        );
        assert_eq!(MoneyMicros::usd(1).percent_of(&MoneyMicros::usd(0)), None);
    }
}
