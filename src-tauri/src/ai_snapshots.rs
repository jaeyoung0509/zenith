//! Backend-owned domain services for usage and activity snapshots.
//!
//! Commands and the background runtime consume these helpers rather than
//! invoking collectors independently. Each service owns its cache contract and
//! one in-flight collection per normalized request key via [`SingleFlight`].
//!
//! - Usage keys derive from the provider selection order and a non-secret
//!   key-presence flag. Response ordering follows the requested provider order.
//! - Activity keys include the inactivity threshold so background and
//!   foreground paths cannot silently publish differently configured results.
//! - Force bypasses a completed cache but joins an already-running compatible
//!   refresh. Configuration changes bump the generation; an old completion
//!   never overwrites the newer cache.
//! - Caches hold *raw* registries/snapshots. Final project enrichment
//!   (dev ports, artifact sizes) is explicit per consumer so a raw refresh
//!   cannot replace an enriched snapshot required by the project view.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::agent_activity::AgentActivityRegistry;
use crate::collection::{activity_request_key, usage_request_key, Admission, SingleFlight};
use crate::models::{AiProviderUsage, AiUsageSnapshot, ProviderId};
use crate::runtime_metrics::RuntimeMetrics;

const USAGE_CACHE_TTL_SECS: u64 = 60;

/// Invalidate under the same lock used by generation-checked publication.
pub fn invalidate_snapshot<T>(cache: &Mutex<Option<T>>, generation: &AtomicU64) {
    let mut guard = cache.lock().unwrap_or_else(|p| p.into_inner());
    generation.fetch_add(1, Ordering::SeqCst);
    *guard = None;
}

/// A detached supervisor always publishes success or worker failure, even when
/// the request that admitted this collection is dropped.
fn supervise_collection<T, P>(
    cache: Arc<Mutex<Option<T>>>,
    generation: Arc<AtomicU64>,
    expected_generation: u64,
    flight: Arc<SingleFlight<T, P>>,
    entry: Arc<crate::collection::Inflight<T, P>>,
    collect: impl FnOnce() -> T + Send + 'static,
) where
    T: Clone + Send + 'static,
    P: Clone + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        let result = tauri::async_runtime::spawn_blocking(collect)
            .await
            .map_err(|error| format!("Snapshot collection failed: {error}"));
        if let Ok(snapshot) = &result {
            let mut guard = cache.lock().unwrap_or_else(|p| p.into_inner());
            if generation.load(Ordering::SeqCst) == expected_generation {
                *guard = Some(snapshot.clone());
            }
        }
        flight.complete(&entry, result).await;
    });
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn usage_cache_hit(
    cache: &Arc<Mutex<Option<AiUsageSnapshot>>>,
    metrics: &Arc<RuntimeMetrics>,
    providers: &[ProviderId],
    now: u64,
) -> Option<AiUsageSnapshot> {
    let guard = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(snapshot) = guard.as_ref() {
        if snapshot.is_fresh_at(now, USAGE_CACHE_TTL_SECS)
            && snapshot.providers.len() == providers.len()
            && snapshot
                .providers
                .iter()
                .zip(providers)
                .all(|(provider, selected)| provider.id == *selected)
        {
            metrics.record_cache_hit();
            return Some(snapshot.clone());
        }
    }
    None
}

/// Fetch a usage snapshot through the shared single-flight.
///
/// `on_provider` receives incremental provider observations for the caller's
/// own progress channel. Late joiners replay buffered observations through
/// their own callback without starting another collector.
//
// Nine explicit shared handles keep the service free of global state and
// usable from both commands and the background runtime.
#[allow(clippy::too_many_arguments)]
pub async fn fetch_usage_snapshot(
    cache: &Arc<Mutex<Option<AiUsageSnapshot>>>,
    singleflight: &Arc<SingleFlight<AiUsageSnapshot, AiProviderUsage>>,
    generation: &Arc<AtomicU64>,
    metrics: &Arc<RuntimeMetrics>,
    collection_service: Arc<crate::ai_providers::ProviderCollectionService>,
    credentials: Arc<dyn crate::ai_providers::CredentialStore>,
    providers: Vec<ProviderId>,
    force: bool,
    on_provider: impl Fn(AiProviderUsage) + Send + Sync + Clone + 'static,
) -> Result<AiUsageSnapshot, String> {
    let gen = generation.load(Ordering::SeqCst);
    let has_openrouter_key = credentials
        .get(ProviderId::OpenRouter)
        .ok()
        .flatten()
        .is_some();
    let key = usage_request_key(&providers, has_openrouter_key);
    let now = unix_timestamp();

    if !force {
        if let Some(snapshot) = usage_cache_hit(cache, metrics, &providers, now) {
            for provider in &snapshot.providers {
                on_provider(provider.clone());
            }
            return Ok(snapshot);
        }
    }

    match singleflight.admit(key.clone(), gen).await {
        Admission::Wait(inflight) => {
            let snapshot = singleflight
                .wait_with_progress(&inflight, |batch| {
                    for provider in batch {
                        on_provider(provider);
                    }
                })
                .await?;
            Ok(snapshot)
        }
        Admission::Own(inflight) => {
            if !force {
                if let Some(snapshot) = usage_cache_hit(cache, metrics, &providers, now) {
                    for provider in &snapshot.providers {
                        singleflight.push_progress(&inflight, provider.clone());
                        on_provider(provider.clone());
                    }
                    singleflight.complete(&inflight, Ok(snapshot.clone())).await;
                    return Ok(snapshot);
                }
            }
            let progress_sf = singleflight.clone();
            let progress_entry = inflight.clone();
            let creds = credentials.clone();
            let provs = providers.clone();
            let service = collection_service.clone();
            supervise_collection(
                cache.clone(),
                generation.clone(),
                gen,
                singleflight.clone(),
                inflight.clone(),
                move || {
                    service.collect_parallel(creds, &provs, move |provider| {
                        progress_sf.push_progress(&progress_entry, provider)
                    })
                },
            );
            singleflight
                .wait_with_progress(&inflight, |batch| {
                    for provider in batch {
                        on_provider(provider);
                    }
                })
                .await
        }
    }
}

fn activity_cache_hit(
    cache: &Arc<Mutex<Option<AgentActivityRegistry>>>,
    metrics: &Arc<RuntimeMetrics>,
    now: u64,
) -> Option<AgentActivityRegistry> {
    let guard = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(registry) = guard.as_ref() {
        if now.saturating_sub(registry.snapshot.observed_at)
            < crate::agent_activity::SNAPSHOT_TTL_SECONDS
        {
            metrics.record_cache_hit();
            return Some(registry.clone());
        }
    }
    None
}

/// Fetch a raw activity registry through the shared single-flight. The cached
/// value is always raw; callers apply explicit enrichment for their view.
pub async fn fetch_activity_registry(
    cache: &Arc<Mutex<Option<AgentActivityRegistry>>>,
    singleflight: &Arc<SingleFlight<AgentActivityRegistry, ()>>,
    generation: &Arc<AtomicU64>,
    metrics: &Arc<RuntimeMetrics>,
    inactivity_threshold_secs: u64,
    force: bool,
) -> Result<AgentActivityRegistry, String> {
    let gen = generation.load(Ordering::SeqCst);
    let key = activity_request_key(inactivity_threshold_secs);
    let now = unix_timestamp();

    if !force {
        if let Some(registry) = activity_cache_hit(cache, metrics, now) {
            return Ok(registry);
        }
    }

    match singleflight.admit(key.clone(), gen).await {
        Admission::Wait(inflight) => singleflight.wait(&inflight).await,
        Admission::Own(inflight) => {
            if !force {
                if let Some(registry) = activity_cache_hit(cache, metrics, now) {
                    singleflight.complete(&inflight, Ok(registry.clone())).await;
                    return Ok(registry);
                }
            }
            supervise_collection(
                cache.clone(),
                generation.clone(),
                gen,
                singleflight.clone(),
                inflight.clone(),
                move || {
                    crate::agent_activity::collect_registry_with_inactivity_threshold(
                        inactivity_threshold_secs,
                    )
                },
            );
            singleflight.wait(&inflight).await
        }
    }
}

/// Explicit project-view enrichment applied to a raw registry clone. The shared
/// cache keeps the raw value; only the response carries ports/artifact sizes.
pub fn enrich_activity_for_project_view(
    mut registry: AgentActivityRegistry,
    dev_store: &Arc<Mutex<crate::dev_ports::DevelopmentPortStore>>,
    storage_state: &Arc<crate::storage_commands::StorageWorkflowState>,
) -> AgentActivityRegistry {
    let listeners = crate::dev_ports::list_listeners(
        dev_store,
        &crate::dev_ports::RealDevPortSystem::default(),
    )
    .unwrap_or_default();
    let artifact_sizes = storage_state.cached_developer_artifact_sizes();
    for project in &mut registry.snapshot.projects {
        if let Some(root) = registry.project_roots.get(&project.identity.id) {
            project.artifact_size_bytes = artifact_sizes.get(root).copied();
            for listener in &listeners {
                if let Some(dir) = listener.working_directory.as_deref() {
                    if let Ok(canon) = std::path::Path::new(dir).canonicalize() {
                        if canon.starts_with(root) && !project.dev_ports.contains(&listener.port) {
                            project.dev_ports.push(listener.port);
                        }
                    }
                }
            }
            project.dev_ports.sort_unstable();
        }
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalidated_collection_cannot_repopulate_cache() {
        let cache = Arc::new(Mutex::new(None));
        let generation = Arc::new(AtomicU64::new(1));
        let flight = Arc::new(SingleFlight::<String, ()>::new());
        let entry = match flight.admit("test".into(), 1).await {
            Admission::Own(entry) => entry,
            Admission::Wait(_) => unreachable!(),
        };
        let (release, released) = std::sync::mpsc::channel();
        supervise_collection(
            cache.clone(),
            generation.clone(),
            1,
            flight.clone(),
            entry.clone(),
            move || {
                released.recv().unwrap();
                "stale".to_string()
            },
        );
        invalidate_snapshot(&cache, &generation);
        release.send(()).unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(5), flight.wait(&entry))
                .await
                .unwrap()
                .unwrap(),
            "stale"
        );
        assert!(cache.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn cancelled_admitting_request_does_not_abandon_collection() {
        let cache = Arc::new(Mutex::new(None));
        let flight = Arc::new(SingleFlight::<String, ()>::new());
        let generation = Arc::new(AtomicU64::new(1));
        let entry = match flight.admit("test".into(), 1).await {
            Admission::Own(entry) => entry,
            Admission::Wait(_) => unreachable!(),
        };
        let (release, released) = std::sync::mpsc::channel();
        supervise_collection(
            cache.clone(),
            generation,
            1,
            flight.clone(),
            entry.clone(),
            move || {
                released.recv().unwrap();
                "finished".to_string()
            },
        );
        let first = {
            let flight = flight.clone();
            let entry = entry.clone();
            tokio::spawn(async move { flight.wait(&entry).await })
        };
        first.abort();
        let _ = first.await;
        release.send(()).unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), flight.wait(&entry))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result, "finished");
        assert_eq!(cache.lock().unwrap().as_deref(), Some("finished"));
    }

    #[tokio::test]
    async fn panicking_collector_completes_waiters_and_allows_retry() {
        let flight = Arc::new(SingleFlight::<String, ()>::new());
        let entry = match flight.admit("test".into(), 1).await {
            Admission::Own(entry) => entry,
            Admission::Wait(_) => unreachable!(),
        };
        supervise_collection(
            Arc::new(Mutex::new(None)),
            Arc::new(AtomicU64::new(1)),
            1,
            flight.clone(),
            entry.clone(),
            || panic!("fixture failure"),
        );
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), flight.wait(&entry))
            .await
            .unwrap();
        assert!(result.is_err());
        assert!(matches!(
            flight.admit("test".into(), 1).await,
            Admission::Own(_)
        ));
    }

    #[allow(clippy::type_complexity)]
    fn test_state() -> (
        Arc<Mutex<Option<AiUsageSnapshot>>>,
        Arc<SingleFlight<AiUsageSnapshot, AiProviderUsage>>,
        Arc<AtomicU64>,
        Arc<RuntimeMetrics>,
    ) {
        let metrics = Arc::new(RuntimeMetrics::new());
        (
            Arc::new(Mutex::new(None)),
            Arc::new(SingleFlight::with_metrics(metrics.clone())),
            Arc::new(AtomicU64::new(1)),
            metrics,
        )
    }

    #[tokio::test]
    async fn concurrent_usage_requests_share_one_collection() {
        let (cache, flight, generation, metrics) = test_state();
        let service = Arc::new(crate::ai_providers::ProviderCollectionService::default());
        let providers = vec![ProviderId::Cursor, ProviderId::GrokBuild];
        let barrier = Arc::new(tokio::sync::Barrier::new(6));
        let mut handles = Vec::new();
        for _ in 0..6 {
            let (cache, flight, generation, metrics) = (
                cache.clone(),
                flight.clone(),
                generation.clone(),
                metrics.clone(),
            );
            let service = service.clone();
            let providers = providers.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                fetch_usage_snapshot(
                    &cache,
                    &flight,
                    &generation,
                    &metrics,
                    service,
                    Arc::new(crate::ai_providers::InMemoryCredentialStore::new()),
                    providers,
                    false,
                    |_| {},
                )
                .await
                .unwrap()
            }));
        }
        let mut snapshots = Vec::new();
        for handle in handles {
            snapshots.push(handle.await.unwrap());
        }
        // All callers observe the same ordered providers from one collection.
        for snapshot in &snapshots {
            assert_eq!(
                snapshot
                    .providers
                    .iter()
                    .map(|p| p.id)
                    .collect::<Vec<_>>(),
                vec![ProviderId::Cursor, ProviderId::GrokBuild]
            );
        }
        let counts = metrics.snapshot();
        assert_eq!(counts.collections_started, 1);
        assert!(counts.coalesced_joins >= 1);
    }

    #[tokio::test]
    async fn forced_usage_joins_running_collection() {
        let (cache, flight, generation, metrics) = test_state();
        let service = Arc::new(crate::ai_providers::ProviderCollectionService::default());
        let providers = vec![ProviderId::Cursor, ProviderId::GrokBuild];
        // Barrier-driven: forced and normal refreshes admitted together must
        // join one compatible running collection instead of duplicating it.
        let barrier = Arc::new(tokio::sync::Barrier::new(4));
        let mut handles = Vec::new();
        for force in [false, true, false, true] {
            let (cache, flight, generation, metrics) = (
                cache.clone(),
                flight.clone(),
                generation.clone(),
                metrics.clone(),
            );
            let service = service.clone();
            let providers = providers.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                fetch_usage_snapshot(
                    &cache,
                    &flight,
                    &generation,
                    &metrics,
                    service,
                    Arc::new(crate::ai_providers::InMemoryCredentialStore::new()),
                    providers,
                    force,
                    |_| {},
                )
                .await
                .unwrap()
            }));
        }
        let mut snapshots = Vec::new();
        for handle in handles {
            snapshots.push(handle.await.unwrap());
        }
        for snapshot in &snapshots {
            assert_eq!(snapshot.providers.len(), snapshots[0].providers.len());
        }
        assert_eq!(metrics.snapshot().collections_started, 1);
    }

    #[tokio::test]
    async fn concurrent_activity_requests_share_one_collection() {
        let metrics = Arc::new(RuntimeMetrics::new());
        let cache = Arc::new(Mutex::new(None));
        let flight = Arc::new(SingleFlight::with_metrics(metrics.clone()));
        let generation = Arc::new(AtomicU64::new(1));
        let barrier = Arc::new(tokio::sync::Barrier::new(4));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let (cache, flight, generation, metrics) = (
                cache.clone(),
                flight.clone(),
                generation.clone(),
                metrics.clone(),
            );
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                fetch_activity_registry(&cache, &flight, &generation, &metrics, 900, false)
                    .await
                    .unwrap()
            }));
        }
        for handle in handles {
            handle.await.unwrap();
        }
        let counts = metrics.snapshot();
        assert_eq!(counts.collections_started, 1);
    }
}
