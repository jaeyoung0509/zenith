use super::registry::ProviderRegistry;
use super::support::{base_provider, command_exists};
use super::{CollectionContext, ProviderAdapter, ProviderDescriptor, ProviderError};
use crate::models::{AiProviderUsage, ProviderId, UsageSupport};

#[derive(Default)]
pub struct ClaudeAdapter;

impl ProviderAdapter for ClaudeAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderRegistry::find(ProviderId::Claude)
            .expect("Claude must exist in registry")
            .to_descriptor()
    }

    fn collect(&self, _ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError> {
        let installed = command_exists("claude");
        let mut provider = base_provider(ProviderId::Claude, "Claude Code", "Claude.ai OAuth");
        provider.installed = installed;
        provider.connected = false;
        provider.support = UsageSupport::Manual;
        provider.status_message = if installed {
            "Claude exposes subscription limits inside `/usage`; no external OAuth usage API is public.".into()
        } else {
            "Claude Code is not installed.".into()
        };
        provider.action_url = Some("https://claude.ai/settings/usage".into());
        Ok(provider)
    }
}
