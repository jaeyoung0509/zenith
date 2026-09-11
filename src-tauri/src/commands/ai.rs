//! AI usage, agent activity, and AI Control Center command handlers.

use super::state::AppState;
use super::support::{lock_or_state_error, run_blocking, unix_timestamp, user_home};
use crate::ai_providers::connect_openrouter;
use crate::ai_snapshots::{enrich_activity_for_project_view, fetch_activity_registry};
use crate::models::{
    AgentActivitySnapshot, AgentActivityStatus, AgentIntegrationInfo, AgentIntegrationResult,
    AgentQuickSessionRow, AgentQuickSummary, AiControlCenterSnapshot, AiControlPreferences,
    AiProviderUsage, AiUsageSnapshot, ControlCenterQuickSummary, IngestedAgentEvent,
    RecommendationPreview,
};
use crate::settings_store;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
#[specta::specta]
pub async fn get_ai_usage(
    on_event: Channel<AiProviderUsage>,
    force: Option<bool>,
    state: State<'_, AppState>,
) -> Result<AiUsageSnapshot, String> {
    let provider_ids = state
        .settings
        .lock()
        .expect("settings poisoned")
        .ai_accounts_quota_providers
        .clone();
    crate::ai_snapshots::fetch_usage_snapshot(
        &state.ai_usage_cache,
        &state.usage_singleflight,
        &state.usage_generation,
        &state.runtime_metrics,
        state.ai_collection_service.clone(),
        state.credentials.clone(),
        provider_ids,
        force.unwrap_or(false),
        move |provider| {
            let _ = on_event.send(provider);
        },
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn get_project_context(
    force: Option<bool>,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<AgentActivitySnapshot, String> {
    let notification_preferences = state
        .settings
        .lock()
        .expect("settings poisoned")
        .agent_notifications
        .clone();
    let threshold_secs = u64::from(notification_preferences.inactivity_threshold_minutes) * 60;
    // Shared raw registry; enrichment stays explicit so a raw refresh never
    // replaces the enriched snapshot required by this view.
    let raw = fetch_activity_registry(
        &state.agent_activity_cache,
        &state.activity_singleflight,
        &state.activity_generation,
        &state.runtime_metrics,
        threshold_secs,
        force.unwrap_or(false),
    )
    .await?;
    let dev_store = state.dev_port_store.clone();
    let storage_state = state.storage_state.clone();
    let enriched = tauri::async_runtime::spawn_blocking(move || {
        let enriched_registry = enrich_activity_for_project_view(raw, &dev_store, &storage_state);
        let snapshot = enriched_registry.snapshot.clone();
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
        snapshot
    })
    .await
    .map_err(|error| format!("Agent activity refresh failed: {error}"))?;
    Ok(enriched)
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
    let threshold_secs = state
        .settings
        .lock()
        .map(|settings| u64::from(settings.agent_notifications.inactivity_threshold_minutes) * 60)
        .unwrap_or(crate::agent_activity::DEFAULT_INACTIVITY_THRESHOLD_SECONDS);
    let registry = fetch_activity_registry(
        &state.agent_activity_cache,
        &state.activity_singleflight,
        &state.activity_generation,
        &state.runtime_metrics,
        threshold_secs,
        false,
    )
    .await
    .map_err(|_| "Agent activity cache is unavailable.".to_string())?;
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
    crate::ai_snapshots::invalidate_snapshot(
        &state.agent_activity_cache,
        &state.activity_generation,
    );
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_ai_control_center(
    force: Option<bool>,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<AiControlCenterSnapshot, String> {
    let force_refresh = force.unwrap_or(false);
    // Fast path under a short lock: a warm snapshot never enters blocking work.
    if !force_refresh {
        let fresh = state
            .ai_control_state
            .lock()
            .expect("ai control poisoned")
            .last_snapshot
            .as_ref()
            .filter(|snapshot| unix_timestamp().saturating_sub(snapshot.observed_at) < 10)
            .cloned();
        if let Some(snapshot) = fresh {
            state.runtime_metrics.record_cache_hit();
            return Ok(snapshot);
        }
    }
    // Clone settings-derived inputs under short locks; no I/O held.
    let (preferences, provider_ids, inactivity_threshold_secs) = {
        let settings = lock_or_state_error(&state.settings, "Settings")?;
        (
            settings.ai_control.clone(),
            settings.ai_accounts_quota_providers.clone(),
            u64::from(settings.agent_notifications.inactivity_threshold_minutes) * 60,
        )
    };
    let config_dir = app_handle
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;

    // Reuse the shared single-flight collections instead of invoking collectors
    // independently. Forced control refreshes join running compatible
    // sub-collections; completed sub-caches keep their own TTL semantics.
    let activity = fetch_activity_registry(
        &state.agent_activity_cache,
        &state.activity_singleflight,
        &state.activity_generation,
        &state.runtime_metrics,
        inactivity_threshold_secs,
        false,
    )
    .await?;
    let usage = crate::ai_snapshots::fetch_usage_snapshot(
        &state.ai_usage_cache,
        &state.usage_singleflight,
        &state.usage_generation,
        &state.runtime_metrics,
        state.ai_collection_service.clone(),
        state.credentials.clone(),
        provider_ids.clone(),
        false,
        |_| {},
    )
    .await?;

    // Acquire the subprocess budget before dispatching blocking Git/listener
    // work. Cheap metrics/cache reads never take a budget and stay responsive.
    // The worker owns its permit until the blocking assembly actually exits,
    // including when the request awaiting it is cancelled.
    let subprocess_permit = state
        .execution_budgets
        .acquire_subprocess()
        .await
        .map_err(|error| error.to_string())?;

    let refresh_lock = state.ai_control_refresh_lock.clone();
    let control = state.ai_control_state.clone();
    let memory_sampler = state.memory_sampler.clone();
    let awake = state.awake_manager.clone();
    let dev_store = state.dev_port_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Running blocking work survives request cancellation, so its permit
        // must live in the worker rather than in the awaiting request.
        let _subprocess_permit = subprocess_permit;
        let _refresh_guard = refresh_lock.lock().expect("ai control refresh poisoned");
        // Recheck after acquiring the refresh lock: a concurrent refresh may
        // have published while we collected shared snapshots.
        if !force_refresh {
            let fresh = control
                .lock()
                .expect("ai control poisoned")
                .last_snapshot
                .as_ref()
                .filter(|snapshot| unix_timestamp().saturating_sub(snapshot.observed_at) < 10)
                .cloned();
            if let Some(snapshot) = fresh {
                return snapshot;
            }
        }
        let now = unix_timestamp();
        // Snapshot the smallest Git inputs under a short lock; Git commands
        // and filesystem fingerprinting run outside the shared state lock.
        let (git_baselines, git_generation) = {
            let guard = control.lock().expect("ai control poisoned");
            (
                guard.git.snapshot_baselines(),
                guard.git.baseline_generation(),
            )
        };
        // Blocking-safe observations outside the Control Center lock.
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
        let normalized = crate::ai_control_center::providers::normalize(&usage, &preferences);
        let budget_statuses =
            crate::ai_control_center::budgets::statuses(&preferences.budgets, &normalized);
        let (git_summaries, git_collected) =
            crate::ai_control_center::git::GitBaselineStore::collect_summaries(
                &git_baselines,
                &activity.project_roots,
                now,
            );
        // Short final merge: no subprocess, disk I/O, or channel sends under
        // the shared lock. Audit persistence happens after the lock is free.
        let (snapshot, audit_store) = {
            let mut guard = control.lock().expect("ai control poisoned");
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
            let _ =
                guard
                    .git
                    .commit_collected(git_generation, &activity.project_roots, git_collected);
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
        snapshot
    })
    .await
    .map_err(|error| error.to_string())
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
            let mut next_settings = lock_or_state_error(&settings_store_state, "Settings")?.clone();
            next_settings.ai_control = preferences.clone();
            let config = app_handle
                .path()
                .app_config_dir()
                .map_err(|error| error.to_string())?;
            settings_store::save(&config, &next_settings)?;
            *lock_or_state_error(&settings_store_state, "Settings")? = next_settings;
            // Apply the native policy only after persistence succeeds, keeping the runtime
            // and the settings file consistent if an atomic settings write is rejected.
            awake_manager.set_control_center_awake_policy(
                preferences.autopilot.keep_awake_for_verified_sessions,
                preferences.autopilot.keep_awake_ac_only,
            );
            let audit_store = {
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
                control.audit.clone()
            };
            // Disk I/O outside the shared lock.
            audit_store.save(&config)
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
    let control = state.ai_control_state.clone();
    let (preferences, inactivity_threshold_secs) = {
        let settings = lock_or_state_error(&state.settings, "Settings")?;
        (
            settings.ai_control.clone(),
            u64::from(settings.agent_notifications.inactivity_threshold_minutes) * 60,
        )
    };
    let config = app_handle
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    // Shared activity collection; filesystem inspection stays outside the
    // shared Control Center lock.
    let registry = fetch_activity_registry(
        &state.agent_activity_cache,
        &state.activity_singleflight,
        &state.activity_generation,
        &state.runtime_metrics,
        inactivity_threshold_secs,
        false,
    )
    .await?;
    tauri::async_runtime::spawn_blocking(move || {
        let now = unix_timestamp();
        let snapshot = crate::ai_control_center::safety::inspect(
            &registry.project_roots,
            &preferences.dismissed_findings,
            now,
        );
        let audit_store = {
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
            control.audit.clone()
        };
        // Disk I/O outside the shared lock.
        let _ = audit_store.save(&config);
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
            let mut settings = lock_or_state_error(&settings_store_state, "Settings")?.clone();
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
            *lock_or_state_error(&settings_store_state, "Settings")? = settings;

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
            let audit_store = control.audit.clone();
            drop(control);
            // Disk I/O outside the shared lock.
            audit_store.save(&config)
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
            let audit_store = control.audit.clone();
            drop(control);
            // Disk I/O outside the shared lock.
            let _ = audit_store.save(&config);
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
            let audit_store = control.audit.clone();
            drop(control);
            // Disk I/O outside the shared lock.
            let _ = audit_store.save(&config);
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
        // Snapshot the baseline under a short lock; Git capture and diffing
        // run outside the shared Control Center lock.
        let baseline = control_state
            .lock()
            .expect("ai control poisoned")
            .git
            .baseline_snapshot(&project_id)
            .ok_or_else(|| "Git baseline is stale or unavailable".to_string())?;
        let now = unix_timestamp();
        let (baseline_head, paths) =
            crate::ai_control_center::git::GitBaselineStore::diff_context_with_baseline(
                &baseline, &root, now,
            );
        let diff =
            crate::ai_control_center::git::explicit_diff(&root, baseline_head.as_deref(), &paths)?;
        let config = app_handle
            .path()
            .app_config_dir()
            .map_err(|error| error.to_string())?;
        let audit_store = {
            let mut control = control_state.lock().expect("ai control poisoned");
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
        let _ = audit_store.save(&config);
        Ok::<_, String>(diff)
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(diff)
}

#[tauri::command]
#[specta::specta]
pub async fn connect_openrouter_oauth(state: State<'_, AppState>) -> Result<(), String> {
    let credentials = state.credentials.clone();
    let key = tauri::async_runtime::spawn_blocking(connect_openrouter)
        .await
        .map_err(|error| error.to_string())??;
    let secret = crate::ai_providers::SecretString::new(key.clone());
    if let Err(error) = credentials.set(crate::models::ProviderId::OpenRouter, secret) {
        // The provider already issued a live key. If it cannot be persisted,
        // revoke it instead of leaving an orphaned credential behind.
        return Err(match crate::ai_providers::revoke_openrouter(&key) {
            Ok(()) => format!(
                "Could not persist the OpenRouter credential; the issued key was revoked. {error}"
            ),
            Err(revoke_error) => format!(
                "Could not persist the OpenRouter credential: {error}. Provider revocation also failed: {revoke_error}"
            ),
        });
    }
    crate::ai_snapshots::invalidate_snapshot(&state.ai_usage_cache, &state.usage_generation);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_ai_provider_credential(
    provider: crate::models::ProviderId,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let credentials = state.credentials.clone();
    let usage_cache = state.ai_usage_cache.clone();
    let usage_generation = state.usage_generation.clone();
    run_blocking(
        move || {
            let existing = credentials.get(provider).map_err(|error| error.to_string())?;
            credentials
                .remove(provider)
                .map_err(|error| error.to_string())?;
            crate::ai_snapshots::invalidate_snapshot(&usage_cache, &usage_generation);
            if provider == crate::models::ProviderId::OpenRouter {
                if let Some(secret) = existing {
                    // Disconnecting must revoke a non-expiring OAuth key at the
                    // provider; the local removal still succeeds either way so
                    // the user is not trapped with a credential they cannot drop.
                    if let Err(error) =
                        crate::ai_providers::revoke_openrouter(secret.expose_secret())
                    {
                        return Err(format!(
                            "OpenRouter credential removed locally, but provider revocation failed: {error}"
                        ));
                    }
                }
            }
            Ok(())
        },
        "Provider disconnect worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub fn get_ai_provider_descriptors() -> Vec<crate::ai_providers::ProviderDescriptor> {
    crate::ai_providers::ProviderRegistry::all()
}
