use super::registry::ProviderRegistry;
use super::support::base_provider;
use super::{CollectionContext, ProviderAdapter, ProviderDescriptor, ProviderError};
use crate::models::{AiProviderUsage, ProviderId, UsageSupport};

#[derive(Default)]
pub struct MistralApiAdapter;

impl ProviderAdapter for MistralApiAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::MistralApi
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderRegistry::find(ProviderId::MistralApi)
            .expect("MistralApi must exist in registry")
            .to_descriptor()
    }

    fn collect(&self, ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError> {
        let key = ctx
            .credentials
            .get(ProviderId::MistralApi)
            .map_err(|e| ProviderError::ExecutionFailed(e.to_string()))?
            .ok_or(ProviderError::CredentialMissing)?;

        let response = ctx
            .http_client
            .get("https://api.mistral.ai/v1/models")
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

        let mut provider = base_provider(ProviderId::MistralApi, "Mistral API", "API Key");
        provider.installed = true;
        provider.connected = true;
        provider.support = UsageSupport::Live;
        provider.status_message = "Mistral API key validated via models endpoint.".into();
        provider.action_url = Some("https://console.mistral.ai/billing/".into());

        Ok(provider)
    }
}
