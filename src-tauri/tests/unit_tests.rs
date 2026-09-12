use std::fs::File;
use std::io::Write;
use tempfile::tempdir;
use zenith_lib::docker::DockerAdapter;
use zenith_lib::models::{
    AwakeBehavior, Category, CleanStrategy, DiskMetrics, RiskTier, Signature,
};
use zenith_lib::platform::PlatformEnvironment;
use zenith_lib::power::{KeepAwakeManager, PowerAssertion};
use zenith_lib::scanner::{DirectoryScanner, ScanEngine, SizeCalculator};
use zenith_lib::signatures::SignatureRegistry;

#[test]
fn test_signature_registry_categories_and_risk_counts() {
    let registry = SignatureRegistry::load_embedded().expect("load embedded");

    // All categories must have valid signatures
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
    assert!(!model_sigs.is_empty(), "Model signatures must not be empty");
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

    // AI and Dev safe signatures check
    let safe_sigs = registry.by_risk(RiskTier::Safe);
    assert!(safe_sigs.iter().any(|s| s.id == "ai.claude.logs"));
    assert!(safe_sigs.iter().any(|s| s.id == "dev.go.build"));
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
        provider: String::new(),
        management_mode: Default::default(),
        artifact_kind: Default::default(),
        consequence: String::new(),
        reclaimable_is_lower_bound: false,
    };

    let items = DirectoryScanner::scan_signature(&signature, &PlatformEnvironment::native());
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
        provider: String::new(),
        management_mode: Default::default(),
        artifact_kind: Default::default(),
        consequence: String::new(),
        reclaimable_is_lower_bound: false,
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
        Some(&[Category::System]),
        &[],
        false,
        &PlatformEnvironment::native(),
        |_| {},
    );
    let items = &result.categories[0].items;
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, "large");
    assert_eq!(items[1].id, "small");

    let excluded = vec!["large".to_string()];
    let filtered = ScanEngine::scan(
        &registry,
        Some(&[Category::System]),
        &excluded,
        false,
        &PlatformEnvironment::native(),
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
    let (total_size, total_count) =
        SizeCalculator::measure_path(dir.path(), &[], &PlatformEnvironment::native());
    assert_eq!(total_count, 2);
    assert!(total_size.logical >= 60000);

    // Measure with exclusion of "excluded_folder"
    let (filtered_size, filtered_count) = SizeCalculator::measure_path(
        dir.path(),
        &["excluded_folder".to_string()],
        &PlatformEnvironment::native(),
    );
    assert_eq!(filtered_count, 1);
    assert_eq!(filtered_size.logical, 10000);
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

    manager.set_rules(vec![rule_ac, rule_always]);
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
    use zenith_lib::platform::path_algebra::PathFlavor;
    use zenith_lib::platform::paths::SimulatedPaths;
    use zenith_lib::platform::KnownFolder;
    use zenith_lib::safety::blacklist::{classify_windows, BlacklistEnvironment, BlacklistVerdict};
    use zenith_lib::safety::Blacklist;

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

    let host = PlatformEnvironment::native();

    let user_cache = Path::new(r"\\?\C:\Users\테스트\.gemini\antigravity-cli\log");
    assert_eq!(
        Blacklist::normalize_path(user_cache),
        PathBuf::from(r"C:\Users\테스트\.gemini\antigravity-cli\log")
    );
    assert_eq!(
        Blacklist::normalize_path(Path::new(r"\\?\UNC\server\share\cache")),
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
    assert_eq!(
        caps.intensive_cleanup.status,
        PlatformFeatureStatus::Unavailable
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
    let res_ps = classify_listener(&input_ps);
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
    let res_vite = classify_listener(&input_vite);
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
    let root = classify_listener(&input(ProcessOwner::Unix(0), own.clone(), &argv));
    assert!(!root.can_release);
    assert_eq!(
        root.blocked_reason.as_deref(),
        Some("Root-owned system process")
    );

    // An unavailable identity (the sentinel `ProcessOwner::current` returns
    // when no SID can be read) fails closed on every platform.
    let unidentified = classify_listener(&input(
        ProcessOwner::Windows(String::new()),
        own.clone(),
        &argv,
    ));
    assert!(!unidentified.can_release);
    assert_eq!(
        unidentified.blocked_reason.as_deref(),
        Some("Process identity is unavailable")
    );

    // The same listener owned by the current, unprivileged user is releasable.
    let allowed = classify_listener(&input(own.clone(), own, &argv));
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
