use super::credentials::CredentialStore;
use super::support::{base_provider, command_exists};
use super::ProviderAdapter;
use crate::models::{AiProviderUsage, UsageSupport};
use std::path::Path;

#[derive(Default)]
pub struct CursorAdapter;

impl ProviderAdapter for CursorAdapter {
    fn id(&self) -> &'static str {
        "cursor"
    }

    fn collect(&self, _credentials: &dyn CredentialStore) -> AiProviderUsage {
        let installed =
            command_exists("cursor-agent") || Path::new("/Applications/Cursor.app").exists();
        let mut provider = base_provider("cursor", "Cursor", "Cursor account");
        provider.installed = installed;
        provider.connected = false;
        provider.support = UsageSupport::Manual;
        provider.status_message = if installed {
            "Cursor does not expose account quota to Zenith; check usage in Cursor settings.".into()
        } else {
            "Cursor is not installed.".into()
        };
        provider.action_url = None;
        provider
    }
}
