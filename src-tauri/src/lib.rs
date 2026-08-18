pub mod cleaner;
pub mod commands;
pub mod docker;
pub mod metrics;
pub mod models;
pub mod models_inventory;
pub mod power;
pub mod safety;
pub mod scanner;
pub mod signatures;

use commands::AppState;
use power::KeepAwakeManager;
use signatures::SignatureRegistry;
use std::sync::{Arc, Mutex};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;

pub fn run() {
    let registry = Arc::new(SignatureRegistry::load_embedded().unwrap_or_default());
    let awake_manager = Arc::new(KeepAwakeManager::new());
    let settings = Arc::new(Mutex::new(models::ZenithSettings::default()));
    let last_scan = Arc::new(Mutex::new(None));

    let app_state = AppState {
        registry,
        awake_manager: awake_manager.clone(),
        settings,
        last_scan,
    };

    tauri::Builder::default()
        .manage(app_state)
        .setup(move |app| {
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
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "toggle_quick" => {
                        if let Some(window) = app.get_webview_window("quick") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(quick_win) = tray.app_handle().get_webview_window("quick") {
                            let is_vis = quick_win.is_visible().unwrap_or(false);
                            if is_vis {
                                let _ = quick_win.hide();
                            } else {
                                let _ = quick_win.show();
                                let _ = quick_win.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            // Optional background thread for Keep Awake watcher (~5s interval, only checks when rules exist)
            let watcher_ref = awake_manager.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(5));
                watcher_ref.evaluate();
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_scan,
            commands::get_last_scan,
            commands::create_delete_plan,
            commands::execute_clean,
            commands::get_memory_metrics,
            commands::get_disk_metrics,
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
