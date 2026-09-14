//! Reviewed storage-management command handlers.
//!
//! Thin Tauri IPC adapters: each handler adapts a `Channel` into the service's
//! progress callback, or forwards the opaque identifiers the interface is
//! allowed to submit. Inventory lifetime, the storage operation gate, the
//! execution budgets, the plan store, and the Trash executor belong to
//! [`crate::services::StorageService`], so no handler decides when a mutation
//! may start or touches backend state directly.

use tauri::ipc::Channel;
use tauri::State;

use crate::commands::AppState;
use crate::models::{
    AppUninstallInspection, DeveloperArtifactScanEvent, DeveloperArtifactScanResult,
    DeveloperWorkspace, InstalledAppInventory, LargeFileScanEvent, LargeFileScanRequest,
    LargeFileScanResult, TrashPlanPreview, TrashResult,
};

#[tauri::command]
#[specta::specta]
pub async fn start_large_file_scan(
    request: LargeFileScanRequest,
    on_event: Channel<LargeFileScanEvent>,
    state: State<'_, AppState>,
) -> Result<LargeFileScanResult, String> {
    state
        .storage_service
        .scan_large_files(request, move |event| {
            let _ = on_event.send(event);
        })
        .await
}

#[tauri::command]
#[specta::specta]
pub fn cancel_large_file_scan(scan_id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.storage_service.cancel_large_file_scan(&scan_id)
}

/// Reveals a scanned large file in the platform file manager.
///
/// The interface submits only the item id: the path is resolved from the
/// backend-owned inventory, so the frontend never has to reassemble a path from
/// display fields (which on Windows produced mixed separators) and a stale id
/// cannot point at a path the scan did not review.
#[tauri::command]
#[specta::specta]
pub async fn reveal_large_file(item_id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.storage_service.reveal_large_file(&item_id).await
}

#[tauri::command]
#[specta::specta]
pub fn prepare_large_file_trash(
    scan_id: String,
    selected_item_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<TrashPlanPreview, String> {
    state
        .storage_service
        .prepare_large_file_trash(&scan_id, &selected_item_ids)
}

#[tauri::command]
#[specta::specta]
pub async fn pick_developer_workspace(
    state: State<'_, AppState>,
) -> Result<Option<DeveloperWorkspace>, String> {
    state.storage_service.pick_developer_workspace().await
}

#[tauri::command]
#[specta::specta]
pub async fn register_developer_home_workspace(
    state: State<'_, AppState>,
) -> Result<DeveloperWorkspace, String> {
    state
        .storage_service
        .register_developer_home_workspace()
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn start_developer_artifact_scan(
    workspace_ids: Vec<String>,
    on_event: Channel<DeveloperArtifactScanEvent>,
    state: State<'_, AppState>,
) -> Result<DeveloperArtifactScanResult, String> {
    state
        .storage_service
        .scan_developer_artifacts(&workspace_ids, move |event| {
            let _ = on_event.send(event);
        })
        .await
}

#[tauri::command]
#[specta::specta]
pub fn cancel_developer_artifact_scan(
    scan_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .storage_service
        .cancel_developer_artifact_scan(&scan_id)
}

#[tauri::command]
#[specta::specta]
pub fn prepare_developer_artifact_cleanup(
    scan_id: String,
    selected_item_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<TrashPlanPreview, String> {
    state
        .storage_service
        .prepare_developer_artifact_trash(&scan_id, &selected_item_ids)
}

#[tauri::command]
#[specta::specta]
pub async fn get_installed_apps(
    state: State<'_, AppState>,
) -> Result<InstalledAppInventory, String> {
    state.storage_service.installed_apps().await
}

#[tauri::command]
#[specta::specta]
pub async fn inspect_app_uninstall(
    app_id: String,
    state: State<'_, AppState>,
) -> Result<AppUninstallInspection, String> {
    state.storage_service.inspect_app_uninstall(&app_id).await
}

#[tauri::command]
#[specta::specta]
pub fn prepare_app_uninstall(
    inspection_id: String,
    selected_related_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<TrashPlanPreview, String> {
    state
        .storage_service
        .prepare_app_uninstall(&inspection_id, &selected_related_ids)
}

#[tauri::command]
#[specta::specta]
pub async fn execute_trash_plan(
    plan_id: uuid::Uuid,
    state: State<'_, AppState>,
) -> Result<TrashResult, String> {
    state.storage_service.execute_trash_plan(plan_id).await
}
