pub mod agent_activity;
pub mod ai_control_center;
pub mod ai_providers;
pub mod ai_snapshots;
pub mod ai_usage;
pub mod applications;
pub mod cache_providers;
pub mod cleaner;
pub mod collection;
pub mod commands;
pub mod dev_ports;
pub mod developer_artifacts;
pub mod diagnostics;
pub mod docker;
pub mod execution_budget;
pub mod ipc_numeric;
pub mod large_files;
pub mod metrics;
pub mod models;
pub mod models_inventory;
pub mod operation_gate;
pub mod orbstack;
pub mod platform;
pub mod power;
pub mod privacy;
pub mod process_owner;
pub mod process_protection;
pub mod runtime_metrics;
pub mod safety;
pub mod scanner;
pub mod settings_store;
pub mod signatures;
pub mod storage_commands;
pub mod tooling;
pub mod trash_manager;

use commands::AppState;
use platform::path_algebra::PathFlavor;
use platform::{NativePlatformCapabilities, PlatformCapabilitiesProvider};
use power::KeepAwakeManager;
use signatures::SignatureRegistry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::utils::config::WindowConfig;
use tauri::utils::TitleBarStyle;
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, Rect, WebviewWindow, WebviewWindowBuilder,
};

/// How long after a dismissal a tray click is treated as part of that same
/// click rather than as a new toggle request.
///
/// Windows delivers the focus loss before the tray mouse-up, so a naive
/// `is_visible()` read on mouse-up re-shows the panel the user just dismissed.
const TRAY_TOGGLE_SUPPRESSION: Duration = Duration::from_millis(400);

/// Adapts a window's declarative configuration to the platform that draws it.
///
/// The overlay title bar and the transparent undecorated quick window are
/// WebKit behaviors. WebView2 draws a native caption bar and does not composite
/// a transparent undecorated window the same way, so the same configuration
/// would reserve dead space and show alpha artifacts on Windows.
fn platform_window_config(mut config: WindowConfig, flavor: PathFlavor) -> WindowConfig {
    if flavor.is_windows() {
        config.title_bar_style = TitleBarStyle::Visible;
        if config.label == "quick" {
            config.transparent = false;
        }
    }
    config
}

pub fn ensure_window(app: &AppHandle, label: &str) -> tauri::Result<WebviewWindow> {
    if let Some(window) = app.get_webview_window(label) {
        return Ok(window);
    }

    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == label)
        .cloned()
        .ok_or_else(|| {
            tauri::Error::AssetNotFound(format!("Window config for {label} not found"))
        })?;
    let config = platform_window_config(config, PathFlavor::current());

    WebviewWindowBuilder::from_config(app, &config)?.build()
}

/// Tracks when the quick panel was last hidden so a tray click that arrives
/// immediately after a dismissal is not mistaken for a request to open it.
#[derive(Default)]
struct QuickPanelVisibility {
    hidden_at: Mutex<Option<Instant>>,
}

impl QuickPanelVisibility {
    fn mark_hidden(&self) {
        let mut hidden_at = self.hidden_at.lock().unwrap_or_else(|p| p.into_inner());
        *hidden_at = Some(Instant::now());
    }

    fn mark_shown(&self) {
        let mut hidden_at = self.hidden_at.lock().unwrap_or_else(|p| p.into_inner());
        *hidden_at = None;
    }

    fn hidden_ago(&self) -> Option<Duration> {
        self.hidden_at
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .map(|hidden_at| hidden_at.elapsed())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayToggle {
    Show,
    Hide,
    /// Consume the click: it belongs to the dismissal that just happened.
    Suppress,
}

fn tray_toggle_action(
    visible: bool,
    hidden_ago: Option<Duration>,
    suppression: Duration,
) -> TrayToggle {
    if visible {
        return TrayToggle::Hide;
    }
    match hidden_ago {
        Some(ago) if ago < suppression => TrayToggle::Suppress,
        _ => TrayToggle::Show,
    }
}

fn hide_quick_panel(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("quick") {
        let _ = window.hide();
    }
    app.state::<QuickPanelVisibility>().mark_hidden();
}

/// Toggles the quick panel in response to an interface request.
///
/// The interface toggle resolves the same tray anchor as a tray click, so the
/// panel is positioned where the menu-bar or notification-area icon is instead
/// of wherever it was last left.
pub fn toggle_quick_panel_from_app(app: &AppHandle) {
    let tray_rect = app
        .tray_by_id("main-tray")
        .and_then(|tray| tray.rect().ok().flatten());
    toggle_quick_panel(app, tray_rect);
}

/// Toggles the quick panel for a user-initiated click, suppressing a click that
/// is really the tail of the dismissal it would otherwise undo.
fn toggle_quick_panel(app: &AppHandle, tray_rect: Option<Rect>) {
    let Ok(window) = ensure_window(app, "quick") else {
        return;
    };
    let visibility = app.state::<QuickPanelVisibility>();
    let visible = window.is_visible().unwrap_or(false);

    match tray_toggle_action(visible, visibility.hidden_ago(), TRAY_TOGGLE_SUPPRESSION) {
        TrayToggle::Hide => {
            let _ = window.hide();
            visibility.mark_hidden();
        }
        TrayToggle::Suppress => {
            // Clear the marker so the next click opens the panel.
            visibility.mark_shown();
        }
        TrayToggle::Show => show_quick_panel_tracked(app, &window, tray_rect),
    }
}

pub fn show_main_window(app: &AppHandle) -> tauri::Result<()> {
    let window = ensure_window(app, "main")?;
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    if app.get_webview_window("quick").is_some() {
        hide_quick_panel(app);
    }
    Ok(())
}

fn tray_anchor(rect: &Rect, scale_factor: f64) -> PhysicalPosition<f64> {
    let position: PhysicalPosition<f64> = rect.position.to_physical(scale_factor);
    let size: tauri::PhysicalSize<f64> = rect.size.to_physical(scale_factor);
    PhysicalPosition::new(position.x + size.width, position.y + size.height)
}

fn monitor_containing_point(
    monitors: &[tauri::Monitor],
    point: PhysicalPosition<f64>,
) -> Option<&tauri::Monitor> {
    monitors.iter().find(|monitor| {
        let origin = monitor.position();
        let bounds = monitor.size();
        point.x >= f64::from(origin.x)
            && point.x < f64::from(origin.x + bounds.width as i32)
            && point.y >= f64::from(origin.y)
            && point.y < f64::from(origin.y + bounds.height as i32)
    })
}

fn quick_panel_position(
    anchor: PhysicalPosition<f64>,
    panel_size: PhysicalSize<u32>,
    work_area_origin: PhysicalPosition<i32>,
    work_area_size: PhysicalSize<u32>,
) -> PhysicalPosition<i32> {
    let max_x = work_area_origin.x + work_area_size.width as i32 - panel_size.width as i32;
    let max_y = work_area_origin.y + work_area_size.height as i32 - panel_size.height as i32;
    let work_area_bottom = work_area_origin.y + work_area_size.height as i32;
    let below = anchor.y.round() as i32 + 6;
    // Open away from the taskbar edge: below the anchor only when the panel
    // fits inside the work area, otherwise above it.
    let y = if below + panel_size.height as i32 <= work_area_bottom {
        below
    } else {
        anchor.y.round() as i32 - panel_size.height as i32 - 6
    };
    PhysicalPosition::new(
        (anchor.x.round() as i32 - panel_size.width as i32)
            .clamp(work_area_origin.x, max_x.max(work_area_origin.x)),
        y.clamp(work_area_origin.y, max_y.max(work_area_origin.y)),
    )
}

fn show_quick_panel(window: &WebviewWindow, tray_rect: Option<Rect>) {
    if let Some(rect) = tray_rect {
        if let Ok(size) = window.outer_size() {
            // Logical tray rectangles need a scale factor before the monitor
            // can be selected. Use the window's factor provisionally, then
            // recompute with the tray monitor's own factor, which differs in
            // mixed-DPI setups.
            let provisional_scale = window.scale_factor().unwrap_or(1.0);
            let provisional_anchor = tray_anchor(&rect, provisional_scale);
            let monitors = window.available_monitors().unwrap_or_default();
            let target = monitor_containing_point(&monitors, provisional_anchor)
                .map(|monitor| {
                    let anchor = tray_anchor(&rect, monitor.scale_factor());
                    let work_area = monitor.work_area();
                    quick_panel_position(anchor, size, work_area.position, work_area.size)
                })
                .unwrap_or_else(|| {
                    PhysicalPosition::new(
                        provisional_anchor.x.round() as i32 - size.width as i32,
                        provisional_anchor.y.round() as i32 + 6,
                    )
                });
            let _ = window.set_position(target);
        }
    }
    let _ = window.show();
    let _ = window.set_focus();
}

/// Shows the quick panel and records that it is open, which clears the
/// dismissal marker a stale tray click would otherwise consume.
fn show_quick_panel_tracked(app: &AppHandle, window: &WebviewWindow, tray_rect: Option<Rect>) {
    show_quick_panel(window, tray_rect);
    app.state::<QuickPanelVisibility>().mark_shown();
}

pub fn run() {
    crate::platform::environment::set_webview_version(tauri::webview_version().ok());
    let environment = Arc::new(crate::platform::PlatformEnvironment::native());
    let registry = Arc::new(SignatureRegistry::load_embedded().unwrap_or_default());
    let awake_manager = Arc::new(KeepAwakeManager::new());
    awake_manager.set_session_validator(crate::agent_activity::has_active_verified_session);
    let settings = Arc::new(Mutex::new(models::ZenithSettings::default()));
    let last_scan = Arc::new(Mutex::new(None));
    let credentials: Arc<dyn crate::ai_providers::CredentialStore> =
        Arc::new(crate::ai_providers::OsCredentialStore::default());
    let ai_collection_service = Arc::new(crate::ai_providers::ProviderCollectionService::default());
    let ai_usage_cache = Arc::new(Mutex::new(None));
    let runtime_metrics = Arc::new(crate::runtime_metrics::RuntimeMetrics::new());
    let usage_singleflight = Arc::new(crate::collection::SingleFlight::with_metrics(
        runtime_metrics.clone(),
    ));
    let usage_generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let delete_plans = Arc::new(Mutex::new(HashMap::new()));
    let storage_operation_gate = operation_gate::StorageOperationGate::default();
    let storage_state = Arc::new(crate::storage_commands::StorageWorkflowState::new());
    let memory_sampler = Arc::new(crate::metrics::MemorySampler::new());
    let memory_termination_store =
        Arc::new(Mutex::new(crate::metrics::MemoryTerminationStore::default()));
    let dev_port_store = Arc::new(Mutex::new(crate::dev_ports::DevelopmentPortStore::default()));
    let agent_activity_cache = Arc::new(Mutex::new(None));
    let activity_singleflight = Arc::new(crate::collection::SingleFlight::with_metrics(
        runtime_metrics.clone(),
    ));
    let activity_generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let execution_budgets = Arc::new(crate::execution_budget::ExecutionBudgets::new());
    let ai_control_state = Arc::new(Mutex::new(
        crate::ai_control_center::state::AiControlCenterState::default(),
    ));
    let ai_control_refresh_lock = Arc::new(Mutex::new(()));
    let ai_control_runtime = Arc::new(crate::ai_control_center::runtime::AiControlRuntime::new(
        memory_sampler.clone(),
        dev_port_store.clone(),
        agent_activity_cache.clone(),
        activity_singleflight.clone(),
        activity_generation.clone(),
        runtime_metrics.clone(),
        ai_control_state.clone(),
        awake_manager.clone(),
        settings.clone(),
    ));
    let platform_capabilities: Arc<dyn PlatformCapabilitiesProvider> =
        Arc::new(NativePlatformCapabilities::new(environment.clone()));
    // The container host is observed once, at the composition root. The
    // adapter never reads the process environment itself.
    let container_host =
        crate::docker::adapter::ContainerHost::from_value(std::env::var("DOCKER_HOST").ok());

    let app_state = AppState {
        environment: environment.clone(),
        container_host,
        registry,
        awake_manager: awake_manager.clone(),
        settings: settings.clone(),
        last_scan,
        credentials,
        ai_collection_service,
        ai_usage_cache,
        usage_singleflight,
        usage_generation,
        delete_plans,
        storage_operation_gate,
        storage_state,
        memory_sampler: memory_sampler.clone(),
        memory_termination_store: memory_termination_store.clone(),
        dev_port_store: dev_port_store.clone(),
        agent_activity_cache: agent_activity_cache.clone(),
        activity_singleflight,
        activity_generation,
        ai_control_state: ai_control_state.clone(),
        ai_control_refresh_lock,
        ai_control_runtime: ai_control_runtime.clone(),
        platform_capabilities,
        runtime_metrics,
        execution_budgets,
        docker_status_cache: Arc::new(Mutex::new(None)),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second launch focuses the existing instance instead of
            // starting a competing process that could corrupt settings.
            let _ = show_main_window(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .manage(app_state)
        .manage(QuickPanelVisibility::default())
        .on_window_event(|window, event| {
            if window.label() != "quick" {
                return;
            }
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = window.hide();
                    window
                        .app_handle()
                        .state::<QuickPanelVisibility>()
                        .mark_hidden();
                }
                // The panel dismisses itself when it loses focus. Recording
                // that here is what makes the following tray mouse-up
                // recognisable as the tail of this dismissal.
                tauri::WindowEvent::Focused(false) => {
                    let _ = window.hide();
                    window
                        .app_handle()
                        .state::<QuickPanelVisibility>()
                        .mark_hidden();
                }
                _ => {}
            }
        })
        .setup(move |app| {
            if let Ok(config_dir) = app.path().app_config_dir() {
                let loaded = settings_store::load(&config_dir);
                app.state::<AppState>()
                    .awake_manager
                    .set_rules(loaded.awake_rules.clone());
                app.state::<AppState>()
                    .awake_manager
                    .set_control_center_awake_policy(
                        loaded.ai_control.autopilot.keep_awake_for_verified_sessions,
                        loaded.ai_control.autopilot.keep_awake_ac_only,
                    );
                *app.state::<AppState>()
                    .settings
                    .lock()
                    .expect("settings poisoned") = loaded;
                app.state::<AppState>()
                    .ai_control_state
                    .lock()
                    .expect("ai control poisoned")
                    .audit = crate::ai_control_center::audit::AuditStore::load(&config_dir);
            }
            let open_dashboard =
                MenuItem::with_id(app, "open_dashboard", "Open Zenith", true, None::<&str>)?;
            let toggle_quick = MenuItem::with_id(
                app,
                "toggle_quick",
                "Toggle Quick Panel",
                true,
                None::<&str>,
            )?;
            let separator = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Zenith", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_dashboard, &toggle_quick, &separator, &quit])?;
            let tray_icon = Image::from_bytes(include_bytes!("../icons/tray-icon.png"))?;

            TrayIconBuilder::with_id("main-tray")
                .icon(tray_icon)
                .icon_as_template(true)
                .tooltip("Zenith - AI & Developer System Manager")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open_dashboard" => {
                        let _ = show_main_window(app);
                    }
                    "toggle_quick" => {
                        let tray_rect = app
                            .tray_by_id("main-tray")
                            .and_then(|tray| tray.rect().ok().flatten());
                        toggle_quick_panel(app, tray_rect);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        rect,
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_quick_panel(tray.app_handle(), Some(rect));
                    }
                })
                .build(app)?;

            let watcher_ref = awake_manager.clone();
            std::thread::spawn(move || loop {
                watcher_ref.wait_for_next_evaluation();
                watcher_ref.evaluate();
            });

            let bg_app = app.handle().clone();
            let bg_runtime = ai_control_runtime.clone();
            std::thread::spawn(move || loop {
                if bg_runtime.are_advisories_enabled() {
                    bg_runtime.tick(Some(&bg_app));
                    bg_runtime.wait_next_tick(std::time::Duration::from_secs(5));
                } else {
                    bg_runtime.wait_next_tick(std::time::Duration::from_secs(60));
                }
            });

            Ok(())
        })
        .invoke_handler(specta_builder().invoke_handler())
        .build(tauri::generate_context!())
        .expect("error while building zenith application")
        .run(|app, event| match event {
            tauri::RunEvent::Ready => {
                let _ = show_main_window(app);
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                let _ = show_main_window(app);
            }
            _ => {}
        });
}

pub fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .dangerously_cast_bigints_to_number()
        .commands(tauri_specta::collect_commands![
            commands::get_ai_usage,
            commands::get_ai_provider_descriptors,
            commands::delete_ai_provider_credential,
            commands::get_project_context,
            commands::request_stop_agent_session,
            commands::get_agent_integrations,
            commands::setup_agent_integration,
            commands::remove_agent_integration,
            commands::get_agent_quick_summary,
            commands::post_agent_event,
            commands::open_in_terminal,
            commands::get_ai_control_center,
            commands::get_ai_control_quick_summary,
            commands::save_ai_control_preferences,
            commands::run_ai_safety_scan,
            commands::dismiss_ai_safety_finding,
            commands::preview_ai_recommendation,
            commands::consume_ai_recommendation_preview,
            commands::get_ai_control_git_diff,
            commands::connect_openrouter_oauth,
            commands::start_scan,
            commands::get_last_scan,
            commands::create_delete_plan,
            commands::execute_clean,
            commands::quick_clean_safe,
            commands::get_memory_metrics,
            commands::terminate_memory_group,
            commands::pick_keep_awake_application,
            commands::get_disk_metrics,
            commands::get_disk_volumes,
            commands::open_storage_settings,
            commands::get_docker_status,
            commands::prune_docker_target,
            commands::get_local_models,
            commands::delete_local_model,
            commands::get_awake_state,
            commands::set_awake_rules,
            commands::set_manual_awake,
            commands::disable_manual_awake,
            commands::get_settings,
            commands::save_settings,
            commands::show_in_file_manager,
            commands::open_dashboard_window,
            commands::get_app_version,
            commands::get_platform_capabilities,
            commands::get_platform_context,
            commands::run_environment_self_check,
            commands::toggle_quick_panel,
            commands::get_diagnostics,
            commands::open_logs_folder,
            commands::list_development_listeners,
            commands::release_development_listener,
            storage_commands::start_large_file_scan,
            storage_commands::cancel_large_file_scan,
            storage_commands::prepare_large_file_trash,
            storage_commands::reveal_large_file,
            storage_commands::pick_developer_workspace,
            storage_commands::register_developer_home_workspace,
            storage_commands::start_developer_artifact_scan,
            storage_commands::cancel_developer_artifact_scan,
            storage_commands::prepare_developer_artifact_cleanup,
            storage_commands::get_installed_apps,
            storage_commands::inspect_app_uninstall,
            storage_commands::prepare_app_uninstall,
            storage_commands::execute_trash_plan,
        ])
}

#[cfg(test)]
mod tests {
    use super::platform_window_config;
    use super::quick_panel_position;
    use super::specta_builder;
    use super::tray_toggle_action;
    use super::QuickPanelVisibility;
    use super::TrayToggle;
    use super::TRAY_TOGGLE_SUPPRESSION;
    use crate::platform::path_algebra::PathFlavor;
    use std::time::Duration;
    use tauri::utils::config::WindowConfig;
    use tauri::utils::TitleBarStyle;
    use tauri::{PhysicalPosition, PhysicalSize};

    #[test]
    fn quick_panel_is_right_aligned_below_tray_icon() {
        let position = quick_panel_position(
            PhysicalPosition::new(1_500.0, 48.0),
            PhysicalSize::new(720, 1_040),
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(3_456, 2_234),
        );
        assert_eq!(position, PhysicalPosition::new(780, 54));
    }

    #[test]
    fn quick_panel_opens_above_the_anchor_when_the_work_area_ends() {
        // The work area excludes a bottom taskbar, so an anchor near the
        // taskbar must place the panel above it instead of under the cursor.
        let position = quick_panel_position(
            PhysicalPosition::new(100.0, 1_900.0),
            PhysicalSize::new(720, 1_040),
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(3_456, 2_234),
        );
        assert_eq!(position, PhysicalPosition::new(0, 854));
    }

    #[test]
    fn quick_panel_respects_a_work_area_origin_on_a_secondary_display() {
        let position = quick_panel_position(
            PhysicalPosition::new(-100.0, 60.0),
            PhysicalSize::new(720, 600),
            PhysicalPosition::new(-1_920, 25),
            PhysicalSize::new(1_920, 1_015),
        );
        assert_eq!(position, PhysicalPosition::new(-820, 66));
    }

    #[test]
    fn windows_window_config_uses_the_native_caption_bar() {
        let config: WindowConfig = serde_json::from_str(
            r#"{
                "label": "main",
                "title": "Zenith",
                "decorations": true,
                "transparent": false,
                "titleBarStyle": "Overlay"
            }"#,
        )
        .expect("parse window config");
        let adapted = platform_window_config(config, PathFlavor::Windows);

        assert_eq!(adapted.title_bar_style, TitleBarStyle::Visible);
        assert!(adapted.decorations);
    }

    #[test]
    fn windows_quick_window_is_not_transparent() {
        let config: WindowConfig = serde_json::from_str(
            r#"{
                "label": "quick",
                "title": "Zenith Quick",
                "decorations": false,
                "transparent": true,
                "alwaysOnTop": true
            }"#,
        )
        .expect("parse window config");
        let adapted = platform_window_config(config, PathFlavor::Windows);

        assert!(
            !adapted.transparent,
            "WebView2 does not composite a transparent undecorated window"
        );
        assert!(!adapted.decorations);
        assert!(adapted.always_on_top);
    }

    #[test]
    fn macos_window_config_is_left_alone() {
        let config: WindowConfig = serde_json::from_str(
            r#"{
                "label": "main",
                "title": "Zenith",
                "decorations": true,
                "transparent": false,
                "titleBarStyle": "Overlay"
            }"#,
        )
        .expect("parse window config");
        let adapted = platform_window_config(config, PathFlavor::Posix);
        assert_eq!(adapted.title_bar_style, TitleBarStyle::Overlay);

        let quick: WindowConfig = serde_json::from_str(
            r#"{"label": "quick", "title": "Zenith Quick", "transparent": true}"#,
        )
        .expect("parse window config");
        assert!(platform_window_config(quick, PathFlavor::Posix).transparent);
    }

    #[test]
    fn a_click_after_a_dismissal_does_not_reopen_the_panel() {
        let suppression = Duration::from_millis(400);

        // Blur dismissal already ran: the click belongs to that dismissal.
        assert_eq!(
            tray_toggle_action(false, Some(Duration::from_millis(20)), suppression),
            TrayToggle::Suppress
        );
        // A click after the suppression window is a genuine open request.
        assert_eq!(
            tray_toggle_action(false, Some(Duration::from_millis(900)), suppression),
            TrayToggle::Show
        );
        // Nothing was hidden recently, so the click opens the panel.
        assert_eq!(
            tray_toggle_action(false, None, suppression),
            TrayToggle::Show
        );
        // A visible panel always hides, however recently it appeared.
        for hidden_ago in [None, Some(Duration::from_millis(1))] {
            assert_eq!(
                tray_toggle_action(true, hidden_ago, suppression),
                TrayToggle::Hide
            );
        }
    }

    #[test]
    fn a_consumed_click_clears_the_dismissal_marker() {
        let visibility = QuickPanelVisibility::default();
        assert!(visibility.hidden_ago().is_none());

        visibility.mark_hidden();
        let hidden_ago = visibility.hidden_ago().expect("just hidden");
        assert!(hidden_ago < Duration::from_secs(5));
        assert_eq!(
            tray_toggle_action(false, Some(hidden_ago), TRAY_TOGGLE_SUPPRESSION),
            TrayToggle::Suppress
        );

        // The suppressed click clears the marker so the next click opens.
        visibility.mark_shown();
        assert!(visibility.hidden_ago().is_none());
        assert_eq!(
            tray_toggle_action(false, visibility.hidden_ago(), TRAY_TOGGLE_SUPPRESSION),
            TrayToggle::Show
        );

        visibility.mark_hidden();
        assert_eq!(
            tray_toggle_action(true, visibility.hidden_ago(), TRAY_TOGGLE_SUPPRESSION),
            TrayToggle::Hide
        );
    }

    #[test]
    #[ignore = "code generation"]
    fn export_typescript_bindings() {
        let builder = specta_builder();
        builder
            .export(
                specta_typescript::Typescript::default(),
                "../src/lib/bindings/tauri.ts",
            )
            .expect("Failed to export TypeScript bindings");
    }

    /// Exports the capability snapshots the frontend renders.
    ///
    /// The frontend consumes this file instead of a hand-written literal, so
    /// its idea of what Windows can do cannot drift from the backend's. The
    /// binding drift gate in CI covers this file at no additional cost.
    #[test]
    #[ignore = "code generation"]
    fn export_platform_capability_golden() {
        use crate::models::{PlatformCapabilities, PlatformKind};
        let golden = serde_json::json!({
            "macos": PlatformCapabilities::macos(),
            "windows": PlatformCapabilities::windows(),
            "linux": PlatformCapabilities::unsupported(PlatformKind::Linux),
            "other": PlatformCapabilities::unsupported(PlatformKind::Other),
        });
        let rendered =
            serde_json::to_string_pretty(&golden).expect("serialize capability golden data") + "\n";
        std::fs::write(
            "../src/lib/bindings/platform-capabilities.golden.json",
            rendered,
        )
        .expect("write capability golden data");
    }
}
