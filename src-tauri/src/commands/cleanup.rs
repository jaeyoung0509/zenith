//! Cleanup scan, private plan, and execution command handlers.
//!
//! Thin Tauri IPC adapters delegating lifecycle, plan storage, and execution
//! to [`crate::services::CleanupService`].

use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::{State, Window};

use super::state::DesktopState;
use crate::events::cleanup::TauriCleanupProgress;
use crate::events::scan::TauriScanProgress;
use crate::models::{
    Category, CleanEvent, CleanResult, CleanupFailure, CleanupProgressSink, PlanPreview,
    PublishedScan, ResumeScanRequest, ScanDiscovery, ScanEvent, ScanProgressSink, ScanRequest,
};

pub use crate::services::select_quick_clean_safe_candidates;

#[tauri::command]
#[specta::specta]
pub async fn start_scan(
    on_event: Channel<ScanEvent>,
    categories: Option<Vec<Category>>,
    window: Window,
    state: State<'_, DesktopState>,
) -> Result<PublishedScan, String> {
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
    if window.label() == "quick" {
        state.cleanup.start_scan_complete(request, progress).await
    } else {
        state.cleanup.start_scan(request, progress).await
    }
}

/// Resumes the one-shot checkpoint owned by the current backend scan.
#[tauri::command]
#[specta::specta]
pub async fn resume_scan(
    on_event: Channel<ScanEvent>,
    scan_id: String,
    continuation_id: String,
    state: State<'_, DesktopState>,
) -> Result<PublishedScan, String> {
    if let Some(refusal) = state.catalog_failure() {
        return Err(refusal);
    }
    let progress: Arc<dyn ScanProgressSink> = Arc::new(TauriScanProgress::new(on_event));
    state
        .cleanup
        .resume_scan(
            ResumeScanRequest {
                scan_id,
                continuation_id,
            },
            progress,
        )
        .await
}

/// Requests cancellation of the scan that reports `scan_id`.
///
/// The id is the one the scan's `Started` event carried, so the interface
/// cancels the scan it is watching rather than one it guesses at. A scan that
/// already finished is not an error: the result is what states whether it was
/// cancelled.
#[tauri::command]
#[specta::specta]
pub fn cancel_scan(scan_id: String, state: State<'_, DesktopState>) -> Result<(), String> {
    state.cleanup.cancel_scan(&scan_id)
}

#[tauri::command]
#[specta::specta]
pub fn get_last_scan(window: Window, state: State<'_, DesktopState>) -> Option<PublishedScan> {
    let published = state.cleanup.get_last_scan()?;
    if window.label() == "quick" && !matches!(published.discovery, ScanDiscovery::Exhausted) {
        return None;
    }
    Some(published)
}

#[tauri::command]
#[specta::specta]
pub async fn create_delete_plan(
    scan_id: String,
    selected_item_ids: Vec<String>,
    state: State<'_, DesktopState>,
) -> Result<PlanPreview, CleanupFailure> {
    state
        .cleanup
        .create_delete_plan(scan_id, selected_item_ids)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn execute_clean(
    plan_id: uuid::Uuid,
    confirmed: bool,
    on_event: Channel<CleanEvent>,
    state: State<'_, DesktopState>,
) -> Result<CleanResult, CleanupFailure> {
    let progress: Arc<dyn CleanupProgressSink> = Arc::new(TauriCleanupProgress::new(on_event));
    state
        .cleanup
        .execute_clean(plan_id, confirmed, progress)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn quick_clean_safe(
    on_event: Channel<CleanEvent>,
    state: State<'_, DesktopState>,
) -> Result<CleanResult, CleanupFailure> {
    let settings = state.settings.snapshot()?;
    let progress: Arc<dyn CleanupProgressSink> = Arc::new(TauriCleanupProgress::new(on_event));
    state.cleanup.quick_clean_safe(&settings, progress).await
}
