//! System metrics, preferences, Keep Awake, diagnostics, and development commands.

use super::state::AppState;
use super::support::{lock_or_state_error, run_blocking};
use crate::docker::DockerAdapter;
use crate::metrics::{DiskMetricsCollector, MemoryInspector};
use crate::models::{
    AwakeBehavior, AwakeRule, AwakeState, DevelopmentListener, DiagnosticsSnapshot, DiskMetrics,
    DiskVolume, DockerStatus, LocalModelItem, MemoryMetrics, MemoryTerminationMode,
    MemoryTerminationResult, PlatformCapabilities, PlatformContext,
    ReleaseDevelopmentListenerResult, ReleaseMode, SelectedApplication, ZenithSettings,
};
use crate::models_inventory::{LocalModelManager, LocalModelScanner};
use crate::platform::PlatformPathsProvider;
use crate::power::ApplicationPicker;
use crate::settings_store;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, State};

#[tauri::command]
#[specta::specta]
pub async fn get_memory_metrics(state: State<'_, AppState>) -> Result<MemoryMetrics, String> {
    let sampler = state.memory_sampler.clone();
    let lease_store = state.memory_termination_store.clone();
    let observation = sampler
        .get_observation(std::time::Duration::from_millis(800))
        .await?;
    Ok(observation.mint_metrics_with_leases(&lease_store))
}

#[tauri::command]
#[specta::specta]
pub async fn terminate_memory_group(
    lease_id: String,
    mode: MemoryTerminationMode,
    state: State<'_, AppState>,
) -> Result<MemoryTerminationResult, String> {
    state
        .platform_capabilities
        .capabilities()
        .require(
            crate::models::PlatformFeature::ProcessTermination,
            crate::models::CapabilityAccess::Mutate,
        )
        .map_err(|e| e.to_string())?;

    let lease_store = state.memory_termination_store.clone();
    run_blocking(
        move || {
            let system = crate::metrics::memory::RealMemorySystem::default();
            MemoryInspector::execute_termination(&lease_id, mode, &lease_store, &system)
        },
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
pub async fn open_storage_settings() -> Result<(), String> {
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
pub async fn get_docker_status(state: State<'_, AppState>) -> Result<DockerStatus, String> {
    const DOCKER_STATUS_TTL: std::time::Duration = std::time::Duration::from_secs(3);
    {
        let cache = state
            .docker_status_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some((status, fetched_at)) = &*cache {
            if fetched_at.elapsed() < DOCKER_STATUS_TTL {
                return Ok(status.clone());
            }
        }
    }

    let _permit = state.execution_budgets.acquire_subprocess().await?;
    let cache_store = state.docker_status_cache.clone();
    let operation_gate = state.storage_operation_gate.clone();
    run_blocking(
        move || {
            operation_gate.run_read(|| {
                // A Docker mutation may have completed while this cache miss
                // waited for the read side of the storage gate.
                {
                    let cache = cache_store.lock().unwrap_or_else(|p| p.into_inner());
                    if let Some((status, fetched_at)) = &*cache {
                        if fetched_at.elapsed() < DOCKER_STATUS_TTL {
                            return Ok(status.clone());
                        }
                    }
                }

                let fresh = DockerAdapter::get_status();
                let mut cache = cache_store.lock().unwrap_or_else(|p| p.into_inner());
                *cache = Some((fresh.clone(), std::time::Instant::now()));
                Ok(fresh)
            })
        },
        "Docker status worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn prune_docker_target(
    signature_id: String,
    state: State<'_, AppState>,
) -> Result<u64, String> {
    state
        .platform_capabilities
        .capabilities()
        .require(
            crate::models::PlatformFeature::Docker,
            crate::models::CapabilityAccess::Mutate,
        )
        .map_err(|e| e.to_string())?;

    let _permit = state.execution_budgets.acquire_subprocess().await?;
    let cache_store = state.docker_status_cache.clone();
    let operation_gate = state.storage_operation_gate.clone();
    run_blocking(
        move || {
            operation_gate.run_write(|| {
                *cache_store.lock().unwrap_or_else(|p| p.into_inner()) = None;
                let result =
                    DockerAdapter::prune_category(&signature_id).map_err(|error| error.to_string());
                // The command may have changed Docker state even if its final
                // status/delta query failed, so never retain a pre-prune snapshot.
                *cache_store.lock().unwrap_or_else(|p| p.into_inner()) = None;
                result
            })
        },
        "Docker cleanup worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn get_local_models(state: State<'_, AppState>) -> Result<Vec<LocalModelItem>, String> {
    state
        .platform_capabilities
        .capabilities()
        .require(
            crate::models::PlatformFeature::LocalModels,
            crate::models::CapabilityAccess::Inspect,
        )
        .map_err(|e| e.to_string())?;

    let _permit = state.execution_budgets.acquire_storage_read().await?;
    let operation_gate = state.storage_operation_gate.clone();
    run_blocking(
        move || operation_gate.run_read(|| Ok(LocalModelScanner::scan_all_models())),
        "Local model scan worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_local_model(
    model_id: String,
    state: State<'_, AppState>,
) -> Result<u64, String> {
    state
        .platform_capabilities
        .capabilities()
        .require(
            crate::models::PlatformFeature::LocalModels,
            crate::models::CapabilityAccess::Mutate,
        )
        .map_err(|e| e.to_string())?;

    let operation_gate = state.storage_operation_gate.clone();
    run_blocking(
        move || {
            operation_gate.run_write(|| {
                LocalModelManager::delete_by_id(&model_id).map_err(|error| error.to_string())
            })
        },
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
    let s = lock_or_state_error(&state.settings, "Settings")?;
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
    let (provider_selection_changed, inactivity_threshold_changed) = {
        let previous = lock_or_state_error(&state.settings, "Settings")?;
        (
            previous.ai_accounts_quota_providers != settings.ai_accounts_quota_providers,
            previous.agent_notifications.inactivity_threshold_minutes
                != settings.agent_notifications.inactivity_threshold_minutes,
        )
    };
    let awake_manager = state.awake_manager.clone();
    let settings_store_state = state.settings.clone();
    let ai_usage_cache = state.ai_usage_cache.clone();
    let agent_activity_cache = state.agent_activity_cache.clone();
    let ai_control_state = state.ai_control_state.clone();
    let usage_generation = state.usage_generation.clone();
    let activity_generation = state.activity_generation.clone();

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
            *lock_or_state_error(&settings_store_state, "Settings")? = settings;
            if provider_selection_changed {
                crate::ai_snapshots::invalidate_snapshot(&ai_usage_cache, &usage_generation);
                ai_control_state
                    .lock()
                    .expect("ai control poisoned")
                    .last_snapshot = None;
            }
            if inactivity_threshold_changed {
                crate::ai_snapshots::invalidate_snapshot(
                    &agent_activity_cache,
                    &activity_generation,
                );
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
pub async fn show_in_file_manager(path: String, state: State<'_, AppState>) -> Result<(), String> {
    state
        .platform_capabilities
        .capabilities()
        .require(
            crate::models::PlatformFeature::SystemActions,
            crate::models::CapabilityAccess::Inspect,
        )
        .map_err(|error| error.to_string())?;
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
    let path_obj = Path::new(path);
    let normalized = crate::platform::NativePlatformPaths::normalize_verbatim_path(path_obj);
    let path_str = normalized.to_string_lossy();
    let expanded = if let Some(relative) = path_str
        .strip_prefix("~/")
        .or_else(|| path_str.strip_prefix("~\\"))
    {
        let home = crate::platform::NativePlatformPaths::new()
            .user_home()
            .ok_or_else(|| "Home environment variable is not set.".to_string())?;
        home.join(relative)
    } else {
        normalized
    };
    let canonical = expanded
        .canonicalize()
        .map_err(|error| format!("Path is no longer available: {error}"))?;
    Ok(crate::platform::NativePlatformPaths::normalize_verbatim_path(&canonical))
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

/// Platform vocabulary and locations the interface renders.
///
/// Copy such as "Move to Trash", "menu bar", or a log directory literal is only
/// true on one platform; the frontend asks for it instead of hardcoding it.
#[tauri::command]
#[specta::specta]
pub fn get_platform_context() -> PlatformContext {
    PlatformContext::current(crate::diagnostics::log_directory_display())
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
            let settings = lock_or_state_error(&settings, "Settings")?.clone();
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
    state
        .platform_capabilities
        .capabilities()
        .require(
            crate::models::PlatformFeature::DevelopmentPorts,
            crate::models::CapabilityAccess::Inspect,
        )
        .map_err(|e| e.to_string())?;

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
    state
        .platform_capabilities
        .capabilities()
        .require(
            crate::models::PlatformFeature::DevelopmentPorts,
            crate::models::CapabilityAccess::Mutate,
        )
        .map_err(|e| e.to_string())?;

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
