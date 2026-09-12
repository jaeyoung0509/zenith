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
    CredentialError, CredentialStore, CredentialStoreAvailability, InMemoryCredentialStore,
    OsCredentialStore, SecretString,
};
pub use openrouter::{authorize_openrouter, connect_openrouter};
pub use registry::{CredentialKind, ProviderDescriptor, ProviderRegistry, PROVIDER_REGISTRY};
pub use service::ProviderCollectionService;

use crate::models::{AiProviderUsage, ProviderId};

pub struct CollectionContext<'a> {
    pub credentials: &'a dyn CredentialStore,
    pub http_client: &'a reqwest::blocking::Client,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    CredentialMissing,
    AuthenticationFailed(String),
    Network(String),
    InvalidResponse(String),
    CliNotInstalled(String),
    CliFailed(String),
    ExecutionFailed(String),
    Timeout,
    Unsupported(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CredentialMissing => write!(f, "Credential not configured"),
            Self::AuthenticationFailed(msg) => write!(f, "Authentication failed: {msg}"),
            Self::Network(msg) => write!(f, "Network error: {msg}"),
            Self::InvalidResponse(msg) => write!(f, "Invalid response: {msg}"),
            Self::CliNotInstalled(msg) => write!(f, "CLI not detected: {msg}"),
            Self::CliFailed(msg) => write!(f, "CLI execution failed: {msg}"),
            Self::ExecutionFailed(msg) => write!(f, "Execution failed: {msg}"),
            Self::Timeout => write!(f, "Operation timed out"),
            Self::Unsupported(msg) => write!(f, "Unsupported: {msg}"),
        }
    }
}

impl std::error::Error for ProviderError {}

pub trait ProviderAdapter: Send + Sync {
    fn id(&self) -> ProviderId;
    fn descriptor(&self) -> ProviderDescriptor;
    fn collect(&self, ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::registry::StaticProviderDescriptor;
    use super::*;
    use crate::models::ObservationScope;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    struct SlowDummyAdapter {
        id: ProviderId,
        delay: Duration,
        active_counter: Arc<AtomicUsize>,
        max_seen: Arc<AtomicUsize>,
        should_fail_timeout: bool,
    }

    impl ProviderAdapter for SlowDummyAdapter {
        fn id(&self) -> ProviderId {
            self.id
        }

        fn descriptor(&self) -> ProviderDescriptor {
            ProviderRegistry::find(self.id)
                .map(StaticProviderDescriptor::to_descriptor)
                .unwrap_or_else(|| ProviderDescriptor {
                    id: self.id,
                    display_name: self.id.as_str().to_string(),
                    scope: ObservationScope::Subscription,
                    credential_kind: CredentialKind::None,
                    source_kind: crate::models::ObservationSourceKind::Manual,
                    supports_quick_panel: false,
                    model_vendor: None,
                    model_identity: None,
                    description: "".into(),
                    default_quota_provider: false,
                })
        }

        fn collect(&self, _ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError> {
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

            if self.should_fail_timeout {
                return Err(ProviderError::Timeout);
            }

            let mut usage = support::base_provider(self.id, self.id.as_str(), "Test");
            usage.connected = true;
            Ok(usage)
        }
    }

    struct PanickingAdapter;
    impl ProviderAdapter for PanickingAdapter {
        fn id(&self) -> ProviderId {
            ProviderId::Claude
        }
        fn descriptor(&self) -> ProviderDescriptor {
            ProviderRegistry::find(ProviderId::Claude)
                .map(StaticProviderDescriptor::to_descriptor)
                .unwrap()
        }
        fn collect(&self, _ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError> {
            panic!("intentional test panic in collector");
        }
    }

    #[test]
    fn bounded_concurrency_never_exceeds_configured_limit() {
        let active = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));

        let mut service = ProviderCollectionService::new_empty().with_concurrency(3);
        let test_ids = [
            ProviderId::Codex,
            ProviderId::Claude,
            ProviderId::OpenCode,
            ProviderId::OpenRouter,
            ProviderId::Antigravity,
            ProviderId::Cursor,
            ProviderId::GrokBuild,
            ProviderId::XaiApi,
        ];
        for &id in &test_ids {
            service.register(Arc::new(SlowDummyAdapter {
                id,
                delay: Duration::from_millis(50),
                active_counter: active.clone(),
                max_seen: max_seen.clone(),
                should_fail_timeout: false,
            }));
        }

        let creds = Arc::new(InMemoryCredentialStore::new());
        let snapshot = service.collect_parallel(creds, &test_ids, |_| {});

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

        let mut service = ProviderCollectionService::new_empty().with_concurrency(4);

        service.register(Arc::new(SlowDummyAdapter {
            id: ProviderId::Codex,
            delay: Duration::from_millis(5),
            active_counter: active.clone(),
            max_seen: max_seen.clone(),
            should_fail_timeout: false,
        }));
        service.register(Arc::new(SlowDummyAdapter {
            id: ProviderId::Claude,
            delay: Duration::from_millis(5),
            active_counter: active.clone(),
            max_seen: max_seen.clone(),
            should_fail_timeout: true,
        }));
        service.register(Arc::new(SlowDummyAdapter {
            id: ProviderId::OpenCode,
            delay: Duration::from_millis(5),
            active_counter: active.clone(),
            max_seen: max_seen.clone(),
            should_fail_timeout: false,
        }));

        let provider_ids = vec![ProviderId::Codex, ProviderId::Claude, ProviderId::OpenCode];
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
            id: ProviderId::Codex,
            delay: Duration::from_millis(5),
            active_counter: active.clone(),
            max_seen: max_seen.clone(),
            should_fail_timeout: false,
        }));
        service.register(Arc::new(PanickingAdapter));

        let provider_ids = vec![ProviderId::Codex, ProviderId::Claude];
        let creds = Arc::new(InMemoryCredentialStore::new());
        let snapshot = service.collect_parallel(creds, &provider_ids, |_| {});

        assert_eq!(snapshot.providers.len(), 2);
        assert!(snapshot.providers[0].connected);
        assert!(!snapshot.providers[1].connected);
        assert!(snapshot.providers[1].status_message.contains("panicked"));
        assert!(
            snapshot.providers[1]
                .status_message
                .contains("intentional test panic in collector"),
            "the panic payload must reach the provider row: {}",
            snapshot.providers[1].status_message
        );
    }

    #[test]
    fn deterministic_output_ordering_matches_requested_ids() {
        let active = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));

        let mut service = ProviderCollectionService::new_empty().with_concurrency(4);
        let ids = [
            ProviderId::Codex,
            ProviderId::Claude,
            ProviderId::OpenCode,
            ProviderId::OpenRouter,
        ];
        for &id in &ids {
            service.register(Arc::new(SlowDummyAdapter {
                id,
                delay: Duration::from_millis(10),
                active_counter: active.clone(),
                max_seen: max_seen.clone(),
                should_fail_timeout: false,
            }));
        }

        let requested = vec![
            ProviderId::OpenRouter,
            ProviderId::Claude,
            ProviderId::Codex,
            ProviderId::OpenCode,
        ];
        let creds = Arc::new(InMemoryCredentialStore::new());
        let snapshot = service.collect_parallel(creds, &requested, |_| {});

        let output_ids = snapshot.providers.iter().map(|p| p.id).collect::<Vec<_>>();
        assert_eq!(output_ids, requested);
    }

    #[test]
    fn provider_identities_and_scopes_are_distinct() {
        // Grok Build vs xAI API
        let grok = ProviderRegistry::find(ProviderId::GrokBuild).unwrap();
        let xai = ProviderRegistry::find(ProviderId::XaiApi).unwrap();
        assert_eq!(grok.scope, ObservationScope::Subscription);
        assert_eq!(xai.scope, ObservationScope::Organization);
        assert_ne!(grok.id, xai.id);

        // Codex vs OpenAI API
        let codex = ProviderRegistry::find(ProviderId::Codex).unwrap();
        let openai = ProviderRegistry::find(ProviderId::OpenAiApi).unwrap();
        assert_eq!(codex.scope, ObservationScope::Subscription);
        assert_eq!(openai.scope, ObservationScope::Organization);
        assert_ne!(codex.id, openai.id);

        // Claude individual vs Anthropic API
        let claude = ProviderRegistry::find(ProviderId::Claude).unwrap();
        let anthropic = ProviderRegistry::find(ProviderId::AnthropicApi).unwrap();
        assert_eq!(claude.scope, ObservationScope::Subscription);
        assert_eq!(anthropic.scope, ObservationScope::Organization);
        assert_ne!(claude.id, anthropic.id);

        // Muse Code vs Meta Model API
        let muse = ProviderRegistry::find(ProviderId::MuseCode).unwrap();
        let meta_api = ProviderRegistry::find(ProviderId::MetaModelApi).unwrap();
        assert_eq!(muse.scope, ObservationScope::Subscription);
        assert_eq!(meta_api.scope, ObservationScope::ApiKey);
        assert_eq!(muse.model_vendor, Some("Meta"));
        assert_eq!(meta_api.model_vendor, Some("Meta"));
        assert_eq!(muse.model_identity, Some("Muse Spark"));
        assert_eq!(meta_api.model_identity, Some("Muse Spark 1.3"));
        assert_ne!(muse.id, meta_api.id);
    }
}
