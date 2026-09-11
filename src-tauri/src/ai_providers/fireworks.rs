use super::registry::ProviderRegistry;
use super::support::base_provider;
use super::{CollectionContext, ProviderAdapter, ProviderDescriptor, ProviderError};
use crate::models::{AiProviderUsage, ProviderId, UsageSupport};

#[derive(Default)]
pub struct FireworksApiAdapter;

impl ProviderAdapter for FireworksApiAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::FireworksApi
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderRegistry::find(ProviderId::FireworksApi)
            .expect("FireworksApi must exist in registry")
            .to_descriptor()
    }

    fn collect(&self, ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError> {
        let key = ctx
            .credentials
            .get(ProviderId::FireworksApi)
            .map_err(|e| ProviderError::ExecutionFailed(e.to_string()))?
            .ok_or(ProviderError::CredentialMissing)?;

        let response = ctx
            .http_client
            .get("https://api.fireworks.ai/inference/v1/models")
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

        let mut provider = base_provider(ProviderId::FireworksApi, "Fireworks API", "API Key");
        provider.installed = true;
        provider.connected = true;
        provider.support = UsageSupport::Live;
        provider.status_message = "Fireworks API key validated via models endpoint.".into();
        provider.action_url = Some("https://fireworks.ai/account/billing".into());

        Ok(provider)
    }
}
