use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageWindow {
    pub label: String,
    pub used_percent: f64,
    pub resets_at: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageSummary {
    pub lifetime_tokens: Option<u64>,
    pub last_7d_tokens: Option<u64>,
    pub peak_daily_tokens: Option<u64>,
    pub current_streak_days: Option<u64>,
    pub local_sessions: Option<u64>,
    pub local_cost_usd: Option<f64>,
    pub usage_usd: Option<f64>,
    pub limit_remaining_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSupport {
    Live,
    Local,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProviderUsage {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub connected: bool,
    pub auth_label: String,
    pub status_message: String,
    pub support: UsageSupport,
    pub windows: Vec<UsageWindow>,
    pub summary: UsageSummary,
    pub action_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiUsageSnapshot {
    pub providers: Vec<AiProviderUsage>,
    pub fetched_at: u64,
}
