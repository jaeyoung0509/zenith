//! System metrics, preferences, Keep Awake, diagnostics, and development commands.
//!
//! Thin Tauri IPC adapters. The capability gates, execution budgets, storage
//! operation gate, Docker status observation, memory leases, development-port
//! store, and settings persistence belong to
//! [`crate::services::SystemService`], so no handler decides when a mutation
//! may start or writes shared state itself. Window, tray, and version commands
//! stay here: they are the desktop shell's own surface, not application use
//! cases.

use tauri::{AppHandle, Manager, State};

use super::state::DesktopState;
use crate::blocking::run_blocking;
use crate::events::notifications::TauriNotifications;
use crate::models::{
    AwakeBehavior, AwakeRule, AwakeState, DevelopmentListener, DiagnosticsSnapshot, DiskMetrics,
    DiskVolume, DockerStatus, LocalModelInventory, MemoryMetrics, MemoryTerminationMode,
    MemoryTerminationResult, PlatformCapabilities, PlatformContext,
    ReleaseDevelopmentListenerResult, ReleaseMode, SelectedApplication, ZenithSettings,
};

#[tauri::command]
#[specta::specta]
pub async fn get_memory_metrics(state: State<'_, DesktopState>) -> Result<MemoryMetrics, String> {
    state.system.memory_metrics().await
}

#[tauri::command]
#[specta::specta]
pub async fn terminate_memory_group(
    lease_id: String,
    mode: MemoryTerminationMode,
    state: State<'_, DesktopState>,
) -> Result<MemoryTerminationResult, String> {
    state.system.terminate_memory_group(&lease_id, mode).await
}

#[tauri::command]
#[specta::specta]
pub async fn pick_keep_awake_application() -> Result<Option<SelectedApplication>, String> {
    run_blocking(
        crate::power::ApplicationPicker::pick,
        "Keep Awake application picker worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn get_disk_metrics(state: State<'_, DesktopState>) -> Result<DiskMetrics, String> {
    state.system.disk_metrics().await
}

#[tauri::command]
#[specta::specta]
pub async fn get_disk_volumes(state: State<'_, DesktopState>) -> Result<Vec<DiskVolume>, String> {
    state.system.disk_volumes().await
}

#[tauri::command]
#[specta::specta]
pub async fn open_storage_settings() -> Result<(), String> {
    run_blocking(
        || {
            use zenith_platform::SystemActionProvider;
            zenith_platform::NativeSystemActions::new().open_storage_settings()
        },
        "Storage settings worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn get_docker_status(state: State<'_, DesktopState>) -> Result<DockerStatus, String> {
    state.system.docker_status().await
}

#[tauri::command]
#[specta::specta]
pub async fn prune_docker_target(
    signature_id: String,
    state: State<'_, DesktopState>,
) -> Result<u64, String> {
    state.system.prune_docker_target(&signature_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_local_models(
    state: State<'_, DesktopState>,
) -> Result<LocalModelInventory, String> {
    state.system.local_models().await
}

#[tauri::command]
#[specta::specta]
/// Deletes one local model and reports the bytes it reclaimed.
/// `None` means the model was deleted but the reclaimed amount could not be
/// measured completely; the interface must not present that as a number.
pub async fn delete_local_model(
    model_id: String,
    state: State<'_, DesktopState>,
) -> Result<crate::ipc_numeric::IpcOptionalU64, String> {
    state.system.delete_local_model(&model_id).await
}

#[tauri::command]
#[specta::specta]
pub fn get_awake_state(state: State<'_, DesktopState>) -> Result<AwakeState, String> {
    Ok(state.system.awake_state())
}

#[tauri::command]
#[specta::specta]
pub async fn set_awake_rules(
    rules: Vec<AwakeRule>,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    state.system.set_awake_rules(rules).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_manual_awake(
    duration_secs: Option<u64>,
    behavior: AwakeBehavior,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    state.system.set_manual_awake(duration_secs, behavior).await
}

#[tauri::command]
#[specta::specta]
pub async fn disable_manual_awake(state: State<'_, DesktopState>) -> Result<(), String> {
    state.system.disable_manual_awake().await
}

#[tauri::command]
#[specta::specta]
pub fn get_settings(state: State<'_, DesktopState>) -> Result<ZenithSettings, String> {
    state.system.settings()
}

#[tauri::command]
#[specta::specta]
pub async fn save_settings(
    settings: ZenithSettings,
    app_handle: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    let config_dir = app_handle
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    let notifications = TauriNotifications::new(app_handle);
    state
        .system
        .save_settings(&config_dir, settings, &notifications)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn show_in_file_manager(
    path: String,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    state.system.reveal_path(&path).await
}

#[tauri::command]
#[specta::specta]
pub async fn open_in_terminal(path: String, state: State<'_, DesktopState>) -> Result<(), String> {
    state.system.open_terminal(&path).await
}

#[tauri::command]
#[specta::specta]
pub fn open_dashboard_window(app_handle: AppHandle) -> Result<(), String> {
    crate::show_main_window(&app_handle).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
#[specta::specta]
pub fn get_platform_capabilities(state: State<'_, DesktopState>) -> PlatformCapabilities {
    state.system.capabilities()
}

/// Platform vocabulary and locations the interface renders.
///
/// Copy such as "Move to Trash", "menu bar", or a log directory literal is only
/// true on one platform; the frontend asks for it instead of hardcoding it.
/// Runs the `--doctor` self-check against the running environment.
///
/// The command line and the interface execute the same assertions: a user who
/// cannot open a terminal can still see which invariant failed.
#[tauri::command]
#[specta::specta]
pub async fn run_environment_self_check(
    state: State<'_, DesktopState>,
) -> Result<crate::diagnostics::doctor::EnvironmentReport, String> {
    state.system.environment_self_check().await
}

#[tauri::command]
#[specta::specta]
pub fn get_platform_context() -> PlatformContext {
    PlatformContext::current(crate::diagnostics::log_directory_display())
}

#[tauri::command]
#[specta::specta]
pub fn toggle_quick_panel(app_handle: AppHandle) -> Result<(), String> {
    crate::toggle_quick_panel_from_app(&app_handle);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_diagnostics(
    app_handle: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<DiagnosticsSnapshot, String> {
    let config_dir = app_handle
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    state.system.diagnostics(&config_dir).await
}

#[tauri::command]
#[specta::specta]
pub async fn open_logs_folder() -> Result<(), String> {
    run_blocking(
        crate::diagnostics::open_logs_folder,
        "Logs folder worker panicked",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn list_development_listeners(
    state: State<'_, DesktopState>,
) -> Result<Vec<DevelopmentListener>, String> {
    state.system.development_listeners().await
}

#[tauri::command]
#[specta::specta]
pub async fn release_development_listener(
    id: String,
    mode: ReleaseMode,
    state: State<'_, DesktopState>,
) -> Result<ReleaseDevelopmentListenerResult, String> {
    state.system.release_development_listener(&id, mode).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_version_is_semver_formatted() {
        let version = get_app_version();
        assert!(!version.is_empty());
        let parts: Vec<&str> = version.split('.').collect();
        assert_eq!(parts.len(), 3, "Expected major.minor.patch semver format");
        for part in parts {
            assert!(
                part.chars().all(|c| c.is_ascii_digit()),
                "Expected numeric version segments"
            );
        }
    }
}
