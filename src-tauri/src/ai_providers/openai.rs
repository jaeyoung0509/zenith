use super::registry::ProviderRegistry;
use super::support::base_provider;
use super::{CollectionContext, ProviderAdapter, ProviderDescriptor, ProviderError};
use crate::models::{AiProviderUsage, ProviderId, UsageSupport};

#[derive(Default)]
pub struct OpenAiApiAdapter;

impl ProviderAdapter for OpenAiApiAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::OpenAiApi
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderRegistry::find(ProviderId::OpenAiApi)
            .expect("OpenAiApi must exist in registry")
            .to_descriptor()
    }

    fn collect(&self, ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError> {
        let key = ctx
            .credentials
            .get(ProviderId::OpenAiApi)
            .map_err(|e| ProviderError::ExecutionFailed(e.to_string()))?
            .ok_or(ProviderError::CredentialMissing)?;

        let response = ctx
            .http_client
            .get("https://api.openai.com/v1/models")
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

        let mut provider = base_provider(ProviderId::OpenAiApi, "OpenAI API", "API Key");
        provider.installed = true;
        provider.connected = true;
        provider.support = UsageSupport::Live;
        provider.status_message = "OpenAI API key validated via models endpoint.".into();
        provider.action_url = Some("https://platform.openai.com/usage".into());

        Ok(provider)
    }
}
