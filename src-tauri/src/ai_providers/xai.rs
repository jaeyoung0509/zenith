use super::registry::ProviderRegistry;
use super::support::{base_provider, command_exists};
use super::{CollectionContext, ProviderAdapter, ProviderDescriptor, ProviderError};
use crate::models::{AiProviderUsage, ProviderId, UsageSupport};
use serde_json::Value;

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

        if let Ok(data) = response.json::<Value>() {
            if let Some(name) = data.get("name").and_then(Value::as_str) {
                provider.auth_label = format!("API Key · {name}");
            }
        }

        Ok(provider)
    }
}
