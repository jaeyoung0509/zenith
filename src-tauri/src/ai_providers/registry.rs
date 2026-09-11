use crate::models::{ObservationScope, ObservationSourceKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    None,
    ApiKey,
    OAuth,
    Cli,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderDescriptor {
    pub id: String,
    pub display_name: String,
    pub scope: ObservationScope,
    pub credential_kind: CredentialKind,
    pub source_kind: ObservationSourceKind,
    pub supports_quick_panel: bool,
    pub model_vendor: Option<String>,
    pub model_identity: Option<String>,
    pub description: String,
    pub default_quota_provider: bool,
}

pub struct StaticProviderDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub scope: ObservationScope,
    pub credential_kind: CredentialKind,
    pub source_kind: ObservationSourceKind,
    pub supports_quick_panel: bool,
    pub model_vendor: Option<&'static str>,
    pub model_identity: Option<&'static str>,
    pub description: &'static str,
    pub default_quota_provider: bool,
}

impl StaticProviderDescriptor {
    pub fn to_descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: self.id.to_string(),
            display_name: self.display_name.to_string(),
            scope: self.scope,
            credential_kind: self.credential_kind,
            source_kind: self.source_kind,
            supports_quick_panel: self.supports_quick_panel,
            model_vendor: self.model_vendor.map(String::from),
            model_identity: self.model_identity.map(String::from),
            description: self.description.to_string(),
            default_quota_provider: self.default_quota_provider,
        }
    }
}

pub static PROVIDER_REGISTRY: &[StaticProviderDescriptor] = &[
    StaticProviderDescriptor {
        id: "codex",
        display_name: "Codex",
        scope: ObservationScope::Subscription,
        credential_kind: CredentialKind::OAuth,
        source_kind: ObservationSourceKind::LiveQuota,
        supports_quick_panel: true,
        model_vendor: Some("OpenAI"),
        model_identity: None,
        description: "Live ChatGPT account limits through the official app server.",
        default_quota_provider: true,
    },
    StaticProviderDescriptor {
        id: "claude",
        display_name: "Claude Code",
        scope: ObservationScope::Subscription,
        credential_kind: CredentialKind::Cli,
        source_kind: ObservationSourceKind::Manual,
        supports_quick_panel: true,
        model_vendor: Some("Anthropic"),
        model_identity: None,
        description: "Local availability with quota checked in Claude /usage.",
        default_quota_provider: true,
    },
    StaticProviderDescriptor {
        id: "opencode",
        display_name: "OpenCode",
        scope: ObservationScope::LocalSessions,
        credential_kind: CredentialKind::Cli,
        source_kind: ObservationSourceKind::LocalEstimate,
        supports_quick_panel: true,
        model_vendor: None,
        model_identity: None,
        description: "Local sessions and cost from connected providers.",
        default_quota_provider: true,
    },
    StaticProviderDescriptor {
        id: "openrouter",
        display_name: "OpenRouter",
        scope: ObservationScope::ApiKey,
        credential_kind: CredentialKind::OAuth,
        source_kind: ObservationSourceKind::LiveAuthoritative,
        supports_quick_panel: true,
        model_vendor: None,
        model_identity: None,
        description: "Live key usage through Zenith OAuth.",
        default_quota_provider: true,
    },
    StaticProviderDescriptor {
        id: "antigravity",
        display_name: "Antigravity",
        scope: ObservationScope::Subscription,
        credential_kind: CredentialKind::OAuth,
        source_kind: ObservationSourceKind::LiveQuota,
        supports_quick_panel: true,
        model_vendor: Some("Google"),
        model_identity: None,
        description: "Live Gemini and Claude/GPT limits from agy.",
        default_quota_provider: true,
    },
    StaticProviderDescriptor {
        id: "cursor",
        display_name: "Cursor",
        scope: ObservationScope::Subscription,
        credential_kind: CredentialKind::None,
        source_kind: ObservationSourceKind::Manual,
        supports_quick_panel: false,
        model_vendor: None,
        model_identity: None,
        description: "Local availability; quota stays in Cursor settings.",
        default_quota_provider: false,
    },
    StaticProviderDescriptor {
        id: "grok-build",
        display_name: "Grok Build",
        scope: ObservationScope::Subscription,
        credential_kind: CredentialKind::None,
        source_kind: ObservationSourceKind::Manual,
        supports_quick_panel: false,
        model_vendor: Some("xAI"),
        model_identity: None,
        description: "Local availability; quota stays in the provider client.",
        default_quota_provider: false,
    },
    StaticProviderDescriptor {
        id: "xai-api",
        display_name: "xAI API",
        scope: ObservationScope::Organization,
        credential_kind: CredentialKind::ApiKey,
        source_kind: ObservationSourceKind::LiveAuthoritative,
        supports_quick_panel: false,
        model_vendor: Some("xAI"),
        model_identity: None,
        description: "xAI team/organization usage and costs via official management API.",
        default_quota_provider: false,
    },
    StaticProviderDescriptor {
        id: "openai-api",
        display_name: "OpenAI API",
        scope: ObservationScope::Organization,
        credential_kind: CredentialKind::ApiKey,
        source_kind: ObservationSourceKind::LiveAuthoritative,
        supports_quick_panel: false,
        model_vendor: Some("OpenAI"),
        model_identity: None,
        description: "Organization API usage and costs separate from Codex subscription.",
        default_quota_provider: false,
    },
    StaticProviderDescriptor {
        id: "anthropic-api",
        display_name: "Anthropic API",
        scope: ObservationScope::Organization,
        credential_kind: CredentialKind::ApiKey,
        source_kind: ObservationSourceKind::LiveAuthoritative,
        supports_quick_panel: false,
        model_vendor: Some("Anthropic"),
        model_identity: None,
        description:
            "Organization API usage and costs separate from Claude individual subscription.",
        default_quota_provider: false,
    },
    StaticProviderDescriptor {
        id: "muse-code",
        display_name: "Muse Code",
        scope: ObservationScope::Subscription,
        credential_kind: CredentialKind::Cli,
        source_kind: ObservationSourceKind::Manual,
        supports_quick_panel: false,
        model_vendor: Some("Meta"),
        model_identity: Some("Muse Spark"),
        description: "Meta terminal coding agent powered by Muse Spark.",
        default_quota_provider: false,
    },
    StaticProviderDescriptor {
        id: "meta-model-api",
        display_name: "Meta Model API",
        scope: ObservationScope::ApiKey,
        credential_kind: CredentialKind::ApiKey,
        source_kind: ObservationSourceKind::LiveAuthoritative,
        supports_quick_panel: false,
        model_vendor: Some("Meta"),
        model_identity: Some("Muse Spark 1.3"),
        description: "Direct API access to Muse Spark models.",
        default_quota_provider: false,
    },
    StaticProviderDescriptor {
        id: "mistral-api",
        display_name: "Mistral API",
        scope: ObservationScope::Organization,
        credential_kind: CredentialKind::ApiKey,
        source_kind: ObservationSourceKind::LiveAuthoritative,
        supports_quick_panel: false,
        model_vendor: Some("Mistral"),
        model_identity: None,
        description: "Official Mistral workspace usage and billing.",
        default_quota_provider: false,
    },
    StaticProviderDescriptor {
        id: "fireworks-api",
        display_name: "Fireworks API",
        scope: ObservationScope::Organization,
        credential_kind: CredentialKind::ApiKey,
        source_kind: ObservationSourceKind::LiveAuthoritative,
        supports_quick_panel: false,
        model_vendor: Some("Fireworks"),
        model_identity: None,
        description: "Official Fireworks AI developer usage and billing.",
        default_quota_provider: false,
    },
];

pub struct ProviderRegistry;

impl ProviderRegistry {
    pub fn all() -> Vec<ProviderDescriptor> {
        PROVIDER_REGISTRY
            .iter()
            .map(StaticProviderDescriptor::to_descriptor)
            .collect()
    }

    pub fn static_all() -> &'static [StaticProviderDescriptor] {
        PROVIDER_REGISTRY
    }

    pub fn find(id: &str) -> Option<&'static StaticProviderDescriptor> {
        let normalized = Self::normalize_provider_id(id).unwrap_or(id);
        PROVIDER_REGISTRY.iter().find(|item| item.id == normalized)
    }

    /// Normalizes legacy provider IDs to their canonical current surfaces.
    /// E.g. "grok" migrates to "grok-build".
    /// Subscriptions are NOT migrated to API identities.
    pub fn normalize_provider_id(id: &str) -> Option<&'static str> {
        match id {
            "grok" => Some("grok-build"),
            other => PROVIDER_REGISTRY
                .iter()
                .find(|item| item.id == other)
                .map(|item| item.id),
        }
    }

    pub fn is_supported(id: &str) -> bool {
        Self::normalize_provider_id(id).is_some()
    }

    pub fn supports_quick_panel(id: &str) -> bool {
        Self::find(id).is_some_and(|item| item.supports_quick_panel)
    }

    pub fn default_account_providers() -> Vec<String> {
        PROVIDER_REGISTRY
            .iter()
            .filter(|item| item.default_quota_provider)
            .map(|item| item.id.to_string())
            .collect()
    }

    pub fn default_quick_panel_providers() -> Vec<String> {
        PROVIDER_REGISTRY
            .iter()
            .filter(|item| item.supports_quick_panel)
            .map(|item| item.id.to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_all_canonical_providers() {
        assert!(ProviderRegistry::find("codex").is_some());
        assert!(ProviderRegistry::find("claude").is_some());
        assert!(ProviderRegistry::find("opencode").is_some());
        assert!(ProviderRegistry::find("openrouter").is_some());
        assert!(ProviderRegistry::find("antigravity").is_some());
        assert!(ProviderRegistry::find("cursor").is_some());
        assert!(ProviderRegistry::find("grok-build").is_some());
        assert!(ProviderRegistry::find("xai-api").is_some());
        assert!(ProviderRegistry::find("openai-api").is_some());
        assert!(ProviderRegistry::find("anthropic-api").is_some());
        assert!(ProviderRegistry::find("muse-code").is_some());
        assert!(ProviderRegistry::find("meta-model-api").is_some());
        assert!(ProviderRegistry::find("mistral-api").is_some());
        assert!(ProviderRegistry::find("fireworks-api").is_some());
    }

    #[test]
    fn grok_migrates_to_grok_build() {
        assert_eq!(
            ProviderRegistry::normalize_provider_id("grok"),
            Some("grok-build")
        );
        let desc = ProviderRegistry::find("grok").unwrap();
        assert_eq!(desc.id, "grok-build");
        assert_eq!(desc.display_name, "Grok Build");
    }

    #[test]
    fn invalid_migrations_are_not_supported() {
        // Must not collapse subscriptions into API scopes
        assert_ne!(
            ProviderRegistry::normalize_provider_id("grok"),
            Some("xai-api")
        );
        assert_ne!(
            ProviderRegistry::normalize_provider_id("claude"),
            Some("anthropic-api")
        );
        assert_ne!(
            ProviderRegistry::normalize_provider_id("codex"),
            Some("openai-api")
        );
        assert_ne!(
            ProviderRegistry::normalize_provider_id("muse-code"),
            Some("meta-model-api")
        );
    }

    #[test]
    fn meta_surfaces_remain_distinct() {
        let muse = ProviderRegistry::find("muse-code").unwrap();
        let api = ProviderRegistry::find("meta-model-api").unwrap();

        assert_eq!(muse.scope, ObservationScope::Subscription);
        assert_eq!(api.scope, ObservationScope::ApiKey);
        assert_eq!(muse.model_vendor, Some("Meta"));
        assert_eq!(api.model_vendor, Some("Meta"));
        assert_eq!(muse.model_identity, Some("Muse Spark"));
        assert_eq!(api.model_identity, Some("Muse Spark 1.3"));
        assert_ne!(muse.id, api.id);
    }

    #[test]
    fn quick_panel_support_is_bounded() {
        assert!(ProviderRegistry::supports_quick_panel("codex"));
        assert!(ProviderRegistry::supports_quick_panel("claude"));
        assert!(ProviderRegistry::supports_quick_panel("opencode"));
        assert!(ProviderRegistry::supports_quick_panel("openrouter"));
        assert!(ProviderRegistry::supports_quick_panel("antigravity"));
        assert!(!ProviderRegistry::supports_quick_panel("cursor"));
        assert!(!ProviderRegistry::supports_quick_panel("grok-build"));
        assert!(!ProviderRegistry::supports_quick_panel("xai-api"));
        assert!(!ProviderRegistry::supports_quick_panel("openai-api"));
    }
}
