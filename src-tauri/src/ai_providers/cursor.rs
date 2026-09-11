use super::registry::ProviderRegistry;
use super::support::{base_provider, command_exists};
use super::{CollectionContext, ProviderAdapter, ProviderDescriptor, ProviderError};
use crate::models::{AiProviderUsage, ProviderId, UsageSupport};
use std::path::Path;

#[derive(Default)]
pub struct CursorAdapter;

impl ProviderAdapter for CursorAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Cursor
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderRegistry::find(ProviderId::Cursor)
            .expect("Cursor must exist in registry")
            .to_descriptor()
    }

    fn collect(&self, _ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError> {
        let installed =
            command_exists("cursor-agent") || Path::new("/Applications/Cursor.app").exists();
        let mut provider = base_provider(ProviderId::Cursor, "Cursor", "Cursor account");
        provider.installed = installed;
        provider.connected = false;
        provider.support = UsageSupport::Manual;
        provider.status_message = if installed {
            "Cursor does not expose account quota to Zenith; check usage in Cursor settings.".into()
        } else {
            "Cursor is not installed.".into()
        };
        provider.action_url = None;
        Ok(provider)
    }
}
