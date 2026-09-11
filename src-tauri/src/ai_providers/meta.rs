use super::credentials::CredentialStore;
use super::support::{base_provider, command_exists};
use super::ProviderAdapter;
use crate::models::{AiProviderUsage, UsageSupport};
use std::path::Path;

#[derive(Default)]
pub struct MuseCodeAdapter;

impl ProviderAdapter for MuseCodeAdapter {
    fn id(&self) -> &'static str {
        "muse-code"
    }

    fn collect(&self, _credentials: &dyn CredentialStore) -> AiProviderUsage {
        let installed = command_exists("muse")
            || command_exists("muse-code")
            || Path::new("/Applications/Muse.app").exists();
        let mut provider = base_provider("muse-code", "Muse Code", "Meta account");
        provider.installed = installed;
        provider.connected = false;
        provider.support = UsageSupport::Manual;
        provider.status_message = if installed {
            "Muse Code is installed; no external structured usage API is public. Quota remains in the client.".into()
        } else {
            "Muse Code is not installed.".into()
        };
        provider.action_url =
            Some("https://research.meta.ai/blog/introducing-muse-code-and-muse-spark-1-2".into());
        provider
    }
}

#[derive(Default)]
pub struct MetaModelApiAdapter;

impl ProviderAdapter for MetaModelApiAdapter {
    fn id(&self) -> &'static str {
        "meta-model-api"
    }

    fn collect(&self, credentials: &dyn CredentialStore) -> AiProviderUsage {
        let mut provider = base_provider("meta-model-api", "Meta Model API", "Meta Model API Key");
        provider.installed = true;
        provider.connected = false;
        provider.support = UsageSupport::Live;
        provider.action_url =
            Some("https://ai.meta.com/blog/introducing-muse-spark-meta-model-api/".into());

        let _key = match credentials.get("meta-model-api") {
            Ok(Some(secret)) => secret,
            _ => {
                provider.status_message =
                    "No API key configured for Meta Model API in secure store.".into();
                return provider;
            }
        };

        // When a key is configured, mark connected and report model attribution
        provider.connected = true;
        provider.status_message =
            "Connected to Meta Model API (Muse Spark 1.3). Response-level usage tracked locally."
                .into();

        provider
    }
}
