//! Cleanup scan, private plan, and execution command handlers.

use super::state::AppState;
use super::support::{run_blocking, unix_timestamp};
use crate::cleaner::CleanExecutor;
use crate::models::{
    Category, CleanEvent, CleanResult, PlanPreview, RiskTier, ScanEvent, ScanResult,
};
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
    let execution_budgets = state.execution_budgets.clone();
    let registry = state.registry.clone();
    let last_scan_store = state.last_scan.clone();
    let operation_gate = state.storage_operation_gate.clone();
    let _permit = execution_budgets.acquire_storage_read().await?;
    let (excluded_signatures, intensive_cleanup) = {
        let settings = state.settings.lock().expect("settings poisoned");
        (
            settings.excluded_signatures.clone(),
            settings.intensive_cleanup,
        )
    };

    let result = tauri::async_runtime::spawn_blocking(move || {
        operation_gate.run_read(|| {
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
        operation_gate.run_write(|| {
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

/// Derives backend-owned Safe-only candidates for Quick Clean.
/// Enforces: risk == Safe, bytes > 0, category enabled in settings.
/// Never includes Rebuild or Manual items.
pub fn select_quick_clean_safe_candidates(
    scan: &ScanResult,
    settings: &crate::models::ZenithSettings,
) -> Vec<String> {
    let mut eligible_ids = Vec::new();
    for category in &scan.categories {
        if !settings.is_category_clean_enabled(category.category) {
            continue;
        }
        for item in &category.items {
            let bytes = item.size.allocated.unwrap_or(item.size.logical);
            if item.risk == RiskTier::Safe && bytes > 0 {
                eligible_ids.push(item.id.clone());
            }
        }
    }
    eligible_ids
}

#[tauri::command]
#[specta::specta]
pub async fn quick_clean_safe(
    on_event: Channel<CleanEvent>,
    state: State<'_, AppState>,
) -> Result<CleanResult, String> {
    let operation_gate = state.storage_operation_gate.clone();
    let registry = state.registry.clone();
    let last_scan_store = state.last_scan.clone();

    // 1. Load current fresh backend scan and sanitized settings
    let (scan, selected_item_ids) = {
        let scan_guard = last_scan_store.lock().expect("last_scan poisoned");
        let scan = scan_guard
            .as_ref()
            .ok_or_else(|| {
                "The scan is no longer current. Scan again before cleaning.".to_string()
            })?
            .clone();

        let now = unix_timestamp();
        scan.validate_for_cleanup(&scan.scan_id, now)
            .map_err(|error| error.to_string())?;

        let settings = state.settings.lock().expect("settings poisoned").clone();
        let eligible_ids = select_quick_clean_safe_candidates(&scan, &settings);

        (scan, eligible_ids)
    };

    if selected_item_ids.is_empty() {
        let now = unix_timestamp();
        return Ok(CleanResult {
            plan_id: uuid::Uuid::new_v4(),
            started_at: now,
            finished_at: now,
            total_reclaimed_bytes: 0,
            total_failed_bytes: 0,
            items: vec![],
            actual_disk_free_delta: Some(0),
        });
    }

    // 2. Build the SafetyPlanner plan from trusted Safe-only IDs
    let plan =
        SafetyPlanner::create_plan_from_scan(&scan, &scan.scan_id, &selected_item_ids, &registry)
            .map_err(|error| error.to_string())?;

    // 3. Execute through the operation gate, invalidating last_scan
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<CleanResult, String> {
        operation_gate.run_write(|| {
            {
                let mut current_scan = last_scan_store.lock().expect("last_scan poisoned");
                let scan_ref = current_scan.as_ref().ok_or_else(|| {
                    "The scan is no longer current. Scan again before cleaning.".to_string()
                })?;
                scan_ref
                    .validate_for_cleanup(&plan.scan_id, unix_timestamp())
                    .map_err(|error| error.to_string())?;
                // Invalidate scan so other windows cannot create plans from stale scan
                *current_scan = None;
            }

            Ok(CleanExecutor::execute(plan, |event| {
                let _ = on_event.send(event);
            }))
        })
    })
    .await
    .map_err(|_| "Quick clean worker thread panicked".to_string())??;

    Ok(result)
}
