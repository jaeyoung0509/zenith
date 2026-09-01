use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct UsageWindow {
    pub label: String,
    pub used_percent: f64,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub resets_at: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct UsageSummary {
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub lifetime_tokens: Option<u64>,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub last_7d_tokens: Option<u64>,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub peak_daily_tokens: Option<u64>,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub current_streak_days: Option<u64>,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub local_sessions: Option<u64>,
    pub local_cost_usd: Option<f64>,
    pub usage_usd: Option<f64>,
    pub limit_remaining_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum UsageSupport {
    Live,
    Local,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct AiUsageSnapshot {
    pub providers: Vec<AiProviderUsage>,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub fetched_at: u64,
}

impl AiUsageSnapshot {
    pub fn is_fresh_at(&self, now: u64, ttl_secs: u64) -> bool {
        now.saturating_sub(self.fetched_at) < ttl_secs
    }
}

#[cfg(test)]
mod tests {
    use super::AiUsageSnapshot;

    #[test]
    fn snapshot_freshness_honors_ttl_boundary_and_clock_skew() {
        let snapshot = AiUsageSnapshot {
            providers: vec![],
            fetched_at: 100,
        };
        assert!(snapshot.is_fresh_at(159, 60));
        assert!(!snapshot.is_fresh_at(160, 60));
        assert!(snapshot.is_fresh_at(90, 60));
    }
}
