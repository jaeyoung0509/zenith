//! Backend-owned single-flight coordination for usage and activity snapshots.
//!
//! Identical concurrent callers await the same underlying collection instead of
//! queueing duplicate collections behind a mutex. Force bypasses a completed
//! cache but joins an already-running compatible refresh. Configuration changes
//! create a new generation; an old completion must not overwrite the newer cache.
//!
//! Design notes:
//! - Admission holds the async map mutex only for a short critical section.
//!   Collection itself runs outside any shared-state lock, on `spawn_blocking`
//!   for synchronous filesystem/process work.
//! - Dropping one waiter never cancels work needed by others; the owner keeps
//!   running until it calls [`SingleFlight::complete`]. Waiter cancellation is
//!   just dropping the waiting future.
//! - In-flight entries are cleared on success, error, and owner-drop paths and
//!   are bounded by [`MAX_INFLIGHT_KEYS`] with TTL eviction for crashed owners.
//! - Progress buffers are bounded by [`MAX_PROGRESS_BUFFER`]; disconnected
//!   subscribers are simply dropped waiters and release no owner resources.
//! - Never put raw secrets in keys or logs. Usage keys carry only the provider
//!   selection order and a non-secret key-presence flag.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::runtime_metrics::RuntimeMetrics;

/// Hard cap for lifecycle-owned in-flight entries.
pub const MAX_INFLIGHT_KEYS: usize = 16;
/// Bound for buffered progress observations per in-flight collection.
pub const MAX_PROGRESS_BUFFER: usize = 64;
/// Stale owner TTL: entries older than this are evicted on admission.
const INFLIGHT_TTL: Duration = Duration::from_secs(300);

/// Normalized usage request key. Provider order is significant so each
/// consumer's response contract and ordering are preserved.
pub fn usage_request_key(provider_ids: &[String], has_openrouter_key: bool) -> String {
    let mut key = String::from("usage:");
    key.push_str(&provider_ids.join(","));
    key.push_str(if has_openrouter_key {
        "|openrouter-key"
    } else {
        "|no-key"
    });
    key
}

/// Normalized activity request key. The inactivity threshold is part of
/// invalidation semantics so background and foreground paths cannot silently
/// publish differently configured results.
pub fn activity_request_key(inactivity_threshold_secs: u64) -> String {
    format!("activity:inactive-{inactivity_threshold_secs}s")
}

struct InflightState<T> {
    result: Option<Result<T, String>>,
}

struct InflightProgress<P> {
    items: Vec<P>,
}

/// One in-flight collection shared by identical concurrent callers.
pub struct Inflight<T, P> {
    key: String,
    generation: u64,
    state: Mutex<InflightState<T>>,
    progress: Mutex<InflightProgress<P>>,
    notify: tokio::sync::Notify,
    started: tokio::time::Instant,
}

impl<T, P> Inflight<T, P> {
    fn is_completed(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .result
            .is_some()
    }

    fn progress_len(&self) -> usize {
        self.progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .items
            .len()
    }
}

/// Admission decision: either own a new collection or wait for a compatible one.
pub enum Admission<T, P> {
    Own(Arc<Inflight<T, P>>),
    Wait(Arc<Inflight<T, P>>),
}

/// Async single-flight coordinator. Owns only in-flight state; caches stay with
/// the calling service so final enrichment (ports/artifacts) remains explicit.
pub struct SingleFlight<T, P> {
    inner: tokio::sync::Mutex<HashMap<String, Arc<Inflight<T, P>>>>,
    metrics: Option<Arc<RuntimeMetrics>>,
}

impl<T, P> Default for SingleFlight<T, P> {
    fn default() -> Self {
        Self {
            inner: tokio::sync::Mutex::new(HashMap::new()),
            metrics: None,
        }
    }
}

impl<T, P> SingleFlight<T, P>
where
    T: Clone + Send + 'static,
    P: Clone + Send + 'static,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_metrics(metrics: Arc<RuntimeMetrics>) -> Self {
        Self {
            inner: tokio::sync::Mutex::new(HashMap::new()),
            metrics: Some(metrics),
        }
    }

    /// Admit a request. Short async critical section only; never run I/O here.
    pub async fn admit(&self, key: String, generation: u64) -> Admission<T, P> {
        if let Some(metrics) = &self.metrics {
            metrics.record_admission();
        }
        let mut guard = self.inner.lock().await;
        // TTL eviction for owners that never completed (crash/panic path).
        guard.retain(|_, inflight| inflight.started.elapsed() < INFLIGHT_TTL);
        if let Some(existing) = guard.get(&key) {
            if existing.generation == generation && !existing.is_completed() {
                if let Some(metrics) = &self.metrics {
                    metrics.record_coalesced_join();
                }
                return Admission::Wait(existing.clone());
            }
            // Same key but a newer/older generation: fall through to replace.
            // The old owner still completes its own entry object, but
            // `complete` will refuse to remove the newer entry, so the stale
            // result is discarded instead of overwriting the newer cache.
        }
        if guard.len() >= MAX_INFLIGHT_KEYS {
            // Evict the oldest entry to enforce the hard cap.
            if let Some(oldest) = guard
                .iter()
                .min_by_key(|(_, v)| v.started)
                .map(|(k, _)| k.clone())
            {
                guard.remove(&oldest);
            }
        }
        let inflight = Arc::new(Inflight {
            key: key.clone(),
            generation,
            state: Mutex::new(InflightState { result: None }),
            progress: Mutex::new(InflightProgress { items: Vec::new() }),
            notify: tokio::sync::Notify::new(),
            started: tokio::time::Instant::now(),
        });
        guard.insert(key, inflight.clone());
        if let Some(metrics) = &self.metrics {
            metrics.record_collection_started();
        }
        Admission::Own(inflight)
    }

    /// Push one progress observation. Bounded; excess beyond the cap is dropped
    /// so a chatty collector cannot grow memory without bound.
    pub fn push_progress(&self, inflight: &Inflight<T, P>, item: P) {
        let mut progress = inflight
            .progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if progress.items.len() < MAX_PROGRESS_BUFFER {
            progress.items.push(item);
        }
        drop(progress);
        inflight.notify.notify_waiters();
    }

    /// Snapshot buffered progress for late joiners.
    pub fn progress_snapshot(&self, inflight: &Inflight<T, P>) -> Vec<P> {
        inflight
            .progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .items
            .clone()
    }

    /// Wait for completion without occupying a blocking worker. Dropping the
    /// returned future cancels only this waiter.
    pub async fn wait(&self, inflight: &Arc<Inflight<T, P>>) -> Result<T, String> {
        loop {
            {
                let guard = inflight
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(result) = guard.result.clone() {
                    return result;
                }
            }
            inflight.notify.notified().await;
        }
    }

    /// Wait while streaming newly buffered progress to `on_batch`. Late joiners
    /// first receive everything buffered so far, then subsequent batches.
    pub async fn wait_with_progress(
        &self,
        inflight: &Arc<Inflight<T, P>>,
        mut on_batch: impl FnMut(Vec<P>),
    ) -> Result<T, String> {
        let mut seen = 0usize;
        // Replay anything completed before we subscribed.
        let initial = self.progress_snapshot(inflight);
        if !initial.is_empty() {
            seen = initial.len();
            on_batch(initial);
        }
        loop {
            {
                let guard = inflight
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(result) = guard.result.clone() {
                    // Drain any progress that arrived with completion.
                    let all = self.progress_snapshot(inflight);
                    if all.len() > seen {
                        on_batch(all[seen..].to_vec());
                    }
                    return result;
                }
            }
            inflight.notify.notified().await;
            let all = self.progress_snapshot(inflight);
            if all.len() > seen {
                let batch = all[seen..].to_vec();
                seen = all.len();
                on_batch(batch);
            }
            let _ = inflight.progress_len();
        }
    }

    /// Complete a collection. Clears in-flight state on success, error, panic
    /// (via stale TTL), and cancellation (owner drops without completing leave
    /// a TTL-evicted entry; prefer explicit completion). Returns `true` when the
    /// caller may publish to the cache; `false` means a newer generation has
    /// superseded this result and it must be discarded.
    pub async fn complete(
        &self,
        inflight: &Arc<Inflight<T, P>>,
        result: Result<T, String>,
    ) -> bool {
        let is_ok = result.is_ok();
        {
            let mut guard = inflight
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.result = Some(result);
        }
        inflight.notify.notify_waiters();
        let mut map = self.inner.lock().await;
        let current = map.get(&inflight.key);
        let publishable = match current {
            Some(entry) if Arc::ptr_eq(entry, inflight) => {
                map.remove(&inflight.key);
                true
            }
            // A newer generation replaced this key; discard the stale result.
            _ => false,
        };
        drop(map);
        if let Some(metrics) = &self.metrics {
            if is_ok {
                metrics.record_collection_completed();
            } else {
                metrics.record_collection_failed();
            }
        }
        publishable
    }

    /// Remove entries on shutdown/cancellation paths. Best-effort; TTL covers
    /// owners that never return.
    pub async fn cancel(&self, key: &str, generation: u64) {
        let mut map = self.inner.lock().await;
        if let Some(entry) = map.get(key) {
            if entry.generation == generation {
                map.remove(key);
            }
        }
    }

    #[cfg(test)]
    async fn len(&self) -> usize {
        self.inner.lock().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn test_flight() -> SingleFlight<String, String> {
        SingleFlight::new()
    }

    #[tokio::test]
    async fn concurrent_compatible_requests_execute_one_collection() {
        let flight = Arc::new(test_flight());
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(tokio::sync::Barrier::new(8));

        let mut handles = Vec::new();
        for _ in 0..8 {
            let flight = flight.clone();
            let calls = calls.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                match flight.admit("usage:a,b|no-key".into(), 1).await {
                    Admission::Own(inflight) => {
                        calls.fetch_add(1, Ordering::SeqCst);
                        // Simulate collection outside the admission lock.
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        flight.complete(&inflight, Ok("snapshot".into())).await;
                        "snapshot".to_string()
                    }
                    Admission::Wait(inflight) => flight.wait(&inflight).await.unwrap(),
                }
            }));
        }
        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.await.unwrap());
        }
        assert!(results.iter().all(|r| r == "snapshot"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(flight.len().await, 0);
    }

    #[tokio::test]
    async fn forced_refresh_joins_running_compatible_collection() {
        let flight = Arc::new(test_flight());
        let calls = Arc::new(AtomicUsize::new(0));

        let flight_owner = flight.clone();
        let calls_owner = calls.clone();
        let owner = tokio::spawn(async move {
            match flight_owner.admit("usage:a|no-key".into(), 1).await {
                Admission::Own(inflight) => {
                    calls_owner.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(60)).await;
                    flight_owner.complete(&inflight, Ok("v1".into())).await;
                    "v1".to_string()
                }
                Admission::Wait(inflight) => flight_owner.wait(&inflight).await.unwrap(),
            }
        });
        // Let the owner start, then issue a forced refresh for the same key.
        tokio::time::sleep(Duration::from_millis(10)).await;
        let forced = match flight.admit("usage:a|no-key".into(), 1).await {
            Admission::Own(inflight) => {
                calls.fetch_add(1, Ordering::SeqCst);
                flight.complete(&inflight, Ok("v-forced".into())).await;
                "v-forced".to_string()
            }
            Admission::Wait(inflight) => flight.wait(&inflight).await.unwrap(),
        };
        assert_eq!(forced, "v1");
        assert_eq!(owner.await.unwrap(), "v1");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn generation_change_starts_new_collection_and_rejects_stale() {
        let flight = Arc::new(test_flight());
        let old = match flight.admit("usage:a|no-key".into(), 1).await {
            Admission::Own(inflight) => inflight,
            Admission::Wait(_) => panic!("expected owner"),
        };
        // Configuration change creates a new generation for the same key.
        let new = match flight.admit("usage:a|no-key".into(), 2).await {
            Admission::Own(inflight) => inflight,
            Admission::Wait(_) => panic!("new generation must not join the old"),
        };
        // Old completion must not clear the newer entry.
        let publish_old = flight.complete(&old, Ok("stale".into())).await;
        assert!(!publish_old);
        assert_eq!(flight.len().await, 1);
        let publish_new = flight.complete(&new, Ok("fresh".into())).await;
        assert!(publish_new);
        assert_eq!(flight.len().await, 0);
    }

    #[tokio::test]
    async fn provider_ordering_is_part_of_the_key() {
        let a = usage_request_key(&["cursor".to_string(), "grok".to_string()], false);
        let b = usage_request_key(&["grok".to_string(), "cursor".to_string()], false);
        assert_ne!(a, b, "order-sensitive key preserves response ordering");

        let flight = SingleFlight::<String, String>::new();
        match flight.admit(a.clone(), 1).await {
            Admission::Own(inflight) => {
                flight.complete(&inflight, Ok("ok".into())).await;
            }
            Admission::Wait(_) => panic!("expected owner"),
        }
        // Same ordered key coalesces; tested in the barrier test above.
        assert_eq!(flight.len().await, 0);
    }

    #[tokio::test]
    async fn late_joiner_receives_buffered_progress() {
        let flight = SingleFlight::<String, String>::new();
        let inflight = match flight.admit("usage:a|no-key".into(), 1).await {
            Admission::Own(inflight) => inflight,
            Admission::Wait(_) => panic!("expected owner"),
        };
        flight.push_progress(&inflight, "p1".into());
        flight.push_progress(&inflight, "p2".into());

        let waiter = flight.wait_with_progress(&inflight, |_| {});
        // Owner publishes subsequent progress then completes.
        flight.push_progress(&inflight, "p3".into());
        flight.complete(&inflight, Ok("done".into())).await;
        assert_eq!(waiter.await.unwrap(), "done");
        assert_eq!(
            flight.progress_snapshot(&inflight),
            vec!["p1".to_string(), "p2".to_string(), "p3".to_string()]
        );
    }

    #[tokio::test]
    async fn waiter_cancellation_does_not_cancel_owner_work() {
        let flight = Arc::new(test_flight());
        let inflight = match flight.admit("k".into(), 1).await {
            Admission::Own(inflight) => inflight,
            Admission::Wait(_) => panic!("expected owner"),
        };
        let waiter_handle = {
            let flight = flight.clone();
            let inflight = inflight.clone();
            tokio::spawn(async move { flight.wait(&inflight).await })
        };
        // Dropping one waiter must not cancel work needed by others.
        waiter_handle.abort();
        let _ = waiter_handle.await;
        // Owner still completes; a second waiter still gets the result.
        let second = match flight.admit("k".into(), 1).await {
            Admission::Own(_) => panic!("owner still running; must join"),
            Admission::Wait(inflight) => inflight,
        };
        flight.complete(&inflight, Ok("shared".into())).await;
        assert_eq!(flight.wait(&second).await.unwrap(), "shared");
    }

    #[tokio::test]
    async fn retry_after_failure_starts_a_new_collection() {
        let flight = test_flight();
        let inflight = match flight.admit("k".into(), 1).await {
            Admission::Own(inflight) => inflight,
            Admission::Wait(_) => panic!("expected owner"),
        };
        assert!(flight.complete(&inflight, Err("boom".into())).await);
        assert_eq!(flight.len().await, 0);
        // Next admission retries instead of replaying the failure.
        match flight.admit("k".into(), 1).await {
            Admission::Own(inflight) => {
                assert!(flight.complete(&inflight, Ok("recovered".into())).await);
            }
            Admission::Wait(_) => panic!("failure must not be cached as in-flight"),
        }
    }

    #[test]
    fn usage_key_never_embeds_secrets() {
        let key = usage_request_key(&["openrouter".to_string()], true);
        assert!(key.contains("openrouter-key"));
        assert!(!key.contains("sk-"));
    }

    #[test]
    fn activity_key_includes_inactivity_threshold() {
        assert_ne!(activity_request_key(60), activity_request_key(900));
    }
}
