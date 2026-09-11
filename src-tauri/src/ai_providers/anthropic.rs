use super::registry::ProviderRegistry;
use super::support::base_provider;
use super::{CollectionContext, ProviderAdapter, ProviderDescriptor, ProviderError};
use crate::models::{AiProviderUsage, ProviderId, UsageSupport};

#[derive(Default)]
pub struct AnthropicApiAdapter;

impl ProviderAdapter for AnthropicApiAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::AnthropicApi
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderRegistry::find(ProviderId::AnthropicApi)
            .expect("AnthropicApi must exist in registry")
            .to_descriptor()
    }

    fn collect(&self, ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError> {
        let key = ctx
            .credentials
            .get(ProviderId::AnthropicApi)
            .map_err(|e| ProviderError::ExecutionFailed(e.to_string()))?
            .ok_or(ProviderError::CredentialMissing)?;

        let response = ctx
            .http_client
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", key.expose_secret())
            .header("anthropic-version", "2023-06-01")
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

        let mut provider = base_provider(ProviderId::AnthropicApi, "Anthropic API", "API Key");
        provider.installed = true;
        provider.connected = true;
        provider.support = UsageSupport::Live;
        provider.status_message = "Anthropic API key validated via models endpoint.".into();
        provider.action_url = Some("https://console.anthropic.com/settings/cost".into());

        Ok(provider)
    }
}
