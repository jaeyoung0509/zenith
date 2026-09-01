use crate::ai_usage::{connect_openrouter, AiUsageCollector};
use crate::cleaner::CleanExecutor;
use crate::docker::DockerAdapter;
use crate::metrics::{DiskMetricsCollector, MemoryInspector};
use crate::models::{
    AgentActivitySnapshot, AgentActivityStatus, AgentIntegrationInfo, AgentIntegrationResult,
    AgentQuickSessionRow, AgentQuickSummary, AiControlCenterSnapshot, AiControlPreferences,
    AiProviderUsage, AiUsageSnapshot, AwakeBehavior, AwakeRule, AwakeState, Category, CleanEvent,
    CleanResult, ControlCenterQuickSummary, DeletePlan, DevelopmentListener, DiagnosticsSnapshot,
    DiskMetrics, DiskVolume, DockerStatus, IngestedAgentEvent, LocalModelItem, MemoryMetrics,
    PlanPreview, PlatformCapabilities, RecommendationPreview, ReleaseDevelopmentListenerResult,
    ReleaseMode, ScanEvent, ScanResult, SelectedApplication, ZenithSettings,
};
use crate::models_inventory::{LocalModelManager, LocalModelScanner};
use crate::operation_gate::StorageOperationGate;
use crate::power::{ApplicationPicker, KeepAwakeManager};
use crate::safety::SafetyPlanner;
use crate::scanner::ScanEngine;
use crate::settings_store;
use crate::signatures::SignatureRegistry;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

pub struct AppState {
    pub registry: Arc<SignatureRegistry>,
    pub awake_manager: Arc<KeepAwakeManager>,
    pub settings: Arc<Mutex<ZenithSettings>>,
    pub last_scan: Arc<Mutex<Option<ScanResult>>>,
    pub openrouter_key: Arc<Mutex<Option<String>>>,
    pub ai_usage_cache: Arc<Mutex<Option<AiUsageSnapshot>>>,
    pub ai_usage_refresh_lock: Arc<Mutex<()>>,
    pub delete_plans: Arc<Mutex<HashMap<uuid::Uuid, DeletePlan>>>,
    pub storage_operation_gate: StorageOperationGate,
    pub storage_state: Arc<crate::storage_commands::StorageWorkflowState>,
    pub memory_sampler: Arc<crate::metrics::MemorySampler>,
    pub dev_port_store: Arc<Mutex<crate::dev_ports::DevelopmentPortStore>>,
    pub agent_activity_cache: Arc<Mutex<Option<crate::agent_activity::AgentActivityRegistry>>>,
    pub ai_control_state: Arc<Mutex<crate::ai_control_center::state::AiControlCenterState>>,
    pub ai_control_refresh_lock: Arc<Mutex<()>>,
    pub ai_control_runtime: Arc<crate::ai_control_center::runtime::AiControlRuntime>,
    pub platform_capabilities: Arc<dyn crate::platform::PlatformCapabilitiesProvider>,
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Run native or filesystem work away from Tauri's command executor.
///
/// Keeping the join/error handling in one place makes it harder for a new
/// command to accidentally put blocking work back on the command thread.
async fn run_blocking<T, F>(work: F, context: &'static str) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| format!("{context}: {error}"))?
}

fn user_home() -> Result<PathBuf, String> {
    crate::platform::NativePlatformPaths::new()
        .home()
        .ok_or_else(|| "User home directory is not available".to_string())
}

fn usage_snapshot_matches_selection(snapshot: &AiUsageSnapshot, provider_ids: &[String]) -> bool {
    snapshot.providers.len() == provider_ids.len()
        && snapshot
            .providers
            .iter()
            .zip(provider_ids)
            .all(|(provider, selected_id)| provider.id == *selected_id)
}

#[tauri::command]
#[specta::specta]
pub async fn get_ai_usage(
    on_event: Channel<AiProviderUsage>,
    force: Option<bool>,
    state: State<'_, AppState>,
) -> Result<AiUsageSnapshot, String> {
    const CACHE_TTL_SECS: u64 = 60;
    let provider_ids = state
        .settings
        .lock()
        .expect("settings poisoned")
        .ai_accounts_quota_providers
        .clone();
    if !force.unwrap_or(false) {
        if let Some(snapshot) = state
            .ai_usage_cache
            .lock()
            .expect("ai_usage_cache poisoned")
            .as_ref()
        {
            if snapshot.is_fresh_at(unix_timestamp(), CACHE_TTL_SECS)
                && usage_snapshot_matches_selection(snapshot, &provider_ids)
            {
                for p in &snapshot.providers {
                    let _ = on_event.send(p.clone());
                }
                return Ok(snapshot.clone());
            }
        }
    }

    let openrouter_key = state
        .openrouter_key
        .lock()
        .expect("openrouter_key poisoned")
        .clone();
    let cache = state.ai_usage_cache.clone();
    let refresh_lock = state.ai_usage_refresh_lock.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _refresh_guard = refresh_lock.lock().expect("ai_usage_refresh_lock poisoned");
        if !force.unwrap_or(false) {
            if let Some(snapshot) = cache.lock().expect("ai_usage_cache poisoned").as_ref() {
                if snapshot.is_fresh_at(unix_timestamp(), CACHE_TTL_SECS)
                    && usage_snapshot_matches_selection(snapshot, &provider_ids)
                {
                    for p in &snapshot.providers {
                        let _ = on_event.send(p.clone());
                    }
                    return snapshot.clone();
                }
            }
        }
        let snapshot =
            AiUsageCollector::collect_parallel(openrouter_key, &provider_ids, |provider| {
                let _ = on_event.send(provider);
            });
        *cache.lock().expect("ai_usage_cache poisoned") = Some(snapshot.clone());
        snapshot
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_project_context(
    force: Option<bool>,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<AgentActivitySnapshot, String> {
    let should_force = force.unwrap_or(false);
    if !should_force {
        if let Some(snapshot) = state
            .agent_activity_cache
            .lock()
            .expect("agent_activity_cache poisoned")
            .as_ref()
        {
            if unix_timestamp().saturating_sub(snapshot.snapshot.observed_at)
                < crate::agent_activity::SNAPSHOT_TTL_SECONDS
            {
                return Ok(snapshot.snapshot.clone());
            }
        }
    }

    let cache = state.agent_activity_cache.clone();
    let dev_store = state.dev_port_store.clone();
    let storage_state = state.storage_state.clone();
    let notification_preferences = state
        .settings
        .lock()
        .expect("settings poisoned")
        .agent_notifications
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut registry = crate::agent_activity::collect_registry_with_inactivity_threshold(
            u64::from(notification_preferences.inactivity_threshold_minutes) * 60,
        );
        let listeners = crate::dev_ports::list_listeners(
            &dev_store,
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
                            if canon.starts_with(root)
                                && !project.dev_ports.contains(&listener.port)
                            {
                                project.dev_ports.push(listener.port);
                            }
                        }
                    }
                }
                project.dev_ports.sort_unstable();
            }
        }
        let snapshot = registry.snapshot.clone();
        {
            let store = crate::agent_activity::global_store();
            let mut guard = store.lock().expect("agent activity store poisoned");
            let _ = crate::agent_activity::notifications::emit_process_advisories(
                &app_handle,
                &snapshot,
                &notification_preferences,
                &mut guard.notification_filter,
            );
        }
        *cache.lock().expect("agent_activity_cache poisoned") = Some(registry);
        snapshot
    })
    .await
    .map_err(|error| format!("Agent activity refresh failed: {error}"))
}

#[tauri::command]
#[specta::specta]
pub async fn request_stop_agent_session(
    session_id: String,
    lease_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let now = unix_timestamp();
    let lease = {
        let store = crate::agent_activity::global_store();
        let mut guard = store.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .stop_leases
            .consume_lease(&session_id, &lease_id, now)?
    };

    let cache = state.agent_activity_cache.clone();
    let runtime = state.ai_control_runtime.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let system = crate::agent_activity::termination::RealTerminationSystem;
        let result = crate::agent_activity::termination::execute_graceful_stop(&lease, &system);
        if result.is_ok() {
            if let Ok(mut cache_guard) = cache.lock() {
                *cache_guard = None;
            }
            runtime.notify_wake();
        }
        result
    })
    .await
    .map_err(|e| format!("Graceful stop failed: {e}"))?
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_integrations() -> Result<Vec<AgentIntegrationInfo>, String> {
    run_blocking(
        || {
            let home = user_home()?;
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
            let mut infos = Vec::new();
            for tool in TOOLS {
                infos.push(crate::agent_activity::hooks::get_integration_info(
                    tool, &home,
                ));
            }
            Ok(infos)
        },
        "Agent integration worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn setup_agent_integration(tool_id: String) -> Result<AgentIntegrationResult, String> {
    run_blocking(
        move || {
            let home = user_home()?;
            crate::agent_activity::hooks::install_integration(&tool_id, &home)
        },
        "Agent integration setup worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn remove_agent_integration(tool_id: String) -> Result<AgentIntegrationResult, String> {
    run_blocking(
        move || {
            let home = user_home()?;
            crate::agent_activity::hooks::uninstall_integration(&tool_id, &home)
        },
        "Agent integration removal worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_quick_summary(
    state: State<'_, AppState>,
) -> Result<Option<AgentQuickSummary>, String> {
    let cached = state
        .agent_activity_cache
        .lock()
        .map_err(|_| "Agent activity cache is unavailable.".to_string())?
        .clone();
    let registry = if let Some(registry) = cached {
        registry
    } else {
        let cache = state.agent_activity_cache.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let fresh = crate::agent_activity::collect_registry();
            *cache
                .lock()
                .map_err(|_| "Agent activity cache is unavailable.".to_string())? =
                Some(fresh.clone());
            Ok::<_, String>(fresh)
        })
        .await
        .map_err(|error| format!("Agent activity refresh failed: {error}"))??
    };
    let mut active_count = 0;
    let mut attention_count = 0;
    let mut rows = Vec::new();

    for project in &registry.snapshot.projects {
        for session in &project.sessions {
            if matches!(
                session.status,
                AgentActivityStatus::Working
                    | AgentActivityStatus::Active
                    | AgentActivityStatus::Starting
            ) {
                active_count += 1;
            }
            if session.attention_reason.is_some() {
                attention_count += 1;
            }
            rows.push(AgentQuickSessionRow {
                session_id: session.id.clone(),
                tool_name: session.tool_name.clone(),
                project_name: project.identity.display_name.clone(),
                status: session.status,
                evidence: session.evidence,
                elapsed_seconds: session.elapsed_seconds,
            });
        }
    }
    for session in &registry.snapshot.unassigned_sessions {
        if matches!(
            session.status,
            AgentActivityStatus::Working
                | AgentActivityStatus::Active
                | AgentActivityStatus::Starting
        ) {
            active_count += 1;
        }
        if session.attention_reason.is_some() {
            attention_count += 1;
        }
        rows.push(AgentQuickSessionRow {
            session_id: session.id.clone(),
            tool_name: session.tool_name.clone(),
            project_name: "Unassigned".to_string(),
            status: session.status,
            evidence: session.evidence,
            elapsed_seconds: session.elapsed_seconds,
        });
    }

    rows.sort_by_key(|b| std::cmp::Reverse(b.elapsed_seconds));
    rows.truncate(3);

    Ok(Some(AgentQuickSummary {
        active_count,
        attention_count,
        sessions: rows,
    }))
}

#[tauri::command]
#[specta::specta]
pub fn post_agent_event(
    event: IngestedAgentEvent,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let event = crate::agent_activity::events::validate_ingested_event(event, unix_timestamp())?;
    let store = crate::agent_activity::global_store();
    let mut guard = store.lock().unwrap();
    guard.record_event(event);
    if let Ok(mut cache_guard) = state.agent_activity_cache.lock() {
        *cache_guard = None;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_ai_control_center(
    force: Option<bool>,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<AiControlCenterSnapshot, String> {
    if !force.unwrap_or(false) {
        if let Some(snapshot) = state
            .ai_control_state
            .lock()
            .expect("ai control poisoned")
            .last_snapshot
            .as_ref()
        {
            if unix_timestamp().saturating_sub(snapshot.observed_at) < 10 {
                return Ok(snapshot.clone());
            }
        }
    }
    let refresh_lock = state.ai_control_refresh_lock.clone();
    let control = state.ai_control_state.clone();
    let activity_cache = state.agent_activity_cache.clone();
    let usage_cache = state.ai_usage_cache.clone();
    let openrouter_key = state
        .openrouter_key
        .lock()
        .expect("openrouter key poisoned")
        .clone();
    let memory_sampler = state.memory_sampler.clone();
    let awake = state.awake_manager.clone();
    let dev_store = state.dev_port_store.clone();
    let (preferences, provider_ids) = {
        let settings = state.settings.lock().expect("settings poisoned");
        (
            settings.ai_control.clone(),
            settings.ai_accounts_quota_providers.clone(),
        )
    };
    let config_dir = app_handle
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = refresh_lock.lock().expect("ai control refresh poisoned");
        let now = unix_timestamp();
        let activity = {
            let cached = activity_cache
                .lock()
                .expect("activity cache poisoned")
                .clone();
            if let Some(value) = cached.filter(|value| {
                now.saturating_sub(value.snapshot.observed_at)
                    < crate::agent_activity::SNAPSHOT_TTL_SECONDS
            }) {
                value
            } else {
                let value = crate::agent_activity::collect_registry();
                *activity_cache.lock().expect("activity cache poisoned") = Some(value.clone());
                value
            }
        };
        let usage = {
            let cached = usage_cache.lock().expect("usage cache poisoned").clone();
            if let Some(value) = cached.filter(|value| {
                value.is_fresh_at(now, 60) && usage_snapshot_matches_selection(value, &provider_ids)
            }) {
                value
            } else {
                let value = AiUsageCollector::collect(openrouter_key, &provider_ids);
                *usage_cache.lock().expect("usage cache poisoned") = Some(value.clone());
                value
            }
        };
        let memory = memory_sampler.sample();
        let awake_state = awake.get_state();
        let listeners = crate::dev_ports::list_listeners(
            &dev_store,
            &crate::dev_ports::RealDevPortSystem::default(),
        )
        .unwrap_or_default();
        let resources = crate::ai_control_center::resources::attribute(
            &activity.snapshot,
            &activity.project_roots,
            &listeners,
            awake_state.power_source,
            preferences.autopilot.keep_awake_ac_only,
        );
        let mut control = control.lock().expect("ai control poisoned");
        let providers = crate::ai_control_center::providers::normalize(&usage, &preferences);
        let providers = crate::ai_control_center::providers::retain_last_success(
            providers,
            &mut control.providers_last_success,
        );
        let budget_statuses =
            crate::ai_control_center::budgets::statuses(&preferences.budgets, &providers);
        let new_items = control.policy.evaluate(
            &resources,
            Some(memory.pressure),
            awake_state.power_source,
            &preferences.autopilot,
            now,
        );
        if !new_items.is_empty() {
            control.recommendations.extend(new_items);
            control
                .recommendations
                .sort_by_key(|item| std::cmp::Reverse(item.created_at));
            control.recommendations.truncate(64);
        }
        let recommendations = control.recommendations.clone();
        let git_summaries = control.git.summaries(&activity.project_roots, now);
        let safety = control.safety.clone();
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
        control.audit.append(
            now,
            "refresh",
            "ok",
            None,
            "AI Control Center local snapshot refreshed",
            preferences.audit_retention_days,
        );
        let _ = control.audit.save(&config_dir);
        let audit = control.audit.entries();
        let snapshot = AiControlCenterSnapshot {
            observed_at: now,
            providers,
            budget_statuses,
            resources,
            recommendations,
            safety,
            git_summaries,
            audit,
            quick_summary,
            keep_awake_active: awake.get_state().active_rule_id.as_deref()
                == Some("ai-control.verified-session"),
            partial_errors,
        };
        control.last_snapshot = Some(snapshot.clone());
        Ok(snapshot)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
#[specta::specta]
pub fn get_ai_control_quick_summary(
    state: State<'_, AppState>,
) -> Option<ControlCenterQuickSummary> {
    state
        .ai_control_state
        .lock()
        .expect("ai control poisoned")
        .last_snapshot
        .as_ref()
        .map(|value| value.quick_summary.clone())
}

#[tauri::command]
#[specta::specta]
pub async fn save_ai_control_preferences(
    preferences: AiControlPreferences,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let preferences = crate::ai_control_center::budgets::sanitize(preferences);
    let retention = preferences.audit_retention_days;
    let settings_store_state = state.settings.clone();
    let awake_manager = state.awake_manager.clone();
    let control_state = state.ai_control_state.clone();
    run_blocking(
        move || {
            if preferences.autopilot.notify_on_battery
                || preferences.autopilot.notify_on_memory_pressure
                || preferences.autopilot.notify_on_session_completion
            {
                crate::ai_control_center::notifications::request_permission_if_needed(&app_handle)?;
            }
            let mut next_settings = settings_store_state
                .lock()
                .expect("settings poisoned")
                .clone();
            next_settings.ai_control = preferences.clone();
            let config = app_handle
                .path()
                .app_config_dir()
                .map_err(|error| error.to_string())?;
            settings_store::save(&config, &next_settings)?;
            *settings_store_state.lock().expect("settings poisoned") = next_settings;
            // Apply the native policy only after persistence succeeds, keeping the runtime
            // and the settings file consistent if an atomic settings write is rejected.
            awake_manager.set_control_center_awake_policy(
                preferences.autopilot.keep_awake_for_verified_sessions,
                preferences.autopilot.keep_awake_ac_only,
            );
            let mut control = control_state.lock().expect("ai control poisoned");
            control.last_snapshot = None;
            control.audit.append(
                unix_timestamp(),
                "preferences",
                "saved",
                None,
                "AI Control preferences updated",
                retention,
            );
            control.audit.save(&config)
        },
        "AI Control preference worker panicked",
    )
    .await?;
    state.ai_control_runtime.notify_wake();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn run_ai_safety_scan(
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::models::SafetySnapshot, String> {
    let cache = state.agent_activity_cache.clone();
    let control = state.ai_control_state.clone();
    let preferences = state
        .settings
        .lock()
        .expect("settings poisoned")
        .ai_control
        .clone();
    let config = app_handle
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let now = unix_timestamp();
        let registry = {
            let cached = cache.lock().expect("activity cache poisoned").clone();
            if let Some(value) = cached.filter(|value| {
                now.saturating_sub(value.snapshot.observed_at)
                    < crate::agent_activity::SNAPSHOT_TTL_SECONDS
            }) {
                value
            } else {
                let value = crate::agent_activity::collect_registry();
                *cache.lock().expect("activity cache poisoned") = Some(value.clone());
                value
            }
        };
        let snapshot = crate::ai_control_center::safety::inspect(
            &registry.project_roots,
            &preferences.dismissed_findings,
            now,
        );
        let mut control = control.lock().expect("ai control poisoned");
        control.safety = snapshot.clone();
        if let Some(last) = &mut control.last_snapshot {
            last.safety = snapshot.clone();
            last.quick_summary.safety_findings = snapshot
                .findings
                .iter()
                .filter(|item| !item.dismissed)
                .count() as u32;
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
        let _ = control.audit.save(&config);
        snapshot
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn dismiss_ai_safety_finding(
    finding_id: String,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let settings_store_state = state.settings.clone();
    let control_state = state.ai_control_state.clone();
    run_blocking(
        move || {
            let mut settings = settings_store_state
                .lock()
                .expect("settings poisoned")
                .clone();
            if !settings.ai_control.dismissed_findings.contains(&finding_id) {
                settings
                    .ai_control
                    .dismissed_findings
                    .push(finding_id.clone());
            }
            settings.ai_control =
                crate::ai_control_center::budgets::sanitize(settings.ai_control.clone());
            let config = app_handle
                .path()
                .app_config_dir()
                .map_err(|error| error.to_string())?;
            settings_store::save(&config, &settings)?;
            let retention = settings.ai_control.audit_retention_days;
            *settings_store_state.lock().expect("settings poisoned") = settings;

            let mut control = control_state.lock().expect("ai control poisoned");
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
            control.audit.save(&config)
        },
        "Safety finding worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn preview_ai_recommendation(
    recommendation_id: String,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<RecommendationPreview, String> {
    let retention = state
        .settings
        .lock()
        .expect("settings poisoned")
        .ai_control
        .audit_retention_days;
    let control_state = state.ai_control_state.clone();
    run_blocking(
        move || {
            let mut control = control_state.lock().expect("ai control poisoned");
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
            let config = app_handle
                .path()
                .app_config_dir()
                .map_err(|error| error.to_string())?;
            let _ = control.audit.save(&config);
            Ok(preview)
        },
        "Recommendation preview worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn consume_ai_recommendation_preview(
    preview_id: String,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<RecommendationPreview, String> {
    let now = unix_timestamp();
    let retention = state
        .settings
        .lock()
        .expect("settings poisoned")
        .ai_control
        .audit_retention_days;
    let control_state = state.ai_control_state.clone();
    run_blocking(
        move || {
            let mut control = control_state.lock().expect("ai control poisoned");
            let preview = control.previews.consume(&preview_id, now)?;
            control.audit.append(
                now,
                "recommendation_preview",
                "consumed",
                None,
                "One-shot recommendation preview consumed",
                retention,
            );
            let config = app_handle
                .path()
                .app_config_dir()
                .map_err(|error| error.to_string())?;
            let _ = control.audit.save(&config);
            Ok(preview)
        },
        "Recommendation preview worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn get_ai_control_git_diff(
    project_id: String,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let root = state
        .agent_activity_cache
        .lock()
        .expect("activity cache poisoned")
        .as_ref()
        .and_then(|value| value.project_roots.get(&project_id))
        .cloned()
        .ok_or_else(|| "Project identity is stale or unavailable".to_string())?;
    let retention = state
        .settings
        .lock()
        .expect("settings poisoned")
        .ai_control
        .audit_retention_days;
    let control_state = state.ai_control_state.clone();
    let diff = tauri::async_runtime::spawn_blocking(move || {
        let (baseline_head, paths) = control_state
            .lock()
            .expect("ai control poisoned")
            .git
            .diff_context(&project_id, &root, unix_timestamp())?;
        let diff =
            crate::ai_control_center::git::explicit_diff(&root, baseline_head.as_deref(), &paths)?;
        let config = app_handle
            .path()
            .app_config_dir()
            .map_err(|error| error.to_string())?;
        let mut control = control_state.lock().expect("ai control poisoned");
        control.audit.append(
            unix_timestamp(),
            "git_diff",
            "viewed",
            Some(project_id),
            "Ephemeral Git diff viewed",
            retention,
        );
        let _ = control.audit.save(&config);
        Ok::<_, String>(diff)
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(diff)
}

#[tauri::command]
#[specta::specta]
pub async fn connect_openrouter_oauth(state: State<'_, AppState>) -> Result<(), String> {
    let openrouter_key = state.openrouter_key.clone();
    let key = tauri::async_runtime::spawn_blocking(connect_openrouter)
        .await
        .map_err(|error| error.to_string())??;
    *openrouter_key.lock().expect("openrouter_key poisoned") = Some(key);
    *state
        .ai_usage_cache
        .lock()
        .expect("ai_usage_cache poisoned") = None;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn start_scan(
    on_event: Channel<ScanEvent>,
    categories: Option<Vec<Category>>,
    state: State<'_, AppState>,
) -> Result<ScanResult, String> {
    let registry = state.registry.clone();
    let last_scan_store = state.last_scan.clone();
    let operation_gate = state.storage_operation_gate.clone();
    let (excluded_signatures, intensive_cleanup) = {
        let settings = state.settings.lock().expect("settings poisoned");
        (
            settings.excluded_signatures.clone(),
            settings.intensive_cleanup,
        )
    };

    let result = tauri::async_runtime::spawn_blocking(move || {
        operation_gate.run(|| {
            let cat_ref = categories.as_deref();
            let result = ScanEngine::scan(
                &registry,
                cat_ref,
                &excluded_signatures,
                intensive_cleanup,
                |event| {
                    let _ = on_event.send(event);
                },
            );
            *last_scan_store.lock().expect("last_scan poisoned") = Some(result.clone());
            result
        })
    })
    .await
    .map_err(|_| "Scan worker thread panicked".to_string())?;

    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub fn get_last_scan(state: State<'_, AppState>) -> Option<ScanResult> {
    state.last_scan.lock().expect("mutex poisoned").clone()
}

#[tauri::command]
#[specta::specta]
pub async fn create_delete_plan(
    scan_id: String,
    selected_item_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<PlanPreview, String> {
    const PLAN_TTL_SECS: u64 = 300;
    let last_scan = state.last_scan.clone();
    let registry = state.registry.clone();
    let delete_plans = state.delete_plans.clone();
    run_blocking(
        move || {
            let scan = last_scan
                .lock()
                .expect("last_scan poisoned")
                .clone()
                .filter(|scan| scan.scan_id == scan_id)
                .ok_or_else(|| {
                    "The scan is no longer current. Scan again before cleaning.".to_string()
                })?;

            let plan = SafetyPlanner::create_plan_from_scan(
                &scan,
                &scan_id,
                &selected_item_ids,
                &registry,
            )
            .map_err(|error| error.to_string())?;
            let preview = plan.preview(PLAN_TTL_SECS);
            let now = unix_timestamp();
            let mut plans = delete_plans.lock().expect("delete_plans poisoned");
            plans.retain(|_, stored| now.saturating_sub(stored.created_at) < PLAN_TTL_SECS);
            if plans.len() >= 64 {
                if let Some(oldest_id) = plans
                    .iter()
                    .min_by_key(|(_, stored)| stored.created_at)
                    .map(|(id, _)| *id)
                {
                    plans.remove(&oldest_id);
                }
            }
            plans.insert(plan.id, plan);
            Ok(preview)
        },
        "Delete plan worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn execute_clean(
    plan_id: uuid::Uuid,
    on_event: Channel<CleanEvent>,
    state: State<'_, AppState>,
) -> Result<CleanResult, String> {
    const PLAN_TTL_SECS: u64 = 300;
    let operation_gate = state.storage_operation_gate.clone();
    let plans = state.delete_plans.clone();
    let last_scan = state.last_scan.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<CleanResult, String> {
        operation_gate.run(|| {
            let plan = plans
                .lock()
                .expect("delete_plans poisoned")
                .remove(&plan_id)
                .ok_or_else(|| "Delete plan not found or already used".to_string())?;
            if unix_timestamp().saturating_sub(plan.created_at) >= PLAN_TTL_SECS {
                return Err("Delete plan expired. Scan again before cleaning.".to_string());
            }
            let scan_is_current = last_scan
                .lock()
                .expect("last_scan poisoned")
                .as_ref()
                .is_some_and(|scan| scan.scan_id == plan.scan_id);
            if !scan_is_current {
                return Err(
                    "The scan changed after this plan was created. Review a new plan.".to_string(),
                );
            }
            Ok(CleanExecutor::execute(plan, |event| {
                let _ = on_event.send(event);
            }))
        })
    })
    .await
    .map_err(|_| "Clean execution thread panicked".to_string())??;

    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub async fn get_memory_metrics(state: State<'_, AppState>) -> Result<MemoryMetrics, String> {
    let sampler = state.memory_sampler.clone();
    tauri::async_runtime::spawn_blocking(move || sampler.sample())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn terminate_process_group(name: String, force: bool) -> Result<usize, String> {
    run_blocking(
        move || MemoryInspector::terminate_group(&name, force),
        "Process termination worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn pick_keep_awake_application() -> Result<Option<SelectedApplication>, String> {
    tauri::async_runtime::spawn_blocking(ApplicationPicker::pick)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn get_disk_metrics() -> Result<DiskMetrics, String> {
    run_blocking(
        || DiskMetricsCollector::get_primary_disk().map_err(|error| error.to_string()),
        "Disk metrics worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn get_disk_volumes() -> Result<Vec<DiskVolume>, String> {
    run_blocking(
        || Ok(DiskMetricsCollector::get_volumes()),
        "Disk volume worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn open_disk_utility() -> Result<(), String> {
    run_blocking(
        || {
            use crate::platform::SystemActionProvider;
            crate::platform::NativeSystemActions::new().open_storage_settings()
        },
        "Storage settings worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn get_docker_status() -> Result<DockerStatus, String> {
    run_blocking(
        || Ok(DockerAdapter::get_status()),
        "Docker status worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn prune_docker_target(signature_id: String) -> Result<u64, String> {
    run_blocking(
        move || DockerAdapter::prune_category(&signature_id).map_err(|error| error.to_string()),
        "Docker cleanup worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn get_local_models() -> Result<Vec<LocalModelItem>, String> {
    run_blocking(
        || Ok(LocalModelScanner::scan_all_models()),
        "Local model scan worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_local_model(model_id: String) -> Result<u64, String> {
    run_blocking(
        move || LocalModelManager::delete_by_id(&model_id).map_err(|error| error.to_string()),
        "Local model deletion worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub fn get_awake_state(state: State<'_, AppState>) -> Result<AwakeState, String> {
    Ok(state.awake_manager.get_state())
}

#[tauri::command]
#[specta::specta]
pub async fn set_awake_rules(
    rules: Vec<AwakeRule>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let awake_manager = state.awake_manager.clone();
    run_blocking(
        move || {
            awake_manager.set_rules(rules);
            Ok(())
        },
        "Keep Awake rule worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn set_manual_awake(
    duration_secs: Option<u64>,
    behavior: AwakeBehavior,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let awake_manager = state.awake_manager.clone();
    run_blocking(
        move || {
            awake_manager
                .set_manual(duration_secs, behavior)
                .map_err(|error| error.to_string())
        },
        "Manual Keep Awake worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn disable_manual_awake(state: State<'_, AppState>) -> Result<(), String> {
    let awake_manager = state.awake_manager.clone();
    run_blocking(
        move || {
            awake_manager.disable_manual();
            Ok(())
        },
        "Manual Keep Awake worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub fn get_settings(state: State<'_, AppState>) -> Result<ZenithSettings, String> {
    let s = state.settings.lock().expect("settings poisoned");
    Ok(s.clone())
}

#[tauri::command]
#[specta::specta]
pub async fn save_settings(
    settings: ZenithSettings,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let settings = settings.sanitize();
    let provider_selection_changed = state
        .settings
        .lock()
        .expect("settings poisoned")
        .ai_accounts_quota_providers
        != settings.ai_accounts_quota_providers;
    let awake_manager = state.awake_manager.clone();
    let settings_store_state = state.settings.clone();
    let ai_usage_cache = state.ai_usage_cache.clone();
    let ai_control_state = state.ai_control_state.clone();

    run_blocking(
        move || {
            if settings.agent_notifications.enabled {
                crate::ai_control_center::notifications::request_permission_if_needed(&app_handle)?;
            }
            let config_dir = app_handle
                .path()
                .app_config_dir()
                .map_err(|error| error.to_string())?;
            settings_store::save(&config_dir, &settings)?;
            awake_manager.set_rules(settings.awake_rules.clone());
            *settings_store_state.lock().expect("settings poisoned") = settings;
            if provider_selection_changed {
                *ai_usage_cache.lock().expect("ai_usage_cache poisoned") = None;
                ai_control_state
                    .lock()
                    .expect("ai control poisoned")
                    .last_snapshot = None;
            }
            Ok(())
        },
        "Settings save worker panicked",
    )
    .await?;
    state.ai_control_runtime.notify_wake();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn reveal_in_finder(path: String) -> Result<(), String> {
    run_blocking(
        move || {
            use crate::platform::SystemActionProvider;
            let path_buf = expand_display_path(&path)?;
            crate::platform::NativeSystemActions::new().reveal_path(&path_buf)
        },
        "File manager worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn open_in_terminal(path: String) -> Result<(), String> {
    run_blocking(
        move || {
            use crate::platform::SystemActionProvider;
            let path_buf = expand_display_path(&path)?;
            crate::platform::NativeSystemActions::new().open_terminal(&path_buf)
        },
        "Terminal worker panicked",
    )
    .await
}

fn expand_display_path(path: &str) -> Result<PathBuf, String> {
    let expanded = if let Some(relative) = path.strip_prefix("~/") {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .ok_or_else(|| "Home environment variable is not set.".to_string())?;
        home.join(relative)
    } else {
        PathBuf::from(path)
    };
    expanded
        .canonicalize()
        .map_err(|error| format!("Path is no longer available: {error}"))
}

#[tauri::command]
#[specta::specta]
pub fn open_dashboard_window(app_handle: AppHandle) -> Result<(), String> {
    crate::show_main_window(&app_handle).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
#[specta::specta]
pub fn get_platform_capabilities(state: State<'_, AppState>) -> PlatformCapabilities {
    state.platform_capabilities.capabilities()
}

#[tauri::command]
#[specta::specta]
pub fn toggle_quick_panel(app_handle: AppHandle) -> Result<(), String> {
    if let Ok(window) = crate::ensure_window(&app_handle, "quick") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_diagnostics(
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<DiagnosticsSnapshot, String> {
    let settings = state.settings.clone();
    run_blocking(
        move || {
            let settings = settings.lock().expect("settings poisoned").clone();
            let config_dir = app_handle
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            Ok(crate::diagnostics::get_snapshot(&settings, &config_dir))
        },
        "Diagnostics worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn open_logs_folder() -> Result<(), String> {
    run_blocking(
        crate::diagnostics::open_logs_folder,
        "Logs folder worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn list_development_listeners(
    state: State<'_, AppState>,
) -> Result<Vec<DevelopmentListener>, String> {
    let store = state.dev_port_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::dev_ports::list_listeners(&store, &crate::dev_ports::RealDevPortSystem::default())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn release_development_listener(
    id: String,
    mode: ReleaseMode,
    state: State<'_, AppState>,
) -> Result<ReleaseDevelopmentListenerResult, String> {
    let store = state.dev_port_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::dev_ports::release_listener(
            &store,
            &crate::dev_ports::RealDevPortSystem::default(),
            &id,
            mode,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_version_is_semver_formatted() {
        let version = get_app_version();
        assert!(!version.is_empty());
        let parts: Vec<&str> = version.split('.').collect();
        assert_eq!(parts.len(), 3, "Expected major.minor.patch semver format");
        for part in parts {
            assert!(
                part.chars().all(|c| c.is_ascii_digit()),
                "Expected numeric version segments"
            );
        }
    }
}
