//! The commands' dependencies must be built from the injected environment.
//!
//! `run_environment_self_check` answers from `AppState.environment`, so the
//! state a test builds from a stated machine has to answer for that machine
//! instead of for the runner. This file exercises the same construction and the
//! same doctor entry point the command uses, without a Tauri app handle.

use std::sync::Arc;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use tauri::WebviewWindowBuilder;
use zenith_lib::commands::AppState;
use zenith_lib::diagnostics::doctor;
use zenith_lib::docker::adapter::ContainerHost;
use zenith_lib::models::PlatformKind;
use zenith_lib::platform::path_algebra::PathFlavor;
use zenith_lib::platform::paths::SimulatedPaths;
use zenith_lib::platform::{KnownFolder, PlatformEnvironment};

/// A stated Windows workstation: the profile, the temporary directory, and the
/// install roots are all on `D:`, so nothing here can be confused with the
/// runner's own macOS profile.
fn stated_windows_machine() -> PlatformEnvironment {
    PlatformEnvironment::simulated(PathFlavor::Windows)
        .with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::Windows)
                .with_home(r"D:\Users\tester")
                .with_temp_dir(r"D:\Users\tester\AppData\Local\Temp")
                .with_local_app_data(r"D:\Users\tester\AppData\Local")
                .with_roaming_app_data(r"D:\Users\tester\AppData\Roaming")
                .with_program_files(r"D:\Program Files")
                .with_program_data(r"D:\ProgramData"),
        ))
        .with_known_folder(KnownFolder::Documents, r"D:\Users\tester\Documents")
        .with_known_folder(KnownFolder::Downloads, r"D:\Users\tester\Downloads")
}

#[test]
fn environment_self_check_uses_the_injected_environment() {
    let environment = Arc::new(stated_windows_machine());
    let state = AppState::new(environment, ContainerHost::unstated());

    let report = doctor::self_check(&state.environment);

    assert_eq!(
        report.platform,
        PlatformKind::Windows,
        "the report must describe the injected machine"
    );
    assert_eq!(
        report.failures,
        0,
        "a stated Windows workstation must satisfy every self-check: {:#?}",
        report
            .checks
            .iter()
            .filter(|check| check.outcome == doctor::SelfCheckOutcome::Fail)
            .collect::<Vec<_>>()
    );
    assert!(
        report
            .fingerprint
            .iter()
            .any(|entry| entry == "path_flavor=windows"),
        "the fingerprint names the stated flavor: {:?}",
        report.fingerprint
    );
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.name == "home_resolves" && check.detail.contains("absolute=true")),
        "the profile check must resolve the stated profile"
    );
}

#[test]
fn app_state_builds_every_shared_handle_from_the_stated_environment() {
    let environment = Arc::new(stated_windows_machine());
    let state = AppState::new(environment.clone(), ContainerHost::unstated());

    // The catalog was loaded against the stated machine, so its signatures
    // resolve through the stated profile rather than the host's.
    let cargo = state
        .registry
        .get("dev.cargo.registry.cache")
        .expect("the embedded catalog is loaded");
    let resolved = state.registry.resolve_paths(cargo, &state.environment);
    assert_eq!(
        resolved,
        vec![std::path::PathBuf::from(
            r"D:\Users\tester\.cargo\registry\cache"
        )],
        "the stated profile decides where a home-relative path resolves"
    );

    // The same value the composition root injected is what every command
    // handler reads, so no handler can silently answer from the host.
    assert!(Arc::ptr_eq(&state.environment, &environment));
}

/// The command surface itself, not just the state: an invoke of the registered
/// command must answer from the injected environment.
#[test]
fn the_self_check_command_answers_from_the_injected_environment() {
    let environment = Arc::new(stated_windows_machine());
    let state = AppState::new(environment, ContainerHost::unstated());

    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            zenith_lib::commands::run_environment_self_check
        ])
        .manage(state)
        .build(mock_context(noop_assets()))
        .expect("the mock application builds");
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("the mock webview builds");

    let response = get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: "run_environment_self_check".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            // The local origin is spelled per platform: `tauri://localhost` on
            // macOS, `http://tauri.localhost` on Windows. Anything else is a
            // remote origin, and a remote origin has to clear the command ACL,
            // which a mock context does not carry.
            url: if cfg!(any(windows, target_os = "android")) {
                "http://tauri.localhost"
            } else {
                "tauri://localhost"
            }
            .parse()
            .unwrap(),
            body: InvokeBody::default(),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        },
    )
    .expect("the command returns a report");

    let report: doctor::EnvironmentReport = response
        .deserialize()
        .expect("the payload is an environment report");
    assert_eq!(report.platform, PlatformKind::Windows);
    assert_eq!(report.failures, 0);
}

/// A catalog that never loaded must refuse the scan instead of reporting an
/// empty result that looks exactly like a clean machine.
#[test]
fn a_failed_catalog_refuses_the_scan_instead_of_reporting_an_empty_one() {
    use zenith_lib::signatures::SignatureRegistry;

    let failure = "Signature catalog could not be loaded: stated catalog defect";
    let state = AppState::with_catalog(
        Arc::new(stated_windows_machine()),
        ContainerHost::unstated(),
        SignatureRegistry::new(),
        Some(failure.to_string()),
    );

    let refusal = state
        .catalog_failure()
        .expect("a failed catalog is a refusal, not a quiet empty scan");
    assert!(refusal.contains(failure), "{refusal}");
    assert!(refusal.contains("unavailable"), "{refusal}");
    assert!(
        state.registry.all().is_empty(),
        "the fallback registry stays empty rather than inventing signatures"
    );

    // A catalog that loaded has nothing to refuse.
    let healthy = AppState::new(
        Arc::new(stated_windows_machine()),
        ContainerHost::unstated(),
    );
    assert_eq!(healthy.catalog_failure(), None);
}
