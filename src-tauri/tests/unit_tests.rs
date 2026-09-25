use std::fs::File;
use std::io::Write;
use tempfile::tempdir;
use zenith_lib::docker::DockerAdapter;
use zenith_lib::models::{
    AwakeBehavior, Category, CleanStrategy, DiskMetrics, RiskTier, Signature,
};
use zenith_lib::power::{KeepAwakeManager, PowerAssertion};
use zenith_lib::scanner::{DirectoryScanner, ScanEngine, SizeCalculator};
use zenith_lib::signatures::SignatureRegistry;
use zenith_platform::path_algebra::PathFlavor;
use zenith_platform::PlatformEnvironment;

#[test]
fn test_signature_registry_categories_and_risk_counts() {
    let registry = SignatureRegistry::load_embedded().expect("load embedded");

    // Generic cleanup owns cache categories; models use their dedicated inventory.
    let ai_sigs = registry.by_category(Category::Ai);
    let dev_sigs = registry.by_category(Category::Developer);
    let container_sigs = registry.by_category(Category::Container);
    let model_sigs = registry.by_category(Category::Model);
    let system_sigs = registry.by_category(Category::System);

    assert!(!ai_sigs.is_empty(), "AI signatures must not be empty");
    assert!(
        !dev_sigs.is_empty(),
        "Developer signatures must not be empty"
    );
    assert!(
        !container_sigs.is_empty(),
        "Container signatures must not be empty"
    );
    assert!(
        model_sigs.is_empty(),
        "Models belong to the dedicated inventory"
    );
    assert!(
        !system_sigs.is_empty(),
        "System signatures must not be empty"
    );

    // Models must ALWAYS be Manual risk tier
    for model_sig in model_sigs {
        assert_eq!(
            model_sig.risk,
            RiskTier::Manual,
            "Model signature {} must have manual risk tier",
            model_sig.id
        );
    }

    // Safe cleanup stays narrow; owner-managed language caches are Rebuild
    // actions because they can trigger compilation or downloads.
    let safe_sigs = registry.by_risk(RiskTier::Safe);
    assert!(safe_sigs
        .iter()
        .any(|s| s.id == "system.intensive.user_app_caches"));
    let rebuild_sigs = registry.by_risk(RiskTier::Rebuild);
    assert!(rebuild_sigs.iter().any(|s| s.id == "dev.go.build"));
    let manual_sigs = registry.by_risk(RiskTier::Manual);
    assert!(manual_sigs.iter().any(|s| s.id == "ai.claude.logs"));
}

#[test]
fn test_temp_scanner_only_includes_known_direct_children() {
    let dir = tempdir().expect("tempdir");
    let known = dir.path().join("codex-session");
    let unrelated = dir.path().join("personal-files");
    std::fs::create_dir_all(&known).unwrap();
    std::fs::create_dir_all(&unrelated).unwrap();
    File::create(known.join("cache.bin"))
        .unwrap()
        .write_all(b"temporary cache")
        .unwrap();
    File::create(unrelated.join("keep.txt"))
        .unwrap()
        .write_all(b"must remain invisible")
        .unwrap();

    let signature = Signature {
        id: "system.test-temp".into(),
        name: "Developer Temp".into(),
        category: Category::System,
        family: Default::default(),
        risk: RiskTier::Safe,
        strategy: CleanStrategy::DeleteDirectory,
        paths: vec![dir.path().to_string_lossy().into_owned()],
        exclusions: vec![],
        description: "test".into(),
        min_age_days: Some(0),
        include_prefixes: vec!["codex-".into()],
        exclude_prefixes: vec![],
        intensive_only: false,
        platforms: vec![],
        discovery: Default::default(),
        unit: None,
        owner: String::new(),
        priority: 0,
        fail_if_running: Vec::new(),
        provider: String::new(),
        provider_id: None,
        artifact_kind: Default::default(),
        consequence: String::new(),
    };

    let items = DirectoryScanner::scan_signature(
        &signature,
        &PlatformEnvironment::native(),
        &zenith_core::application::dto::scan::NeverCancelled,
    );
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].path, known.to_string_lossy());
    assert!(items[0].is_selected);
}

#[test]
fn test_scan_hides_empty_paths_and_orders_largest_first() {
    let dir = tempdir().expect("tempdir");
    let large = dir.path().join("large.cache");
    let small = dir.path().join("small.cache");
    File::create(&large)
        .unwrap()
        .write_all(&vec![0u8; 16 * 1024])
        .unwrap();
    File::create(&small)
        .unwrap()
        .write_all(&vec![0u8; 1024])
        .unwrap();

    let signature = |id: &str, path: String| Signature {
        id: id.into(),
        name: id.into(),
        category: Category::System,
        family: Default::default(),
        risk: RiskTier::Safe,
        strategy: CleanStrategy::DeleteDirectory,
        paths: vec![path],
        exclusions: vec![],
        description: "test".into(),
        min_age_days: None,
        include_prefixes: vec![],
        exclude_prefixes: vec![],
        intensive_only: false,
        platforms: vec![],
        discovery: Default::default(),
        unit: None,
        owner: String::new(),
        priority: 0,
        fail_if_running: Vec::new(),
        provider: String::new(),
        provider_id: None,
        artifact_kind: Default::default(),
        consequence: String::new(),
    };

    let mut registry = SignatureRegistry::new();
    registry.register(signature("large", large.to_string_lossy().into_owned()));
    registry.register(signature("small", small.to_string_lossy().into_owned()));
    registry.register(signature(
        "missing",
        dir.path()
            .join("missing.cache")
            .to_string_lossy()
            .into_owned(),
    ));

    let result = ScanEngine::scan(
        &registry,
        &zenith_lib::cleaner::LifecycleProviderRegistry::new(Vec::new()),
        &zenith_lib::cleaner::OwnerProviderRegistry::new(Vec::new()),
        Some(&[Category::System]),
        &[],
        false,
        &PlatformEnvironment::native(),
        &zenith_lib::models::NeverCancelled,
        |_| {},
    );
    let items = &result.categories[0].items;
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, "large");
    assert_eq!(items[1].id, "small");

    let excluded = vec!["large".to_string()];
    let filtered = ScanEngine::scan(
        &registry,
        &zenith_lib::cleaner::LifecycleProviderRegistry::new(Vec::new()),
        &zenith_lib::cleaner::OwnerProviderRegistry::new(Vec::new()),
        Some(&[Category::System]),
        &excluded,
        false,
        &PlatformEnvironment::native(),
        &zenith_lib::models::NeverCancelled,
        |_| {},
    );
    assert_eq!(filtered.categories[0].items.len(), 1);
    assert_eq!(filtered.categories[0].items[0].id, "small");
}

#[test]
fn test_docker_size_parser() {
    assert_eq!(DockerAdapter::parse_docker_size("0B"), 0);
    assert_eq!(DockerAdapter::parse_docker_size("512B"), 512);
    assert_eq!(DockerAdapter::parse_docker_size("1KB"), 1024);
    assert_eq!(
        DockerAdapter::parse_docker_size("1.5MB"),
        (1.5 * 1024.0 * 1024.0) as u64
    );
    assert_eq!(
        DockerAdapter::parse_docker_size("10GB"),
        10 * 1024 * 1024 * 1024
    );
    assert_eq!(
        DockerAdapter::parse_docker_size("2.5TB"),
        (2.5 * 1024.0 * 1024.0 * 1024.0 * 1024.0) as u64
    );
}

#[test]
fn test_size_calculator_recursive_and_exclusions() {
    let dir = tempdir().expect("tempdir");

    let keep_file = dir.path().join("keep.log");
    File::create(&keep_file)
        .unwrap()
        .write_all(&vec![0u8; 10000])
        .unwrap();

    let exclude_dir = dir.path().join("excluded_folder");
    std::fs::create_dir(&exclude_dir).unwrap();
    File::create(exclude_dir.join("excluded.dat"))
        .unwrap()
        .write_all(&vec![0u8; 50000])
        .unwrap();

    // Measure without exclusions
    let total = SizeCalculator::measure_path_full(dir.path(), &[], &PlatformEnvironment::native());
    assert_eq!(total.file_count, 2);
    assert!(total.size.logical >= 60000);

    // Measure with exclusion of "excluded_folder"
    let filtered = SizeCalculator::measure_path_full(
        dir.path(),
        &["excluded_folder".to_string()],
        &PlatformEnvironment::native(),
    );
    assert_eq!(filtered.file_count, 1);
    assert_eq!(filtered.size.logical, 10000);
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn test_power_assertion_raii_lifecycle() {
    {
        let assertion = PowerAssertion::acquire(
            AwakeBehavior::PreventSystemSleep,
            "Zenith Rust Test Assertion",
        );
        assert!(assertion.is_ok(), "PowerAssertion acquire must succeed");
        let assertion = assertion.unwrap();
        assert_eq!(assertion.behavior, AwakeBehavior::PreventSystemSleep);
        // assertion automatically drops here at end of scope
    }

    // KeepAwakeManager test
    let manager = KeepAwakeManager::new();
    let state_initial = manager.get_state();
    assert!(!state_initial.is_active);

    // Set manual awake
    manager
        .set_manual(Some(3600), AwakeBehavior::PreventSystemSleep)
        .expect("set manual awake");

    let state_manual = manager.get_state();
    assert!(state_manual.is_active);
    assert!(state_manual.manual_expires_at.is_some());

    // Disable manual awake
    manager.disable_manual();
    let state_after = manager.get_state();
    assert!(!state_after.is_active);
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[test]
fn test_power_assertion_fails_closed_without_native_adapter() {
    let assertion = PowerAssertion::acquire(
        AwakeBehavior::PreventSystemSleep,
        "Zenith Rust Test Assertion",
    );
    assert!(assertion.is_err());

    let manager = KeepAwakeManager::new();
    assert!(manager
        .set_manual(Some(3600), AwakeBehavior::PreventSystemSleep)
        .is_err());
    assert!(!manager.get_state().is_active);
}

#[test]
fn test_keep_awake_power_conditions_and_ac_awareness() {
    use std::sync::Arc;
    use zenith_lib::models::{AwakeRule, AwakeRuleStatus, PowerCondition, PowerSourceType};
    use zenith_lib::power::{MockPowerSource, NativeAssertionProvider};

    let power_mock = Arc::new(MockPowerSource::new(PowerSourceType::Battery));
    let assertion_mock = Arc::new(NativeAssertionProvider::new());
    let manager = KeepAwakeManager::with_providers(power_mock.clone(), assertion_mock);

    let rule_ac = AwakeRule {
        id: "rule.ac".to_string(),
        app_name: "AC App".to_string(),
        executable_pattern: "non_existent_process_abc".to_string(),
        requires_process_pattern: None,
        application: None,
        agent_ids: Vec::new(),
        behavior: AwakeBehavior::PreventSystemSleep,
        power_condition: PowerCondition::AcPowerOnly,
        enabled: true,
    };

    let rule_always = AwakeRule {
        id: "rule.always".to_string(),
        app_name: "Always App".to_string(),
        executable_pattern: "non_existent_process_def".to_string(),
        requires_process_pattern: None,
        application: None,
        agent_ids: Vec::new(),
        behavior: AwakeBehavior::KeepDisplayAwake,
        power_condition: PowerCondition::Always,
        enabled: true,
    };

    manager
        .set_rules(vec![rule_ac, rule_always])
        .expect("rules within limit");
    let state = manager.get_state();

    assert_eq!(state.rule_evaluations.len(), 2);
    assert_eq!(state.power_source, PowerSourceType::Battery);
    assert_eq!(
        state.rule_evaluations[0].status,
        AwakeRuleStatus::WaitingProcess
    );
    assert!(!state.rule_evaluations[0].is_power_eligible);
    assert!(state.rule_evaluations[1].is_power_eligible);
}

#[test]
fn test_windows_blacklist_and_path_defense() {
    use std::path::Path;
    use zenith_lib::safety::blacklist::{classify_windows, BlacklistEnvironment, BlacklistVerdict};
    use zenith_lib::safety::Blacklist;
    use zenith_platform::path_algebra::PathFlavor;
    use zenith_platform::paths::SimulatedPaths;
    use zenith_platform::KnownFolder;

    // Stated environment: the profile lives on `Z:`, the redirected Documents
    // folder lives on `D:`, and nothing here mirrors this host's layout.
    let platform = PlatformEnvironment::simulated(PathFlavor::Windows)
        .with_roots(std::sync::Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::Windows)
                .with_home(r"Z:\Users\tester")
                .with_temp_dir(r"Z:\Users\tester\AppData\Local\Temp")
                .with_local_app_data(r"Z:\Users\tester\AppData\Local")
                .with_roaming_app_data(r"Z:\Users\tester\AppData\Roaming")
                .with_program_files(r"Z:\Program Files")
                .with_program_data(r"Z:\ProgramData"),
        ))
        .with_known_folder(KnownFolder::Documents, r"D:\OneDrive\Documents");
    let environment = BlacklistEnvironment::from_environment(&platform);

    // Drive roots, system directories, and the users root on every drive
    // letter, including the 32-bit Program Files tail.
    for drive in ["C:", "D:", "Z:"] {
        let roots = [
            format!("{drive}:"),
            format!(r"{drive}\"),
            format!(r"{drive}\Windows\System32"),
            format!(r"{drive}\Program Files"),
            format!(r"{drive}\Program Files (x86)"),
            format!(r"{drive}\ProgramData\App"),
            format!(r"{drive}\Users"),
        ];
        for path_str in roots {
            let verdict = classify_windows(&path_str, &environment);
            assert!(
                verdict.is_denied(),
                "expected {path_str} to be denied, got {verdict:?}"
            );
            assert!(
                Blacklist::validate_with(Path::new(&path_str), &platform).is_err(),
                "expected {path_str} to be rejected"
            );
        }
    }

    // Only the `X:\Users` root itself is protected: descendants (temp
    // directories, projects, caches) stay scannable and cleanable.
    for path in [
        r"Z:\Users\tester\AppData\Local\Temp\zenith-test",
        r"Z:\Users\tester\dev\repo",
        r"D:\Users\tester\dev\repo",
        r"Z:\Users\tester\.cargo\registry\cache",
    ] {
        assert_eq!(
            classify_windows(path, &environment),
            BlacklistVerdict::Allowed,
            "expected {path} to stay cleanable"
        );
    }

    // Alternate data streams, trailing aliases, and unresolvable 8.3 aliases.
    for path in [
        r"Z:\Users\tester\dev\file.txt:stream",
        r"Z:\Users\tester\dev\folder.",
        r"Z:\Users\tester\dev\folder ",
        r"C:\PROGRA~1\Vendor",
        r"Z:\Users\tester\dev\CON",
    ] {
        let verdict = classify_windows(path, &environment);
        assert!(
            verdict.is_denied(),
            "expected {path} to be denied, got {verdict:?}"
        );
    }
    for path in [
        r"Z:\Users\tester\dev\file.txt:stream",
        r"Z:\Users\tester\dev\folder.",
        r"Z:\Users\tester\dev\folder ",
    ] {
        assert!(
            Blacklist::validate_with(Path::new(path), &platform).is_err(),
            "expected {path} to be rejected"
        );
    }

    // A content folder redirected outside the profile is still protected.
    assert!(
        classify_windows(r"D:\OneDrive\Documents\report.docx", &environment).is_denied(),
        "a redirected Documents folder must be protected"
    );
    // The same document spelled through the stated profile is cleanable.
    assert_eq!(
        classify_windows(r"Z:\Users\tester\dev\report.docx", &environment),
        BlacklistVerdict::Allowed
    );
}

#[cfg(target_os = "windows")]
#[test]
fn test_windows_verbatim_paths_preserve_blacklist_boundaries() {
    use std::path::{Path, PathBuf};
    use zenith_lib::safety::Blacklist;
    use zenith_platform::path_algebra;

    let host = PlatformEnvironment::native();

    let user_cache = Path::new(r"\\?\C:\Users\테스트\.gemini\antigravity-cli\log");
    assert_eq!(
        path_algebra::normalize_lexical(user_cache),
        PathBuf::from(r"C:\Users\테스트\.gemini\antigravity-cli\log")
    );
    assert_eq!(
        path_algebra::normalize_lexical(Path::new(r"\\?\UNC\server\share\cache")),
        PathBuf::from(r"\\server\share\cache")
    );
    assert!(!Blacklist::is_blacklisted_with(user_cache, &host));
    assert!(Blacklist::validate_with(user_cache, &host).is_ok());

    assert!(Blacklist::is_blacklisted_with(Path::new(r"\\?\C:\"), &host));
    assert!(Blacklist::is_blacklisted_with(
        Path::new(r"\\?\C:\Windows\System32"),
        &host
    ));
    assert!(Blacklist::is_blacklisted_with(
        Path::new(r"\\?\C:\safe\file.txt:stream"),
        &host
    ));
    assert!(Blacklist::is_blacklisted_with(
        Path::new(r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy1"),
        &host
    ));
    assert!(Blacklist::is_blacklisted_with(
        Path::new(r"\\.\PhysicalDrive0"),
        &host
    ));
}

#[test]
fn test_windows_reparse_point_symlink_defense() {
    use std::path::Path;
    use zenith_lib::safety::SymlinkGuard;

    let non_existent = Path::new("C:\\path\\does\\not\\exist\\123");
    assert!(!SymlinkGuard::is_symlink(non_existent));
}

#[test]
fn test_windows_platform_capabilities_batch2() {
    use zenith_lib::models::{PlatformCapabilities, PlatformFeatureStatus, PlatformKind};

    let caps = PlatformCapabilities::windows();
    assert_eq!(caps.platform, PlatformKind::Windows);
    assert_eq!(caps.system_actions.status, PlatformFeatureStatus::Available);
    assert_eq!(caps.cleanup.status, PlatformFeatureStatus::Available);
    assert_eq!(caps.large_files.status, PlatformFeatureStatus::Available);
    assert_eq!(
        caps.developer_artifacts.status,
        PlatformFeatureStatus::Available
    );
    assert_eq!(
        caps.installed_apps.status,
        PlatformFeatureStatus::Unavailable
    );
    // Windows has its own intensive signatures, so the broader scope is
    // implemented rather than refused.
    assert_eq!(
        caps.intensive_cleanup.status,
        PlatformFeatureStatus::Available
    );
    assert_eq!(
        caps.app_uninstall.status,
        PlatformFeatureStatus::Unavailable
    );
    assert_eq!(caps.memory_metrics.status, PlatformFeatureStatus::Available);
    assert_eq!(
        caps.process_termination.status,
        PlatformFeatureStatus::Available
    );
    assert_eq!(
        caps.development_ports.status,
        PlatformFeatureStatus::Available
    );
    assert_eq!(caps.keep_awake.status, PlatformFeatureStatus::Available);
    assert_eq!(caps.local_models.status, PlatformFeatureStatus::Available);
    assert_eq!(caps.docker.status, PlatformFeatureStatus::Available);
    assert_eq!(
        caps.ai_integrations.status,
        PlatformFeatureStatus::Available
    );
    // Both readings have Windows adapters: CPU through `sysinfo`, battery
    // through `GetSystemPowerStatus`.
    assert_eq!(caps.cpu_metrics.status, PlatformFeatureStatus::Available);
    assert_eq!(
        caps.battery_metrics.status,
        PlatformFeatureStatus::Available
    );
}

#[test]
fn test_windows_dev_ports_classification_defense() {
    use std::path::Path;
    use zenith_lib::dev_ports::{classify_listener, ProcessClassificationInput};
    use zenith_lib::process_owner::ProcessOwner;

    let current_owner = ProcessOwner::Windows("S-1-5-21-test".to_string());

    // Protected PowerShell / Windows Terminal
    let input_ps = ProcessClassificationInput {
        pid: 4500,
        owner: Some(current_owner.clone()),
        current_owner: current_owner.clone(),
        zenith_pid: 9999,
        port: 8080,
        raw_command: "powershell.exe",
        process_name: "powershell.exe",
        exe_path: Some(Path::new(
            "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
        )),
        cwd: Some(Path::new("C:\\Users\\test")),
        argv: &["powershell.exe".to_string()],
        started_at: Some(100),
    };
    let res_ps = classify_listener(&input_ps, PathFlavor::Windows);
    assert!(!res_ps.can_release);
    assert!(res_ps.blocked_reason.is_some());

    // Allowlisted Vite dev server on Windows
    let input_vite = ProcessClassificationInput {
        pid: 5600,
        owner: Some(current_owner.clone()),
        current_owner,
        zenith_pid: 9999,
        port: 5173,
        raw_command: "node.exe",
        process_name: "node.exe",
        exe_path: Some(Path::new("C:\\Program Files\\nodejs\\node.exe")),
        cwd: Some(Path::new("C:\\Users\\test\\projects\\my-app")),
        argv: &[
            "node.exe".to_string(),
            "C:\\Users\\test\\projects\\my-app\\node_modules\\vite\\bin\\vite.js".to_string(),
        ],
        started_at: Some(200),
    };
    let res_vite = classify_listener(&input_vite, PathFlavor::Windows);
    assert!(res_vite.can_release);
    assert_eq!(res_vite.server_name, "Vite");
}

#[test]
fn port_release_refuses_a_privileged_or_unidentified_owner_only() {
    use std::path::Path;
    use zenith_lib::dev_ports::{classify_listener, ProcessClassificationInput};
    use zenith_lib::process_owner::ProcessOwner;

    fn input<'a>(
        owner: ProcessOwner,
        current_owner: ProcessOwner,
        argv: &'a [String],
    ) -> ProcessClassificationInput<'a> {
        ProcessClassificationInput {
            pid: 5600,
            owner: Some(owner),
            current_owner,
            zenith_pid: 9999,
            port: 5173,
            raw_command: "node.exe",
            process_name: "node.exe",
            exe_path: Some(Path::new("C:\\Program Files\\nodejs\\node.exe")),
            cwd: Some(Path::new("C:\\Users\\test\\projects\\my-app")),
            argv,
            started_at: Some(200),
        }
    }

    let argv = [
        "node.exe".to_string(),
        "C:\\Users\\test\\projects\\my-app\\node_modules\\vite\\bin\\vite.js".to_string(),
    ];
    let own = ProcessOwner::Unix(501);

    // A system owner is refused, and the refusal names the reason.
    let root = classify_listener(
        &input(ProcessOwner::Unix(0), own.clone(), &argv),
        PathFlavor::Windows,
    );
    assert!(!root.can_release);
    assert_eq!(
        root.blocked_reason.as_deref(),
        Some("Root-owned system process")
    );

    // An unavailable identity (the sentinel `ProcessOwner::current` returns
    // when no SID can be read) fails closed on every platform.
    let unidentified = classify_listener(
        &input(ProcessOwner::Windows(String::new()), own.clone(), &argv),
        PathFlavor::Windows,
    );
    assert!(!unidentified.can_release);
    assert_eq!(
        unidentified.blocked_reason.as_deref(),
        Some("Process identity is unavailable")
    );

    // The same listener owned by the current, unprivileged user is releasable.
    let allowed = classify_listener(&input(own.clone(), own, &argv), PathFlavor::Windows);
    assert!(
        allowed.can_release,
        "an unprivileged same-user dev server must stay releasable: {:?}",
        allowed.blocked_reason
    );
    assert_eq!(allowed.blocked_reason, None);
    assert_eq!(allowed.server_name, "Vite");
}

#[test]
fn test_real_ipc_model_rejects_unsafe_u64_values() {
    let metrics = DiskMetrics {
        mount_point: "/".into(),
        total_bytes: zenith_lib::ipc_numeric::MAX_SAFE_INTEGER + 1,
        used_bytes: 0,
        free_bytes: 0,
        available_bytes: 0,
        percent_used: 0.0,
    };

    let error = serde_json::to_string(&metrics).unwrap_err().to_string();
    assert!(error.contains("Number.MAX_SAFE_INTEGER"));
}

/// The CPU and battery readings carry the same wire rule as `DiskMetrics`:
/// every integer crosses IPC as a JSON number inside the range JavaScript can
/// represent, and an unsafe value is refused in both directions rather than
/// silently losing precision.
#[test]
fn test_cpu_and_battery_metrics_keep_their_ipc_integers_javascript_safe() {
    use zenith_lib::ipc_numeric::MAX_SAFE_INTEGER;
    use zenith_lib::models::{
        BatteryChargeState, BatteryMetrics, BatteryPresence, CpuMetrics, CpuSampleState,
        PowerSourceType,
    };

    let measured = CpuMetrics {
        state: CpuSampleState::Fresh,
        usage_percent: Some(41.5),
        sample_interval_ms: Some(1_000),
        sampled_at: Some(1_700_000_000_000),
        stale_after_ms: 3_000,
        cores: 8,
        reason: None,
    };
    let json = serde_json::to_value(&measured).expect("a measured reading serializes");
    assert!(json["sample_interval_ms"].is_number());
    assert_eq!(json["sample_interval_ms"], serde_json::json!(1_000));
    assert_eq!(json["sampled_at"], serde_json::json!(1_700_000_000_000u64));
    assert_eq!(json["stale_after_ms"], serde_json::json!(3_000));

    // A reading that was never measured keeps its keys and reports null rather
    // than dropping the field or filling it in.
    let warmup = CpuMetrics {
        state: CpuSampleState::Warmup,
        usage_percent: None,
        sample_interval_ms: None,
        sampled_at: None,
        ..measured.clone()
    };
    let json = serde_json::to_value(&warmup).expect("a warm-up reading serializes");
    assert!(json["usage_percent"].is_null());
    assert!(json["sample_interval_ms"].is_null());
    assert!(json["sampled_at"].is_null());
    assert_eq!(json["stale_after_ms"], serde_json::json!(3_000));
    assert_eq!(json["state"], serde_json::json!("warmup"));

    for unsafe_cpu in [
        CpuMetrics {
            sample_interval_ms: Some(MAX_SAFE_INTEGER + 1),
            ..measured.clone()
        },
        CpuMetrics {
            sampled_at: Some(MAX_SAFE_INTEGER + 1),
            ..measured.clone()
        },
        CpuMetrics {
            stale_after_ms: MAX_SAFE_INTEGER + 1,
            ..measured.clone()
        },
    ] {
        let error = serde_json::to_string(&unsafe_cpu)
            .expect_err("an unsafe integer must be refused")
            .to_string();
        assert!(error.contains("Number.MAX_SAFE_INTEGER"), "{error}");
    }

    let at_boundary = CpuMetrics {
        sample_interval_ms: Some(MAX_SAFE_INTEGER),
        stale_after_ms: MAX_SAFE_INTEGER,
        ..measured
    };
    let encoded = serde_json::to_string(&at_boundary).expect("the boundary is representable");
    assert_eq!(
        serde_json::from_str::<CpuMetrics>(&encoded).expect("the reading round-trips"),
        at_boundary
    );

    let discharging = BatteryMetrics {
        presence: BatteryPresence::Present,
        charge_state: BatteryChargeState::Discharging,
        percent: Some(72.0),
        time_remaining_seconds: Some(4_200),
        power_source: PowerSourceType::Battery,
        sampled_at: Some(1_700_000_000_000),
        reason: None,
    };
    let json = serde_json::to_value(&discharging).expect("a battery reading serializes");
    assert!(json["time_remaining_seconds"].is_number());
    assert_eq!(json["time_remaining_seconds"], serde_json::json!(4_200));
    assert!(json["sampled_at"].is_number());
    assert_eq!(json["power_source"], serde_json::json!("battery"));

    let unsafe_battery = BatteryMetrics {
        time_remaining_seconds: Some(MAX_SAFE_INTEGER + 1),
        ..discharging.clone()
    };
    let error = serde_json::to_string(&unsafe_battery)
        .expect_err("an unsafe estimate must be refused")
        .to_string();
    assert!(error.contains("Number.MAX_SAFE_INTEGER"), "{error}");

    let unsafe_sampled = BatteryMetrics {
        sampled_at: Some(MAX_SAFE_INTEGER + 1),
        ..discharging.clone()
    };
    assert!(serde_json::to_string(&unsafe_sampled).is_err());

    // A payload arriving from the other side is guarded the same way.
    let payload = format!(
        r#"{{"presence":"present","charge_state":"discharging","percent":72.0,"time_remaining_seconds":{},"power_source":"battery","sampled_at":null,"reason":null}}"#,
        MAX_SAFE_INTEGER + 1
    );
    let error = serde_json::from_str::<BatteryMetrics>(&payload)
        .expect_err("an unsafe estimate must be refused")
        .to_string();
    assert!(error.contains("Number.MAX_SAFE_INTEGER"), "{error}");

    // The states a battery-less machine reports keep their nulls.
    let absent = BatteryMetrics {
        presence: BatteryPresence::Absent,
        charge_state: BatteryChargeState::Unknown,
        percent: None,
        time_remaining_seconds: None,
        ..discharging
    };
    let json = serde_json::to_value(&absent).expect("an absent battery serializes");
    assert_eq!(json["presence"], serde_json::json!("absent"));
    assert_eq!(json["charge_state"], serde_json::json!("unknown"));
    assert!(json["percent"].is_null());
    assert!(json["time_remaining_seconds"].is_null());
}

/// The macOS adapter must describe the machine it is running on, including the
/// machines that have no battery at all: a desktop reports `Absent`, never a
/// battery at zero percent.
#[cfg(target_os = "macos")]
#[test]
fn test_live_battery_provider_describes_this_machine() {
    use zenith_lib::models::{BatteryChargeState, BatteryPresence};
    use zenith_lib::power::{battery_metrics_from_reading, BatteryProvider, SystemBatteryProvider};

    let reading = SystemBatteryProvider::new().read();

    match reading.presence {
        BatteryPresence::Present => {
            let percent = reading
                .percent
                .expect("a battery the platform lists reports its charge");
            assert!(
                (0.0..=100.0).contains(&percent),
                "a charge is a percentage of capacity: {percent}"
            );
            let metrics = battery_metrics_from_reading(reading, None);
            assert_eq!(metrics.presence, BatteryPresence::Present);
            assert_eq!(metrics.percent, Some(percent));
            assert_ne!(
                metrics.charge_state,
                BatteryChargeState::Unknown,
                "a present battery on macOS states whether it is charging: {metrics:?}"
            );
        }
        BatteryPresence::Absent => {
            assert_eq!(
                reading.percent, None,
                "a machine without a battery has no charge to report"
            );
            let metrics = battery_metrics_from_reading(reading, None);
            assert_eq!(metrics.percent, None);
            assert_eq!(metrics.charge_state, BatteryChargeState::Unknown);
            assert_eq!(metrics.time_remaining_seconds, None);
        }
        BatteryPresence::Unavailable => panic!(
            "macOS has a battery adapter, so an unavailable reading is a failed probe: {:?}",
            reading.reason
        ),
    }
}

#[test]
fn test_local_model_and_app_inventory_serialization_enforces_safe_integers() {
    use zenith_lib::models::{
        AppInstallSource, InstalledApp, InstalledAppInventory, LocalModelInventory, LocalModelItem,
        ModelSource, ObservationQuality,
    };

    let safe_model = LocalModelInventory {
        items: vec![LocalModelItem {
            id: "test".into(),
            name: "test".into(),
            source: ModelSource::Ollama,
            path: "/path".into(),
            size_bytes: 42,
            format: None,
            parameter_size: None,
            quantization: None,
            last_modified: Some(100),
            quality: ObservationQuality::Fresh,
            incomplete_reason: None,
            skipped_entries: 0,
        }],
        quality: ObservationQuality::Fresh,
        skipped_entry_count: 0,
        incomplete_reasons: Vec::new(),
    };
    let json = serde_json::to_string(&safe_model).expect("safe model serializes");
    assert!(json.contains("\"size_bytes\":42"));

    let unsafe_model = LocalModelInventory {
        items: vec![],
        quality: ObservationQuality::Partial,
        skipped_entry_count: zenith_lib::ipc_numeric::MAX_SAFE_INTEGER + 1,
        incomplete_reasons: Vec::new(),
    };
    let error = serde_json::to_string(&unsafe_model)
        .unwrap_err()
        .to_string();
    assert!(error.contains("Number.MAX_SAFE_INTEGER"));

    let safe_app = InstalledAppInventory {
        apps: vec![InstalledApp {
            id: "app".into(),
            name: "App".into(),
            bundle_id: None,
            version: None,
            display_path: "/Applications/App.app".into(),
            executable_name: None,
            logical_size: 100,
            allocated_size: 1024,
            modified_at: None,
            install_source: AppInstallSource::ApplicationBundle,
            is_running: false,
            is_system_protected: false,
            quality: ObservationQuality::Fresh,
            size_quality: ObservationQuality::Fresh,
            incomplete_reason: None,
            skipped_entries: 0,
        }],
        quality: ObservationQuality::Fresh,
        skipped_entry_count: 0,
        incomplete_reasons: Vec::new(),
    };
    let app_json = serde_json::to_string(&safe_app).expect("safe app serializes");
    assert!(app_json.contains("\"logical_size\":100"));

    let unsafe_app = InstalledAppInventory {
        apps: vec![],
        quality: ObservationQuality::Partial,
        skipped_entry_count: zenith_lib::ipc_numeric::MAX_SAFE_INTEGER + 1,
        incomplete_reasons: Vec::new(),
    };
    let app_error = serde_json::to_string(&unsafe_app).unwrap_err().to_string();
    assert!(app_error.contains("Number.MAX_SAFE_INTEGER"));

    use zenith_lib::models::{TrashPlanPreview, TrashResult};

    let safe_preview = TrashPlanPreview {
        id: uuid::Uuid::nil(),
        item_count: 1,
        logical_size: 100,
        allocated_size: 1024,
        expires_at: 5000,
        size_is_lower_bound: true,
    };
    let preview_json = serde_json::to_string(&safe_preview).expect("safe preview serializes");
    assert!(preview_json.contains("\"size_is_lower_bound\":true"));

    let unsafe_preview = TrashPlanPreview {
        id: uuid::Uuid::nil(),
        item_count: 1,
        logical_size: zenith_lib::ipc_numeric::MAX_SAFE_INTEGER + 1,
        allocated_size: 1024,
        expires_at: 5000,
        size_is_lower_bound: false,
    };
    let preview_error = serde_json::to_string(&unsafe_preview)
        .unwrap_err()
        .to_string();
    assert!(preview_error.contains("Number.MAX_SAFE_INTEGER"));

    let safe_result = TrashResult {
        moved_count: 1,
        failed_count: 0,
        skipped_count: 0,
        moved_allocated_size: 1024,
        items: vec![],
        size_is_lower_bound: true,
    };
    let result_json = serde_json::to_string(&safe_result).expect("safe result serializes");
    assert!(result_json.contains("\"size_is_lower_bound\":true"));

    let unsafe_result = TrashResult {
        moved_count: 1,
        failed_count: 0,
        skipped_count: 0,
        moved_allocated_size: zenith_lib::ipc_numeric::MAX_SAFE_INTEGER + 1,
        items: vec![],
        size_is_lower_bound: false,
    };
    let result_error = serde_json::to_string(&unsafe_result)
        .unwrap_err()
        .to_string();
    assert!(result_error.contains("Number.MAX_SAFE_INTEGER"));
}

/// The domain's authorization types must stay off the interface.
///
/// `DeletePlan`, `DeleteTarget`, and the captured filesystem identities are
/// mutation authority: a caller that holds one can replay a deletion without a
/// fresh scan. They carry no `serde` or `specta` derives, and this asserts the
/// consequence rather than the attribute — the bindings are generated from the
/// same Specta builder the commands use, so a type that reached the registry
/// would show up here as a declaration.
///
/// Declarations are matched by name, not by substring: `createDeletePlan` is a
/// command name and carries no authority, while a `DeletePlan_Serialize` type
/// alias is exactly the leak this guards against.
#[test]
fn generated_bindings_never_declare_authorization_types() {
    const AUTHORIZATION_TYPES: [&str; 6] = [
        "DeletePlan",
        "DeleteTarget",
        "FileIdentity",
        "CleanupIdentity",
        "ReviewedFileIdentity",
        "ModifiedStamp",
    ];

    let bindings = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/bindings/tauri.ts"),
    )
    .expect("the generated TypeScript bindings are committed to the repository");

    // An empty or truncated file would satisfy the negative check below, so the
    // surface it is supposed to describe is asserted first.
    for expected in ["export type PlanPreview", "export type ScanItem"] {
        assert!(
            bindings.contains(expected),
            "the bindings no longer declare `{expected}`; the leak check would pass on a file that describes nothing"
        );
    }

    let mut leaked = Vec::new();
    for line in bindings.lines() {
        let Some(declaration) = line.strip_prefix("export type ") else {
            continue;
        };
        let name = declaration
            .split([' ', '='])
            .next()
            .unwrap_or_default()
            .to_string();
        // Specta emits `Name_Serialize` / `Name_Deserialize` companions for
        // types whose two directions differ; the base name is the identifier.
        let base = name.split('_').next().unwrap_or_default().to_string();
        if AUTHORIZATION_TYPES.contains(&base.as_str()) {
            leaked.push(name);
        }
    }

    assert!(
        leaked.is_empty(),
        "authorization state crossed the IPC boundary as {leaked:?}; project it through `zenith_core::application::dto` instead"
    );
}
