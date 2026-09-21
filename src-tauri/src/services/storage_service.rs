//! Application service for the reviewed storage-management workflows.
//!
//! Large Files, Developer Artifact Review, and App Uninstaller share one
//! lifecycle shape: the backend produces a bounded inventory, the user reviews
//! opaque IDs from it, the backend captures a private one-shot Trash plan, and
//! execution revalidates every target before it moves. That lifecycle lives
//! here, not in the command handlers.
//!
//! The service owns what the issue that introduced it named as application
//! invariants:
//!
//! * the storage operation gate and the execution budgets, so a handler never
//!   decides when a mutation may start;
//! * the ephemeral inventories, the cancellation registries, the reviewed
//!   workspace registry, and the shared one-shot plan store, so no caller can
//!   read or edit that state directly;
//! * the Trash executor, so the OS Trash adapter stays unreachable from every
//!   other module.
//!
//! The command layer keeps the Tauri `Channel` and the capability-free calling
//! convention; the `Channel` is only an outer progress adapter.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use zenith_platform::PlatformEnvironment;

use super::cancellation::CancellationRegistry;
use super::plan_store::{PlanLifecycle, PlanStore};
use super::progress::{DeveloperArtifactScanSink, LargeFileScanSink};
use crate::applications::{AppInspectionRecord, AppInventory, ApplicationScanner};
use crate::developer_artifacts::{
    result_from_inventory, DeveloperArtifactInventory, DeveloperArtifactScanner,
    DeveloperWorkspaceRecord, FolderAccess,
};
use crate::execution_budget::ExecutionBudgets;
use crate::large_files::{LargeFileInventory, LargeFileScanner};
use crate::models::{
    AppUninstallInspection, CapabilityAccess, DeveloperArtifactScanEvent,
    DeveloperArtifactScanResult, DeveloperWorkspace, InstalledAppInventory, LargeFileScanEvent,
    LargeFileScanRequest, LargeFileScanResult, PlatformCapabilitiesProvider, PlatformFeature,
    TrashPlanPreview, TrashResult,
};
use crate::operation_gate::StorageOperationGate;
use crate::trash_manager::{TrashExecutor, TrashPlan, TrashPlanner};

const INVENTORY_TTL_SECS: u64 = 15 * 60;

/// The reviewed-storage workflows' ephemeral state.
///
/// Private to this module: the service is the only reader and writer, so a
/// handler cannot inspect an inventory, expire a plan, or edit a cancellation
/// registry behind the lifecycle.
struct StorageWorkflowState {
    large_file_inventory: Mutex<Option<LargeFileInventory>>,
    large_file_cancel: CancellationRegistry,
    developer_artifact_inventory: Mutex<Option<DeveloperArtifactInventory>>,
    developer_artifact_cancel: CancellationRegistry,
    app_inventory: Mutex<Option<AppInventory>>,
    app_inspection: Mutex<Option<AppInspectionRecord>>,
    /// Reviewed Trash plans, bounded and expiring exactly like cleanup plans.
    trash_plans: PlanStore<TrashPlan>,
    workspaces: Mutex<HashMap<String, DeveloperWorkspaceRecord>>,
}

impl Default for StorageWorkflowState {
    fn default() -> Self {
        Self {
            large_file_inventory: Mutex::new(None),
            large_file_cancel: CancellationRegistry::for_scans(),
            developer_artifact_inventory: Mutex::new(None),
            developer_artifact_cancel: CancellationRegistry::for_scans(),
            app_inventory: Mutex::new(None),
            app_inspection: Mutex::new(None),
            trash_plans: PlanStore::new(PlanLifecycle::trash()),
            workspaces: Mutex::new(HashMap::new()),
        }
    }
}

impl StorageWorkflowState {
    /// Cached per-project artifact sizes for the project views.
    fn cached_developer_artifact_sizes(&self) -> HashMap<PathBuf, u64> {
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

    /// Stores a reviewed Trash plan under the shared bounded lifecycle.
    ///
    /// The store owns TTL, capacity, and eviction, so this workflow cannot
    /// drift from the cleanup plan lifecycle by editing a map directly.
    fn store_plan(&self, plan: TrashPlan) -> Result<(), String> {
        self.trash_plans
            .insert(plan, unix_timestamp())
            .map_err(|error| error.to_string())
    }
}

/// Application service owning every reviewed storage-management workflow.
pub struct StorageService {
    state: Arc<StorageWorkflowState>,
    operation_gate: StorageOperationGate,
    budgets: Arc<ExecutionBudgets>,
    environment: Arc<PlatformEnvironment>,
    trash_executor: Arc<TrashExecutor>,
    system_actions: Arc<dyn zenith_platform::SystemActionProvider>,
    platform_capabilities: Arc<dyn PlatformCapabilitiesProvider>,
}

impl StorageService {
    pub fn new(
        operation_gate: StorageOperationGate,
        budgets: Arc<ExecutionBudgets>,
        environment: Arc<PlatformEnvironment>,
        trash_executor: Arc<TrashExecutor>,
        system_actions: Arc<dyn zenith_platform::SystemActionProvider>,
        platform_capabilities: Arc<dyn PlatformCapabilitiesProvider>,
    ) -> Self {
        Self {
            state: Arc::new(StorageWorkflowState::default()),
            operation_gate,
            budgets,
            environment,
            trash_executor,
            system_actions,
            platform_capabilities,
        }
    }

    /// Per-project measured artifact sizes for the project views.
    ///
    /// A read of cached inventory state, not a new scan: the caller gets the
    /// sizes the last completed inventory measured.
    pub fn cached_developer_artifact_sizes(&self) -> HashMap<PathBuf, u64> {
        self.state.cached_developer_artifact_sizes()
    }

    /// Scans the approved user-content roots for large files.
    ///
    /// The worker acquires the storage read gate and a storage-read budget
    /// permit itself, so the caller never decides when the scan may run.
    pub async fn scan_large_files(
        &self,
        request: LargeFileScanRequest,
        progress: Arc<dyn LargeFileScanSink>,
    ) -> Result<LargeFileScanResult, String> {
        self.platform_capabilities
            .capabilities()
            .require(PlatformFeature::LargeFiles, CapabilityAccess::Inspect)
            .map_err(|error| error.to_string())?;

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_worker = cancel.clone();
        let operation_gate = self.operation_gate.clone();
        let environment = self.environment.clone();
        let worker_state = self.state.clone();
        let permit = self.budgets.acquire_storage_read().await?;

        crate::blocking::run_blocking(
            move || {
                let _permit = permit;
                operation_gate.run_read(|| {
                    let mut emitted_result: Option<LargeFileScanResult> = None;
                    let mut active_scan_id: Option<String> = None;
                    let cancel_for_event = cancel.clone();
                    let inventory_result = LargeFileScanner::scan(
                        &environment,
                        &request,
                        cancel_for_worker,
                        |event| {
                            if let LargeFileScanEvent::Started { scan_id } = &event {
                                active_scan_id = Some(scan_id.clone());
                                worker_state.register_large_file_cancel(
                                    scan_id.clone(),
                                    cancel_for_event.clone(),
                                );
                            }
                            if let LargeFileScanEvent::Finished { result } = &event {
                                emitted_result = Some(result.clone());
                            }
                            progress.emit(event);
                        },
                    );
                    if let Some(scan_id) = active_scan_id.as_deref() {
                        worker_state.remove_large_file_cancel(scan_id);
                    }
                    let inventory = inventory_result?;
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
                    *worker_state
                        .large_file_inventory
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(inventory);
                    Ok::<_, String>(result)
                })
            },
            "Large-file scan worker panicked",
        )
        .await
    }

    /// Asks a running large-file scan to stop.
    pub fn cancel_large_file_scan(&self, scan_id: &str) -> Result<(), String> {
        let cancel = self
            .state
            .large_file_cancel_signal(scan_id)
            .ok_or_else(|| "Large-file scan is no longer running".to_string())?;
        cancel.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Reveals a scanned large file in the platform file manager.
    ///
    /// The caller submits only the item id: the path is resolved from the
    /// backend-owned inventory, so the interface never reassembles a path from
    /// display fields and a stale id cannot point at a path the scan did not
    /// review.
    pub async fn reveal_large_file(&self, item_id: &str) -> Result<(), String> {
        self.platform_capabilities
            .capabilities()
            .require(PlatformFeature::SystemActions, CapabilityAccess::Inspect)
            .map_err(|error| error.to_string())?;

        let inventory = self
            .state
            .large_file_inventory
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .filter(|inventory| {
                zenith_core::domain::is_within_window(
                    inventory.created_at,
                    unix_timestamp(),
                    INVENTORY_TTL_SECS,
                )
            })
            .ok_or_else(|| "Large-file inventory expired. Scan again.".to_string())?;

        let path = inventory
            .records
            .get(item_id)
            .ok_or_else(|| "That item is no longer part of the current scan.".to_string())?
            .path
            .clone();

        if !crate::large_files::is_allowed_large_file_path(&self.environment, &path) {
            return Err("That path is no longer inside an approved folder.".to_string());
        }

        let actions = self.system_actions.clone();
        crate::blocking::run_blocking(
            move || actions.reveal_path(&path),
            "File manager worker panicked",
        )
        .await
    }

    /// Builds a one-shot Trash plan from reviewed large-file IDs.
    pub fn prepare_large_file_trash(
        &self,
        scan_id: &str,
        selected_item_ids: &[String],
    ) -> Result<TrashPlanPreview, String> {
        self.platform_capabilities
            .capabilities()
            .require(PlatformFeature::LargeFiles, CapabilityAccess::Mutate)
            .map_err(|error| error.to_string())?;

        let inventory = self
            .state
            .large_file_inventory
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .filter(|inventory| inventory.scan_id == scan_id)
            .filter(|inventory| {
                zenith_core::domain::is_within_window(
                    inventory.created_at,
                    unix_timestamp(),
                    INVENTORY_TTL_SECS,
                )
            })
            .ok_or_else(|| "Large-file inventory expired. Scan again.".to_string())?;

        self.store_reviewed_plan(TrashPlanner::from_large_files(
            &inventory,
            selected_item_ids,
        )?)
    }

    /// Opens the native folder picker and registers the chosen workspace.
    pub async fn pick_developer_workspace(&self) -> Result<Option<DeveloperWorkspace>, String> {
        let environment = self.environment.clone();
        let worker_state = self.state.clone();
        crate::blocking::run_blocking(
            move || {
                crate::developer_artifacts::pick_workspace(&environment, &worker_state.workspaces)
            },
            "Developer workspace picker worker panicked",
        )
        .await
    }

    /// Registers the canonical current-user home as a review scope.
    pub async fn register_developer_home_workspace(&self) -> Result<DeveloperWorkspace, String> {
        let environment = self.environment.clone();
        let worker_state = self.state.clone();
        crate::blocking::run_blocking(
            move || {
                crate::developer_artifacts::register_home_workspace(
                    &environment,
                    &worker_state.workspaces,
                )
            },
            "Developer home workspace worker panicked",
        )
        .await
    }

    /// Scans the registered workspaces for generated developer artifacts.
    pub async fn scan_developer_artifacts(
        &self,
        workspace_ids: &[String],
        progress: Arc<dyn DeveloperArtifactScanSink>,
    ) -> Result<DeveloperArtifactScanResult, String> {
        self.platform_capabilities
            .capabilities()
            .require(
                PlatformFeature::DeveloperArtifacts,
                CapabilityAccess::Inspect,
            )
            .map_err(|error| error.to_string())?;

        // Resolve the workspace set and answer the Downloads consent prompt
        // before taking the storage read gate. macOS parks the probing thread
        // until the user answers, and every mutating storage command queues
        // behind that gate, so probing inside it would stall unrelated
        // operations on a dialog.
        let workspaces =
            crate::developer_artifacts::workspace_snapshot(workspace_ids, &self.state.workspaces)?;
        let downloads_access = if workspaces.iter().any(|workspace| workspace.whole_home) {
            let environment = self.environment.clone();
            crate::blocking::run_blocking(
                move || -> Result<_, String> {
                    Ok(crate::developer_artifacts::probe_downloads_access(
                        &environment,
                    ))
                },
                "Developer artifact downloads probe panicked",
            )
            .await?
        } else {
            // A scan that never looks at Downloads must not raise the prompt.
            FolderAccess::NotGated
        };

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_worker = cancel.clone();
        let operation_gate = self.operation_gate.clone();
        let environment = self.environment.clone();
        let worker_state = self.state.clone();
        let permit = self.budgets.acquire_storage_read().await?;

        crate::blocking::run_blocking(
            move || {
                let _permit = permit;
                operation_gate.run_read(|| {
                    let mut emitted_result: Option<DeveloperArtifactScanResult> = None;
                    let mut active_scan_id: Option<String> = None;
                    let cancel_for_event = cancel.clone();
                    let inventory_result = DeveloperArtifactScanner::scan_workspaces(
                        &environment,
                        &workspaces,
                        downloads_access,
                        cancel_for_worker,
                        |event| {
                            if let DeveloperArtifactScanEvent::Started { scan_id, .. } = &event {
                                active_scan_id = Some(scan_id.clone());
                                worker_state.register_developer_artifact_cancel(
                                    scan_id.clone(),
                                    cancel_for_event.clone(),
                                );
                            }
                            if let DeveloperArtifactScanEvent::Finished { result } = &event {
                                emitted_result = Some(result.clone());
                            }
                            progress.emit(event);
                        },
                    );
                    if let Some(scan_id) = active_scan_id.as_deref() {
                        worker_state.remove_developer_artifact_cancel(scan_id);
                    }
                    let inventory = inventory_result?;
                    let result =
                        emitted_result.unwrap_or_else(|| result_from_inventory(&inventory));
                    *worker_state
                        .developer_artifact_inventory
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(inventory);
                    Ok::<_, String>(result)
                })
            },
            "Developer artifact scan worker panicked",
        )
        .await
    }

    /// Asks a running developer-artifact scan to stop.
    pub fn cancel_developer_artifact_scan(&self, scan_id: &str) -> Result<(), String> {
        let cancel = self
            .state
            .developer_artifact_cancel_signal(scan_id)
            .ok_or_else(|| "Developer artifact scan is no longer running".to_string())?;
        cancel.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Builds a one-shot Trash plan from reviewed artifact IDs.
    pub fn prepare_developer_artifact_trash(
        &self,
        scan_id: &str,
        selected_item_ids: &[String],
    ) -> Result<TrashPlanPreview, String> {
        self.platform_capabilities
            .capabilities()
            .require(
                PlatformFeature::DeveloperArtifacts,
                CapabilityAccess::Mutate,
            )
            .map_err(|error| error.to_string())?;

        let inventory = self
            .state
            .developer_artifact_inventory
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .filter(|inventory| inventory.scan_id == scan_id)
            .filter(DeveloperArtifactInventory::is_fresh)
            .ok_or_else(|| "Developer artifact inventory expired. Scan again.".to_string())?;

        self.store_reviewed_plan(TrashPlanner::from_developer_artifacts(
            &inventory,
            selected_item_ids,
        )?)
    }

    /// Lists the installed applications the uninstaller may review.
    pub async fn installed_apps(&self) -> Result<InstalledAppInventory, String> {
        self.platform_capabilities
            .capabilities()
            .require(PlatformFeature::InstalledApps, CapabilityAccess::Inspect)
            .map_err(|error| error.to_string())?;

        let operation_gate = self.operation_gate.clone();
        let environment = self.environment.clone();
        let permit = self.budgets.acquire_storage_read().await?;
        let inventory = crate::blocking::run_blocking(
            move || -> Result<_, String> {
                let _permit = permit;
                Ok(operation_gate.run_read(|| ApplicationScanner::scan(&environment)))
            },
            "Application inventory worker panicked",
        )
        .await?;

        let quality = inventory.quality;
        let skipped_entry_count = inventory.skipped_entry_count;
        let incomplete_reasons = inventory.incomplete_reasons.clone();
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
        *self
            .state
            .app_inventory
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(inventory);

        Ok(InstalledAppInventory {
            apps,
            quality,
            skipped_entry_count,
            incomplete_reasons,
        })
    }

    /// Inspects one installed application and the data related to it.
    pub async fn inspect_app_uninstall(
        &self,
        app_id: &str,
    ) -> Result<AppUninstallInspection, String> {
        self.platform_capabilities
            .capabilities()
            .require(PlatformFeature::AppUninstall, CapabilityAccess::Inspect)
            .map_err(|error| error.to_string())?;

        let operation_gate = self.operation_gate.clone();
        let environment = self.environment.clone();
        let app_id = app_id.to_string();
        let permit = self.budgets.acquire_storage_read().await?;
        let inventory = self
            .state
            .app_inventory
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .filter(|inventory| {
                zenith_core::domain::is_within_window(
                    inventory.created_at,
                    unix_timestamp(),
                    INVENTORY_TTL_SECS,
                )
            })
            .ok_or_else(|| "Application inventory expired. Refresh applications.".to_string())?;

        let inspection = crate::blocking::run_blocking(
            move || {
                let _permit = permit;
                operation_gate
                    .run_read(|| ApplicationScanner::inspect(&environment, &inventory, &app_id))
            },
            "App inspection worker panicked",
        )
        .await?;

        let result = inspection.inspection.clone();
        *self
            .state
            .app_inspection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(inspection);
        Ok(result)
    }

    /// Builds a one-shot Trash plan from a reviewed app inspection.
    pub fn prepare_app_uninstall(
        &self,
        inspection_id: &str,
        selected_related_ids: &[String],
    ) -> Result<TrashPlanPreview, String> {
        self.platform_capabilities
            .capabilities()
            .require(PlatformFeature::AppUninstall, CapabilityAccess::Mutate)
            .map_err(|error| error.to_string())?;

        let inspection = self
            .state
            .app_inspection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .filter(|inspection| inspection.inspection.inspection_id == inspection_id)
            .filter(|inspection| {
                zenith_core::domain::is_within_window(
                    inspection.created_at,
                    unix_timestamp(),
                    INVENTORY_TTL_SECS,
                )
            })
            .ok_or_else(|| "App uninstall review expired. Review the app again.".to_string())?;

        self.store_reviewed_plan(TrashPlanner::from_app_inspection(
            &self.environment,
            &inspection,
            selected_related_ids,
        )?)
    }

    /// Executes a reviewed Trash plan exactly once.
    ///
    /// The plan is consumed inside the storage write gate, so two windows
    /// cannot move the same reviewed items and the plan cannot be replayed.
    pub async fn execute_trash_plan(&self, plan_id: uuid::Uuid) -> Result<TrashResult, String> {
        let operation_gate = self.operation_gate.clone();
        let environment = self.environment.clone();
        let executor = self.trash_executor.clone();
        let state = self.state.clone();

        crate::blocking::run_blocking(
            move || {
                execute_trash_plan_in_gate(
                    &state,
                    &operation_gate,
                    &environment,
                    &executor,
                    plan_id,
                    unix_timestamp,
                )
            },
            "Trash execution worker panicked",
        )
        .await
    }

    /// Stores a freshly built plan and returns its preview with the store's TTL.
    ///
    /// The expiry the user sees comes from the same lifecycle that enforces it,
    /// so the displayed window cannot drift from the one that refuses a stale
    /// plan.
    fn store_reviewed_plan(&self, plan: TrashPlan) -> Result<TrashPlanPreview, String> {
        let preview = plan.preview(self.state.trash_plans.ttl_seconds());
        self.state.store_plan(plan)?;
        Ok(preview)
    }
}

fn execute_trash_plan_in_gate(
    state: &StorageWorkflowState,
    operation_gate: &StorageOperationGate,
    environment: &PlatformEnvironment,
    executor: &TrashExecutor,
    plan_id: uuid::Uuid,
    now: impl FnOnce() -> u64,
) -> Result<TrashResult, String> {
    operation_gate.run_write(|| {
        let plan = state
            .trash_plans
            .take_valid(plan_id, now())
            .map_err(|error| error.to_string())?;
        Ok(executor.execute(environment, plan))
    })
}

impl StorageWorkflowState {
    fn register_large_file_cancel(&self, scan_id: String, signal: Arc<AtomicBool>) {
        self.large_file_cancel.register(scan_id, signal);
    }

    fn register_developer_artifact_cancel(&self, scan_id: String, signal: Arc<AtomicBool>) {
        self.developer_artifact_cancel.register(scan_id, signal);
    }

    fn remove_large_file_cancel(&self, scan_id: &str) {
        self.large_file_cancel.remove(scan_id);
    }

    fn remove_developer_artifact_cancel(&self, scan_id: &str) {
        self.developer_artifact_cancel.remove(scan_id);
    }

    fn large_file_cancel_signal(&self, scan_id: &str) -> Option<Arc<AtomicBool>> {
        self.large_file_cancel.signal(scan_id)
    }

    fn developer_artifact_cancel_signal(&self, scan_id: &str) -> Option<Arc<AtomicBool>> {
        self.developer_artifact_cancel.signal(scan_id)
    }
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
    use zenith_platform::path_algebra::PathFlavor;

    /// The storage workflow's side of cancellation: the handle a scan
    /// registers is the one its cancel reaches, and removing it ends that.
    /// The registry's own bounds (TTL, cap, recovery) are tested where the type
    /// lives, in `services::cancellation`.
    #[test]
    fn a_registered_scan_cancel_reaches_the_signal_it_registered() {
        let state = StorageWorkflowState::default();
        let signal = Arc::new(AtomicBool::new(false));
        state.register_large_file_cancel("scan-1".to_string(), signal.clone());

        let reached = state
            .large_file_cancel_signal("scan-1")
            .expect("the registered handle is reachable");
        reached.store(true, Ordering::Relaxed);
        assert!(signal.load(Ordering::Relaxed));

        state.remove_large_file_cancel("scan-1");
        assert!(
            state.large_file_cancel_signal("scan-1").is_none(),
            "a finished scan's handle is removed, not left to expire"
        );
    }

    #[test]
    fn trash_plan_ttl_is_checked_after_waiting_for_the_write_gate() {
        let state = Arc::new(StorageWorkflowState::default());
        let plan_id = uuid::Uuid::new_v4();
        state
            .trash_plans
            .insert(
                TrashPlan {
                    id: plan_id,
                    created_at: 1_000,
                    inventory_id: "inventory".to_string(),
                    targets: Vec::new(),
                },
                1_000,
            )
            .unwrap();

        let operation_gate = StorageOperationGate::default();
        let blocking_gate = operation_gate.clone();
        let (read_entered_tx, read_entered_rx) = mpsc::channel();
        let (release_read_tx, release_read_rx) = mpsc::channel();
        let reader = thread::spawn(move || {
            blocking_gate.run_read(|| {
                read_entered_tx.send(()).unwrap();
                release_read_rx.recv().unwrap();
            });
        });
        read_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("read operation did not acquire the gate");

        let now = Arc::new(AtomicU64::new(1_299));
        let worker_state = state.clone();
        let worker_gate = operation_gate.clone();
        let worker_now = now.clone();
        let backend = Arc::new(zenith_platform::MockTrashBackend::new());
        let worker_backend = backend.clone();
        let (write_attempted_tx, write_attempted_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let environment = PlatformEnvironment::simulated(PathFlavor::current());
            let executor = TrashExecutor::new(worker_backend);
            write_attempted_tx.send(()).unwrap();
            execute_trash_plan_in_gate(
                &worker_state,
                &worker_gate,
                &environment,
                &executor,
                plan_id,
                || worker_now.load(Ordering::SeqCst),
            )
        });

        // The request began while the plan was fresh. It expires while the
        // worker waits for the read operation to release the write gate.
        write_attempted_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("Trash execution did not attempt to enter the write gate");
        now.store(1_300, Ordering::SeqCst);
        release_read_tx.send(()).unwrap();

        let error = worker
            .join()
            .unwrap()
            .expect_err("a plan stale at mutation time must be refused");
        assert!(error.contains("expired"), "unexpected error: {error}");
        assert!(backend.moved().is_empty(), "the Trash port was not reached");
        reader.join().unwrap();
    }
}
