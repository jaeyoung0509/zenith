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
    };

    let items = DirectoryScanner::scan_signature(&signature);
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

    let result = ScanEngine::scan(&registry, Some(&[Category::System]), &[], false, |_| {});
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
    let (total_size, total_count) = SizeCalculator::measure_path(dir.path(), &[]);
    assert_eq!(total_count, 2);
    assert!(total_size.logical >= 60000);

    // Measure with exclusion of "excluded_folder"
    let (filtered_size, filtered_count) =
        SizeCalculator::measure_path(dir.path(), &["excluded_folder".to_string()]);
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
        behavior: AwakeBehavior::PreventSystemSleep,
        power_condition: PowerCondition::AcPowerOnly,
        enabled: true,
    };

    let rule_always = AwakeRule {
        id: "rule.always".to_string(),
        app_name: "Always App".to_string(),
        executable_pattern: "non_existent_process_def".to_string(),
        requires_process_pattern: None,
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
    use zenith_lib::safety::Blacklist;

    // Drive root
    assert!(Blacklist::is_blacklisted(Path::new("C:\\")));
    assert!(Blacklist::is_blacklisted(Path::new("D:/")));

    // Windows System directories
    assert!(Blacklist::is_blacklisted(Path::new("C:\\Windows")));
    assert!(Blacklist::is_blacklisted(Path::new(
        "C:\\Windows\\System32"
    )));
    assert!(Blacklist::is_blacklisted(Path::new("C:\\Program Files")));
    assert!(Blacklist::is_blacklisted(Path::new(
        "C:\\Program Files (x86)"
    )));
    assert!(Blacklist::is_blacklisted(Path::new("C:\\Users")));

    // Alternate Data Streams and trailing aliases
    assert!(Blacklist::is_blacklisted(Path::new(
        "C:\\safe\\file.txt:stream"
    )));
    assert!(Blacklist::is_blacklisted(Path::new("C:\\safe\\folder.")));
    assert!(Blacklist::is_blacklisted(Path::new("C:\\safe\\folder ")));
}

#[cfg(target_os = "windows")]
#[test]
fn test_windows_verbatim_paths_preserve_blacklist_boundaries() {
    use std::path::{Path, PathBuf};
    use zenith_lib::safety::Blacklist;

    let user_cache = Path::new(r"\\?\C:\Users\테스트\.gemini\antigravity-cli\log");
    assert_eq!(
        Blacklist::normalize_path(user_cache),
        PathBuf::from(r"C:\Users\테스트\.gemini\antigravity-cli\log")
    );
    assert_eq!(
        Blacklist::normalize_path(Path::new(r"\\?\UNC\server\share\cache")),
        PathBuf::from(r"\\server\share\cache")
    );
    assert!(!Blacklist::is_blacklisted(user_cache));
    assert!(Blacklist::validate(user_cache).is_ok());

    assert!(Blacklist::is_blacklisted(Path::new(r"\\?\C:\")));
    assert!(Blacklist::is_blacklisted(Path::new(
        r"\\?\C:\Windows\System32"
    )));
    assert!(Blacklist::is_blacklisted(Path::new(
        r"\\?\C:\safe\file.txt:stream"
    )));
    assert!(Blacklist::is_blacklisted(Path::new(
        r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy1"
    )));
    assert!(Blacklist::is_blacklisted(Path::new(r"\\.\PhysicalDrive0")));
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
    assert_eq!(caps.installed_apps.status, PlatformFeatureStatus::Available);
    assert_eq!(caps.app_uninstall.status, PlatformFeatureStatus::Available);
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

    // Protected PowerShell / Windows Terminal
    let input_ps = ProcessClassificationInput {
        pid: 4500,
        uid: Some(1000),
        current_user_uid: 1000,
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
        uid: Some(1000),
        current_user_uid: 1000,
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
