use super::credentials::CredentialStore;
use super::registry::ProviderRegistry;
use super::support::{failed_provider, now_secs};
use super::ProviderAdapter;
use crate::models::{AiProviderUsage, AiUsageSnapshot};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const DEFAULT_MAX_CONCURRENCY: usize = 4;
pub const DEFAULT_PROVIDER_TIMEOUT: Duration = Duration::from_secs(25);

pub struct ProviderCollectionService {
    adapters: HashMap<&'static str, Arc<dyn ProviderAdapter>>,
    max_concurrency: usize,
    provider_timeout: Duration,
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
            provider_timeout: DEFAULT_PROVIDER_TIMEOUT,
        }
    }

    pub fn with_concurrency(mut self, max: usize) -> Self {
        self.max_concurrency = max.max(1);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.provider_timeout = timeout;
        self
    }

    pub fn register(&mut self, adapter: Arc<dyn ProviderAdapter>) {
        self.adapters.insert(adapter.id(), adapter);
    }

    pub fn collect_parallel<F>(
        &self,
        credentials: Arc<dyn CredentialStore>,
        provider_ids: &[String],
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
        let (task_tx, task_rx) = std::sync::mpsc::channel::<(usize, String)>();
        for (index, id) in provider_ids.iter().enumerate() {
            let _ = task_tx.send((index, id.clone()));
        }
        drop(task_tx);

        let task_rx = Arc::new(Mutex::new(task_rx));
        let on_provider = Arc::new(on_provider);
        let results = Arc::new(Mutex::new(vec![None; provider_ids.len()]));
        let adapters = Arc::new(self.adapters.clone());
        let provider_timeout = self.provider_timeout;

        let mut handles = Vec::with_capacity(pool_size);
        for _ in 0..pool_size {
            let task_rx = task_rx.clone();
            let on_provider = on_provider.clone();
            let results = results.clone();
            let adapters = adapters.clone();
            let credentials = credentials.clone();

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

                let normalized_id = ProviderRegistry::normalize_provider_id(&id).unwrap_or(&id);
                let adapter = adapters.get(normalized_id).cloned();
                let creds = credentials.clone();
                let provider_name = ProviderRegistry::find(normalized_id)
                    .map(|d| d.display_name)
                    .unwrap_or("Unknown provider")
                    .to_string();

                let usage = match adapter {
                    Some(adapter) => {
                        let (done_tx, done_rx) = std::sync::mpsc::channel();
                        let _worker = std::thread::spawn(move || {
                            let res =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    adapter.collect(&*creds)
                                }));
                            let _ = done_tx.send(res);
                        });

                        match done_rx.recv_timeout(provider_timeout) {
                            Ok(Ok(usage)) => usage,
                            Ok(Err(_panic)) => {
                                failed_provider(&id, &provider_name, "Collector panicked.")
                            }
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                                failed_provider(&id, &provider_name, "Collection timed out.")
                            }
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                failed_provider(
                                    &id,
                                    &provider_name,
                                    "Collector disconnected unexpectedly.",
                                )
                            }
                        }
                    }
                    None => failed_provider(&id, &provider_name, "Unknown provider"),
                };

                on_provider(usage.clone());
                let mut res_guard = results.lock().unwrap_or_else(|p| p.into_inner());
                res_guard[index] = Some(usage);
            }));
        }

        for handle in handles {
            let _ = handle.join();
        }

        let mut guard = results.lock().unwrap_or_else(|p| p.into_inner());
        let providers = guard
            .drain(..)
            .map(|item| {
                item.unwrap_or_else(|| failed_provider("unknown", "Unknown", "Missing result"))
            })
            .collect();

        AiUsageSnapshot {
            providers,
            fetched_at: now_secs(),
        }
    }
}
