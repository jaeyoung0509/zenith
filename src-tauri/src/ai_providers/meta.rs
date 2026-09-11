use super::registry::ProviderRegistry;
use super::support::{base_provider, command_exists};
use super::{CollectionContext, ProviderAdapter, ProviderDescriptor, ProviderError};
use crate::models::{AiProviderUsage, ProviderId, UsageSupport};
use std::path::Path;

#[derive(Default)]
pub struct MuseCodeAdapter;

impl ProviderAdapter for MuseCodeAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::MuseCode
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderRegistry::find(ProviderId::MuseCode)
            .expect("MuseCode must exist in registry")
            .to_descriptor()
    }

    fn collect(&self, _ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError> {
        let installed = command_exists("muse")
            || command_exists("muse-code")
            || Path::new("/Applications/Muse.app").exists();
        let mut provider = base_provider(ProviderId::MuseCode, "Muse Code", "Meta CLI");
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
        Ok(provider)
    }
}

#[derive(Default)]
pub struct MetaModelApiAdapter;

impl ProviderAdapter for MetaModelApiAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::MetaModelApi
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderRegistry::find(ProviderId::MetaModelApi)
            .expect("MetaModelApi must exist in registry")
            .to_descriptor()
    }

    fn collect(&self, ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError> {
        let _key = ctx
            .credentials
            .get(ProviderId::MetaModelApi)
            .map_err(|e| ProviderError::ExecutionFailed(e.to_string()))?
            .ok_or(ProviderError::CredentialMissing)?;

        let mut provider = base_provider(
            ProviderId::MetaModelApi,
            "Meta Model API",
            "Meta Model API Key",
        );
        provider.installed = true;
        provider.connected = true;
        provider.support = UsageSupport::Local;
        provider.action_url =
            Some("https://ai.meta.com/blog/introducing-muse-spark-meta-model-api/".into());
        provider.status_message =
            "Meta Model API key stored in secure credential store (local session tracking).".into();

        Ok(provider)
    }
}
