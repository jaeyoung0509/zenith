pub mod agent_activity;
pub mod ai_control_center;
pub mod ai_providers;
pub mod ai_snapshots;
pub mod ai_usage;
pub mod applications;
pub mod blocking;
pub mod cache_providers;
pub mod cleaner;
pub mod collection;
pub mod commands;
pub mod composition;
pub mod dev_ports;
pub mod developer_artifacts;
pub mod diagnostics;
pub mod docker;
pub mod events;
pub mod execution_budget;
pub mod git;
pub mod hash;
// The wire rule for a `u64` that crosses IPC is a property of the contract,
// not of the desktop adapter, so it is defined in `zenith_core` and re-exported
// here: `#[serde(with = "crate::ipc_numeric::u64")]` keeps working in both
// crates against the single definition.
pub use zenith_core::ipc_numeric;
pub mod large_files;
pub mod metrics;
pub mod models;
pub mod models_inventory;
pub mod operation_gate;
pub mod orbstack;
pub mod power;
pub mod privacy;
pub mod process_owner;
pub mod process_protection;
pub mod runtime_health;
pub mod runtime_metrics;
pub mod safety;
pub mod scanner;
pub mod services;
pub mod settings_store;
pub mod signatures;
pub mod storage_commands;
pub mod tooling;
pub mod trash_manager;

use commands::DesktopState;
use std::sync::{Arc, Mutex};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::utils::config::WindowConfig;
use tauri::utils::TitleBarStyle;
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, Rect, WebviewWindow, WebviewWindowBuilder,
};
use zenith_platform::path_algebra::PathFlavor;

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

/// Tracks a pending trailing tray release so a tray click that dismisses the
/// quick panel does not immediately reopen it on mouse-up.
#[derive(Default)]
struct QuickPanelVisibility {
    pending_tray_mouse_up: Mutex<bool>,
}

impl QuickPanelVisibility {
    fn arm_pending_tray_mouse_up(&self) {
        let mut pending = self
            .pending_tray_mouse_up
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *pending = true;
    }

    fn clear_pending_tray_mouse_up(&self) {
        let mut pending = self
            .pending_tray_mouse_up
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *pending = false;
    }

    /// Consumes and clears the pending trailing release flag, returning true if one was armed.
    fn take_pending_tray_mouse_up(&self) -> bool {
        let mut pending = self
            .pending_tray_mouse_up
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let was_pending = *pending;
        *pending = false;
        was_pending
    }

    fn on_tray_mouse_down(&self, visible: bool) {
        if visible {
            self.arm_pending_tray_mouse_up();
        } else {
            self.clear_pending_tray_mouse_up();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayToggle {
    Show,
    Hide,
}

fn tray_toggle_action(visible: bool) -> TrayToggle {
    if visible {
        TrayToggle::Hide
    } else {
        TrayToggle::Show
    }
}

fn hide_quick_panel(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("quick") {
        let _ = window.hide();
    }
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

/// Toggles the quick panel between shown and hidden.
fn toggle_quick_panel(app: &AppHandle, tray_rect: Option<Rect>) {
    let window = match ensure_window(app, "quick") {
        Ok(window) => window,
        Err(error) => {
            crate::diagnostics::report_startup_failure(
                "The quick panel could not be created",
                &error.to_string(),
            );
            return;
        }
    };
    let visible = window.is_visible().unwrap_or(false);

    match tray_toggle_action(visible) {
        TrayToggle::Hide => {
            let _ = window.hide();
        }
        TrayToggle::Show => {
            if let Err(error) = show_quick_panel_tracked(app, &window, tray_rect) {
                crate::diagnostics::report_startup_failure(
                    "The quick panel could not be shown",
                    &error.to_string(),
                );
            }
        }
    }
}

/// Handles tray mouse-up events. If this mouse-up is the tail of a tray press
/// that already dismissed the panel on mouse-down, it is suppressed.
fn handle_tray_mouse_up(app: &AppHandle, tray_rect: Option<Rect>) {
    let visibility = app.state::<QuickPanelVisibility>();
    if visibility.take_pending_tray_mouse_up() {
        return;
    }
    toggle_quick_panel(app, tray_rect);
}

/// Shows the main window, reporting a failure where the user can see it.
///
/// Every user-initiated path goes through this: dropping the error would leave
/// a machine whose webview runtime is missing, too old, or policy-blocked with
/// a tray icon and no explanation.
pub fn show_main_window_or_report(app: &AppHandle) {
    if let Err(error) = show_main_window(app) {
        crate::diagnostics::report_startup_failure(
            "The main window could not be created",
            &error.to_string(),
        );
    }
}

pub fn show_main_window(app: &AppHandle) -> tauri::Result<()> {
    let window = ensure_window(app, "main")?;
    let _ = window.unminimize();
    window.show()?;
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

fn show_quick_panel(window: &WebviewWindow, tray_rect: Option<Rect>) -> tauri::Result<()> {
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
    window.show()?;
    let _ = window.set_focus();
    Ok(())
}

/// Shows the quick panel and clears any pending trailing release.
fn show_quick_panel_tracked(
    app: &AppHandle,
    window: &WebviewWindow,
    tray_rect: Option<Rect>,
) -> tauri::Result<()> {
    show_quick_panel(window, tray_rect)?;
    app.state::<QuickPanelVisibility>()
        .clear_pending_tray_mouse_up();
    Ok(())
}

pub fn run() {
    zenith_platform::subprocess::set_error_sink(|message| {
        crate::diagnostics::log_error("subprocess", message)
    });
    zenith_platform::environment::set_webview_version(tauri::webview_version().ok());
    let environment = Arc::new(zenith_platform::PlatformEnvironment::native());
    // The container host is observed once, at the composition root. The
    // adapter never reads the process environment itself.
    let container_host =
        crate::docker::adapter::ContainerHost::from_value(std::env::var("DOCKER_HOST").ok());
    let app_state = composition::desktop_state(environment, container_host);
    let awake_manager = app_state.system.awake_manager();
    let ai_control_runtime = app_state.ai.runtime();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second launch focuses the existing instance instead of
            // starting a competing process that could corrupt settings.
            show_main_window_or_report(app);
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
                    hide_quick_panel(window.app_handle());
                }
                // The panel dismisses itself when it loses focus (e.g. clicking
                // outside onto the desktop or another window). Trailing release
                // suppression is managed exclusively by tray mouse down/up events,
                // so outside dismissal leaves subsequent tray clicks unsuppressed.
                tauri::WindowEvent::Focused(false) => {
                    hide_quick_panel(window.app_handle());
                }
                _ => {}
            }
        })
        .setup(move |app| {
            let config_dir = match app.path().app_config_dir() {
                Ok(config_dir) => Some(config_dir),
                Err(error) => {
                    // Without the config directory every stored setting falls
                    // back to its default, so the reason is the only record
                    // that the user's own preferences were not applied.
                    crate::diagnostics::log_error(
                        "startup",
                        &format!(
                            "Settings directory is unavailable; stored settings were not loaded: {error}"
                        ),
                    );
                    None
                }
            };
            if let Some(config_dir) = config_dir {
                let loaded = settings_store::load(&config_dir);
                let state = app.state::<DesktopState>();
                if let Err(error) = state.system.apply_startup_policy(&loaded) {
                    crate::diagnostics::log_error(
                        "startup",
                        &format!("Stored Keep Awake rules were not applied: {error}"),
                    );
                }
                // The settings file is authoritative at startup; a snapshot
                // that cannot be published says so instead of quietly leaving
                // the user's preferences unapplied.
                if let Err(error) = state.settings.replace(loaded) {
                    crate::diagnostics::log_error(
                        "startup",
                        &format!("Stored settings were not applied: {error}"),
                    );
                }
                state.ai.restore_audit(&config_dir);
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
                        show_main_window_or_report(app);
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
                        button_state,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        match button_state {
                            MouseButtonState::Down => {
                                let is_visible = app
                                    .get_webview_window("quick")
                                    .and_then(|w| w.is_visible().ok())
                                    .unwrap_or(false);
                                let visibility = app.state::<QuickPanelVisibility>();
                                visibility.on_tray_mouse_down(is_visible);
                                if is_visible {
                                    hide_quick_panel(app);
                                }
                            }
                            MouseButtonState::Up => {
                                handle_tray_mouse_up(app, Some(rect));
                            }
                        }
                    }
                })
                .build(app)?;

            let watcher_ref = awake_manager.clone();
            std::thread::spawn(move || loop {
                watcher_ref.run_evaluation_cycle();
            });

            // The background tick delivers native advisories; the transport
            // adapter is built here so the runtime never names the plugin.
            let bg_notifications = crate::events::notifications::TauriNotifications::new(
                app.handle().clone(),
            );
            let bg_runtime = ai_control_runtime.clone();
            std::thread::spawn(move || loop {
                if bg_runtime.are_advisories_enabled() {
                    bg_runtime.run_background_tick(Some(&bg_notifications));
                    bg_runtime.wait_next_tick(std::time::Duration::from_secs(5));
                } else {
                    bg_runtime.wait_next_tick(std::time::Duration::from_secs(60));
                }
            });

            Ok(())
        })
        .invoke_handler(specta_builder().invoke_handler());

    let app = match builder.build(tauri::generate_context!()) {
        Ok(app) => app,
        Err(error) => {
            // A build failure happens before any window exists and release
            // builds have no console, so the report is written and raised here
            // instead of panicking where nobody can read it.
            crate::diagnostics::report_fatal_startup_failure(
                "Zenith could not start",
                &error.to_string(),
            );
            std::process::exit(1);
        }
    };

    app.run(|app, event| match event {
        tauri::RunEvent::Ready => {
            show_main_window_or_report(app);
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => {
            show_main_window_or_report(app);
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
            commands::open_project_in_terminal,
            commands::get_ai_control_center,
            commands::get_ai_runtime_health,
            commands::get_ai_control_quick_summary,
            commands::save_ai_control_preferences,
            commands::run_ai_safety_scan,
            commands::dismiss_ai_safety_finding,
            commands::preview_ai_recommendation,
            commands::consume_ai_recommendation_preview,
            commands::get_ai_control_git_diff,
            commands::connect_openrouter_oauth,
            commands::start_scan,
            commands::resume_scan,
            commands::cancel_scan,
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
            commands::open_full_disk_access_settings,
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
    use tauri::utils::config::WindowConfig;
    use tauri::utils::TitleBarStyle;
    use tauri::{PhysicalPosition, PhysicalSize};
    use zenith_platform::path_algebra::PathFlavor;

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
    fn tray_toggle_action_distinguishes_open_and_close() {
        assert_eq!(tray_toggle_action(false), TrayToggle::Show);
        assert_eq!(tray_toggle_action(true), TrayToggle::Hide);
    }

    #[test]
    fn a_consumed_click_clears_the_dismissal_marker() {
        let visibility = QuickPanelVisibility::default();
        assert!(!visibility.take_pending_tray_mouse_up());

        // Arm on mouse-down while visible.
        visibility.on_tray_mouse_down(true);
        assert!(visibility.take_pending_tray_mouse_up());

        // Consuming it once clears it, so the next query returns false.
        assert!(!visibility.take_pending_tray_mouse_up());
    }

    #[test]
    fn quick_panel_visibility_state_machine_handles_dismissal_and_reopen() {
        let visibility = QuickPanelVisibility::default();
        let mut visible = false;

        // 1. hidden -> tray click (down + up) -> shown
        assert!(!visible);
        visibility.on_tray_mouse_down(visible);
        assert!(!visibility.take_pending_tray_mouse_up());
        visible = true;

        // 2. shown -> user clicks tray icon: mouse-down dismisses and arms suppression
        assert!(visible);
        visibility.on_tray_mouse_down(visible);
        visible = false; // mouse-down hid the panel

        // 3. trailing mouse-up from the same tray click arrives -> suppressed
        assert!(visibility.take_pending_tray_mouse_up());
        assert!(!visible);

        // 4. next genuine tray click (down + up) -> shown immediately
        visibility.on_tray_mouse_down(visible);
        assert!(!visibility.take_pending_tray_mouse_up());
        visible = true;
        assert!(visible);
    }

    #[test]
    fn quick_panel_dismissal_outside_tray_reopens_immediately_on_subsequent_click() {
        let visibility = QuickPanelVisibility::default();

        // Panel was open, then an outside click caused focus loss and dismissed
        // the panel directly without any tray mouse-down event.
        let visible = false;
        // Notice: on_tray_mouse_down is NOT called.

        // Immediate subsequent tray click (even 0ms later) must open the panel without suppression:
        visibility.on_tray_mouse_down(visible);
        assert!(!visibility.take_pending_tray_mouse_up());
        let action = tray_toggle_action(visible);
        assert_eq!(action, TrayToggle::Show);
    }

    #[test]
    fn long_press_on_tray_icon_still_suppresses_trailing_mouse_up() {
        let visibility = QuickPanelVisibility::default();

        // Panel visible -> user presses down and holds long (no TTL expiration)
        visibility.on_tray_mouse_down(true);

        // Trailing release is still recognized as the tail of dismissal and suppressed:
        assert!(visibility.take_pending_tray_mouse_up());

        // And a subsequent click when hidden opens normally:
        visibility.on_tray_mouse_down(false);
        assert!(!visibility.take_pending_tray_mouse_up());
        assert_eq!(tray_toggle_action(false), TrayToggle::Show);
    }

    #[test]
    fn abandoned_tray_press_cleared_by_next_press_when_hidden() {
        let visibility = QuickPanelVisibility::default();

        // User pressed down on tray while visible, then dragged cursor away (mouse-up never delivered to tray).
        visibility.on_tray_mouse_down(true);

        // Next interaction: user clicks tray while panel is hidden.
        // Mouse-down while hidden immediately clears any stale pending state.
        visibility.on_tray_mouse_down(false);
        assert!(!visibility.take_pending_tray_mouse_up());
        assert_eq!(tray_toggle_action(false), TrayToggle::Show);
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
    /// frontend CI job runs this test and fails on a diff of the exported file,
    /// next to the TypeScript binding export.
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

    /// Exports the platform vocabulary the browser preview renders.
    ///
    /// The preview can be pointed at Windows, Linux, or macOS, and each
    /// selection has to show that platform's own nouns. The values come from
    /// the same `PlatformContext::for_platform` the command serves, so the
    /// preview cannot drift from the backend's copy. Each platform is described
    /// by a stated environment, which is the only way to render all three from
    /// one runner.
    #[test]
    #[ignore = "code generation"]
    fn export_platform_context_golden() {
        use crate::models::{PlatformContext, PlatformKind};

        let golden = serde_json::json!({
            "macos": PlatformContext::for_preview(PlatformKind::Macos),
            "windows": PlatformContext::for_preview(PlatformKind::Windows),
            "linux": PlatformContext::for_preview(PlatformKind::Linux),
        });
        let rendered = serde_json::to_string_pretty(&golden)
            .expect("serialize platform context golden")
            + "\n";
        std::fs::write("../src/lib/bindings/platform-context.golden.json", rendered)
            .expect("write platform context golden");
    }
}
