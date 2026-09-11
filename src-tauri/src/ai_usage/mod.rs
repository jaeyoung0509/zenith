use crate::ai_providers::{
    CredentialStore, InMemoryCredentialStore, ProviderCollectionService, SecretString,
};
use crate::models::{AiProviderUsage, AiUsageSnapshot, ProviderId};
use std::sync::Arc;

pub struct AiUsageCollector;

impl AiUsageCollector {
    pub fn collect(openrouter_key: Option<String>, provider_ids: &[ProviderId]) -> AiUsageSnapshot {
        Self::collect_parallel(openrouter_key, provider_ids, |_| {})
    }

    pub fn collect_parallel<F>(
        openrouter_key: Option<String>,
        provider_ids: &[ProviderId],
        on_provider: F,
    ) -> AiUsageSnapshot
    where
        F: Fn(AiProviderUsage) + Send + Sync + 'static,
    {
        let credentials = Arc::new(InMemoryCredentialStore::default());
        if let Some(key) = openrouter_key {
            let _ = credentials.set(ProviderId::OpenRouter, SecretString::new(key));
        }
        let service = ProviderCollectionService::default();
        service.collect_parallel(credentials, provider_ids, on_provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_runs_only_selected_providers_in_configured_order() {
        let selected = vec![ProviderId::Cursor, ProviderId::GrokBuild];
        let snapshot = AiUsageCollector::collect(None, &selected);
        assert_eq!(
            snapshot
                .providers
                .iter()
                .map(|provider| provider.id)
                .collect::<Vec<_>>(),
            vec![ProviderId::Cursor, ProviderId::GrokBuild]
        );
    }
}
