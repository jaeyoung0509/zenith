use super::credentials::CredentialStore;
use super::support::base_provider;
use super::ProviderAdapter;
use crate::models::{AiProviderUsage, UsageSupport};
use std::time::Duration;

#[derive(Default)]
pub struct FireworksApiAdapter;

impl ProviderAdapter for FireworksApiAdapter {
    fn id(&self) -> &'static str {
        "fireworks-api"
    }

    fn collect(&self, credentials: &dyn CredentialStore) -> AiProviderUsage {
        let mut provider = base_provider("fireworks-api", "Fireworks API", "Fireworks API Key");
        provider.installed = true;
        provider.connected = false;
        provider.support = UsageSupport::Live;
        provider.action_url = Some("https://fireworks.ai/account/billing".into());

        let key = match credentials.get("fireworks-api") {
            Ok(Some(secret)) => secret,
            _ => {
                provider.status_message =
                    "No API key configured for Fireworks API in secure store.".into();
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
                let msg = format!("Failed to create Fireworks HTTP client: {err}");
                crate::diagnostics::log_error("ai_providers", &msg);
                provider.status_message = msg;
                return provider;
            }
        };

        match client
            .get("https://api.fireworks.ai/inference/v1/models")
            .bearer_auth(key.expose_secret())
            .send()
        {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    provider.connected = true;
                    provider.status_message =
                        "Live authoritative organization connection via official Fireworks API."
                            .into();
                } else if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    provider.connected = false;
                    provider.status_message =
                        "Fireworks API authentication failed (invalid API key).".into();
                } else {
                    provider.status_message = format!("Fireworks API returned status {status}");
                }
            }
            Err(err) => {
                let msg = format!("Fireworks API request failed: {err}");
                crate::diagnostics::log_error("ai_providers", &msg);
                provider.status_message = msg;
            }
        }

        provider
    }
}
