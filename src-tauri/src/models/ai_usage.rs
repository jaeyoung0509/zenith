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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
pub enum ProviderId {
    #[serde(rename = "codex")]
    Codex,
    #[serde(rename = "claude")]
    Claude,
    #[serde(rename = "opencode")]
    OpenCode,
    #[serde(rename = "openrouter")]
    OpenRouter,
    #[serde(rename = "antigravity")]
    Antigravity,
    #[serde(rename = "cursor")]
    Cursor,
    #[serde(rename = "grok-build", alias = "grok")]
    GrokBuild,
    #[serde(rename = "xai-api")]
    XaiApi,
    #[serde(rename = "openai-api")]
    OpenAiApi,
    #[serde(rename = "anthropic-api")]
    AnthropicApi,
    #[serde(rename = "muse-code")]
    MuseCode,
    #[serde(rename = "meta-model-api")]
    MetaModelApi,
    #[serde(rename = "mistral-api")]
    MistralApi,
    #[serde(rename = "fireworks-api")]
    FireworksApi,
}

impl ProviderId {
    pub const ALL: &'static [ProviderId] = &[
        ProviderId::Codex,
        ProviderId::Claude,
        ProviderId::OpenCode,
        ProviderId::OpenRouter,
        ProviderId::Antigravity,
        ProviderId::Cursor,
        ProviderId::GrokBuild,
        ProviderId::XaiApi,
        ProviderId::OpenAiApi,
        ProviderId::AnthropicApi,
        ProviderId::MuseCode,
        ProviderId::MetaModelApi,
        ProviderId::MistralApi,
        ProviderId::FireworksApi,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
            Self::OpenRouter => "openrouter",
            Self::Antigravity => "antigravity",
            Self::Cursor => "cursor",
            Self::GrokBuild => "grok-build",
            Self::XaiApi => "xai-api",
            Self::OpenAiApi => "openai-api",
            Self::AnthropicApi => "anthropic-api",
            Self::MuseCode => "muse-code",
            Self::MetaModelApi => "meta-model-api",
            Self::MistralApi => "mistral-api",
            Self::FireworksApi => "fireworks-api",
        }
    }

    pub fn parse_legacy(s: &str) -> Option<Self> {
        match s.trim() {
            "codex" => Some(Self::Codex),
            "claude" => Some(Self::Claude),
            "opencode" => Some(Self::OpenCode),
            "openrouter" => Some(Self::OpenRouter),
            "antigravity" => Some(Self::Antigravity),
            "cursor" => Some(Self::Cursor),
            "grok" | "grok-build" => Some(Self::GrokBuild),
            "xai-api" => Some(Self::XaiApi),
            "openai-api" => Some(Self::OpenAiApi),
            "anthropic-api" => Some(Self::AnthropicApi),
            "muse-code" => Some(Self::MuseCode),
            "meta-model-api" => Some(Self::MetaModelApi),
            "mistral-api" => Some(Self::MistralApi),
            "fireworks-api" => Some(Self::FireworksApi),
            _ => None,
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ProviderId {
    type Err = UnknownProviderError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_legacy(s).ok_or_else(|| UnknownProviderError(s.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownProviderError(pub String);

impl std::fmt::Display for UnknownProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Unknown AI provider ID: '{}'", self.0)
    }
}

impl std::error::Error for UnknownProviderError {}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct AiProviderUsage {
    pub id: ProviderId,
    pub name: String,
    pub installed: bool,
    pub connected: bool,
    pub auth_label: String,
    pub status_message: String,
    pub support: UsageSupport,
    pub windows: Vec<UsageWindow>,
    pub summary: UsageSummary,
    pub action_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_vendor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_identity: Option<String>,
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
