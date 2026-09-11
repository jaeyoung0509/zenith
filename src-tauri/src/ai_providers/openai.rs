use super::credentials::CredentialStore;
use super::support::base_provider;
use super::ProviderAdapter;
use crate::models::{AiProviderUsage, UsageSupport};
use std::time::Duration;

#[derive(Default)]
pub struct OpenAiApiAdapter;

impl ProviderAdapter for OpenAiApiAdapter {
    fn id(&self) -> &'static str {
        "openai-api"
    }

    fn collect(&self, credentials: &dyn CredentialStore) -> AiProviderUsage {
        let mut provider = base_provider("openai-api", "OpenAI API", "OpenAI API Key");
        provider.installed = true;
        provider.connected = false;
        provider.support = UsageSupport::Live;
        provider.action_url = Some("https://platform.openai.com/usage".into());

        let key = match credentials.get("openai-api") {
            Ok(Some(secret)) => secret,
            _ => {
                provider.status_message =
                    "No API key configured for OpenAI API in secure store.".into();
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
                let msg = format!("Failed to create OpenAI HTTP client: {err}");
                crate::diagnostics::log_error("ai_providers", &msg);
                provider.status_message = msg;
                return provider;
            }
        };

        // Validate key against models endpoint
        match client
            .get("https://api.openai.com/v1/models")
            .bearer_auth(key.expose_secret())
            .send()
        {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    provider.connected = true;
                    provider.status_message =
                        "Live authoritative organization connection via official OpenAI API."
                            .into();
                } else if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    provider.connected = false;
                    provider.status_message =
                        "OpenAI API authentication failed (invalid API key).".into();
                } else {
                    provider.status_message = format!("OpenAI API returned status {status}");
                }
            }
            Err(err) => {
                let msg = format!("OpenAI API request failed: {err}");
                crate::diagnostics::log_error("ai_providers", &msg);
                provider.status_message = msg;
            }
        }

        provider
    }
}
