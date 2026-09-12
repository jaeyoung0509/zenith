use super::credentials::CredentialStore;
use super::registry::ProviderRegistry;
use super::support::{base_provider, failed_provider, now_secs};
use super::{CollectionContext, ProviderAdapter, ProviderDescriptor, ProviderError};
use crate::models::{AiProviderUsage, AiUsageSnapshot, ProviderId, UsageSupport};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const DEFAULT_MAX_CONCURRENCY: usize = 4;

struct SafeClientInner {
    client: Option<reqwest::blocking::Client>,
}

impl Drop for SafeClientInner {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            if tokio::runtime::Handle::try_current().is_ok() {
                let _ = std::thread::spawn(move || drop(client)).join();
            } else {
                drop(client);
            }
        }
    }
}

#[derive(Clone)]
pub struct HttpClientWrapper(Arc<SafeClientInner>);

impl Default for HttpClientWrapper {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClientWrapper {
    pub fn new() -> Self {
        let build = || {
            reqwest::blocking::Client::builder()
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default()
        };
        let client = if tokio::runtime::Handle::try_current().is_ok() {
            std::thread::spawn(build).join().unwrap_or_default()
        } else {
            build()
        };

        Self(Arc::new(SafeClientInner {
            client: Some(client),
        }))
    }

    pub fn client(&self) -> &reqwest::blocking::Client {
        self.0
            .client
            .as_ref()
            .expect("HttpClientWrapper inner client must be present")
    }
}

pub struct ProviderCollectionService {
    adapters: HashMap<ProviderId, Arc<dyn ProviderAdapter>>,
    max_concurrency: usize,
    http_client: HttpClientWrapper,
}

impl Default for ProviderCollectionService {
    fn default() -> Self {
        let mut service = Self::new_empty();
        service.register(Arc::new(super::codex::CodexAdapter));
        service.register(Arc::new(super::claude::ClaudeAdapter));
        service.register(Arc::new(super::opencode::OpenCodeAdapter));
        service.register(Arc::new(super::openrouter::OpenRouterAdapter));
        service.register(Arc::new(super::antigravity::AntigravityAdapter));
        service.register(Arc::new(super::cursor::CursorAdapter));
        service.register(Arc::new(super::xai::GrokBuildAdapter));
        service.register(Arc::new(super::xai::XaiApiAdapter));
        service.register(Arc::new(super::openai::OpenAiApiAdapter));
        service.register(Arc::new(super::anthropic::AnthropicApiAdapter));
        service.register(Arc::new(super::meta::MuseCodeAdapter));
        service.register(Arc::new(super::meta::MetaModelApiAdapter));
        service.register(Arc::new(super::mistral::MistralApiAdapter));
        service.register(Arc::new(super::fireworks::FireworksApiAdapter));
        service
    }
}

impl ProviderCollectionService {
    pub fn new_empty() -> Self {
        Self {
            adapters: HashMap::new(),
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
            http_client: HttpClientWrapper::new(),
        }
    }

    pub fn with_concurrency(mut self, max: usize) -> Self {
        self.max_concurrency = max.max(1);
        self
    }

    pub fn register(&mut self, adapter: Arc<dyn ProviderAdapter>) {
        self.adapters.insert(adapter.id(), adapter);
    }

    pub fn collect_parallel<F>(
        &self,
        credentials: Arc<dyn CredentialStore>,
        provider_ids: &[ProviderId],
        on_provider: F,
    ) -> AiUsageSnapshot
    where
        F: Fn(AiProviderUsage) + Send + Sync + 'static,
    {
        if provider_ids.is_empty() {
            return AiUsageSnapshot {
                providers: vec![],
                fetched_at: now_secs(),
            };
        }

        let pool_size = self.max_concurrency.min(provider_ids.len()).max(1);
        let (task_tx, task_rx) = std::sync::mpsc::channel::<(usize, ProviderId)>();
        for (index, id) in provider_ids.iter().enumerate() {
            let _ = task_tx.send((index, *id));
        }
        drop(task_tx);

        let task_rx = Arc::new(Mutex::new(task_rx));
        let on_provider = Arc::new(on_provider);
        let results = Arc::new(Mutex::new(vec![None; provider_ids.len()]));
        let adapters = Arc::new(self.adapters.clone());
        let http_client = self.http_client.clone();

        let mut handles = Vec::with_capacity(pool_size);
        for _ in 0..pool_size {
            let task_rx = task_rx.clone();
            let on_provider = on_provider.clone();
            let results = results.clone();
            let adapters = adapters.clone();
            let credentials = credentials.clone();
            let http_client = http_client.clone();

            handles.push(std::thread::spawn(move || loop {
                let task = {
                    let guard = match task_rx.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    guard.recv().ok()
                };

                let Some((index, id)) = task else {
                    break;
                };

                let adapter = adapters.get(&id).cloned();
                let creds = credentials.clone();
                let provider_name = ProviderRegistry::find(id)
                    .map(|d| d.display_name)
                    .unwrap_or("Unknown provider");

                let usage = match adapter {
                    Some(adapter) => {
                        let ctx = CollectionContext {
                            credentials: &*creds,
                            http_client: http_client.client(),
                        };
                        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            adapter.collect(&ctx)
                        }));

                        match res {
                            Ok(Ok(usage)) => usage,
                            Ok(Err(err)) => map_error_to_usage(&adapter.descriptor(), err),
                            // The payload is the cause, but the row crosses
                            // IPC: sanitize what the user sees, and keep the
                            // raw text in the diagnostics log below.
                            Err(panic) => {
                                let payload = panic_payload_text(panic.as_ref());
                                crate::diagnostics::log_error(
                                    "provider",
                                    &format!("Collector panicked unexpectedly: {payload}"),
                                );
                                failed_provider(
                                    id,
                                    provider_name,
                                    &crate::diagnostics::sanitize_log(&format!(
                                        "Collector panicked unexpectedly: {payload}"
                                    )),
                                )
                            }
                        }
                    }
                    None => failed_provider(id, provider_name, "Unknown provider"),
                };

                {
                    let mut res_guard = results.lock().unwrap_or_else(|p| p.into_inner());
                    res_guard[index] = Some(usage.clone());
                }
                on_provider(usage);
            }));
        }

        for handle in handles {
            if let Err(panic) = handle.join() {
                crate::diagnostics::log_error(
                    "provider",
                    &format!(
                        "Usage collector worker panicked: {}",
                        panic_payload_text(panic.as_ref())
                    ),
                );
            }
        }

        let mut guard = results.lock().unwrap_or_else(|p| p.into_inner());
        let providers = guard
            .drain(..)
            .zip(provider_ids.iter().copied())
            .map(|(item, id)| {
                item.unwrap_or_else(|| {
                    let name = ProviderRegistry::find(id)
                        .map(|d| d.display_name)
                        .unwrap_or("Unknown");
                    failed_provider(id, name, "Missing result")
                })
            })
            .collect();

        AiUsageSnapshot {
            providers,
            fetched_at: now_secs(),
        }
    }
}

/// The text of a panic payload, for the provider failure message and the log.
///
/// `catch_unwind` and `JoinHandle::join` hand back the payload as `dyn Any`;
/// that message is the only record of what the collector was doing when it
/// died, and it used to be discarded.
fn panic_payload_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else if let Some(text) = payload.downcast_ref::<&'static str>() {
        (*text).to_string()
    } else {
        "no message".to_string()
    }
}

fn map_error_to_usage(descriptor: &ProviderDescriptor, err: ProviderError) -> AiProviderUsage {
    let auth_label = match descriptor.credential_kind {
        super::registry::CredentialKind::None => "Local Account",
        super::registry::CredentialKind::ApiKey => "API Key",
        super::registry::CredentialKind::OAuth => "OAuth",
        super::registry::CredentialKind::Cli => "CLI",
    };
    let mut usage = base_provider(descriptor.id, &descriptor.display_name, auth_label);
    usage.support = UsageSupport::Manual;
    usage.connected = false;
    usage.model_vendor = descriptor.model_vendor.clone();
    usage.model_identity = descriptor.model_identity.clone();
    usage.status_message = match err {
        ProviderError::CredentialMissing => "API key not configured in secure settings.".into(),
        ProviderError::AuthenticationFailed(msg) => format!("Authentication failed: {msg}"),
        ProviderError::Network(msg) => format!("Network error: {msg}"),
        ProviderError::InvalidResponse(msg) => format!("Invalid response: {msg}"),
        ProviderError::CliNotInstalled(msg) => format!("CLI not detected: {msg}"),
        ProviderError::CliFailed(msg) => format!("CLI execution failed: {msg}"),
        ProviderError::ExecutionFailed(msg) => format!("Execution failed: {msg}"),
        ProviderError::Timeout => "Collection timed out.".into(),
        ProviderError::Unsupported(msg) => format!("Unsupported: {msg}"),
    };
    // Provider errors can echo a request URL or subprocess output. Redact before
    // the text crosses IPC, even though current providers keep keys in headers.
    usage.status_message = crate::diagnostics::sanitize_log(&usage.status_message);
    usage
}
