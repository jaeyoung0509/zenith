//! Cleanup scan, private plan, and execution command handlers.

use super::state::AppState;
use super::support::{run_blocking, unix_timestamp};
use crate::cleaner::CleanExecutor;
use crate::models::{Category, CleanEvent, CleanResult, PlanPreview, ScanEvent, ScanResult};
use crate::safety::SafetyPlanner;
use crate::scanner::ScanEngine;
use tauri::ipc::Channel;
use tauri::State;

#[tauri::command]
#[specta::specta]
pub async fn start_scan(
    on_event: Channel<ScanEvent>,
    categories: Option<Vec<Category>>,
    state: State<'_, AppState>,
) -> Result<ScanResult, String> {
    let registry = state.registry.clone();
    let last_scan_store = state.last_scan.clone();
    let operation_gate = state.storage_operation_gate.clone();
    let (excluded_signatures, intensive_cleanup) = {
        let settings = state.settings.lock().expect("settings poisoned");
        (
            settings.excluded_signatures.clone(),
            settings.intensive_cleanup,
        )
    };

    let result = tauri::async_runtime::spawn_blocking(move || {
        operation_gate.run(|| {
            let cat_ref = categories.as_deref();
            let result = ScanEngine::scan(
                &registry,
                cat_ref,
                &excluded_signatures,
                intensive_cleanup,
                |event| {
                    let _ = on_event.send(event);
                },
            );
            *last_scan_store.lock().expect("last_scan poisoned") = Some(result.clone());
            result
        })
    })
    .await
    .map_err(|_| "Scan worker thread panicked".to_string())?;

    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub fn get_last_scan(state: State<'_, AppState>) -> Option<ScanResult> {
    state.last_scan.lock().expect("mutex poisoned").clone()
}

#[tauri::command]
#[specta::specta]
pub async fn create_delete_plan(
    scan_id: String,
    selected_item_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<PlanPreview, String> {
    const PLAN_TTL_SECS: u64 = 300;
    let last_scan = state.last_scan.clone();
    let registry = state.registry.clone();
    let delete_plans = state.delete_plans.clone();
    run_blocking(
        move || {
            let scan = last_scan
                .lock()
                .expect("last_scan poisoned")
                .clone()
                .filter(|scan| scan.scan_id == scan_id)
                .ok_or_else(|| {
                    "The scan is no longer current. Scan again before cleaning.".to_string()
                })?;

            let plan = SafetyPlanner::create_plan_from_scan(
                &scan,
                &scan_id,
                &selected_item_ids,
                &registry,
            )
            .map_err(|error| error.to_string())?;
            let mut preview = plan.preview(PLAN_TTL_SECS);
            preview.expires_at = preview.expires_at.min(
                scan.finished_at
                    .saturating_add(u64::from(ScanResult::VALID_FOR_SECONDS)),
            );
            let now = unix_timestamp();
            let mut plans = delete_plans.lock().expect("delete_plans poisoned");
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
        },
        "Delete plan worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn execute_clean(
    plan_id: uuid::Uuid,
    on_event: Channel<CleanEvent>,
    state: State<'_, AppState>,
) -> Result<CleanResult, String> {
    const PLAN_TTL_SECS: u64 = 300;
    let operation_gate = state.storage_operation_gate.clone();
    let plans = state.delete_plans.clone();
    let last_scan = state.last_scan.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<CleanResult, String> {
        operation_gate.run(|| {
            let plan = plans
                .lock()
                .expect("delete_plans poisoned")
                .remove(&plan_id)
                .ok_or_else(|| "Delete plan not found or already used".to_string())?;
            if unix_timestamp()
                .checked_sub(plan.created_at)
                .is_none_or(|age| age >= PLAN_TTL_SECS)
            {
                return Err("Delete plan expired. Scan again before cleaning.".to_string());
            }
            {
                let mut scan = last_scan.lock().expect("last_scan poisoned");
                scan.as_ref()
                    .ok_or_else(|| {
                        "The scan is no longer current. Scan again before cleaning.".to_string()
                    })?
                    .validate_for_cleanup(&plan.scan_id, unix_timestamp())
                    .map_err(|error| error.to_string())?;
                // Even a partial cleanup changes the observation. Other windows
                // must not create another plan from the pre-cleanup inventory.
                *scan = None;
            }
            Ok(CleanExecutor::execute(plan, |event| {
                let _ = on_event.send(event);
            }))
        })
    })
    .await
    .map_err(|_| "Clean execution thread panicked".to_string())??;

    Ok(result)
}
