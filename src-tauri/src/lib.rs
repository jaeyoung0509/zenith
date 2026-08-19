pub mod ai_usage;
pub mod cleaner;
pub mod commands;
pub mod docker;
pub mod metrics;
pub mod models;
pub mod models_inventory;
pub mod power;
pub mod safety;
pub mod scanner;
pub mod settings_store;
pub mod signatures;
pub mod tooling;

use commands::AppState;
use power::KeepAwakeManager;
use signatures::SignatureRegistry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, Rect, WebviewWindow, WebviewWindowBuilder,
};

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

    WebviewWindowBuilder::from_config(app, &config)?.build()
}

fn tray_anchor(window: &WebviewWindow, rect: Rect) -> PhysicalPosition<f64> {
    let scale = window.scale_factor().unwrap_or(1.0);
    let position: PhysicalPosition<f64> = rect.position.to_physical(scale);
    let size: tauri::PhysicalSize<f64> = rect.size.to_physical(scale);
    PhysicalPosition::new(position.x + size.width, position.y + size.height)
}

fn quick_panel_position(
    anchor: PhysicalPosition<f64>,
    panel_size: PhysicalSize<u32>,
    monitor_origin: PhysicalPosition<i32>,
    monitor_size: PhysicalSize<u32>,
) -> PhysicalPosition<i32> {
    let max_x = monitor_origin.x + monitor_size.width as i32 - panel_size.width as i32;
    let max_y = monitor_origin.y + monitor_size.height as i32 - panel_size.height as i32;
    PhysicalPosition::new(
        (anchor.x.round() as i32 - panel_size.width as i32)
            .clamp(monitor_origin.x, max_x.max(monitor_origin.x)),
        (anchor.y.round() as i32 + 6).clamp(monitor_origin.y, max_y.max(monitor_origin.y)),
    )
}

fn show_quick_panel(window: &WebviewWindow, click_position: Option<PhysicalPosition<f64>>) {
    if let Some(position) = click_position {
        if let Ok(size) = window.outer_size() {
            let mut target = PhysicalPosition::new(
                position.x.round() as i32 - size.width as i32,
                position.y.round() as i32 + 6,
            );

            if let Ok(monitors) = window.available_monitors() {
                if let Some(monitor) = monitors.iter().find(|monitor| {
                    let origin = monitor.position();
                    let bounds = monitor.size();
                    position.x >= f64::from(origin.x)
                        && position.x < f64::from(origin.x + bounds.width as i32)
                        && position.y >= f64::from(origin.y)
                        && position.y < f64::from(origin.y + bounds.height as i32)
                }) {
                    target =
                        quick_panel_position(position, size, *monitor.position(), *monitor.size());
                }
            }
            let _ = window.set_position(target);
        }
    }
    let _ = window.show();
    let _ = window.set_focus();
}

pub fn run() {
    let registry = Arc::new(SignatureRegistry::load_embedded().unwrap_or_default());
    let awake_manager = Arc::new(KeepAwakeManager::new());
    let settings = Arc::new(Mutex::new(models::ZenithSettings::default()));
    let last_scan = Arc::new(Mutex::new(None));
    let openrouter_key = Arc::new(Mutex::new(None));
    let ai_usage_cache = Arc::new(Mutex::new(None));
    let ai_usage_refresh_lock = Arc::new(Mutex::new(()));
    let delete_plans = Arc::new(Mutex::new(HashMap::new()));
    let operation_lock = Arc::new(Mutex::new(()));

    let app_state = AppState {
        registry,
        awake_manager: awake_manager.clone(),
        settings,
        last_scan,
        openrouter_key,
        ai_usage_cache,
        ai_usage_refresh_lock,
        delete_plans,
        operation_lock,
    };

    tauri::Builder::default()
        .manage(app_state)
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "quick" {
                    // Quick panel hides to stay responsive
                    api.prevent_close();
                    let _ = window.hide();
                } else if window.label() == "main" {
                    // Main dashboard is destroyed on close to release WKWebView memory back to the OS.
                    // The tray keeps the application alive.
                }
            }
        })
        .setup(move |app| {
            if let Ok(config_dir) = app.path().app_config_dir() {
                let loaded = settings_store::load(&config_dir);
                app.state::<AppState>()
                    .awake_manager
                    .set_rules(loaded.awake_rules.clone());
                *app.state::<AppState>().settings.lock().unwrap() = loaded;
            }
            // Create exactly one macOS menu-bar icon. It is a monochrome
            // template image so macOS can adapt it to light/dark menu bars.
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
                        if let Ok(window) = ensure_window(app, "main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "toggle_quick" => {
                        if let Ok(window) = ensure_window(app, "quick") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                let position = app
                                    .tray_by_id("main-tray")
                                    .and_then(|tray| tray.rect().ok().flatten())
                                    .map(|rect| tray_anchor(&window, rect));
                                show_quick_panel(&window, position);
                            }
                        }
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
                        let app = tray.app_handle();
                        if let Ok(quick_win) = ensure_window(app, "quick") {
                            let is_vis = quick_win.is_visible().unwrap_or(false);
                            if is_vis {
                                let _ = quick_win.hide();
                            } else {
                                let position = tray_anchor(&quick_win, rect);
                                show_quick_panel(&quick_win, Some(position));
                            }
                        }
                    }
                })
                .build(app)?;

            // Optional background thread for Keep Awake watcher (~5s interval, only checks when rules exist)
            let watcher_ref = awake_manager.clone();
            std::thread::spawn(move || loop {
                watcher_ref.wait_for_next_evaluation();
                watcher_ref.evaluate();
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_ai_usage,
            commands::connect_openrouter_oauth,
            commands::start_scan,
            commands::get_last_scan,
            commands::create_delete_plan,
            commands::execute_clean,
            commands::get_memory_metrics,
            commands::terminate_process_group,
            commands::pick_keep_awake_application,
            commands::get_disk_metrics,
            commands::get_disk_volumes,
            commands::open_disk_utility,
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
            commands::reveal_in_finder,
            commands::open_dashboard_window,
            commands::toggle_quick_panel,
        ])
        .run(tauri::generate_context!())
        .expect("error while running zenith application");
}

#[cfg(test)]
mod tests {
    use super::quick_panel_position;
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
    fn quick_panel_is_clamped_inside_active_display() {
        let position = quick_panel_position(
            PhysicalPosition::new(100.0, 1_900.0),
            PhysicalSize::new(720, 1_040),
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(3_456, 2_234),
        );
        assert_eq!(position, PhysicalPosition::new(0, 1_194));
    }
}
