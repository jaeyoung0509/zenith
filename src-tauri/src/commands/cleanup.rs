//! Cleanup scan, private plan, and execution command handlers.
//!
//! Thin Tauri IPC adapters delegating lifecycle, plan storage, and execution
//! to [`crate::services::CleanupService`].

use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::State;

use super::state::DesktopState;
use crate::events::cleanup::TauriCleanupProgress;
use crate::events::scan::TauriScanProgress;
use crate::models::{
    Category, CleanEvent, CleanResult, CleanupProgressSink, PlanPreview, ScanEvent,
    ScanProgressSink, ScanRequest, ScanResult,
};

pub use crate::services::select_quick_clean_safe_candidates;

#[tauri::command]
#[specta::specta]
pub async fn start_scan(
    on_event: Channel<ScanEvent>,
    categories: Option<Vec<Category>>,
    state: State<'_, DesktopState>,
) -> Result<ScanResult, String> {
    if let Some(refusal) = state.catalog_failure() {
        return Err(refusal);
    }
    let settings = state.settings.snapshot()?;
    let request = ScanRequest {
        categories,
        excluded_signatures: settings.excluded_signatures,
        intensive_cleanup: settings.intensive_cleanup,
    };
    let progress: Arc<dyn ScanProgressSink> = Arc::new(TauriScanProgress::new(on_event));
    state.cleanup.start_scan(request, progress).await
}

#[tauri::command]
#[specta::specta]
pub fn get_last_scan(state: State<'_, DesktopState>) -> Option<ScanResult> {
    state.cleanup.get_last_scan()
}

#[tauri::command]
#[specta::specta]
pub async fn create_delete_plan(
    scan_id: String,
    selected_item_ids: Vec<String>,
    state: State<'_, DesktopState>,
) -> Result<PlanPreview, String> {
    state
        .cleanup
        .create_delete_plan(scan_id, selected_item_ids)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn execute_clean(
    plan_id: uuid::Uuid,
    on_event: Channel<CleanEvent>,
    state: State<'_, DesktopState>,
) -> Result<CleanResult, String> {
    let progress: Arc<dyn CleanupProgressSink> = Arc::new(TauriCleanupProgress::new(on_event));
    state.cleanup.execute_clean(plan_id, progress).await
}

#[tauri::command]
#[specta::specta]
pub async fn quick_clean_safe(
    on_event: Channel<CleanEvent>,
    state: State<'_, DesktopState>,
) -> Result<CleanResult, String> {
    let settings = state.settings.snapshot()?;
    let progress: Arc<dyn CleanupProgressSink> = Arc::new(TauriCleanupProgress::new(on_event));
    state.cleanup.quick_clean_safe(&settings, progress).await
}
