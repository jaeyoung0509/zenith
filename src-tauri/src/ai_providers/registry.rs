use crate::models::{ObservationScope, ObservationSourceKind, ProviderId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    None,
    ApiKey,
    #[serde(rename = "oauth")]
    OAuth,
    Cli,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderDescriptor {
    pub id: ProviderId,
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
    pub id: ProviderId,
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
            id: self.id,
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
        id: ProviderId::Codex,
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
        id: ProviderId::Claude,
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
        id: ProviderId::OpenCode,
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
        id: ProviderId::OpenRouter,
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
        id: ProviderId::Antigravity,
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
        id: ProviderId::Cursor,
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
        id: ProviderId::GrokBuild,
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
        id: ProviderId::XaiApi,
        display_name: "xAI API",
        scope: ObservationScope::Organization,
        credential_kind: CredentialKind::ApiKey,
        source_kind: ObservationSourceKind::CredentialValidated,
        supports_quick_panel: false,
        model_vendor: Some("xAI"),
        model_identity: None,
        description: "xAI API key validation (models endpoint).",
        default_quota_provider: false,
    },
    StaticProviderDescriptor {
        id: ProviderId::OpenAiApi,
        display_name: "OpenAI API",
        scope: ObservationScope::Organization,
        credential_kind: CredentialKind::ApiKey,
        source_kind: ObservationSourceKind::CredentialValidated,
        supports_quick_panel: false,
        model_vendor: Some("OpenAI"),
        model_identity: None,
        description: "OpenAI API key validation (models endpoint).",
        default_quota_provider: false,
    },
    StaticProviderDescriptor {
        id: ProviderId::AnthropicApi,
        display_name: "Anthropic API",
        scope: ObservationScope::Organization,
        credential_kind: CredentialKind::ApiKey,
        source_kind: ObservationSourceKind::CredentialValidated,
        supports_quick_panel: false,
        model_vendor: Some("Anthropic"),
        model_identity: None,
        description: "Anthropic API key validation (models endpoint).",
        default_quota_provider: false,
    },
    StaticProviderDescriptor {
        id: ProviderId::MuseCode,
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
        id: ProviderId::MetaModelApi,
        display_name: "Meta Model API",
        scope: ObservationScope::ApiKey,
        credential_kind: CredentialKind::ApiKey,
        source_kind: ObservationSourceKind::LocalEstimate,
        supports_quick_panel: false,
        model_vendor: Some("Meta"),
        model_identity: Some("Muse Spark 1.3"),
        description: "Meta Model API key stored in secure credential store (unvalidated).",
        default_quota_provider: false,
    },
    StaticProviderDescriptor {
        id: ProviderId::MistralApi,
        display_name: "Mistral API",
        scope: ObservationScope::Organization,
        credential_kind: CredentialKind::ApiKey,
        source_kind: ObservationSourceKind::CredentialValidated,
        supports_quick_panel: false,
        model_vendor: Some("Mistral"),
        model_identity: None,
        description: "Mistral API key validation (models endpoint).",
        default_quota_provider: false,
    },
    StaticProviderDescriptor {
        id: ProviderId::FireworksApi,
        display_name: "Fireworks API",
        scope: ObservationScope::Organization,
        credential_kind: CredentialKind::ApiKey,
        source_kind: ObservationSourceKind::CredentialValidated,
        supports_quick_panel: false,
        model_vendor: Some("Fireworks"),
        model_identity: None,
        description: "Fireworks API key validation (models endpoint).",
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

    pub fn find(id: ProviderId) -> Option<&'static StaticProviderDescriptor> {
        PROVIDER_REGISTRY.iter().find(|item| item.id == id)
    }

    pub fn find_legacy(id: &str) -> Option<&'static StaticProviderDescriptor> {
        ProviderId::parse_legacy(id).and_then(Self::find)
    }

    /// Normalizes legacy provider string IDs to typed ProviderId.
    /// E.g. "grok" migrates to ProviderId::GrokBuild.
    pub fn normalize_provider_id(id: &str) -> Option<ProviderId> {
        ProviderId::parse_legacy(id)
    }

    pub fn is_supported(id: ProviderId) -> bool {
        Self::find(id).is_some()
    }

    pub fn supports_quick_panel(id: ProviderId) -> bool {
        Self::find(id).is_some_and(|item| item.supports_quick_panel)
    }

    pub fn default_account_providers() -> Vec<ProviderId> {
        PROVIDER_REGISTRY
            .iter()
            .filter(|item| item.default_quota_provider)
            .map(|item| item.id)
            .collect()
    }

    pub fn default_quick_panel_providers() -> Vec<ProviderId> {
        PROVIDER_REGISTRY
            .iter()
            .filter(|item| item.supports_quick_panel)
            .map(|item| item.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_all_canonical_providers() {
        for id in ProviderId::ALL {
            assert!(ProviderRegistry::find(*id).is_some());
        }
    }

    #[test]
    fn grok_migrates_to_grok_build() {
        assert_eq!(
            ProviderRegistry::normalize_provider_id("grok"),
            Some(ProviderId::GrokBuild)
        );
        let desc = ProviderRegistry::find_legacy("grok").unwrap();
        assert_eq!(desc.id, ProviderId::GrokBuild);
        assert_eq!(desc.display_name, "Grok Build");
    }

    #[test]
    fn invalid_migrations_are_not_supported() {
        // Must not collapse subscriptions into API scopes
        assert_ne!(
            ProviderRegistry::normalize_provider_id("grok"),
            Some(ProviderId::XaiApi)
        );
        assert_ne!(
            ProviderRegistry::normalize_provider_id("claude"),
            Some(ProviderId::AnthropicApi)
        );
        assert_ne!(
            ProviderRegistry::normalize_provider_id("codex"),
            Some(ProviderId::OpenAiApi)
        );
        assert_ne!(
            ProviderRegistry::normalize_provider_id("muse-code"),
            Some(ProviderId::MetaModelApi)
        );
    }

    #[test]
    fn meta_surfaces_remain_distinct() {
        let muse = ProviderRegistry::find(ProviderId::MuseCode).unwrap();
        let api = ProviderRegistry::find(ProviderId::MetaModelApi).unwrap();

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
        assert!(ProviderRegistry::supports_quick_panel(ProviderId::Codex));
        assert!(ProviderRegistry::supports_quick_panel(ProviderId::Claude));
        assert!(ProviderRegistry::supports_quick_panel(ProviderId::OpenCode));
        assert!(ProviderRegistry::supports_quick_panel(
            ProviderId::OpenRouter
        ));
        assert!(ProviderRegistry::supports_quick_panel(
            ProviderId::Antigravity
        ));
        assert!(!ProviderRegistry::supports_quick_panel(ProviderId::Cursor));
        assert!(!ProviderRegistry::supports_quick_panel(
            ProviderId::GrokBuild
        ));
        assert!(!ProviderRegistry::supports_quick_panel(ProviderId::XaiApi));
        assert!(!ProviderRegistry::supports_quick_panel(
            ProviderId::OpenAiApi
        ));
    }
}
