use crate::ai_usage::{connect_openrouter, AiUsageCollector};
use crate::cleaner::CleanExecutor;
use crate::docker::DockerAdapter;
use crate::metrics::{DiskMetricsCollector, MemoryInspector};
use crate::models::{
    AiUsageSnapshot, AwakeBehavior, AwakeRule, AwakeState, Category, CleanEvent, CleanResult,
    DeletePlan, DiskMetrics, DiskVolume, DockerStatus, LocalModelItem, MemoryMetrics, ScanEvent,
    ScanItem, ScanResult, SelectedApplication, ZenithSettings,
};
use crate::models_inventory::LocalModelScanner;
use crate::power::{ApplicationPicker, KeepAwakeManager};
use crate::safety::SafetyPlanner;
use crate::scanner::ScanEngine;
use crate::signatures::SignatureRegistry;
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

pub struct AppState {
    pub registry: Arc<SignatureRegistry>,
    pub awake_manager: Arc<KeepAwakeManager>,
    pub settings: Arc<Mutex<ZenithSettings>>,
    pub last_scan: Arc<Mutex<Option<ScanResult>>>,
    pub openrouter_key: Arc<Mutex<Option<String>>>,
}

#[tauri::command]
pub async fn get_ai_usage(state: State<'_, AppState>) -> Result<AiUsageSnapshot, String> {
    let openrouter_key = state.openrouter_key.lock().unwrap().clone();
    tauri::async_runtime::spawn_blocking(move || AiUsageCollector::collect(openrouter_key))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn connect_openrouter_oauth(state: State<'_, AppState>) -> Result<(), String> {
    let openrouter_key = state.openrouter_key.clone();
    let key = tauri::async_runtime::spawn_blocking(connect_openrouter)
        .await
        .map_err(|error| error.to_string())??;
    *openrouter_key.lock().unwrap() = Some(key);
    Ok(())
}

#[tauri::command]
pub async fn start_scan(
    on_event: Channel<ScanEvent>,
    categories: Option<Vec<Category>>,
    state: State<'_, AppState>,
) -> Result<ScanResult, String> {
    let registry = state.registry.clone();
    let last_scan_store = state.last_scan.clone();

    let result = std::thread::spawn(move || {
        let cat_ref = categories.as_deref();
        ScanEngine::scan(&registry, cat_ref, |event| {
            let _ = on_event.send(event);
        })
    })
    .join()
    .map_err(|_| "Scan worker thread panicked".to_string())?;

    let mut last = last_scan_store.lock().unwrap();
    *last = Some(result.clone());

    Ok(result)
}

#[tauri::command]
pub fn get_last_scan(state: State<'_, AppState>) -> Option<ScanResult> {
    state.last_scan.lock().unwrap().clone()
}

#[tauri::command]
pub fn create_delete_plan(
    items: Vec<ScanItem>,
    state: State<'_, AppState>,
) -> Result<DeletePlan, String> {
    SafetyPlanner::create_plan(&items, &state.registry).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn execute_clean(
    plan: DeletePlan,
    on_event: Channel<CleanEvent>,
) -> Result<CleanResult, String> {
    let result = std::thread::spawn(move || {
        CleanExecutor::execute(plan, |event| {
            let _ = on_event.send(event);
        })
    })
    .join()
    .map_err(|_| "Clean execution thread panicked".to_string())?;

    Ok(result)
}

#[tauri::command]
pub fn get_memory_metrics() -> Result<MemoryMetrics, String> {
    Ok(MemoryInspector::get_metrics())
}

#[tauri::command]
pub fn terminate_process_group(name: String, force: bool) -> Result<usize, String> {
    MemoryInspector::terminate_group(&name, force)
}

#[tauri::command]
pub async fn pick_keep_awake_application() -> Result<Option<SelectedApplication>, String> {
    tauri::async_runtime::spawn_blocking(ApplicationPicker::pick)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn get_disk_metrics() -> Result<DiskMetrics, String> {
    DiskMetricsCollector::get_primary_disk().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_disk_volumes() -> Vec<DiskVolume> {
    DiskMetricsCollector::get_volumes()
}

#[tauri::command]
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
pub fn get_docker_status() -> Result<DockerStatus, String> {
    Ok(DockerAdapter::get_status())
}

#[tauri::command]
pub fn prune_docker_target(signature_id: String) -> Result<u64, String> {
    DockerAdapter::prune_category(&signature_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_local_models() -> Result<Vec<LocalModelItem>, String> {
    Ok(LocalModelScanner::scan_all_models())
}

#[tauri::command]
pub fn delete_local_model(path: String) -> Result<u64, String> {
    LocalModelScanner::delete_model(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_awake_state(state: State<'_, AppState>) -> Result<AwakeState, String> {
    state.awake_manager.evaluate();
    Ok(state.awake_manager.get_state())
}

#[tauri::command]
pub fn set_awake_rules(rules: Vec<AwakeRule>, state: State<'_, AppState>) -> Result<(), String> {
    state.awake_manager.set_rules(rules);
    Ok(())
}

#[tauri::command]
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
pub fn disable_manual_awake(state: State<'_, AppState>) -> Result<(), String> {
    state.awake_manager.disable_manual();
    Ok(())
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<ZenithSettings, String> {
    let s = state.settings.lock().unwrap();
    Ok(s.clone())
}

#[tauri::command]
pub fn save_settings(settings: ZenithSettings, state: State<'_, AppState>) -> Result<(), String> {
    state.awake_manager.set_rules(settings.awake_rules.clone());
    let mut s = state.settings.lock().unwrap();
    *s = settings;
    Ok(())
}

#[tauri::command]
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
pub fn open_dashboard_window(app_handle: AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("main") {
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
pub fn toggle_quick_panel(app_handle: AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("quick") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
    Ok(())
}
