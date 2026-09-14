//! AI usage, agent activity, and AI Control Center application service.
//!
//! The service owns everything the AI surfaces need to stay consistent: the
//! provider credential store and its collection service, the usage and
//! activity caches with their single-flight and generation contracts, the
//! Control Center state, its refresh lock, and the background runtime.
//!
//! Handlers pass IPC inputs in and get a typed result out. They never lock a
//! cache, invalidate a snapshot, or decide when a collection may start.
//!
//! Locking policy: the AI caches are disposable observations, so a poisoned
//! lock is recovered and overwritten rather than wedging the surface; the
//! settings authority fails closed and is never bypassed.

use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::ai_control_center::runtime::AiControlRuntime;
use crate::ai_control_center::state::AiControlCenterState;
use crate::ai_providers::{CredentialStore, ProviderCollectionService};
use crate::collection::SingleFlight;
use crate::execution_budget::ExecutionBudgets;
use crate::metrics::MemorySampler;
use crate::models::{
    AgentActivitySnapshot, AgentActivityStatus, AgentIntegrationInfo, AgentIntegrationResult,
    AgentQuickSessionRow, AgentQuickSummary, AiControlCenterSnapshot, AiControlPreferences,
    AiProviderUsage, AiUsageSnapshot, CapabilityAccess, ControlCenterQuickSummary,
    IngestedAgentEvent, PlatformFeature, RecommendationPreview, SafetySnapshot, ZenithSettings,
};
use crate::power::KeepAwakeManager;
use crate::runtime_metrics::RuntimeMetrics;
use crate::services::desktop_notifications::DesktopNotifications;
use crate::services::progress::ProviderUsageSink;
use crate::services::settings_service::{
    SettingsAuthority, SettingsChange, SettingsChangeReaction,
};
use crate::services::StorageService;
use zenith_platform::{PlatformCapabilitiesProvider, PlatformEnvironment};

/// Resolves a user root against the environment the process was described by,
/// never the host's.
fn user_home(environment: &PlatformEnvironment) -> Result<std::path::PathBuf, String> {
    environment
        .user_home()
        .ok_or_else(|| "User home directory is not available".to_string())
}

/// The Control Center state is a disposable observation, so a poisoned lock is
/// recovered and overwritten rather than wedging every AI surface.
fn lock_control(state: &Arc<Mutex<AiControlCenterState>>) -> MutexGuard<'_, AiControlCenterState> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// How long a freshly observed Control Center snapshot answers without work.
const CONTROL_CENTER_SNAPSHOT_TTL_SECS: u64 = 10;

/// The AI surfaces: provider usage, agent activity, and the Control Center.
pub struct AiService {
    environment: Arc<PlatformEnvironment>,
    platform_capabilities: Arc<dyn PlatformCapabilitiesProvider>,
    settings: Arc<SettingsAuthority>,
    credentials: Arc<dyn CredentialStore>,
    collection: Arc<ProviderCollectionService>,
    usage_cache: Arc<Mutex<Option<AiUsageSnapshot>>>,
    usage_singleflight: Arc<SingleFlight<AiUsageSnapshot, AiProviderUsage>>,
    usage_generation: Arc<AtomicU64>,
    activity_cache: Arc<Mutex<Option<crate::agent_activity::AgentActivityRegistry>>>,
    activity_singleflight: Arc<SingleFlight<crate::agent_activity::AgentActivityRegistry, ()>>,
    activity_generation: Arc<AtomicU64>,
    control_state: Arc<Mutex<AiControlCenterState>>,
    control_refresh_lock: Arc<Mutex<()>>,
    runtime: Arc<AiControlRuntime>,
    runtime_metrics: Arc<RuntimeMetrics>,
    budgets: Arc<ExecutionBudgets>,
    memory_sampler: Arc<MemorySampler>,
    dev_ports: Arc<Mutex<crate::dev_ports::DevelopmentPortStore>>,
    awake: Arc<KeepAwakeManager>,
    storage: Arc<StorageService>,
}

impl AiService {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        environment: Arc<PlatformEnvironment>,
        platform_capabilities: Arc<dyn PlatformCapabilitiesProvider>,
        settings: Arc<SettingsAuthority>,
        credentials: Arc<dyn CredentialStore>,
        collection: Arc<ProviderCollectionService>,
        usage_cache: Arc<Mutex<Option<AiUsageSnapshot>>>,
        usage_singleflight: Arc<SingleFlight<AiUsageSnapshot, AiProviderUsage>>,
        usage_generation: Arc<AtomicU64>,
        activity_cache: Arc<Mutex<Option<crate::agent_activity::AgentActivityRegistry>>>,
        activity_singleflight: Arc<SingleFlight<crate::agent_activity::AgentActivityRegistry, ()>>,
        activity_generation: Arc<AtomicU64>,
        control_state: Arc<Mutex<AiControlCenterState>>,
        control_refresh_lock: Arc<Mutex<()>>,
        runtime: Arc<AiControlRuntime>,
        runtime_metrics: Arc<RuntimeMetrics>,
        budgets: Arc<ExecutionBudgets>,
        memory_sampler: Arc<MemorySampler>,
        dev_ports: Arc<Mutex<crate::dev_ports::DevelopmentPortStore>>,
        awake: Arc<KeepAwakeManager>,
        storage: Arc<StorageService>,
    ) -> Self {
        Self {
            environment,
            platform_capabilities,
            settings,
            credentials,
            collection,
            usage_cache,
            usage_singleflight,
            usage_generation,
            activity_cache,
            activity_singleflight,
            activity_generation,
            control_state,
            control_refresh_lock,
            runtime,
            runtime_metrics,
            budgets,
            memory_sampler,
            dev_ports,
            awake,
            storage,
        }
    }

    /// The background runtime, for the thread the desktop shell drives.
    pub fn runtime(&self) -> Arc<AiControlRuntime> {
        self.runtime.clone()
    }

    /// Restores the persisted audit entries at startup.
    ///
    /// The audit store is a persistence concern of this service, so reading it
    /// back is a call here rather than a lock a startup path takes for itself.
    pub fn restore_audit(&self, config_dir: &Path) {
        self.control().audit = crate::ai_control_center::audit::AuditStore::load(config_dir);
    }

    fn control(&self) -> MutexGuard<'_, AiControlCenterState> {
        self.control_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn require(&self, feature: PlatformFeature, access: CapabilityAccess) -> Result<(), String> {
        self.platform_capabilities
            .capabilities()
            .require(feature, access)
            .map_err(|error| error.to_string())
    }

    /// Collects (or reuses) a provider usage snapshot, reporting each provider
    /// observation through `progress` as it arrives.
    pub async fn usage_snapshot(
        &self,
        force: bool,
        progress: Arc<dyn ProviderUsageSink>,
    ) -> Result<AiUsageSnapshot, String> {
        let provider_ids = self.settings.snapshot()?.ai_accounts_quota_providers;
        crate::ai_snapshots::fetch_usage_snapshot(
            &self.usage_cache,
            &self.usage_singleflight,
            &self.usage_generation,
            &self.runtime_metrics,
            self.collection.clone(),
            self.credentials.clone(),
            provider_ids,
            force,
            move |provider| progress.emit(provider),
        )
        .await
    }

    /// Collects agent activity, enriches it for the project view, and delivers
    /// the possibly-inactive advisories.
    pub async fn project_context(
        &self,
        force: bool,
        notifications: Arc<dyn DesktopNotifications>,
    ) -> Result<AgentActivitySnapshot, String> {
        let notification_preferences = self.settings.snapshot()?.agent_notifications;
        let threshold_secs = u64::from(notification_preferences.inactivity_threshold_minutes) * 60;
        // Shared raw registry; enrichment stays explicit so a raw refresh never
        // replaces the enriched snapshot required by this view.
        let raw = crate::ai_snapshots::fetch_activity_registry(
            &self.activity_cache,
            &self.activity_singleflight,
            &self.activity_generation,
            &self.runtime_metrics,
            &self.environment,
            threshold_secs,
            force,
        )
        .await?;
        let dev_store = self.dev_ports.clone();
        let storage_service = self.storage.clone();
        let environment = self.environment.clone();
        crate::blocking::run_blocking(
            move || {
                let enriched_registry = crate::ai_snapshots::enrich_activity_for_project_view(
                    raw,
                    &dev_store,
                    &storage_service,
                    &environment,
                );
                let snapshot = enriched_registry.snapshot.clone();
                {
                    let store = crate::agent_activity::global_store();
                    let mut guard = store
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let _ = notifications.emit_process_advisories(
                        &snapshot,
                        &notification_preferences,
                        &mut guard.notification_filter,
                    );
                }
                Ok(snapshot)
            },
            "Agent activity refresh worker panicked",
        )
        .await
    }

    /// Consumes a stop lease and terminates the exact observed process group.
    ///
    /// The lease was minted against a fresh snapshot; consuming it here is what
    /// makes the request one-shot, and the cache is dropped afterwards so the
    /// next observation reflects the termination rather than the previous view.
    pub async fn stop_agent_session(&self, session_id: &str, lease_id: &str) -> Result<(), String> {
        let now = unix_timestamp();
        let lease = {
            let store = crate::agent_activity::global_store();
            let mut guard = store
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.stop_leases.consume_lease(session_id, lease_id, now)?
        };

        let cache = self.activity_cache.clone();
        let runtime = self.runtime.clone();
        let environment = self.environment.clone();
        crate::blocking::run_blocking(
            move || {
                let system = crate::agent_activity::termination::RealTerminationSystem;
                let result = crate::agent_activity::termination::execute_graceful_stop(
                    &lease,
                    &system,
                    &environment,
                );
                if result.is_ok() {
                    if let Ok(mut cache_guard) = cache.lock() {
                        *cache_guard = None;
                    }
                    runtime.notify_wake();
                }
                result
            },
            "Graceful stop worker panicked",
        )
        .await
    }

    pub async fn agent_integrations(&self) -> Result<Vec<AgentIntegrationInfo>, String> {
        self.require(PlatformFeature::AiIntegrations, CapabilityAccess::Inspect)?;
        let environment = self.environment.clone();
        crate::blocking::run_blocking(
            move || {
                let home = user_home(&environment)?;
                const TOOLS: &[&str] = &[
                    "antigravity",
                    "claude",
                    "cursor",
                    "grok",
                    "copilot",
                    "gemini",
                    "codex",
                    "opencode",
                ];
                Ok(TOOLS
                    .iter()
                    .map(|tool| crate::agent_activity::hooks::get_integration_info(tool, &home))
                    .collect())
            },
            "Agent integration worker panicked",
        )
        .await
    }

    pub async fn setup_agent_integration(
        &self,
        tool_id: &str,
    ) -> Result<AgentIntegrationResult, String> {
        self.require(PlatformFeature::AiIntegrations, CapabilityAccess::Mutate)?;
        let environment = self.environment.clone();
        let tool_id = tool_id.to_string();
        crate::blocking::run_blocking(
            move || {
                let home = user_home(&environment)?;
                crate::agent_activity::hooks::install_integration(&tool_id, &home)
            },
            "Agent integration setup worker panicked",
        )
        .await
    }

    pub async fn remove_agent_integration(
        &self,
        tool_id: &str,
    ) -> Result<AgentIntegrationResult, String> {
        self.require(PlatformFeature::AiIntegrations, CapabilityAccess::Mutate)?;
        let environment = self.environment.clone();
        let tool_id = tool_id.to_string();
        crate::blocking::run_blocking(
            move || {
                let home = user_home(&environment)?;
                crate::agent_activity::hooks::uninstall_integration(&tool_id, &home)
            },
            "Agent integration removal worker panicked",
        )
        .await
    }

    /// The compact activity summary the menu-bar panel renders.
    ///
    /// The aggregation is a domain rule (which statuses count as active, how
    /// rows are ordered, how many are shown), so it lives here rather than in
    /// the handler that happens to request it.
    pub async fn agent_quick_summary(&self) -> Result<Option<AgentQuickSummary>, String> {
        let threshold_secs = self
            .settings
            .snapshot()
            .map(|settings| {
                u64::from(settings.agent_notifications.inactivity_threshold_minutes) * 60
            })
            .unwrap_or(crate::agent_activity::DEFAULT_INACTIVITY_THRESHOLD_SECONDS);
        let registry = crate::ai_snapshots::fetch_activity_registry(
            &self.activity_cache,
            &self.activity_singleflight,
            &self.activity_generation,
            &self.runtime_metrics,
            &self.environment,
            threshold_secs,
            false,
        )
        .await
        .map_err(|error| format!("Agent activity cache is unavailable: {error}"))?;

        let mut active_count = 0;
        let mut attention_count = 0;
        let mut rows = Vec::new();

        let push_session = |session: &crate::models::AgentSession,
                            project_name: &str,
                            active_count: &mut u32,
                            attention_count: &mut u32,
                            rows: &mut Vec<AgentQuickSessionRow>| {
            if matches!(
                session.status,
                AgentActivityStatus::Working
                    | AgentActivityStatus::Active
                    | AgentActivityStatus::Starting
            ) {
                *active_count += 1;
            }
            if session.attention_reason.is_some() {
                *attention_count += 1;
            }
            rows.push(AgentQuickSessionRow {
                session_id: session.id.clone(),
                tool_name: session.tool_name.clone(),
                project_name: project_name.to_string(),
                status: session.status,
                evidence: session.evidence,
                elapsed_seconds: session.elapsed_seconds,
            });
        };

        for project in &registry.snapshot.projects {
            for session in &project.sessions {
                push_session(
                    session,
                    &project.identity.display_name,
                    &mut active_count,
                    &mut attention_count,
                    &mut rows,
                );
            }
        }
        for session in &registry.snapshot.unassigned_sessions {
            push_session(
                session,
                "Unassigned",
                &mut active_count,
                &mut attention_count,
                &mut rows,
            );
        }

        rows.sort_by_key(|row| std::cmp::Reverse(row.elapsed_seconds));
        rows.truncate(3);

        Ok(Some(AgentQuickSummary {
            active_count,
            attention_count,
            sessions: rows,
        }))
    }

    /// Ingests a hook-delivered event and invalidates the cached view.
    pub fn ingest_agent_event(&self, event: IngestedAgentEvent) -> Result<(), String> {
        let event =
            crate::agent_activity::events::validate_ingested_event(event, unix_timestamp())?;
        let store = crate::agent_activity::global_store();
        let mut guard = store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.record_event(event);
        crate::ai_snapshots::invalidate_snapshot(&self.activity_cache, &self.activity_generation);
        Ok(())
    }

    /// Builds (or reuses) the AI Control Center snapshot.
    pub async fn control_center(
        &self,
        force: bool,
        config_dir: &Path,
    ) -> Result<AiControlCenterSnapshot, String> {
        // Fast path under a short lock: a warm snapshot never enters blocking work.
        if !force {
            let fresh = self
                .control()
                .last_snapshot
                .as_ref()
                .filter(|snapshot| {
                    unix_timestamp().saturating_sub(snapshot.observed_at)
                        < CONTROL_CENTER_SNAPSHOT_TTL_SECS
                })
                .cloned();
            if let Some(snapshot) = fresh {
                self.runtime_metrics.record_cache_hit();
                return Ok(snapshot);
            }
        }
        // Clone settings-derived inputs under short locks; no I/O held.
        let settings = self.settings.snapshot()?;
        let preferences = settings.ai_control.clone();
        let provider_ids = settings.ai_accounts_quota_providers.clone();
        let inactivity_threshold_secs =
            u64::from(settings.agent_notifications.inactivity_threshold_minutes) * 60;
        let config_dir = config_dir.to_path_buf();

        // Reuse the shared single-flight collections instead of invoking
        // collectors independently. Forced control refreshes join running
        // compatible sub-collections; completed sub-caches keep their own TTL
        // semantics.
        let activity = crate::ai_snapshots::fetch_activity_registry(
            &self.activity_cache,
            &self.activity_singleflight,
            &self.activity_generation,
            &self.runtime_metrics,
            &self.environment,
            inactivity_threshold_secs,
            false,
        )
        .await?;
        let usage = crate::ai_snapshots::fetch_usage_snapshot(
            &self.usage_cache,
            &self.usage_singleflight,
            &self.usage_generation,
            &self.runtime_metrics,
            self.collection.clone(),
            self.credentials.clone(),
            provider_ids,
            false,
            |_| {},
        )
        .await?;

        // Acquire the subprocess budget before dispatching blocking Git/listener
        // work. Cheap metrics/cache reads never take a budget and stay
        // responsive. The worker owns its permit until the blocking assembly
        // actually exits, including when the request awaiting it is cancelled.
        let subprocess_permit = self
            .budgets
            .acquire_subprocess()
            .await
            .map_err(|error| error.to_string())?;

        let refresh_lock = self.control_refresh_lock.clone();
        let control = self.control_state.clone();
        let memory_sampler = self.memory_sampler.clone();
        let awake = self.awake.clone();
        let dev_store = self.dev_ports.clone();
        let environment = self.environment.clone();
        crate::blocking::run_blocking(
            move || {
                // Running blocking work survives request cancellation, so its
                // permit must live in the worker rather than in the awaiting
                // request.
                let _subprocess_permit = subprocess_permit;
                let _refresh_guard = refresh_lock
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                // Recheck after acquiring the refresh lock: a concurrent refresh
                // may have published while we collected shared snapshots.
                if !force {
                    let fresh = lock_control(&control)
                        .last_snapshot
                        .as_ref()
                        .filter(|snapshot| {
                            unix_timestamp().saturating_sub(snapshot.observed_at)
                                < CONTROL_CENTER_SNAPSHOT_TTL_SECS
                        })
                        .cloned();
                    if let Some(snapshot) = fresh {
                        return Ok(snapshot);
                    }
                }
                let now = unix_timestamp();
                // Snapshot the smallest Git inputs under a short lock; Git commands
                // and filesystem fingerprinting run outside the shared state lock.
                let (git_baselines, git_generation) = {
                    let guard = lock_control(&control);
                    (
                        guard.git.snapshot_baselines(),
                        guard.git.baseline_generation(),
                    )
                };
                // Blocking-safe observations outside the Control Center lock.
                let memory = memory_sampler.sample();
                let awake_state = awake.get_state();
                let listeners = crate::dev_ports::list_listeners_with_context(
                    &dev_store,
                    &crate::dev_ports::RealDevPortSystem::new(environment.flavor()),
                    environment.user_home().as_deref(),
                )
                .unwrap_or_default();
                let resources = crate::ai_control_center::resources::attribute(
                    &activity.snapshot,
                    &activity.project_roots,
                    &listeners,
                    awake_state.power_source,
                    preferences.autopilot.keep_awake_ac_only,
                );
                let normalized =
                    crate::ai_control_center::providers::normalize(&usage, &preferences);
                let budget_statuses =
                    crate::ai_control_center::budgets::statuses(&preferences.budgets, &normalized);
                let (git_summaries, git_collected) =
                    crate::ai_control_center::git::GitBaselineStore::collect_summaries(
                        &git_baselines,
                        &activity.project_roots,
                        now,
                    );
                // Short final merge: no subprocess, disk I/O, or channel sends
                // under the shared lock. Audit persistence happens after the lock
                // is free.
                let (snapshot, audit_store) = {
                    let mut guard = lock_control(&control);
                    let providers = crate::ai_control_center::providers::retain_last_success(
                        normalized,
                        &mut guard.providers_last_success,
                    );
                    let new_items = guard.policy.evaluate(
                        &resources,
                        Some(memory.pressure),
                        awake_state.power_source,
                        &preferences.autopilot,
                        now,
                    );
                    if !new_items.is_empty() {
                        guard.recommendations.extend(new_items);
                        guard
                            .recommendations
                            .sort_by_key(|item| std::cmp::Reverse(item.created_at));
                        guard.recommendations.truncate(64);
                    }
                    // Superseded Git observations are discarded, never applied over
                    // newer baselines or restored for removed projects.
                    let _ = guard.git.commit_collected(
                        git_generation,
                        &activity.project_roots,
                        git_collected,
                    );
                    let recommendations = guard.recommendations.clone();
                    let safety = guard.safety.clone();
                    let notification_errors: Vec<String> = Vec::new();
                    let partial_errors = providers
                        .iter()
                        .filter_map(|item| item.partial_error.clone())
                        .chain(activity.snapshot.partial_errors.clone())
                        .chain(notification_errors)
                        .collect::<Vec<_>>();
                    let quality = if !partial_errors.is_empty()
                        || safety.quality == crate::models::ObservationQuality::Partial
                    {
                        crate::models::ObservationQuality::Partial
                    } else {
                        crate::models::ObservationQuality::Fresh
                    };
                    let quick_summary = ControlCenterQuickSummary {
                        observed_at: now,
                        active_sessions: resources.len() as u32,
                        budget_alerts: budget_statuses
                            .iter()
                            .filter(|item| !item.crossed_thresholds.is_empty())
                            .count() as u32,
                        safety_findings: safety
                            .findings
                            .iter()
                            .filter(|item| !item.dismissed)
                            .count() as u32,
                        quality,
                    };
                    guard.audit.append(
                        now,
                        "refresh",
                        "ok",
                        None,
                        "AI Control Center local snapshot refreshed",
                        preferences.audit_retention_days,
                    );
                    let audit_entries = guard.audit.entries();
                    let audit_store = guard.audit.clone();
                    let snapshot = AiControlCenterSnapshot {
                        observed_at: now,
                        providers,
                        budget_statuses,
                        resources,
                        recommendations,
                        safety,
                        git_summaries,
                        audit: audit_entries,
                        quick_summary,
                        keep_awake_active: awake.get_state().active_rule_id.as_deref()
                            == Some("ai-control.verified-session"),
                        partial_errors,
                    };
                    guard.last_snapshot = Some(snapshot.clone());
                    (snapshot, audit_store)
                };
                // Disk I/O outside the shared lock.
                let _ = audit_store.save(&config_dir);
                Ok(snapshot)
            },
            "AI Control Center refresh worker panicked",
        )
        .await
    }

    /// The last published Control Center summary, without observing anything.
    pub fn control_quick_summary(&self) -> Option<ControlCenterQuickSummary> {
        self.control()
            .last_snapshot
            .as_ref()
            .map(|value| value.quick_summary.clone())
    }

    /// Persists AI Control preferences, applies the native policy, and
    /// invalidates the published snapshot and the audit store it belongs to.
    pub async fn save_control_preferences(
        &self,
        preferences: AiControlPreferences,
        config_dir: &Path,
        notifications: Arc<dyn DesktopNotifications>,
    ) -> Result<(), String> {
        let preferences = crate::ai_control_center::budgets::sanitize(preferences);
        let retention = preferences.audit_retention_days;
        let config_dir = config_dir.to_path_buf();
        let settings = self.settings.clone();
        let awake_manager = self.awake.clone();
        let control_state = self.control_state.clone();
        crate::blocking::run_blocking(
            move || {
                if preferences.autopilot.notify_on_battery
                    || preferences.autopilot.notify_on_memory_pressure
                    || preferences.autopilot.notify_on_session_completion
                {
                    notifications.request_permission()?;
                }
                settings.update_and_persist(&config_dir, |next_settings| {
                    next_settings.ai_control = preferences.clone();
                })?;
                // Apply the native policy only after persistence succeeds,
                // keeping the runtime and the settings file consistent if an
                // atomic settings write is rejected.
                awake_manager.set_control_center_awake_policy(
                    preferences.autopilot.keep_awake_for_verified_sessions,
                    preferences.autopilot.keep_awake_ac_only,
                );
                let audit_store = {
                    let mut control = control_state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    control.last_snapshot = None;
                    control.audit.append(
                        unix_timestamp(),
                        "preferences",
                        "saved",
                        None,
                        "AI Control preferences updated",
                        retention,
                    );
                    control.audit.clone()
                };
                // Disk I/O outside the shared lock.
                audit_store.save(&config_dir)
            },
            "AI Control preference worker panicked",
        )
        .await?;
        self.runtime.notify_wake();
        Ok(())
    }

    /// Runs the safety audit and republishes it into the Control Center state.
    pub async fn run_safety_scan(&self, config_dir: &Path) -> Result<SafetySnapshot, String> {
        let control = self.control_state.clone();
        let environment = self.environment.clone();
        let settings = self.settings.snapshot()?;
        let preferences = settings.ai_control.clone();
        let inactivity_threshold_secs =
            u64::from(settings.agent_notifications.inactivity_threshold_minutes) * 60;
        let config_dir = config_dir.to_path_buf();
        // Shared activity collection; filesystem inspection stays outside the
        // shared Control Center lock.
        let registry = crate::ai_snapshots::fetch_activity_registry(
            &self.activity_cache,
            &self.activity_singleflight,
            &self.activity_generation,
            &self.runtime_metrics,
            &self.environment,
            inactivity_threshold_secs,
            false,
        )
        .await?;
        crate::blocking::run_blocking(
            move || {
                let now = unix_timestamp();
                let snapshot = crate::ai_control_center::safety::inspect(
                    &environment,
                    &registry.project_roots,
                    &preferences.dismissed_findings,
                    now,
                );
                let audit_store = {
                    let mut control = control
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    control.safety = snapshot.clone();
                    if let Some(last) = &mut control.last_snapshot {
                        last.safety = snapshot.clone();
                        last.quick_summary.safety_findings = snapshot
                            .findings
                            .iter()
                            .filter(|item| !item.dismissed)
                            .count()
                            as u32;
                        if snapshot.quality == crate::models::ObservationQuality::Partial {
                            last.quick_summary.quality = crate::models::ObservationQuality::Partial;
                        }
                    }
                    control.audit.append(
                        now,
                        "safety_scan",
                        "ok",
                        None,
                        &snapshot.status_message,
                        preferences.audit_retention_days,
                    );
                    control.audit.clone()
                };
                // Disk I/O outside the shared lock.
                let _ = audit_store.save(&config_dir);
                Ok(snapshot)
            },
            "AI safety scan worker panicked",
        )
        .await
    }

    /// Dismisses a finding and records the dismissal in the user's settings.
    pub async fn dismiss_safety_finding(
        &self,
        finding_id: &str,
        config_dir: &Path,
    ) -> Result<(), String> {
        let settings = self.settings.clone();
        let control_state = self.control_state.clone();
        let finding_id = finding_id.to_string();
        let config_dir = config_dir.to_path_buf();
        crate::blocking::run_blocking(
            move || {
                let (_, retention) = settings.update_and_persist(&config_dir, |next_settings| {
                    if !next_settings
                        .ai_control
                        .dismissed_findings
                        .contains(&finding_id)
                    {
                        next_settings
                            .ai_control
                            .dismissed_findings
                            .push(finding_id.clone());
                    }
                    next_settings.ai_control = crate::ai_control_center::budgets::sanitize(
                        next_settings.ai_control.clone(),
                    );
                    next_settings.ai_control.audit_retention_days
                })?;

                let mut control = control_state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(item) = control
                    .safety
                    .findings
                    .iter_mut()
                    .find(|item| item.id == finding_id)
                {
                    item.dismissed = true;
                }
                if let Some(last) = &mut control.last_snapshot {
                    if let Some(item) = last
                        .safety
                        .findings
                        .iter_mut()
                        .find(|item| item.id == finding_id)
                    {
                        item.dismissed = true;
                    }
                    last.quick_summary.safety_findings = last
                        .safety
                        .findings
                        .iter()
                        .filter(|item| !item.dismissed)
                        .count() as u32;
                }
                control.audit.append(
                    unix_timestamp(),
                    "finding_dismissed",
                    "ok",
                    None,
                    "Safety finding dismissed",
                    retention,
                );
                let audit_store = control.audit.clone();
                drop(control);
                // Disk I/O outside the shared lock.
                audit_store.save(&config_dir)
            },
            "Safety finding worker panicked",
        )
        .await
    }

    /// Mints a one-shot preview for a recommendation the UI is showing.
    pub async fn preview_recommendation(
        &self,
        recommendation_id: &str,
        config_dir: &Path,
    ) -> Result<RecommendationPreview, String> {
        let retention = self.settings.snapshot()?.ai_control.audit_retention_days;
        let control_state = self.control_state.clone();
        let recommendation_id = recommendation_id.to_string();
        let config_dir = config_dir.to_path_buf();
        crate::blocking::run_blocking(
            move || {
                let mut control = control_state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let item = control
                    .recommendations
                    .iter()
                    .find(|item| item.id == recommendation_id)
                    .cloned()
                    .ok_or_else(|| "Recommendation is stale or unavailable".to_string())?;
                let now = unix_timestamp();
                let preview = control.previews.create(&item, now);
                control.audit.append(
                    now,
                    "recommendation_preview",
                    "created",
                    item.project_id.clone(),
                    "One-shot recommendation preview created",
                    retention,
                );
                let audit_store = control.audit.clone();
                drop(control);
                // Disk I/O outside the shared lock.
                let _ = audit_store.save(&config_dir);
                Ok(preview)
            },
            "Recommendation preview worker panicked",
        )
        .await
    }

    /// Consumes a preview. The store enforces one-shot semantics and TTL.
    pub async fn consume_recommendation_preview(
        &self,
        preview_id: &str,
        config_dir: &Path,
    ) -> Result<RecommendationPreview, String> {
        let now = unix_timestamp();
        let retention = self.settings.snapshot()?.ai_control.audit_retention_days;
        let control_state = self.control_state.clone();
        let preview_id = preview_id.to_string();
        let config_dir = config_dir.to_path_buf();
        crate::blocking::run_blocking(
            move || {
                let mut control = control_state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let preview = control.previews.consume(&preview_id, now)?;
                control.audit.append(
                    now,
                    "recommendation_preview",
                    "consumed",
                    None,
                    "One-shot recommendation preview consumed",
                    retention,
                );
                let audit_store = control.audit.clone();
                drop(control);
                // Disk I/O outside the shared lock.
                let _ = audit_store.save(&config_dir);
                Ok(preview)
            },
            "Recommendation preview worker panicked",
        )
        .await
    }

    /// The explicit diff between the captured baseline and the working tree.
    pub async fn control_git_diff(
        &self,
        project_id: &str,
        config_dir: &Path,
    ) -> Result<String, String> {
        let root = self
            .activity_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .and_then(|value| value.project_roots.get(project_id))
            .cloned()
            .ok_or_else(|| "Project identity is stale or unavailable".to_string())?;
        let retention = self.settings.snapshot()?.ai_control.audit_retention_days;
        let control_state = self.control_state.clone();
        let project_id = project_id.to_string();
        let config_dir = config_dir.to_path_buf();
        crate::blocking::run_blocking(
            move || {
                // Snapshot the baseline under a short lock; Git capture and diffing
                // run outside the shared Control Center lock.
                let baseline = control_state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .git
                    .baseline_snapshot(&project_id)
                    .ok_or_else(|| "Git baseline is stale or unavailable".to_string())?;
                let now = unix_timestamp();
                let (baseline_head, paths) =
                    crate::ai_control_center::git::GitBaselineStore::diff_context_with_baseline(
                        &baseline, &root, now,
                    );
                let diff = crate::ai_control_center::git::explicit_diff(
                    &root,
                    baseline_head.as_deref(),
                    &paths,
                )?;
                let audit_store = {
                    let mut control = control_state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    control.audit.append(
                        unix_timestamp(),
                        "git_diff",
                        "viewed",
                        Some(project_id),
                        "Ephemeral Git diff viewed",
                        retention,
                    );
                    control.audit.clone()
                };
                // Disk I/O outside the shared lock.
                let _ = audit_store.save(&config_dir);
                Ok(diff)
            },
            "AI Control Center Git diff worker panicked",
        )
        .await
    }

    /// Runs the OpenRouter OAuth flow and persists the issued key.
    pub async fn connect_openrouter(&self) -> Result<(), String> {
        let credentials = self.credentials.clone();
        let store = credentials.clone();
        let key = crate::blocking::run_blocking(
            move || {
                // The credential store answers before the provider flow starts: an
                // OAuth key that cannot be persisted would have to be revoked by
                // hand.
                crate::ai_providers::authorize_openrouter(
                    store.as_ref(),
                    crate::ai_providers::connect_openrouter,
                )
            },
            "OpenRouter OAuth worker panicked",
        )
        .await?;
        let secret = crate::ai_providers::SecretString::new(key);
        if let Err(error) = credentials.set(crate::models::ProviderId::OpenRouter, secret) {
            // OpenRouter's documented key-deletion API requires a management
            // key, so an OAuth key cannot self-revoke. Say so instead of
            // pretending the issued key was invalidated.
            return Err(format!(
                "Could not persist the OpenRouter credential ({error}). OpenRouter issued a key that could not be stored; revoke it manually in the OpenRouter dashboard."
            ));
        }
        crate::ai_snapshots::invalidate_snapshot(&self.usage_cache, &self.usage_generation);
        Ok(())
    }

    /// Removes a provider credential stored by Zenith.
    pub async fn delete_provider_credential(
        &self,
        provider: crate::models::ProviderId,
    ) -> Result<(), String> {
        let credentials = self.credentials.clone();
        let usage_cache = self.usage_cache.clone();
        let usage_generation = self.usage_generation.clone();
        crate::blocking::run_blocking(
            move || {
                // Local disconnect only: the provider key is removed from
                // Zenith. OpenRouter manages this key with a management
                // credential that Zenith never holds, so the UI links to the
                // dashboard for manual deletion rather than claiming a remote
                // revocation.
                credentials
                    .remove(provider)
                    .map_err(|error| error.to_string())?;
                crate::ai_snapshots::invalidate_snapshot(&usage_cache, &usage_generation);
                Ok(())
            },
            "Provider disconnect worker panicked",
        )
        .await
    }
}

impl SettingsChangeReaction for AiService {
    fn before_save(
        &self,
        next: &ZenithSettings,
        notifications: &dyn DesktopNotifications,
    ) -> Result<(), String> {
        if next.agent_notifications.enabled {
            notifications.request_permission()?;
        }
        Ok(())
    }

    fn saved(&self, change: &SettingsChange) {
        if change.provider_selection_changed {
            crate::ai_snapshots::invalidate_snapshot(&self.usage_cache, &self.usage_generation);
            self.control().last_snapshot = None;
        }
        if change.inactivity_threshold_changed {
            crate::ai_snapshots::invalidate_snapshot(
                &self.activity_cache,
                &self.activity_generation,
            );
        }
        self.runtime.notify_wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_providers::InMemoryCredentialStore;
    use crate::operation_gate::StorageOperationGate;
    use zenith_platform::MockTrashBackend;

    struct TestSystemActions;

    impl zenith_platform::SystemActionProvider for TestSystemActions {
        fn reveal_path(&self, _path: &Path) -> Result<(), String> {
            Ok(())
        }

        fn open_folder(&self, _path: &Path) -> Result<(), String> {
            Ok(())
        }

        fn open_terminal(&self, _path: &Path) -> Result<(), String> {
            Ok(())
        }

        fn open_storage_settings(&self) -> Result<(), String> {
            Ok(())
        }
    }

    struct TestCapabilitiesProvider(crate::models::PlatformCapabilities);

    impl PlatformCapabilitiesProvider for TestCapabilitiesProvider {
        fn capabilities(&self) -> crate::models::PlatformCapabilities {
            self.0.clone()
        }
    }

    fn service_with_capabilities(capabilities: crate::models::PlatformCapabilities) -> AiService {
        let environment = Arc::new(PlatformEnvironment::native());
        let platform_capabilities: Arc<dyn PlatformCapabilitiesProvider> =
            Arc::new(TestCapabilitiesProvider(capabilities));
        let settings = Arc::new(SettingsAuthority::new(ZenithSettings::default()));
        let runtime_metrics = Arc::new(RuntimeMetrics::new());
        let budgets = Arc::new(ExecutionBudgets::new());
        let memory_sampler = Arc::new(MemorySampler::new());
        let dev_ports = Arc::new(Mutex::new(crate::dev_ports::DevelopmentPortStore::default()));
        let activity_cache = Arc::new(Mutex::new(None));
        let activity_singleflight = Arc::new(SingleFlight::with_metrics(runtime_metrics.clone()));
        let activity_generation = Arc::new(AtomicU64::new(1));
        let control_state = Arc::new(Mutex::new(AiControlCenterState::default()));
        let awake = Arc::new(KeepAwakeManager::new());
        let runtime = Arc::new(AiControlRuntime::new(
            memory_sampler.clone(),
            dev_ports.clone(),
            environment.clone(),
            activity_cache.clone(),
            activity_singleflight.clone(),
            activity_generation.clone(),
            runtime_metrics.clone(),
            control_state.clone(),
            awake.clone(),
            settings.clone(),
        ));
        let storage = Arc::new(StorageService::new(
            StorageOperationGate::default(),
            budgets.clone(),
            environment.clone(),
            Arc::new(crate::trash_manager::TrashExecutor::new(Arc::new(
                MockTrashBackend::default(),
            ))),
            Arc::new(TestSystemActions),
            platform_capabilities.clone(),
        ));

        AiService::new(
            environment,
            platform_capabilities,
            settings,
            Arc::new(InMemoryCredentialStore::new()),
            Arc::new(ProviderCollectionService::default()),
            Arc::new(Mutex::new(None)),
            Arc::new(SingleFlight::with_metrics(runtime_metrics.clone())),
            Arc::new(AtomicU64::new(1)),
            activity_cache,
            activity_singleflight,
            activity_generation,
            control_state,
            Arc::new(Mutex::new(())),
            runtime,
            runtime_metrics,
            budgets,
            memory_sampler,
            dev_ports,
            awake,
            storage,
        )
    }

    #[test]
    fn agent_integrations_require_service_capability_authorization() {
        let service = service_with_capabilities(crate::models::PlatformCapabilities::unsupported(
            crate::models::PlatformKind::Linux,
        ));

        let inspect = tauri::async_runtime::block_on(service.agent_integrations())
            .expect_err("inspection requires AI Integrations");
        let setup = tauri::async_runtime::block_on(service.setup_agent_integration("codex"))
            .expect_err("setup requires AI Integrations");
        let removal = tauri::async_runtime::block_on(service.remove_agent_integration("codex"))
            .expect_err("removal requires AI Integrations");

        for error in [inspect, setup, removal] {
            assert!(error.contains("AiIntegrations"), "{error}");
        }
    }
}
