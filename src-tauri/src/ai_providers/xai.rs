use super::credentials::CredentialStore;
use super::support::{base_provider, command_exists};
use super::ProviderAdapter;
use crate::models::{AiProviderUsage, UsageSupport};
use serde_json::Value;
use std::time::Duration;

#[derive(Default)]
pub struct GrokBuildAdapter;

impl ProviderAdapter for GrokBuildAdapter {
    fn id(&self) -> &'static str {
        "grok-build"
    }

    fn collect(&self, _credentials: &dyn CredentialStore) -> AiProviderUsage {
        let installed = command_exists("grok");
        let mut provider = base_provider("grok-build", "Grok Build", "xAI account");
        provider.installed = installed;
        provider.connected = false;
        provider.support = UsageSupport::Manual;
        provider.status_message = if installed {
            "Grok Build does not expose account quota to Zenith; check usage in the provider client.".into()
        } else {
            "Grok Build is not installed.".into()
        };
        provider.action_url = None;
        provider
    }
}

#[derive(Default)]
pub struct XaiApiAdapter;

impl ProviderAdapter for XaiApiAdapter {
    fn id(&self) -> &'static str {
        "xai-api"
    }

    fn collect(&self, credentials: &dyn CredentialStore) -> AiProviderUsage {
        let mut provider = base_provider("xai-api", "xAI API", "xAI API Key");
        provider.installed = true;
        provider.connected = false;
        provider.support = UsageSupport::Live;
        provider.action_url = Some("https://console.x.ai/team/billing".into());

        let key = match credentials.get("xai-api") {
            Ok(Some(secret)) => secret,
            _ => {
                provider.status_message =
                    "No API key configured for xAI API in secure store.".into();
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
                let msg = format!("Failed to create xAI HTTP client: {err}");
                crate::diagnostics::log_error("ai_providers", &msg);
                provider.status_message = msg;
                return provider;
            }
        };

        // Official xAI API endpoint: check key validity / management info
        match client
            .get("https://api.x.ai/v1/api-key")
            .bearer_auth(key.expose_secret())
            .send()
        {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    provider.connected = true;
                    provider.status_message =
                        "Live authoritative team connection via official xAI API.".into();
                    if let Ok(data) = response.json::<Value>() {
                        if let Some(name) = data.get("name").and_then(Value::as_str) {
                            provider.auth_label = format!("API Key · {name}");
                        }
                    }
                } else if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    provider.connected = false;
                    provider.status_message =
                        "xAI API authentication failed (invalid API key).".into();
                } else {
                    provider.status_message = format!("xAI API returned status {status}");
                }
            }
            Err(err) => {
                let msg = format!("xAI API request failed: {err}");
                crate::diagnostics::log_error("ai_providers", &msg);
                provider.status_message = msg;
            }
        }

        provider
    }
}
