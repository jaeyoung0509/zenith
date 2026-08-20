fn main() {
    const COMMANDS: &[&str] = &[
        "get_ai_usage",
        "connect_openrouter_oauth",
        "start_scan",
        "get_last_scan",
        "create_delete_plan",
        "execute_clean",
        "get_memory_metrics",
        "terminate_process_group",
        "pick_keep_awake_application",
        "get_disk_metrics",
        "get_disk_volumes",
        "open_disk_utility",
        "get_docker_status",
        "prune_docker_target",
        "get_local_models",
        "delete_local_model",
        "get_awake_state",
        "set_awake_rules",
        "set_manual_awake",
        "disable_manual_awake",
        "get_settings",
        "save_settings",
        "reveal_in_finder",
        "open_dashboard_window",
        "toggle_quick_panel",
        "get_app_version",
        "get_diagnostics",
        "open_logs_folder",
    ];

    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to build Zenith's Tauri manifest");
}
