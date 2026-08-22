use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::tempdir;
use zenith_lib::cleaner::CleanExecutor;
use zenith_lib::models::{
    Category, CategoryResult, CleanFailureReason, CleanStrategy, FileSize, RiskTier, ScanItem,
    ScanResult, Signature, ZenithError,
};
use zenith_lib::safety::{Blacklist, SafeTreeDeleter, SafetyPlanner, SymlinkGuard, ToctouGuard};
use zenith_lib::scanner::SizeCalculator;
use zenith_lib::signatures::SignatureRegistry;

#[test]
fn test_blacklist_root_and_home_rejection() {
    // 1. Root / must be rejected
    assert!(Blacklist::is_blacklisted(Path::new("/")));
    assert!(Blacklist::validate(Path::new("/")).is_err());

    // 2. Home ~ must be rejected
    if let Ok(home) = std::env::var("HOME") {
        let home_path = PathBuf::from(&home);
        assert!(Blacklist::is_blacklisted(&home_path));
        assert!(Blacklist::validate(&home_path).is_err());
    }
}

#[test]
fn test_blacklist_system_directories_rejection() {
    let sys_paths = [
        "/System",
        "/System/Library",
        "/bin",
        "/sbin",
        "/usr",
        "/usr/bin",
        "/etc",
        "/var",
        "/private",
        "/Applications",
        "/Library",
    ];

    for path_str in &sys_paths {
        let path = Path::new(path_str);
        assert!(
            Blacklist::is_blacklisted(path),
            "Expected {} to be blacklisted",
            path_str
        );
        assert!(Blacklist::validate(path).is_err());
    }
}

#[test]
fn test_blacklist_sensitive_user_directories() {
    if let Ok(home) = std::env::var("HOME") {
        let home_path = PathBuf::from(&home);
        let sensitive = [
            ".ssh",
            ".ssh/id_rsa",
            ".gnupg",
            ".aws",
            ".aws/credentials",
            ".azure",
            ".kube",
            "Library/Keychains",
            "Desktop",
            "Documents",
            "Pictures",
            "Movies",
            "Music",
        ];

        for rel in &sensitive {
            let full_path = home_path.join(rel);
            assert!(
                Blacklist::is_blacklisted(&full_path),
                "Expected {} to be blacklisted",
                full_path.display()
            );
            assert!(Blacklist::validate(&full_path).is_err());
        }
    }
}

#[test]
fn test_blacklist_parent_traversal_attacks() {
    if let Ok(home) = std::env::var("HOME") {
        let home_path = PathBuf::from(&home);
        // Attempting to escape via ../ into .ssh
        let attack_path = home_path.join(".cache/foo/../../.ssh");
        assert!(
            Blacklist::validate(&attack_path).is_err(),
            "Expected traversal attack to be rejected"
        );

        let attack_root = PathBuf::from("/Users/../System");
        assert!(Blacklist::validate(&attack_root).is_err());
    }
}

#[test]
fn test_blacklist_git_directory_rejection() {
    let git_dir = Path::new("/tmp/some-project/.git");
    assert!(Blacklist::is_blacklisted(git_dir));
    assert!(Blacklist::validate(git_dir).is_err());

    let git_file = Path::new("/tmp/some-project/.git/config");
    assert!(Blacklist::is_blacklisted(git_file));
}

#[test]
fn test_toctou_identity_verification_and_abort() {
    let dir = tempdir().expect("failed to create temp dir");
    let test_file = dir.path().join("cache.dat");

    // 1. Create original file
    {
        let mut f = File::create(&test_file).expect("failed to create file");
        f.write_all(b"original data").expect("failed to write");
    }

    // 2. Capture identity during scan
    let identity = ToctouGuard::capture(&test_file).expect("failed to capture identity");
    assert!(ToctouGuard::verify(&test_file, &identity).is_ok());

    // 3. Delete file and recreate as directory to simulate TOCTOU change
    fs::remove_file(&test_file).expect("failed to remove file");
    fs::create_dir(&test_file).expect("failed to create dir in place of file");

    // 4. Verification must now abort
    let verify_result = ToctouGuard::verify(&test_file, &identity);
    assert!(verify_result.is_err());
    match verify_result {
        Err(ZenithError::ChangedSinceScan(_)) => {}
        other => panic!("Expected ChangedSinceScan, got {:?}", other),
    }
}

#[test]
fn test_symlink_safety_and_no_escape() {
    let dir = tempdir().expect("failed to create temp dir");
    let outside_dir = tempdir().expect("failed to create outside temp dir");

    let outside_file = outside_dir.path().join("secret.txt");
    {
        let mut f = File::create(&outside_file).expect("create outside file");
        f.write_all(b"sensitive content")
            .expect("write outside file");
    }

    // Create a symlink inside the fixture pointing outside
    let symlink_path = dir.path().join("cache_link");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside_file, &symlink_path).expect("create symlink");
        assert!(SymlinkGuard::is_symlink(&symlink_path));

        // Size calculation on the directory with symlink must only measure the link, not traverse outside
        let (size, count) = SizeCalculator::measure_path(dir.path(), &[]);
        assert_eq!(count, 1);
        assert!(size.logical > 0);
    }
}

#[test]
fn test_safety_planner_rejects_unknown_signatures() {
    let registry = SignatureRegistry::load_embedded().expect("load embedded signatures");

    let fake_item = ScanItem {
        id: "unknown.signature.123".to_string(),
        signature_id: "unknown.signature.123".to_string(),
        name: "Fake Cache".to_string(),
        category: Category::Ai,
        risk: RiskTier::Safe,
        path: "/tmp/fake-cache".to_string(),
        size: FileSize::new(1024, Some(1024)),
        file_count: 1,
        description: "fake".to_string(),
        is_selected: true,
        last_modified: None,
        exists: true,
    };

    let plan_res = SafetyPlanner::create_plan(&[fake_item], &registry);
    assert!(plan_res.is_err());
    match plan_res {
        Err(ZenithError::SignatureMismatch(_)) => {}
        other => panic!("Expected SignatureMismatch, got {:?}", other),
    }
}

#[test]
fn test_safety_planner_rejects_path_outside_signature_scope() {
    let registry = SignatureRegistry::load_embedded().expect("load embedded signatures");
    let dir = tempdir().expect("tempdir");
    let forged_path = dir.path().join("codex-forged");
    fs::create_dir(&forged_path).unwrap();

    let forged_item = ScanItem {
        id: "system.developer_temp.0.codex-forged".into(),
        signature_id: "system.developer_temp".into(),
        name: "Forged temp item".into(),
        category: Category::System,
        risk: RiskTier::Safe,
        path: forged_path.to_string_lossy().into_owned(),
        size: FileSize::new(1024, Some(1024)),
        file_count: 1,
        description: "must not be planned".into(),
        is_selected: true,
        last_modified: None,
        exists: true,
    };

    let result = SafetyPlanner::create_plan(&[forged_item], &registry);
    assert!(matches!(result, Err(ZenithError::SignatureMismatch(_))));
}

#[test]
fn test_cleaner_delete_contents_preserves_root_directory() {
    let dir = tempdir().expect("create temp dir");
    let cache_root = dir.path().join("cargo_cache");
    fs::create_dir(&cache_root).expect("create cache root");

    // Add subfiles and subdirectories
    let subfile = cache_root.join("test.crate");
    File::create(&subfile)
        .unwrap()
        .write_all(b"dummy crate")
        .unwrap();
    let subdir = cache_root.join("subfolder");
    fs::create_dir(&subdir).unwrap();
    File::create(subdir.join("inner.bin"))
        .unwrap()
        .write_all(b"inner")
        .unwrap();

    let mut registry = SignatureRegistry::load_embedded().unwrap();
    registry.register(Signature {
        id: "test.delete-contents".into(),
        name: "Test cache".into(),
        category: Category::Developer,
        risk: RiskTier::Safe,
        strategy: CleanStrategy::DeleteContents,
        paths: vec![cache_root.to_string_lossy().into_owned()],
        exclusions: vec![],
        description: "test-only signature".into(),
        min_age_days: None,
        include_prefixes: vec![],
        exclude_prefixes: vec![],
        intensive_only: false,
    });

    let scan_item = ScanItem {
        id: "test.delete-contents".to_string(),
        signature_id: "test.delete-contents".to_string(),
        name: "Cargo Registry Cache".to_string(),
        category: Category::Developer,
        risk: RiskTier::Safe,
        path: cache_root.to_string_lossy().to_string(),
        size: FileSize::new(2048, Some(2048)),
        file_count: 2,
        description: "test".to_string(),
        is_selected: true,
        last_modified: None,
        exists: true,
    };

    let plan = SafetyPlanner::create_plan(&[scan_item], &registry).expect("create plan");
    assert_eq!(plan.targets.len(), 1);

    let clean_res = CleanExecutor::execute(plan, |_| {});
    assert_eq!(clean_res.items.len(), 1);
    assert!(clean_res.items[0].success);

    // Root cache dir must still exist!
    assert!(cache_root.exists());
    assert!(cache_root.is_dir());

    // Inner subfiles must be deleted
    assert!(!subfile.exists());
    assert!(!subdir.exists());
}

#[test]
fn frontend_selection_must_resolve_against_trusted_scan() {
    let registry = SignatureRegistry::load_embedded().unwrap();
    let scan = ScanResult {
        scan_id: "trusted-scan".into(),
        started_at: 1,
        finished_at: 2,
        categories: vec![CategoryResult {
            category: Category::Developer,
            display_name: "Developer".into(),
            items: vec![],
            total_bytes: 0,
            safe_bytes: 0,
            rebuild_bytes: 0,
            manual_bytes: 0,
        }],
        total_bytes: 0,
        safe_bytes: 0,
        rebuild_bytes: 0,
        manual_bytes: 0,
    };

    let forged = vec!["frontend-supplied-arbitrary-path".to_string()];
    assert!(matches!(
        SafetyPlanner::create_plan_from_scan(&scan, "trusted-scan", &forged, &registry),
        Err(ZenithError::InvalidPlan(_))
    ));
    assert!(matches!(
        SafetyPlanner::create_plan_from_scan(&scan, "stale-scan", &forged, &registry),
        Err(ZenithError::InvalidPlan(_))
    ));
}

#[test]
fn manual_strategy_never_enters_generic_cleaner() {
    let dir = tempdir().unwrap();
    let model_root = dir.path().join("model");
    fs::create_dir(&model_root).unwrap();
    let mut registry = SignatureRegistry::load_embedded().unwrap();
    registry.register(Signature {
        id: "test.manual-model".into(),
        name: "Manual model".into(),
        category: Category::Model,
        risk: RiskTier::Manual,
        strategy: CleanStrategy::Manual,
        paths: vec![model_root.to_string_lossy().into_owned()],
        exclusions: vec![],
        description: "adapter-only".into(),
        min_age_days: None,
        include_prefixes: vec![],
        exclude_prefixes: vec![],
        intensive_only: false,
    });
    let item = ScanItem {
        id: "test.manual-model".into(),
        signature_id: "test.manual-model".into(),
        name: "Manual model".into(),
        category: Category::Model,
        risk: RiskTier::Manual,
        path: model_root.to_string_lossy().into_owned(),
        size: FileSize::new(1, Some(1)),
        file_count: 1,
        description: "adapter-only".into(),
        is_selected: true,
        last_modified: None,
        exists: true,
    };

    assert!(matches!(
        SafetyPlanner::create_plan(&[item], &registry),
        Err(ZenithError::UnsupportedManualOperation(_))
    ));
    assert!(model_root.exists());
}

#[test]
fn recursive_delete_preserves_nested_git_and_declared_exclusions() {
    let dir = tempdir().unwrap();
    let cache_root = dir.path().join("cache");
    let nested = cache_root.join("nested");
    let git = nested.join(".git");
    let excluded = nested.join("settings.json");
    let removable = nested.join("cache.bin");
    fs::create_dir_all(&git).unwrap();
    fs::write(git.join("config"), b"protected").unwrap();
    fs::write(&excluded, b"settings").unwrap();
    fs::write(&removable, b"cache").unwrap();

    let exclusions = vec![excluded.to_string_lossy().into_owned()];
    let report = SafeTreeDeleter::delete_contents(&cache_root, &exclusions);
    assert!(report.is_success());

    assert!(cache_root.exists());
    assert!(git.join("config").exists());
    assert!(excluded.exists());
    assert!(!removable.exists());
}

#[test]
fn test_ancestor_symlink_escape_rejection() {
    let dir = tempdir().expect("create temp dir");
    let trusted_root = dir.path().join("cargo");
    fs::create_dir_all(&trusted_root).unwrap();

    let outside_dir = tempdir().expect("create outside temp dir");
    let precious_file = outside_dir.path().join("precious_data.txt");
    fs::write(&precious_file, b"cannot be deleted").unwrap();

    // Create an intermediate symlink: cargo/registry -> outside_dir
    let symlink_dir = trusted_root.join("registry");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside_dir.path(), &symlink_dir).expect("create symlink");

        let target_path = symlink_dir.join("cache");
        // Verify ancestor symlink detection
        let validation_res =
            SymlinkGuard::validate_no_symlink_ancestors(&target_path, &trusted_root);
        assert!(
            validation_res.is_err(),
            "Ancestor symlink must be rejected!"
        );
        assert!(matches!(validation_res, Err(ZenithError::SymlinkEscape(_))));

        // Precious file outside must remain intact
        assert!(precious_file.exists());
    }
}

#[test]
fn test_sparse_file_zero_allocated_bytes() {
    let size = FileSize::new(100 * 1024 * 1024, Some(0));
    assert_eq!(size.reclaimable(), 0);

    let size_unknown = FileSize::new(100 * 1024 * 1024, None);
    assert_eq!(size_unknown.reclaimable(), 100 * 1024 * 1024);
}

#[test]
fn test_symlink_ancestor_above_signature_root_rejection() {
    let base_dir = tempdir().expect("create base temp dir");
    let outside_dir = tempdir().expect("create outside temp dir");
    let precious = outside_dir.path().join("precious_code.rs");
    fs::write(&precious, b"fn important() {}").unwrap();

    // Create intermediate symlink: base_dir/.cargo -> outside_dir
    let symlink_dot_cargo = base_dir.path().join(".cargo");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside_dir.path(), &symlink_dot_cargo).expect("create symlink");

        let signature_target = symlink_dot_cargo.join("registry").join("cache");

        // Validate that checking against base_dir detects the .cargo symlink
        let validation_res =
            SymlinkGuard::validate_components_between(&signature_target, base_dir.path());
        assert!(
            validation_res.is_err(),
            "Symlink above signature root must be rejected!"
        );
        assert!(matches!(validation_res, Err(ZenithError::SymlinkEscape(_))));

        assert!(precious.exists());
    }
}

#[test]
fn test_signature_root_itself_symlink_rejection() {
    let base_dir = tempdir().expect("create base temp dir");
    let outside_dir = tempdir().expect("create outside temp dir");
    let precious = outside_dir.path().join("precious.txt");
    fs::write(&precious, b"cannot delete").unwrap();

    // signature root itself is a symlink: base_dir/cache -> outside_dir
    let symlink_cache = base_dir.path().join("cache");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside_dir.path(), &symlink_cache).expect("create symlink");

        let validation_res =
            SymlinkGuard::validate_components_between(&symlink_cache, base_dir.path());
        assert!(
            validation_res.is_err(),
            "Signature root as symlink must be rejected!"
        );
        assert!(matches!(validation_res, Err(ZenithError::SymlinkEscape(_))));

        assert!(precious.exists());
    }
}

#[test]
fn test_docker_prune_target_can_create_plan() {
    let registry = SignatureRegistry::load_embedded().expect("load embedded signatures");
    let docker_item = ScanItem {
        id: "container.docker.builder".to_string(),
        signature_id: "container.docker.builder".to_string(),
        name: "Docker Build Cache".to_string(),
        category: Category::Container,
        risk: RiskTier::Safe,
        path: "docker://buildkit/cache".to_string(),
        size: FileSize::new(1024 * 1024, Some(1024 * 1024)),
        file_count: 1,
        description: "Docker build cache".to_string(),
        is_selected: true,
        last_modified: None,
        exists: true,
    };

    let plan = SafetyPlanner::create_plan(&[docker_item], &registry)
        .expect("DockerPrune target must successfully create a plan");
    assert_eq!(plan.targets.len(), 1);
    assert_eq!(plan.targets[0].strategy, CleanStrategy::DockerPrune);
}

#[test]
fn test_stale_temp_toctou_recheck_aborts_on_new_file() {
    let dir = tempdir().expect("create temp dir");
    let temp_child = dir.path().join("active_tool_cache");
    fs::create_dir(&temp_child).unwrap();

    // Create a file modified right now
    let new_file = temp_child.join("active.log");
    fs::write(&new_file, b"actively writing").unwrap();

    let mut registry = SignatureRegistry::load_embedded().unwrap();
    registry.register(Signature {
        id: "test.stale_temp".into(),
        name: "Test stale temp".into(),
        category: Category::Developer,
        risk: RiskTier::Safe,
        strategy: CleanStrategy::DeleteDirectory,
        paths: vec![dir.path().to_string_lossy().into_owned()],
        exclusions: vec![],
        description: "stale temp test".into(),
        min_age_days: Some(3),
        include_prefixes: vec![],
        exclude_prefixes: vec![],
        intensive_only: false,
    });

    let scan_item = ScanItem {
        id: "test.stale_temp.0.active_tool_cache".into(),
        signature_id: "test.stale_temp".into(),
        name: "active_tool_cache".into(),
        category: Category::Developer,
        risk: RiskTier::Safe,
        path: temp_child.to_string_lossy().into_owned(),
        size: FileSize::new(1024, Some(1024)),
        file_count: 1,
        description: "test".into(),
        is_selected: true,
        last_modified: None,
        exists: true,
    };

    let plan = SafetyPlanner::create_plan(&[scan_item], &registry).expect("create plan");
    assert_eq!(plan.targets[0].min_age_days, Some(3));

    let clean_res = CleanExecutor::execute(plan, |_| {});
    assert_eq!(clean_res.items.len(), 1);
    assert!(!clean_res.items[0].success);
    assert_eq!(
        clean_res.items[0].failure_reason,
        Some(CleanFailureReason::ChangedSinceScan)
    );
    // Active directory must be preserved
    assert!(temp_child.exists());
    assert!(new_file.exists());
}

#[test]
fn test_antigravity_cache_exclusions_preserve_onboarding_and_auth() {
    let dir = tempdir().expect("create temp dir");
    let cache_dir = dir.path().join("cache");
    fs::create_dir_all(&cache_dir).unwrap();

    let onboarding_file = cache_dir.join("onboarding.json");
    let project_id_file = cache_dir.join("default_project_id.txt");
    let transient_cache = cache_dir.join("ephemeral_prompt_cache.bin");

    fs::write(&onboarding_file, b"{\"onboardingComplete\": true}").unwrap();
    fs::write(&project_id_file, b"project-12345").unwrap();
    fs::write(&transient_cache, b"transient session data").unwrap();

    let registry = SignatureRegistry::load_embedded().expect("load embedded");
    let gemini_sig = registry
        .get("ai.gemini.cache")
        .expect("ai.gemini.cache signature exists");

    // Verify exclusions contain onboarding.json and default_project_id.txt
    assert!(gemini_sig
        .exclusions
        .iter()
        .any(|ex| ex == "onboarding.json" || ex.ends_with("onboarding.json")));
    assert!(gemini_sig
        .exclusions
        .iter()
        .any(|ex| ex == "default_project_id.txt" || ex.ends_with("default_project_id.txt")));

    // Measure size with signature exclusions
    let (measured_size, measured_count) =
        SizeCalculator::measure_path(&cache_dir, &gemini_sig.exclusions);
    assert_eq!(
        measured_count, 1,
        "Only transient_cache should be counted as reclaimable"
    );
    assert_eq!(
        measured_size.logical,
        b"transient session data".len() as u64
    );

    // Perform delete_contents
    let report = SafeTreeDeleter::delete_contents(&cache_dir, &gemini_sig.exclusions);
    assert!(report.is_success());
    assert_eq!(report.deleted_files, 1);
    assert_eq!(report.skipped_files, 2);

    // Onboarding and project ID must be preserved!
    assert!(
        onboarding_file.exists(),
        "onboarding.json must be preserved"
    );
    assert!(
        project_id_file.exists(),
        "default_project_id.txt must be preserved"
    );
    assert!(!transient_cache.exists(), "transient_cache must be deleted");
}
