use crate::applications::{AppInspectionRecord, AppInventory, ApplicationScanner};
use crate::commands::AppState;
use crate::developer_artifacts::{
    result_from_inventory, DeveloperArtifactInventory, DeveloperArtifactScanner,
};
use crate::large_files::{LargeFileInventory, LargeFileScanner};
use crate::models::{
    AppUninstallInspection, DeveloperArtifactScanEvent, DeveloperArtifactScanResult,
    DeveloperWorkspace, InstalledApp, LargeFileScanEvent, LargeFileScanRequest,
    LargeFileScanResult, TrashPlanPreview, TrashResult,
};
use crate::trash_manager::{TrashExecutor, TrashPlan, TrashPlanner};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::ipc::Channel;
use tauri::State;

static LARGE_FILE_INVENTORY: LazyLock<Mutex<Option<LargeFileInventory>>> =
    LazyLock::new(|| Mutex::new(None));
static LARGE_FILE_CANCEL: LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static DEVELOPER_ARTIFACT_INVENTORY: LazyLock<Mutex<Option<DeveloperArtifactInventory>>> =
    LazyLock::new(|| Mutex::new(None));
static DEVELOPER_ARTIFACT_CANCEL: LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static APP_INVENTORY: LazyLock<Mutex<Option<AppInventory>>> = LazyLock::new(|| Mutex::new(None));
static APP_INSPECTION: LazyLock<Mutex<Option<AppInspectionRecord>>> =
    LazyLock::new(|| Mutex::new(None));
pub(crate) static TRASH_PLANS: LazyLock<Mutex<HashMap<uuid::Uuid, TrashPlan>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
const INVENTORY_TTL_SECS: u64 = 15 * 60;
const PLAN_TTL_SECS: u64 = 5 * 60;

fn is_fresh_at(created_at: u64, ttl_secs: u64, now: u64) -> bool {
    now.saturating_sub(created_at) < ttl_secs
}

#[cfg(test)]
pub(crate) fn clear_trash_plans_for_test() {
    TRASH_PLANS.lock().expect("TRASH_PLANS poisoned").clear();
}

#[cfg(test)]
pub(crate) fn trash_plans_len_for_test() -> usize {
    TRASH_PLANS.lock().expect("TRASH_PLANS poisoned").len()
}

#[tauri::command]
#[specta::specta]
pub async fn start_large_file_scan(
    request: LargeFileScanRequest,
    on_event: Channel<LargeFileScanEvent>,
    state: State<'_, AppState>,
) -> Result<LargeFileScanResult, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_worker = cancel.clone();
    let operation_gate = state.storage_operation_gate.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        operation_gate.run(|| {
            let mut emitted_result: Option<LargeFileScanResult> = None;
            let inventory = LargeFileScanner::scan(&request, cancel_for_worker, |event| {
                if let LargeFileScanEvent::Started { scan_id } = &event {
                    LARGE_FILE_CANCEL
                        .lock()
                        .expect("LARGE_FILE_CANCEL poisoned")
                        .insert(scan_id.clone(), cancel.clone());
                }
                if let LargeFileScanEvent::Finished { result } = &event {
                    emitted_result = Some(result.clone());
                }
                let _ = on_event.send(event);
            })?;
            LARGE_FILE_CANCEL
                .lock()
                .expect("LARGE_FILE_CANCEL poisoned")
                .remove(&inventory.scan_id);
            let result = emitted_result.unwrap_or_else(|| {
                let mut items = inventory
                    .records
                    .values()
                    .map(|record| record.item.clone())
                    .collect::<Vec<_>>();
                items.sort_by(|left, right| {
                    right
                        .allocated_size
                        .cmp(&left.allocated_size)
                        .then_with(|| right.logical_size.cmp(&left.logical_size))
                        .then_with(|| left.name.cmp(&right.name))
                });
                LargeFileScanResult {
                    scan_id: inventory.scan_id.clone(),
                    items,
                    entries_scanned: inventory.entries_scanned,
                    skipped_entries: inventory.skipped_entries,
                    cancelled: true,
                    truncated: inventory.truncated,
                }
            });
            *LARGE_FILE_INVENTORY
                .lock()
                .expect("LARGE_FILE_INVENTORY poisoned") = Some(inventory);
            Ok::<_, String>(result)
        })
    })
    .await
    .map_err(|_| "Large-file scan worker panicked".to_string())??;
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub async fn pick_developer_workspace() -> Result<Option<DeveloperWorkspace>, String> {
    tauri::async_runtime::spawn_blocking(crate::developer_artifacts::pick_workspace)
        .await
        .map_err(|_| "Developer workspace picker worker panicked".to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn start_developer_artifact_scan(
    workspace_ids: Vec<String>,
    on_event: Channel<DeveloperArtifactScanEvent>,
    state: State<'_, AppState>,
) -> Result<DeveloperArtifactScanResult, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_worker = cancel.clone();
    let operation_gate = state.storage_operation_gate.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        operation_gate.run(|| {
            let mut emitted_result: Option<DeveloperArtifactScanResult> = None;
            let inventory =
                DeveloperArtifactScanner::scan(&workspace_ids, cancel_for_worker, |event| {
                    if let DeveloperArtifactScanEvent::Started { scan_id, .. } = &event {
                        DEVELOPER_ARTIFACT_CANCEL
                            .lock()
                            .expect("DEVELOPER_ARTIFACT_CANCEL poisoned")
                            .insert(scan_id.clone(), cancel.clone());
                    }
                    if let DeveloperArtifactScanEvent::Finished { result } = &event {
                        emitted_result = Some(result.clone());
                    }
                    let _ = on_event.send(event);
                })?;
            DEVELOPER_ARTIFACT_CANCEL
                .lock()
                .expect("DEVELOPER_ARTIFACT_CANCEL poisoned")
                .remove(&inventory.scan_id);
            let result = emitted_result.unwrap_or_else(|| result_from_inventory(&inventory));
            *DEVELOPER_ARTIFACT_INVENTORY
                .lock()
                .expect("DEVELOPER_ARTIFACT_INVENTORY poisoned") = Some(inventory);
            Ok::<_, String>(result)
        })
    })
    .await
    .map_err(|_| "Developer artifact scan worker panicked".to_string())??;
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub fn cancel_developer_artifact_scan(scan_id: String) -> Result<(), String> {
    let cancel = DEVELOPER_ARTIFACT_CANCEL
        .lock()
        .expect("DEVELOPER_ARTIFACT_CANCEL poisoned")
        .get(&scan_id)
        .cloned()
        .ok_or_else(|| "Developer artifact scan is no longer running".to_string())?;
    cancel.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn prepare_developer_artifact_cleanup(
    scan_id: String,
    selected_item_ids: Vec<String>,
) -> Result<TrashPlanPreview, String> {
    let inventory = DEVELOPER_ARTIFACT_INVENTORY
        .lock()
        .expect("DEVELOPER_ARTIFACT_INVENTORY poisoned")
        .clone()
        .filter(|inventory| inventory.scan_id == scan_id)
        .filter(DeveloperArtifactInventory::is_fresh)
        .ok_or_else(|| "Developer artifact inventory expired. Scan again.".to_string())?;
    let plan = TrashPlanner::from_developer_artifacts(&inventory, &selected_item_ids)?;
    let preview = plan.preview();
    store_plan(plan);
    Ok(preview)
}

#[tauri::command]
#[specta::specta]
pub fn cancel_large_file_scan(scan_id: String) -> Result<(), String> {
    let cancel = LARGE_FILE_CANCEL
        .lock()
        .expect("LARGE_FILE_CANCEL poisoned")
        .get(&scan_id)
        .cloned()
        .ok_or_else(|| "Large-file scan is no longer running".to_string())?;
    cancel.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn prepare_large_file_trash(
    scan_id: String,
    selected_item_ids: Vec<String>,
) -> Result<TrashPlanPreview, String> {
    let inventory = LARGE_FILE_INVENTORY
        .lock()
        .expect("LARGE_FILE_INVENTORY poisoned")
        .clone()
        .filter(|inventory| inventory.scan_id == scan_id)
        .filter(|inventory| is_fresh_at(inventory.created_at, INVENTORY_TTL_SECS, unix_timestamp()))
        .ok_or_else(|| "Large-file inventory expired. Scan again.".to_string())?;
    let plan = TrashPlanner::from_large_files(&inventory, &selected_item_ids)?;
    let preview = plan.preview();
    store_plan(plan);
    Ok(preview)
}

#[tauri::command]
#[specta::specta]
pub async fn get_installed_apps(state: State<'_, AppState>) -> Result<Vec<InstalledApp>, String> {
    let operation_gate = state.storage_operation_gate.clone();
    let inventory =
        tauri::async_runtime::spawn_blocking(move || operation_gate.run(ApplicationScanner::scan))
            .await
            .map_err(|_| "Application inventory worker panicked".to_string())?;
    let mut apps = inventory
        .records
        .values()
        .map(|record| record.app.clone())
        .collect::<Vec<_>>();
    apps.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    *APP_INVENTORY.lock().expect("APP_INVENTORY poisoned") = Some(inventory);
    Ok(apps)
}

#[tauri::command]
#[specta::specta]
pub async fn inspect_app_uninstall(
    app_id: String,
    state: State<'_, AppState>,
) -> Result<AppUninstallInspection, String> {
    let operation_gate = state.storage_operation_gate.clone();
    let inspection = tauri::async_runtime::spawn_blocking(move || {
        operation_gate.run(|| {
            let inventory = APP_INVENTORY
                .lock()
                .expect("APP_INVENTORY poisoned")
                .clone()
                .filter(|inventory| {
                    is_fresh_at(inventory.created_at, INVENTORY_TTL_SECS, unix_timestamp())
                })
                .ok_or_else(|| {
                    "Application inventory expired. Refresh applications.".to_string()
                })?;
            ApplicationScanner::inspect(&inventory, &app_id)
        })
    })
    .await
    .map_err(|_| "App inspection worker panicked".to_string())??;
    let result = inspection.inspection.clone();
    *APP_INSPECTION.lock().expect("APP_INSPECTION poisoned") = Some(inspection);
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub fn prepare_app_uninstall(
    inspection_id: String,
    selected_related_ids: Vec<String>,
) -> Result<TrashPlanPreview, String> {
    let inspection = APP_INSPECTION
        .lock()
        .expect("APP_INSPECTION poisoned")
        .clone()
        .filter(|inspection| inspection.inspection.inspection_id == inspection_id)
        .filter(|inspection| {
            is_fresh_at(inspection.created_at, INVENTORY_TTL_SECS, unix_timestamp())
        })
        .ok_or_else(|| "App uninstall review expired. Review the app again.".to_string())?;
    let plan = TrashPlanner::from_app_inspection(&inspection, &selected_related_ids)?;
    let preview = plan.preview();
    store_plan(plan);
    Ok(preview)
}

#[tauri::command]
#[specta::specta]
pub async fn execute_trash_plan(
    plan_id: uuid::Uuid,
    state: State<'_, AppState>,
) -> Result<TrashResult, String> {
    let operation_gate = state.storage_operation_gate.clone();
    tauri::async_runtime::spawn_blocking(move || {
        operation_gate.run(|| {
            let plan = TRASH_PLANS
                .lock()
                .expect("TRASH_PLANS poisoned")
                .remove(&plan_id)
                .ok_or_else(|| "Trash plan not found or already used".to_string())?;
            if plan.is_expired() {
                return Err("Trash plan expired. Review the items again.".to_string());
            }
            Ok(TrashExecutor::execute(plan))
        })
    })
    .await
    .map_err(|_| "Trash execution worker panicked".to_string())?
}

pub(crate) fn store_plan(plan: TrashPlan) {
    let now = unix_timestamp();
    let mut plans = TRASH_PLANS.lock().expect("TRASH_PLANS poisoned");
    plans.retain(|_, plan| is_fresh_at(plan.created_at, PLAN_TTL_SECS, now));
    if plans.len() >= 64 {
        if let Some(oldest) = plans
            .iter()
            .min_by_key(|(_, plan)| plan.created_at)
            .map(|(id, _)| *id)
        {
            plans.remove(&oldest);
        }
    }
    plans.insert(plan.id, plan);
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::{is_fresh_at, INVENTORY_TTL_SECS, PLAN_TTL_SECS};

    #[test]
    fn inventory_and_plan_ttl_boundaries_fail_closed() {
        let now = 1_000;

        assert!(is_fresh_at(
            now - (INVENTORY_TTL_SECS - 1),
            INVENTORY_TTL_SECS,
            now
        ));
        assert!(!is_fresh_at(
            now - INVENTORY_TTL_SECS,
            INVENTORY_TTL_SECS,
            now
        ));
        assert!(!is_fresh_at(
            now - (INVENTORY_TTL_SECS + 1),
            INVENTORY_TTL_SECS,
            now
        ));

        assert!(is_fresh_at(now - (PLAN_TTL_SECS - 1), PLAN_TTL_SECS, now));
        assert!(!is_fresh_at(now - PLAN_TTL_SECS, PLAN_TTL_SECS, now));
        assert!(!is_fresh_at(now - (PLAN_TTL_SECS + 1), PLAN_TTL_SECS, now));
    }
}
