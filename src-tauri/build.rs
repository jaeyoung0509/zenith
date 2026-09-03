fn main() {
    const COMMANDS: &[&str] = &[
        "get_ai_usage",
        "get_project_context",
        "request_stop_agent_session",
        "get_agent_integrations",
        "setup_agent_integration",
        "remove_agent_integration",
        "get_agent_quick_summary",
        "post_agent_event",
        "open_in_terminal",
        "get_ai_control_center",
        "get_ai_control_quick_summary",
        "save_ai_control_preferences",
        "run_ai_safety_scan",
        "dismiss_ai_safety_finding",
        "preview_ai_recommendation",
        "consume_ai_recommendation_preview",
        "get_ai_control_git_diff",
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
        "open_storage_settings",
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
        "show_in_file_manager",
        "open_dashboard_window",
        "toggle_quick_panel",
        "get_app_version",
        "get_platform_capabilities",
        "get_diagnostics",
        "open_logs_folder",
        "list_development_listeners",
        "release_development_listener",
        "start_large_file_scan",
        "cancel_large_file_scan",
        "prepare_large_file_trash",
        "pick_developer_workspace",
        "register_developer_home_workspace",
        "start_developer_artifact_scan",
        "cancel_developer_artifact_scan",
        "prepare_developer_artifact_cleanup",
        "get_installed_apps",
        "inspect_app_uninstall",
        "prepare_app_uninstall",
        "execute_trash_plan",
    ];

    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));

    #[cfg(windows)]
    let attributes = {
        // tauri-build normally embeds the application manifest in binaries
        // only. Rust test harnesses need the same Common Controls v6 manifest
        // or Windows aborts them with STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139).
        embed_windows_manifest_for_all_targets();
        attributes.windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest())
    };

    tauri_build::try_build(attributes).expect("failed to build Zenith's Tauri manifest");
}

#[cfg(windows)]
fn embed_windows_manifest_for_all_targets() {
    let manifest = std::env::current_dir()
        .expect("resolve src-tauri directory")
        .join("windows-app-manifest.xml");

    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
        manifest
            .to_str()
            .expect("Windows manifest path must be valid Unicode")
    );
    println!("cargo:rustc-link-arg=/WX");
}
