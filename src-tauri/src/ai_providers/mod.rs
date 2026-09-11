pub mod anthropic;
pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod credentials;
pub mod cursor;
pub mod fireworks;
pub mod meta;
pub mod mistral;
pub mod openai;
pub mod opencode;
pub mod openrouter;
pub mod registry;
pub mod service;
pub mod support;
pub mod xai;

pub use credentials::{
    CredentialError, CredentialStore, InMemoryCredentialStore, OsCredentialStore, SecretString,
};
pub use openrouter::connect_openrouter;
pub use registry::{CredentialKind, ProviderDescriptor, ProviderRegistry, PROVIDER_REGISTRY};
pub use service::ProviderCollectionService;

use crate::models::AiProviderUsage;

pub trait ProviderAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn collect(&self, credentials: &dyn CredentialStore) -> AiProviderUsage;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ObservationScope;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    struct SlowDummyAdapter {
        id: &'static str,
        delay: Duration,
        active_counter: Arc<AtomicUsize>,
        max_seen: Arc<AtomicUsize>,
    }

    impl ProviderAdapter for SlowDummyAdapter {
        fn id(&self) -> &'static str {
            self.id
        }

        fn collect(&self, _credentials: &dyn CredentialStore) -> AiProviderUsage {
            let active = self.active_counter.fetch_add(1, Ordering::SeqCst) + 1;
            let mut current_max = self.max_seen.load(Ordering::SeqCst);
            while active > current_max {
                match self.max_seen.compare_exchange_weak(
                    current_max,
                    active,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(actual) => current_max = actual,
                }
            }
            std::thread::sleep(self.delay);
            self.active_counter.fetch_sub(1, Ordering::SeqCst);

            let mut usage = support::base_provider(self.id, self.id, "Test");
            usage.connected = true;
            usage
        }
    }

    struct PanickingAdapter;
    impl ProviderAdapter for PanickingAdapter {
        fn id(&self) -> &'static str {
            "panicking"
        }
        fn collect(&self, _credentials: &dyn CredentialStore) -> AiProviderUsage {
            panic!("intentional test panic in collector");
        }
    }

    #[test]
    fn bounded_concurrency_never_exceeds_configured_limit() {
        let active = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));

        let mut service = ProviderCollectionService::new_empty().with_concurrency(3);
        for id in &["p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8"] {
            service.register(Arc::new(SlowDummyAdapter {
                id,
                delay: Duration::from_millis(50),
                active_counter: active.clone(),
                max_seen: max_seen.clone(),
            }));
        }

        let provider_ids = vec![
            "p1".into(),
            "p2".into(),
            "p3".into(),
            "p4".into(),
            "p5".into(),
            "p6".into(),
            "p7".into(),
            "p8".into(),
        ];

        let creds = Arc::new(InMemoryCredentialStore::new());
        let snapshot = service.collect_parallel(creds, &provider_ids, |_| {});

        assert_eq!(snapshot.providers.len(), 8);
        assert!(
            max_seen.load(Ordering::SeqCst) <= 3,
            "Max concurrent active tasks was {}, expected <= 3",
            max_seen.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn slow_provider_times_out_without_blocking_others() {
        let active = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));

        let mut service = ProviderCollectionService::new_empty()
            .with_concurrency(4)
            .with_timeout(Duration::from_millis(50));

        service.register(Arc::new(SlowDummyAdapter {
            id: "fast1",
            delay: Duration::from_millis(5),
            active_counter: active.clone(),
            max_seen: max_seen.clone(),
        }));
        service.register(Arc::new(SlowDummyAdapter {
            id: "slow",
            delay: Duration::from_millis(250),
            active_counter: active.clone(),
            max_seen: max_seen.clone(),
        }));
        service.register(Arc::new(SlowDummyAdapter {
            id: "fast2",
            delay: Duration::from_millis(5),
            active_counter: active.clone(),
            max_seen: max_seen.clone(),
        }));

        let provider_ids = vec!["fast1".into(), "slow".into(), "fast2".into()];
        let creds = Arc::new(InMemoryCredentialStore::new());
        let snapshot = service.collect_parallel(creds, &provider_ids, |_| {});

        assert_eq!(snapshot.providers.len(), 3);
        assert!(snapshot.providers[0].connected);
        assert!(!snapshot.providers[1].connected);
        assert!(snapshot.providers[1].status_message.contains("timed out"));
        assert!(snapshot.providers[2].connected);
    }

    #[test]
    fn panicking_adapter_is_isolated() {
        let active = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));

        let mut service = ProviderCollectionService::new_empty().with_concurrency(4);
        service.register(Arc::new(SlowDummyAdapter {
            id: "fast",
            delay: Duration::from_millis(5),
            active_counter: active.clone(),
            max_seen: max_seen.clone(),
        }));
        service.register(Arc::new(PanickingAdapter));

        let provider_ids = vec!["fast".into(), "panicking".into()];
        let creds = Arc::new(InMemoryCredentialStore::new());
        let snapshot = service.collect_parallel(creds, &provider_ids, |_| {});

        assert_eq!(snapshot.providers.len(), 2);
        assert!(snapshot.providers[0].connected);
        assert!(!snapshot.providers[1].connected);
        assert!(snapshot.providers[1].status_message.contains("panicked"));
    }

    #[test]
    fn deterministic_output_ordering_matches_requested_ids() {
        let active = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));

        let mut service = ProviderCollectionService::new_empty().with_concurrency(4);
        for id in &["a", "b", "c", "d"] {
            service.register(Arc::new(SlowDummyAdapter {
                id,
                delay: Duration::from_millis(10),
                active_counter: active.clone(),
                max_seen: max_seen.clone(),
            }));
        }

        let requested = vec!["d".into(), "b".into(), "a".into(), "c".into()];
        let creds = Arc::new(InMemoryCredentialStore::new());
        let snapshot = service.collect_parallel(creds, &requested, |_| {});

        let output_ids = snapshot
            .providers
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(output_ids, vec!["d", "b", "a", "c"]);
    }

    #[test]
    fn provider_identities_and_scopes_are_distinct() {
        // Grok Build vs xAI API
        let grok = ProviderRegistry::find("grok-build").unwrap();
        let xai = ProviderRegistry::find("xai-api").unwrap();
        assert_eq!(grok.scope, ObservationScope::Subscription);
        assert_eq!(xai.scope, ObservationScope::Organization);
        assert_ne!(grok.id, xai.id);

        // Codex vs OpenAI API
        let codex = ProviderRegistry::find("codex").unwrap();
        let openai = ProviderRegistry::find("openai-api").unwrap();
        assert_eq!(codex.scope, ObservationScope::Subscription);
        assert_eq!(openai.scope, ObservationScope::Organization);
        assert_ne!(codex.id, openai.id);

        // Claude individual vs Anthropic API
        let claude = ProviderRegistry::find("claude").unwrap();
        let anthropic = ProviderRegistry::find("anthropic-api").unwrap();
        assert_eq!(claude.scope, ObservationScope::Subscription);
        assert_eq!(anthropic.scope, ObservationScope::Organization);
        assert_ne!(claude.id, anthropic.id);

        // Muse Code vs Meta Model API
        let muse = ProviderRegistry::find("muse-code").unwrap();
        let meta_api = ProviderRegistry::find("meta-model-api").unwrap();
        assert_eq!(muse.scope, ObservationScope::Subscription);
        assert_eq!(meta_api.scope, ObservationScope::ApiKey);
        assert_eq!(muse.model_vendor, Some("Meta"));
        assert_eq!(meta_api.model_vendor, Some("Meta"));
        assert_eq!(muse.model_identity, Some("Muse Spark"));
        assert_eq!(meta_api.model_identity, Some("Muse Spark 1.3"));
        assert_ne!(muse.id, meta_api.id);
    }
}
