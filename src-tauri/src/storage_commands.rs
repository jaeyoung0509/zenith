use crate::applications::{AppInspectionRecord, AppInventory, ApplicationScanner};
use crate::large_files::{LargeFileInventory, LargeFileScanner};
use crate::models::{
    AppUninstallInspection, InstalledApp, LargeFileScanEvent, LargeFileScanRequest,
    LargeFileScanResult, TrashPlanPreview, TrashResult,
};
use crate::trash_manager::{TrashExecutor, TrashPlan, TrashPlanner};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::ipc::Channel;

static LARGE_FILE_INVENTORY: LazyLock<Mutex<Option<LargeFileInventory>>> =
    LazyLock::new(|| Mutex::new(None));
static LARGE_FILE_CANCEL: LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static APP_INVENTORY: LazyLock<Mutex<Option<AppInventory>>> = LazyLock::new(|| Mutex::new(None));
static APP_INSPECTION: LazyLock<Mutex<Option<AppInspectionRecord>>> =
    LazyLock::new(|| Mutex::new(None));
static TRASH_PLANS: LazyLock<Mutex<HashMap<uuid::Uuid, TrashPlan>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static STORAGE_OPERATION_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

const INVENTORY_TTL_SECS: u64 = 15 * 60;
const PLAN_TTL_SECS: u64 = 5 * 60;

#[tauri::command]
#[specta::specta]
pub async fn start_large_file_scan(
    request: LargeFileScanRequest,
    on_event: Channel<LargeFileScanEvent>,
) -> Result<LargeFileScanResult, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_worker = cancel.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _guard = STORAGE_OPERATION_LOCK.lock().unwrap();
        let mut emitted_result: Option<LargeFileScanResult> = None;
        let inventory = LargeFileScanner::scan(&request, cancel_for_worker, |event| {
            if let LargeFileScanEvent::Started { scan_id } = &event {
                LARGE_FILE_CANCEL
                    .lock()
                    .unwrap()
                    .insert(scan_id.clone(), cancel.clone());
            }
            if let LargeFileScanEvent::Finished { result } = &event {
                emitted_result = Some(result.clone());
            }
            let _ = on_event.send(event);
        })?;
        LARGE_FILE_CANCEL.lock().unwrap().remove(&inventory.scan_id);
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
        *LARGE_FILE_INVENTORY.lock().unwrap() = Some(inventory);
        Ok::<_, String>(result)
    })
    .await
    .map_err(|_| "Large-file scan worker panicked".to_string())??;
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub fn cancel_large_file_scan(scan_id: String) -> Result<(), String> {
    let cancel = LARGE_FILE_CANCEL
        .lock()
        .unwrap()
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
        .unwrap()
        .clone()
        .filter(|inventory| inventory.scan_id == scan_id)
        .filter(|inventory| {
            unix_timestamp().saturating_sub(inventory.created_at) < INVENTORY_TTL_SECS
        })
        .ok_or_else(|| "Large-file inventory expired. Scan again.".to_string())?;
    let plan = TrashPlanner::from_large_files(&inventory, &selected_item_ids)?;
    let preview = plan.preview();
    store_plan(plan);
    Ok(preview)
}

#[tauri::command]
#[specta::specta]
pub async fn get_installed_apps() -> Result<Vec<InstalledApp>, String> {
    let inventory = tauri::async_runtime::spawn_blocking(|| {
        let _guard = STORAGE_OPERATION_LOCK.lock().unwrap();
        ApplicationScanner::scan()
    })
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
    *APP_INVENTORY.lock().unwrap() = Some(inventory);
    Ok(apps)
}

#[tauri::command]
#[specta::specta]
pub async fn inspect_app_uninstall(app_id: String) -> Result<AppUninstallInspection, String> {
    let inventory = APP_INVENTORY
        .lock()
        .unwrap()
        .clone()
        .filter(|inventory| {
            unix_timestamp().saturating_sub(inventory.created_at) < INVENTORY_TTL_SECS
        })
        .ok_or_else(|| "Application inventory expired. Refresh applications.".to_string())?;
    let inspection = tauri::async_runtime::spawn_blocking(move || {
        let _guard = STORAGE_OPERATION_LOCK.lock().unwrap();
        ApplicationScanner::inspect(&inventory, &app_id)
    })
    .await
    .map_err(|_| "App inspection worker panicked".to_string())??;
    let result = inspection.inspection.clone();
    *APP_INSPECTION.lock().unwrap() = Some(inspection);
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
        .unwrap()
        .clone()
        .filter(|inspection| inspection.inspection.inspection_id == inspection_id)
        .filter(|inspection| {
            unix_timestamp().saturating_sub(inspection.created_at) < INVENTORY_TTL_SECS
        })
        .ok_or_else(|| "App uninstall review expired. Review the app again.".to_string())?;
    let plan = TrashPlanner::from_app_inspection(&inspection, &selected_related_ids)?;
    let preview = plan.preview();
    store_plan(plan);
    Ok(preview)
}

#[tauri::command]
#[specta::specta]
pub async fn execute_trash_plan(plan_id: uuid::Uuid) -> Result<TrashResult, String> {
    let plan = TRASH_PLANS
        .lock()
        .unwrap()
        .remove(&plan_id)
        .ok_or_else(|| "Trash plan not found or already used".to_string())?;
    if plan.is_expired() {
        return Err("Trash plan expired. Review the items again.".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = STORAGE_OPERATION_LOCK.lock().unwrap();
        TrashExecutor::execute(plan)
    })
    .await
    .map_err(|_| "Trash execution worker panicked".to_string())
}

fn store_plan(plan: TrashPlan) {
    let now = unix_timestamp();
    let mut plans = TRASH_PLANS.lock().unwrap();
    plans.retain(|_, plan| now.saturating_sub(plan.created_at) < PLAN_TTL_SECS);
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
