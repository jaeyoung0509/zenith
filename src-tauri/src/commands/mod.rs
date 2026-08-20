use crate::ai_usage::{connect_openrouter, AiUsageCollector};
use crate::cleaner::CleanExecutor;
use crate::docker::DockerAdapter;
use crate::metrics::{DiskMetricsCollector, MemoryInspector};
use crate::models::{
    AiUsageSnapshot, AwakeBehavior, AwakeRule, AwakeState, Category, CleanEvent, CleanResult,
    DeletePlan, DiagnosticsSnapshot, DiskMetrics, DiskVolume, DockerStatus, LocalModelItem,
    MemoryMetrics, PlanPreview, ScanEvent, ScanResult, SelectedApplication, ZenithSettings,
};
use crate::models_inventory::{LocalModelManager, LocalModelScanner};
use crate::power::{ApplicationPicker, KeepAwakeManager};
use crate::safety::SafetyPlanner;
use crate::scanner::ScanEngine;
use crate::settings_store;
use crate::signatures::SignatureRegistry;
use std::collections::HashMap;
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
    pub operation_lock: Arc<Mutex<()>>,
    pub memory_sampler: Arc<crate::metrics::MemorySampler>,
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[tauri::command]
#[specta::specta]
pub async fn get_ai_usage(
    force: Option<bool>,
    state: State<'_, AppState>,
) -> Result<AiUsageSnapshot, String> {
    const CACHE_TTL_SECS: u64 = 60;
    if !force.unwrap_or(false) {
        if let Some(snapshot) = state.ai_usage_cache.lock().unwrap().as_ref() {
            if snapshot.is_fresh_at(unix_timestamp(), CACHE_TTL_SECS) {
                return Ok(snapshot.clone());
            }
        }
    }

    let openrouter_key = state.openrouter_key.lock().unwrap().clone();
    let cache = state.ai_usage_cache.clone();
    let refresh_lock = state.ai_usage_refresh_lock.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _refresh_guard = refresh_lock.lock().unwrap();
        if !force.unwrap_or(false) {
            if let Some(snapshot) = cache.lock().unwrap().as_ref() {
                if snapshot.is_fresh_at(unix_timestamp(), CACHE_TTL_SECS) {
                    return snapshot.clone();
                }
            }
        }
        let snapshot = AiUsageCollector::collect(openrouter_key);
        *cache.lock().unwrap() = Some(snapshot.clone());
        snapshot
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn connect_openrouter_oauth(state: State<'_, AppState>) -> Result<(), String> {
    let openrouter_key = state.openrouter_key.clone();
    let key = tauri::async_runtime::spawn_blocking(connect_openrouter)
        .await
        .map_err(|error| error.to_string())??;
    *openrouter_key.lock().unwrap() = Some(key);
    *state.ai_usage_cache.lock().unwrap() = None;
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
    let operation_lock = state.operation_lock.clone();
    let excluded_signatures = state.settings.lock().unwrap().excluded_signatures.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        let _operation_guard = operation_lock.lock().unwrap();
        let cat_ref = categories.as_deref();
        let result = ScanEngine::scan(&registry, cat_ref, &excluded_signatures, |event| {
            let _ = on_event.send(event);
        });
        *last_scan_store.lock().unwrap() = Some(result.clone());
        result
    })
    .await
    .map_err(|_| "Scan worker thread panicked".to_string())?;

    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub fn get_last_scan(state: State<'_, AppState>) -> Option<ScanResult> {
    state.last_scan.lock().unwrap().clone()
}

#[tauri::command]
#[specta::specta]
pub fn create_delete_plan(
    scan_id: String,
    selected_item_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<PlanPreview, String> {
    const PLAN_TTL_SECS: u64 = 300;
    let scan = state
        .last_scan
        .lock()
        .unwrap()
        .clone()
        .filter(|scan| scan.scan_id == scan_id)
        .ok_or_else(|| "The scan is no longer current. Scan again before cleaning.".to_string())?;

    let plan =
        SafetyPlanner::create_plan_from_scan(&scan, &scan_id, &selected_item_ids, &state.registry)
            .map_err(|e| e.to_string())?;
    let preview = plan.preview(PLAN_TTL_SECS);
    let now = unix_timestamp();
    let mut plans = state.delete_plans.lock().unwrap();
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
}

#[tauri::command]
#[specta::specta]
pub async fn execute_clean(
    plan_id: uuid::Uuid,
    on_event: Channel<CleanEvent>,
    state: State<'_, AppState>,
) -> Result<CleanResult, String> {
    const PLAN_TTL_SECS: u64 = 300;
    let operation_lock = state.operation_lock.clone();
    let plans = state.delete_plans.clone();
    let last_scan = state.last_scan.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<CleanResult, String> {
        let _operation_guard = operation_lock.lock().unwrap();
        let plan = plans
            .lock()
            .unwrap()
            .remove(&plan_id)
            .ok_or_else(|| "Delete plan not found or already used".to_string())?;
        if unix_timestamp().saturating_sub(plan.created_at) >= PLAN_TTL_SECS {
            return Err("Delete plan expired. Scan again before cleaning.".to_string());
        }
        let scan_is_current = last_scan
            .lock()
            .unwrap()
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
pub fn terminate_process_group(name: String, force: bool) -> Result<usize, String> {
    MemoryInspector::terminate_group(&name, force)
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
pub fn get_disk_metrics() -> Result<DiskMetrics, String> {
    DiskMetricsCollector::get_primary_disk().map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_disk_volumes() -> Vec<DiskVolume> {
    DiskMetricsCollector::get_volumes()
}

#[tauri::command]
#[specta::specta]
pub fn open_disk_utility() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .args(["-a", "Disk Utility"])
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn get_docker_status() -> Result<DockerStatus, String> {
    Ok(DockerAdapter::get_status())
}

#[tauri::command]
#[specta::specta]
pub fn prune_docker_target(signature_id: String) -> Result<u64, String> {
    DockerAdapter::prune_category(&signature_id).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_local_models() -> Result<Vec<LocalModelItem>, String> {
    Ok(LocalModelScanner::scan_all_models())
}

#[tauri::command]
#[specta::specta]
pub fn delete_local_model(model_id: String) -> Result<u64, String> {
    LocalModelManager::delete_by_id(&model_id).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_awake_state(state: State<'_, AppState>) -> Result<AwakeState, String> {
    Ok(state.awake_manager.get_state())
}

#[tauri::command]
#[specta::specta]
pub fn set_awake_rules(rules: Vec<AwakeRule>, state: State<'_, AppState>) -> Result<(), String> {
    state.awake_manager.set_rules(rules);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_manual_awake(
    duration_secs: Option<u64>,
    behavior: AwakeBehavior,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .awake_manager
        .set_manual(duration_secs, behavior)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn disable_manual_awake(state: State<'_, AppState>) -> Result<(), String> {
    state.awake_manager.disable_manual();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn get_settings(state: State<'_, AppState>) -> Result<ZenithSettings, String> {
    let s = state.settings.lock().unwrap();
    Ok(s.clone())
}

#[tauri::command]
#[specta::specta]
pub fn save_settings(
    settings: ZenithSettings,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let settings = settings.sanitize();
    let config_dir = app_handle
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    settings_store::save(&config_dir, &settings)?;
    state.awake_manager.set_rules(settings.awake_rules.clone());
    let mut s = state.settings.lock().unwrap();
    *s = settings;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn reveal_in_finder(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-R", &path])
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn open_dashboard_window(app_handle: AppHandle) -> Result<(), String> {
    if let Ok(window) = crate::ensure_window(&app_handle, "main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
    // Hide quick panel if open
    if let Some(quick) = app_handle.get_webview_window("quick") {
        let _ = quick.hide();
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
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
pub fn get_diagnostics(
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<DiagnosticsSnapshot, String> {
    let settings = state.settings.lock().unwrap().clone();
    let config_dir = app_handle
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    Ok(crate::diagnostics::get_snapshot(&settings, &config_dir))
}

#[tauri::command]
#[specta::specta]
pub fn open_logs_folder() -> Result<(), String> {
    crate::diagnostics::open_logs_folder()
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
