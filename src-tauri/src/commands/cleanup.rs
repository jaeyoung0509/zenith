//! Cleanup scan, private plan, and execution command handlers.
//!
//! Thin Tauri IPC adapters delegating lifecycle, plan storage, and execution
//! to [`crate::services::CleanupService`].

use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::State;

use super::state::AppState;
use super::support::lock_or_state_error;
use crate::models::{
    Category, CleanEvent, CleanResult, PlanPreview, ScanEvent, ScanRequest, ScanResult,
};

pub use crate::services::select_quick_clean_safe_candidates;

#[tauri::command]
#[specta::specta]
pub async fn start_scan(
    on_event: Channel<ScanEvent>,
    categories: Option<Vec<Category>>,
    state: State<'_, AppState>,
) -> Result<ScanResult, String> {
    if let Some(refusal) = state.catalog_failure() {
        return Err(refusal);
    }
    let (excluded_signatures, intensive_cleanup) = {
        let settings = lock_or_state_error(&state.settings, "Settings")?;
        (
            settings.excluded_signatures.clone(),
            settings.intensive_cleanup,
        )
    };
    let request = ScanRequest {
        categories,
        excluded_signatures,
        intensive_cleanup,
    };
    let sink = Arc::new(move |event: ScanEvent| {
        let _ = on_event.send(event);
    });
    state.cleanup_service.start_scan(request, sink).await
}

#[tauri::command]
#[specta::specta]
pub fn get_last_scan(state: State<'_, AppState>) -> Option<ScanResult> {
    state.cleanup_service.get_last_scan()
}

#[tauri::command]
#[specta::specta]
pub async fn create_delete_plan(
    scan_id: String,
    selected_item_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<PlanPreview, String> {
    state
        .cleanup_service
        .create_delete_plan(scan_id, selected_item_ids)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn execute_clean(
    plan_id: uuid::Uuid,
    on_event: Channel<CleanEvent>,
    state: State<'_, AppState>,
) -> Result<CleanResult, String> {
    let sink = Arc::new(move |event: CleanEvent| {
        let _ = on_event.send(event);
    });
    state.cleanup_service.execute_clean(plan_id, sink).await
}

#[tauri::command]
#[specta::specta]
pub async fn quick_clean_safe(
    on_event: Channel<CleanEvent>,
    state: State<'_, AppState>,
) -> Result<CleanResult, String> {
    let settings = {
        let guard = lock_or_state_error(&state.settings, "Settings")?;
        guard.clone()
    };
    let sink = Arc::new(move |event: CleanEvent| {
        let _ = on_event.send(event);
    });
    state
        .cleanup_service
        .quick_clean_safe(&settings, sink)
        .await
}
