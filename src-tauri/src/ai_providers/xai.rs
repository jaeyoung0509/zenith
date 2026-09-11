use super::registry::ProviderRegistry;
use super::support::{base_provider, command_exists};
use super::{CollectionContext, ProviderAdapter, ProviderDescriptor, ProviderError};
use crate::models::{AiProviderUsage, ProviderId, UsageSupport};

#[derive(Default)]
pub struct GrokBuildAdapter;

impl ProviderAdapter for GrokBuildAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::GrokBuild
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderRegistry::find(ProviderId::GrokBuild)
            .expect("GrokBuild must exist in registry")
            .to_descriptor()
    }

    fn collect(&self, _ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError> {
        let installed = command_exists("grok");
        let mut provider = base_provider(ProviderId::GrokBuild, "Grok Build", "xAI account");
        provider.installed = installed;
        provider.connected = false;
        provider.support = UsageSupport::Manual;
        provider.status_message = if installed {
            "Grok Build does not expose account quota to Zenith; check usage in the provider client.".into()
        } else {
            "Grok Build was not detected in PATH or known tool locations.".into()
        };
        provider.action_url = None;
        Ok(provider)
    }
}

#[derive(Default)]
pub struct XaiApiAdapter;

impl ProviderAdapter for XaiApiAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::XaiApi
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderRegistry::find(ProviderId::XaiApi)
            .expect("XaiApi must exist in registry")
            .to_descriptor()
    }

    fn collect(&self, ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError> {
        let key = ctx
            .credentials
            .get(ProviderId::XaiApi)
            .map_err(|e| ProviderError::ExecutionFailed(e.to_string()))?
            .ok_or(ProviderError::CredentialMissing)?;

        let response = ctx
            .http_client
            .get("https://api.x.ai/v1/api-key")
            .bearer_auth(key.expose_secret())
            .send()
            .map_err(|err| ProviderError::Network(err.to_string()))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(ProviderError::AuthenticationFailed(
                "Invalid API key or insufficient permissions.".into(),
            ));
        }
        if !status.is_success() {
            return Err(ProviderError::Network(format!(
                "API returned HTTP status {status}"
            )));
        }

        let mut provider = base_provider(ProviderId::XaiApi, "xAI API", "API Key");
        provider.installed = true;
        provider.connected = true;
        provider.support = UsageSupport::Live;
        provider.status_message = "xAI API key validated via official xAI API.".into();
        provider.action_url = Some("https://console.x.ai/team/billing".into());
        // The provider-assigned key nickname can contain a person, team, or
        // machine name, so it is deliberately never read or surfaced.
        Ok(provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_supplied_key_nicknames_are_never_surfaced() {
        // The adapter must not copy `name` from the upstream response.
        let source = include_str!("xai.rs");
        let body = source
            .split("impl ProviderAdapter for XaiApiAdapter")
            .nth(1)
            .and_then(|rest| rest.split("#[cfg(test)]").next())
            .expect("adapter is defined");
        assert!(
            !body.contains(r#"data.get("name")"#),
            "xAI key nickname must not be read into auth_label"
        );
        let provider = base_provider(ProviderId::XaiApi, "xAI API", "API Key");
        assert_eq!(provider.auth_label, "API Key");
    }
}
