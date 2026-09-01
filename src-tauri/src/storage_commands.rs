use crate::applications::{AppInspectionRecord, AppInventory, ApplicationScanner};
use crate::commands::AppState;
use crate::developer_artifacts::{
    result_from_inventory, DeveloperArtifactInventory, DeveloperArtifactScanner,
    DeveloperWorkspaceRecord,
};
use crate::large_files::{LargeFileInventory, LargeFileScanner};
use crate::models::{
    AppUninstallInspection, DeveloperArtifactScanEvent, DeveloperArtifactScanResult,
    DeveloperWorkspace, InstalledApp, LargeFileScanEvent, LargeFileScanRequest,
    LargeFileScanResult, TrashPlanPreview, TrashResult,
};
use crate::trash_manager::{TrashExecutor, TrashPlan, TrashPlanner};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::ipc::Channel;
use tauri::State;

const INVENTORY_TTL_SECS: u64 = 15 * 60;
const PLAN_TTL_SECS: u64 = 5 * 60;

#[derive(Default)]
pub struct StorageWorkflowState {
    pub large_file_inventory: Mutex<Option<LargeFileInventory>>,
    pub large_file_cancel: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub developer_artifact_inventory: Mutex<Option<DeveloperArtifactInventory>>,
    pub developer_artifact_cancel: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub app_inventory: Mutex<Option<AppInventory>>,
    pub app_inspection: Mutex<Option<AppInspectionRecord>>,
    pub trash_plans: Mutex<HashMap<uuid::Uuid, TrashPlan>>,
    pub workspaces: Mutex<HashMap<String, DeveloperWorkspaceRecord>>,
}

impl StorageWorkflowState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cached_developer_artifact_sizes(&self) -> HashMap<PathBuf, u64> {
        let inventory = self
            .developer_artifact_inventory
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .filter(DeveloperArtifactInventory::is_fresh);
        let mut sizes = HashMap::new();
        if let Some(inventory) = inventory {
            for record in inventory.records.values() {
                let total = sizes.entry(record.project_root.clone()).or_insert(0u64);
                *total = total.saturating_add(record.artifact.allocated_bytes);
            }
        }
        sizes
    }

    pub fn store_plan(&self, plan: TrashPlan) {
        let now = unix_timestamp();
        let mut plans = self
            .trash_plans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
}

fn is_fresh_at(created_at: u64, ttl_secs: u64, now: u64) -> bool {
    now.saturating_sub(created_at) < ttl_secs
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
    let storage_state = state.storage_state.clone();
    let worker_storage_state = storage_state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        operation_gate.run(|| {
            let mut emitted_result: Option<LargeFileScanResult> = None;
            let cancel_for_event = cancel.clone();
            let inventory = LargeFileScanner::scan(&request, cancel_for_worker, |event| {
                if let LargeFileScanEvent::Started { scan_id } = &event {
                    worker_storage_state
                        .large_file_cancel
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(scan_id.clone(), cancel_for_event.clone());
                }
                if let LargeFileScanEvent::Finished { result } = &event {
                    emitted_result = Some(result.clone());
                }
                let _ = on_event.send(event);
            })?;
            worker_storage_state
                .large_file_cancel
                .lock()
                .unwrap_or_else(|p| p.into_inner())
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
            *worker_storage_state
                .large_file_inventory
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(inventory);
            Ok::<_, String>(result)
        })
    })
    .await
    .map_err(|_| "Large-file scan worker panicked".to_string())??;
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub async fn pick_developer_workspace(
    state: State<'_, AppState>,
) -> Result<Option<DeveloperWorkspace>, String> {
    let storage_state = state.storage_state.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::developer_artifacts::pick_workspace(&storage_state.workspaces)
    })
    .await
    .map_err(|_| "Developer workspace picker worker panicked".to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn register_developer_home_workspace(
    state: State<'_, AppState>,
) -> Result<DeveloperWorkspace, String> {
    let storage_state = state.storage_state.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::developer_artifacts::register_home_workspace(&storage_state.workspaces)
    })
    .await
    .map_err(|_| "Developer home workspace worker panicked".to_string())?
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
    let storage_state = state.storage_state.clone();
    let worker_storage_state = storage_state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        operation_gate.run(|| {
            let mut emitted_result: Option<DeveloperArtifactScanResult> = None;
            let cancel_for_event = cancel.clone();
            let inventory = DeveloperArtifactScanner::scan(
                &workspace_ids,
                &worker_storage_state.workspaces,
                cancel_for_worker,
                |event| {
                    if let DeveloperArtifactScanEvent::Started { scan_id, .. } = &event {
                        worker_storage_state
                            .developer_artifact_cancel
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .insert(scan_id.clone(), cancel_for_event.clone());
                    }
                    if let DeveloperArtifactScanEvent::Finished { result } = &event {
                        emitted_result = Some(result.clone());
                    }
                    let _ = on_event.send(event);
                },
            )?;
            worker_storage_state
                .developer_artifact_cancel
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&inventory.scan_id);
            let result = emitted_result.unwrap_or_else(|| result_from_inventory(&inventory));
            *worker_storage_state
                .developer_artifact_inventory
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(inventory);
            Ok::<_, String>(result)
        })
    })
    .await
    .map_err(|_| "Developer artifact scan worker panicked".to_string())??;
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub fn cancel_developer_artifact_scan(
    scan_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let cancel = state
        .storage_state
        .developer_artifact_cancel
        .lock()
        .unwrap_or_else(|p| p.into_inner())
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
    state: State<'_, AppState>,
) -> Result<TrashPlanPreview, String> {
    let inventory = state
        .storage_state
        .developer_artifact_inventory
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .filter(|inventory| inventory.scan_id == scan_id)
        .filter(DeveloperArtifactInventory::is_fresh)
        .ok_or_else(|| "Developer artifact inventory expired. Scan again.".to_string())?;
    let plan = TrashPlanner::from_developer_artifacts(&inventory, &selected_item_ids)?;
    let preview = plan.preview();
    state.storage_state.store_plan(plan);
    Ok(preview)
}

#[tauri::command]
#[specta::specta]
pub fn cancel_large_file_scan(scan_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let cancel = state
        .storage_state
        .large_file_cancel
        .lock()
        .unwrap_or_else(|p| p.into_inner())
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
    state: State<'_, AppState>,
) -> Result<TrashPlanPreview, String> {
    let inventory = state
        .storage_state
        .large_file_inventory
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .filter(|inventory| inventory.scan_id == scan_id)
        .filter(|inventory| is_fresh_at(inventory.created_at, INVENTORY_TTL_SECS, unix_timestamp()))
        .ok_or_else(|| "Large-file inventory expired. Scan again.".to_string())?;
    let plan = TrashPlanner::from_large_files(&inventory, &selected_item_ids)?;
    let preview = plan.preview();
    state.storage_state.store_plan(plan);
    Ok(preview)
}

#[tauri::command]
#[specta::specta]
pub async fn get_installed_apps(state: State<'_, AppState>) -> Result<Vec<InstalledApp>, String> {
    let operation_gate = state.storage_operation_gate.clone();
    let storage_state = state.storage_state.clone();
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
    *storage_state
        .app_inventory
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(inventory);
    Ok(apps)
}

#[tauri::command]
#[specta::specta]
pub async fn inspect_app_uninstall(
    app_id: String,
    state: State<'_, AppState>,
) -> Result<AppUninstallInspection, String> {
    let operation_gate = state.storage_operation_gate.clone();
    let storage_state = state.storage_state.clone();
    let worker_storage_state = storage_state.clone();
    let inspection = tauri::async_runtime::spawn_blocking(move || {
        operation_gate.run(|| {
            let inventory = worker_storage_state
                .app_inventory
                .lock()
                .unwrap_or_else(|p| p.into_inner())
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
    *storage_state
        .app_inspection
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(inspection);
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub fn prepare_app_uninstall(
    inspection_id: String,
    selected_related_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<TrashPlanPreview, String> {
    let inspection = state
        .storage_state
        .app_inspection
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .filter(|inspection| inspection.inspection.inspection_id == inspection_id)
        .filter(|inspection| {
            is_fresh_at(inspection.created_at, INVENTORY_TTL_SECS, unix_timestamp())
        })
        .ok_or_else(|| "App uninstall review expired. Review the app again.".to_string())?;
    let plan = TrashPlanner::from_app_inspection(&inspection, &selected_related_ids)?;
    let preview = plan.preview();
    state.storage_state.store_plan(plan);
    Ok(preview)
}

#[tauri::command]
#[specta::specta]
pub async fn execute_trash_plan(
    plan_id: uuid::Uuid,
    state: State<'_, AppState>,
) -> Result<TrashResult, String> {
    let operation_gate = state.storage_operation_gate.clone();
    let storage_state = state.storage_state.clone();
    tauri::async_runtime::spawn_blocking(move || {
        operation_gate.run(|| {
            let plan = storage_state
                .trash_plans
                .lock()
                .unwrap_or_else(|p| p.into_inner())
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
