use super::credentials::CredentialStore;
use super::support::base_provider;
use super::ProviderAdapter;
use crate::models::{AiProviderUsage, UsageSupport};
use std::time::Duration;

#[derive(Default)]
pub struct AnthropicApiAdapter;

impl ProviderAdapter for AnthropicApiAdapter {
    fn id(&self) -> &'static str {
        "anthropic-api"
    }

    fn collect(&self, credentials: &dyn CredentialStore) -> AiProviderUsage {
        let mut provider = base_provider("anthropic-api", "Anthropic API", "Anthropic API Key");
        provider.installed = true;
        provider.connected = false;
        provider.support = UsageSupport::Live;
        provider.action_url = Some("https://console.anthropic.com/settings/cost".into());

        let key = match credentials.get("anthropic-api") {
            Ok(Some(secret)) => secret,
            _ => {
                provider.status_message =
                    "No API key configured for Anthropic API in secure store.".into();
                return provider;
            }
        };

        let client = match reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(err) => {
                let msg = format!("Failed to create Anthropic HTTP client: {err}");
                crate::diagnostics::log_error("ai_providers", &msg);
                provider.status_message = msg;
                return provider;
            }
        };

        // Validate key against models endpoint with required anthropic-version header
        match client
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", key.expose_secret())
            .header("anthropic-version", "2023-06-01")
            .send()
        {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    provider.connected = true;
                    provider.status_message =
                        "Live authoritative organization connection via official Anthropic API."
                            .into();
                } else if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    provider.connected = false;
                    provider.status_message =
                        "Anthropic API authentication failed (invalid API key).".into();
                } else {
                    provider.status_message = format!("Anthropic API returned status {status}");
                }
            }
            Err(err) => {
                let msg = format!("Anthropic API request failed: {err}");
                crate::diagnostics::log_error("ai_providers", &msg);
                provider.status_message = msg;
            }
        }

        provider
    }
}
