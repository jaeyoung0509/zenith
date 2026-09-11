use super::credentials::CredentialStore;
use super::support::{base_provider, command_exists};
use super::ProviderAdapter;
use crate::models::{AiProviderUsage, UsageSupport};

#[derive(Default)]
pub struct ClaudeAdapter;

impl ProviderAdapter for ClaudeAdapter {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn collect(&self, _credentials: &dyn CredentialStore) -> AiProviderUsage {
        let installed = command_exists("claude");
        let mut provider = base_provider("claude", "Claude Code", "Claude.ai OAuth");
        provider.installed = installed;
        provider.connected = false;
        provider.support = UsageSupport::Manual;
        provider.status_message = if installed {
            "Claude exposes subscription limits inside `/usage`; no external OAuth usage API is public.".into()
        } else {
            "Claude Code is not installed.".into()
        };
        provider.action_url = Some("https://claude.ai/settings/usage".into());
        provider
    }
}
