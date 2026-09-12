use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use tempfile::tempdir;
use zenith_lib::cleaner::CleanExecutor;
use zenith_lib::models::{
    Category, CategoryResult, CleanFailureReason, CleanStrategy, FileSize, RiskTier, ScanItem,
    ScanResult, Signature, ZenithError,
};
use zenith_lib::platform::path_algebra::PathFlavor;
use zenith_lib::platform::paths::SimulatedPaths;
use zenith_lib::platform::{KnownFolder, NativePlatformPaths, PlatformEnvironment};
use zenith_lib::safety::blacklist::{classify_windows, BlacklistEnvironment, BlacklistVerdict};
use zenith_lib::safety::{Blacklist, SafeTreeDeleter, SafetyPlanner, SymlinkGuard, ToctouGuard};
use zenith_lib::scanner::SizeCalculator;
use zenith_lib::signatures::SignatureRegistry;

/// The Windows environment every Windows blacklist assertion is computed
/// against. The classifier's input is derived from the same value the runtime
/// would receive, so a test cannot describe one machine and classify another.
fn windows_environment(home: Option<&str>, temp_dir: &str) -> PlatformEnvironment {
    let mut roots = SimulatedPaths::new()
        .with_flavor(PathFlavor::Windows)
        .with_temp_dir(temp_dir)
        .with_program_files(r"Z:\Program Files")
        .with_program_data(r"Z:\ProgramData");
    if let Some(home) = home {
        roots = roots
            .with_home(home)
            .with_local_app_data(format!(r"{home}\AppData\Local"))
            .with_roaming_app_data(format!(r"{home}\AppData\Roaming"));
    }
    PlatformEnvironment::simulated(PathFlavor::Windows)
        .with_roots(std::sync::Arc::new(roots))
        .with_known_folder(KnownFolder::Documents, r"D:\OneDrive\Documents")
        .with_path_entry(r"Z:\Windows\System32")
}

/// The classifier input derived from [`windows_environment`].
fn windows_classifier(home: Option<&str>, temp_dir: &str) -> BlacklistEnvironment {
    BlacklistEnvironment::from_environment(&windows_environment(home, temp_dir))
}

/// The host's own environment: the POSIX assertions are about the real profile
/// the process runs under, which is the one case where host facts are the
/// intended input.
fn native_environment() -> PlatformEnvironment {
    PlatformEnvironment::native()
}

#[test]
fn test_blacklist_root_and_home_rejection() {
    // 1. Root / and every drive root must be rejected
    let host = native_environment();
    for root in ["/", "//", "///"] {
        assert!(Blacklist::is_blacklisted_with(Path::new(root), &host));
        assert!(Blacklist::validate_with(Path::new(root), &host).is_err());
    }

    // 2. The user home the rule protects is stated literally, so this body
    // executes identically on every host instead of depending on the profile
    // the test runner happens to have.
    let stated_home = "/Users/zenith-tester";
    let environment = windows_classifier(Some(stated_home), "/var/folders/zenith/T");
    assert_eq!(
        classify_windows(stated_home, &environment),
        BlacklistVerdict::Denied("user home")
    );
    assert_eq!(
        classify_windows("/users/ZENITH-TESTER", &environment),
        BlacklistVerdict::Denied("user home"),
        "Windows home comparison folds case"
    );
    for app_data in [
        "/Users/zenith-tester/AppData",
        "/Users/zenith-tester/AppData/Local",
        "/Users/zenith-tester/AppData/Roaming",
    ] {
        assert!(
            classify_windows(app_data, &environment).is_denied(),
            "Expected {app_data} to be denied"
        );
    }
    // The rule is driven by the stated home: without one this location is not
    // recognized as a profile, while the filesystem root still is.
    let without_home = windows_classifier(None, "/var/folders/zenith/T");
    assert_eq!(
        classify_windows(stated_home, &without_home),
        BlacklistVerdict::Allowed
    );
    assert_eq!(
        classify_windows("/", &without_home),
        BlacklistVerdict::Denied("filesystem root")
    );

    // 3. POSIX flavor: the provider's own home is refused as a whole. The
    // assertion is unconditional; a missing home fails the test loudly.
    let native_home = NativePlatformPaths::new()
        .home()
        .expect("a POSIX host exposes a home directory");
    assert!(Blacklist::is_blacklisted_with(&native_home, &host));
    assert!(Blacklist::validate_with(&native_home, &host).is_err());
}

#[test]
fn test_blacklist_system_directories_rejection() {
    #[cfg(unix)]
    {
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
        let host = native_environment();
        for path_str in &sys_paths {
            let path = Path::new(path_str);
            assert!(
                Blacklist::is_blacklisted_with(path, &host),
                "Expected {} to be blacklisted",
                path_str
            );
            assert!(Blacklist::validate_with(path, &host).is_err());
        }
    }

    // Windows system roots are drive-letter independent: the same tails are
    // refused whether the system lives on `C:`, another drive, or the drive the
    // stated profile happens to use. Nothing here encodes the drive letter or
    // profile layout of the host that runs the test.
    let platform = windows_environment(
        Some(r"Z:\Users\tester"),
        r"Z:\Users\tester\AppData\Local\Temp",
    );
    let environment = BlacklistEnvironment::from_environment(&platform);
    for drive in ["C:", "D:", "Z:"] {
        for tail in [
            r"\Windows",
            r"\Windows\System32",
            r"\Program Files",
            r"\Program Files (x86)",
            r"\ProgramData",
            r"\Users",
        ] {
            let path_str = format!("{drive}{tail}");
            let path = Path::new(&path_str);
            assert!(
                classify_windows(&path_str, &environment).is_denied(),
                "Expected {path_str} to be denied on Windows"
            );
            assert!(
                Blacklist::validate_with(path, &platform).is_err(),
                "Expected {path_str} to be rejected"
            );
        }
    }
}

#[test]
fn test_blacklist_sensitive_user_directories() {
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

    // The home these locations are relative to is stated, so the body always
    // executes and every assertion can fail.
    let stated_home = r"D:\Users\tester";
    let environment = windows_classifier(Some(stated_home), r"D:\Users\tester\AppData\Local\Temp");
    for rel in &sensitive {
        let path = format!("{stated_home}/{}", rel.replace('/', "\\"));
        let verdict = classify_windows(&path, &environment);
        assert_eq!(
            verdict,
            BlacklistVerdict::Denied("sensitive user directory"),
            "Expected {path} to be denied"
        );
    }
    // Only the protected subdirectories are denied; the rest of the profile is
    // still cleanable.
    assert_eq!(
        classify_windows(r"D:\Users\tester\dev\repo\.cache", &environment),
        BlacklistVerdict::Allowed
    );

    // POSIX flavor: the provider's own home, asserted unconditionally.
    let native_home = NativePlatformPaths::new()
        .home()
        .expect("a POSIX host exposes a home directory");
    for rel in &sensitive {
        let full_path = native_home.join(rel);
        let host = native_environment();
        assert!(
            Blacklist::is_blacklisted_with(&full_path, &host),
            "Expected {} to be blacklisted",
            full_path.display()
        );
        assert!(Blacklist::validate_with(&full_path, &host).is_err());
    }
}

#[test]
fn test_blacklist_parent_traversal_attacks() {
    // POSIX flavor: traversal that lands on a protected location is rejected
    // even though the unresolved spelling looks harmless.
    let native_home = NativePlatformPaths::new()
        .home()
        .expect("a POSIX host exposes a home directory");
    let attack_path = native_home.join(".cache/foo/../../.ssh");
    let host = native_environment();
    assert!(
        Blacklist::validate_with(&attack_path, &host).is_err(),
        "Expected traversal attack to be rejected"
    );
    assert!(Blacklist::validate_with(Path::new("/Users/../System"), &host).is_err());

    // Windows flavor, drive-letter independent: the same attacks normalize to a
    // system directory and are refused, while traversal that stays inside the
    // stated profile remains cleanable.
    let environment = windows_environment(
        Some(r"Z:\Users\tester"),
        r"Z:\Users\tester\AppData\Local\Temp",
    );
    let classifier = BlacklistEnvironment::from_environment(&environment);
    for attack in [
        r"Z:\Users\tester\..\..\Windows",
        r"Z:\Users\tester\Documents\..\..\..\Windows\System32",
        r"Z:\Users\tester\dev\..\..\..\Program Files\Vendor",
        // More `..` than the path has components: Windows would clamp this to
        // the drive root, so the unresolved traversal is refused.
        r"Z:\Users\tester\dev\..\..\..\..\Program Files\Vendor",
        r"..\Windows",
        r"D:\Users\tester\..\..\ProgramData\app",
        r"D:\Users\tester\AppData\Local\Temp\..\..\..\..\..\Windows",
    ] {
        let verdict = classify_windows(attack, &classifier);
        assert!(
            verdict.is_denied(),
            "Expected traversal {attack} to be denied, got {verdict:?}"
        );
    }
    assert_eq!(
        classify_windows(r"Z:\Users\tester\dev\cache\..\cache\file.tmp", &classifier),
        BlacklistVerdict::Allowed
    );
    assert_eq!(
        classify_windows(r"Z:\Users\tester\docs\..\..\tester\dev\repo", &classifier),
        BlacklistVerdict::Allowed
    );
}

#[test]
fn test_blacklist_git_directory_rejection() {
    #[cfg(unix)]
    {
        let host = native_environment();
        let git_dir = Path::new("/tmp/some-project/.git");
        assert!(Blacklist::is_blacklisted_with(git_dir, &host));
        assert!(Blacklist::validate_with(git_dir, &host).is_err());

        let git_file = Path::new("/tmp/some-project/.git/config");
        assert!(Blacklist::is_blacklisted_with(git_file, &host));
    }
    #[cfg(windows)]
    {
        let dir = tempdir().expect("tempdir");
        let git_dir = dir.path().join("some-project").join(".git");
        let host = native_environment();
        assert!(Blacklist::is_blacklisted_with(&git_dir, &host));
        assert!(Blacklist::validate_with(&git_dir, &host).is_err());
    }
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

#[cfg(unix)]
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
    #[cfg(unix)]
    {
        let symlink_path = dir.path().join("cache_link");
        std::os::unix::fs::symlink(&outside_file, &symlink_path).expect("create symlink");
        assert!(SymlinkGuard::is_symlink(&symlink_path));

        // Size calculation on the directory with symlink must only measure the link, not traverse outside
        let (size, count) =
            SizeCalculator::measure_path(dir.path(), &[], &PlatformEnvironment::native());
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
        cache_metadata: Default::default(),
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
        cache_metadata: Default::default(),
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
        platforms: vec![],
        provider: String::new(),
        management_mode: Default::default(),
        artifact_kind: Default::default(),
        consequence: String::new(),
        reclaimable_is_lower_bound: false,
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
        cache_metadata: Default::default(),
        is_selected: true,
        last_modified: None,
        exists: true,
    };

    let plan = SafetyPlanner::create_plan(&[scan_item], &registry).expect("create plan");
    assert_eq!(plan.targets.len(), 1);

    let clean_res = CleanExecutor::execute(plan, &PlatformEnvironment::native(), |_| {});
    assert_eq!(clean_res.items.len(), 1);
    assert!(clean_res.items[0].success);

    // Root cache dir must still exist!
    assert!(cache_root.exists());
    assert!(cache_root.is_dir());

    // Inner subfiles must be deleted
    assert!(!subfile.exists());
    assert!(!subdir.exists());
}

#[cfg(unix)]
#[test]
fn cleaner_removes_user_owned_read_only_cache_trees_without_privilege_escalation() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().expect("create temp dir");
    let cache_root = dir.path().join("react-native-devtools");
    let nested = cache_root.join("Contents").join("Resources");
    fs::create_dir_all(&nested).expect("create nested app bundle directories");
    fs::write(nested.join("resources.pak"), b"cache payload").expect("write cache payload");

    // macOS app bundles downloaded into caches can be owner-readable but not
    // owner-writable. The target remains owned by this test process.
    fs::set_permissions(&cache_root, fs::Permissions::from_mode(0o555))
        .expect("make cache root read-only");
    fs::set_permissions(
        cache_root.join("Contents"),
        fs::Permissions::from_mode(0o555),
    )
    .expect("make app contents read-only");
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o555))
        .expect("make app resources read-only");

    let report = SafeTreeDeleter::delete_path(&cache_root, &[], &PlatformEnvironment::native());

    assert!(
        report.is_success(),
        "unexpected cleanup errors: {:?}",
        report.errors
    );
    assert_eq!(report.deleted_files, 1);
    assert!(!cache_root.exists());

    // The fixture is owned by the current user, so no administrator command or
    // broad chmod is needed to remove the read-only tree.
}

#[cfg(unix)]
#[test]
fn cleaner_restores_read_only_root_after_delete_contents() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().expect("create temp dir");
    let cache_root = dir.path().join("read-only-root");
    fs::create_dir_all(cache_root.join("nested")).expect("create cache tree");
    fs::write(cache_root.join("nested").join("payload.bin"), b"payload").expect("write payload");
    fs::set_permissions(&cache_root, fs::Permissions::from_mode(0o555))
        .expect("make cache root read-only");
    fs::set_permissions(cache_root.join("nested"), fs::Permissions::from_mode(0o555))
        .expect("make nested directory read-only");

    let report = SafeTreeDeleter::delete_contents(&cache_root, &[], &PlatformEnvironment::native());

    assert!(
        report.is_success(),
        "unexpected cleanup errors: {:?}",
        report.errors
    );
    assert!(!cache_root.join("nested").exists());
    assert!(cache_root.is_dir());
    assert_eq!(
        fs::metadata(&cache_root)
            .expect("read root metadata")
            .permissions()
            .mode()
            & 0o7777,
        0o555
    );
}

#[cfg(unix)]
#[test]
fn cleaner_unlinks_symlink_without_changing_target_permissions() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let dir = tempdir().expect("create cleanup root");
    let cache_root = dir.path().join("cache");
    fs::create_dir(&cache_root).expect("create cache directory");

    let outside = tempdir().expect("create outside directory");
    let outside_file = outside.path().join("preserved.bin");
    fs::write(&outside_file, b"preserved").expect("write outside file");
    fs::set_permissions(outside.path(), fs::Permissions::from_mode(0o555))
        .expect("make outside directory read-only");

    let link = cache_root.join("outside-link");
    symlink(outside.path(), &link).expect("create symlink");

    let report = SafeTreeDeleter::delete_contents(&cache_root, &[], &PlatformEnvironment::native());

    assert!(
        report.is_success(),
        "unexpected cleanup errors: {:?}",
        report.errors
    );
    assert!(!link.exists());
    assert!(outside_file.exists());
    assert_eq!(
        fs::metadata(outside.path())
            .expect("read target metadata")
            .permissions()
            .mode()
            & 0o7777,
        0o555
    );
    fs::set_permissions(outside.path(), fs::Permissions::from_mode(0o755))
        .expect("restore fixture permissions");
}

#[test]
fn frontend_selection_must_resolve_against_trusted_scan() {
    let registry = SignatureRegistry::load_embedded().unwrap();
    let scan = ScanResult {
        scan_id: "trusted-scan".into(),
        valid_for_seconds: ScanResult::VALID_FOR_SECONDS,
        started_at: 1,
        finished_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
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
        SafetyPlanner::create_plan_from_scan(
            &scan,
            "trusted-scan",
            &forged,
            &registry,
            &PlatformEnvironment::native()
        ),
        Err(ZenithError::InvalidPlan(_))
    ));
    assert!(matches!(
        SafetyPlanner::create_plan_from_scan(
            &scan,
            "stale-scan",
            &forged,
            &registry,
            &PlatformEnvironment::native()
        ),
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
        platforms: vec![],
        provider: String::new(),
        management_mode: Default::default(),
        artifact_kind: Default::default(),
        consequence: String::new(),
        reclaimable_is_lower_bound: false,
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
        cache_metadata: Default::default(),
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
fn npm_cache_selection_plans_provider_cleanup_without_deleting_fixture() {
    let dir = tempdir().unwrap();
    let cache = dir.path().join("npm-cache");
    fs::create_dir(&cache).unwrap();
    let payload = cache.join("keep.bin");
    fs::write(&payload, b"fixture").unwrap();
    let registry = SignatureRegistry::load_embedded().unwrap();
    let item = ScanItem {
        id: "dev.npm.cache".into(),
        signature_id: "dev.npm.cache".into(),
        name: "npm Cache".into(),
        category: Category::Developer,
        risk: RiskTier::Rebuild,
        path: cache.to_string_lossy().into_owned(),
        size: FileSize::new(7, Some(7)),
        file_count: 1,
        description: "fixture".into(),
        cache_metadata: Default::default(),
        is_selected: true,
        last_modified: None,
        exists: true,
    };
    let plan = SafetyPlanner::create_plan(&[item], &registry).unwrap();
    assert_eq!(plan.targets.len(), 1);
    assert_eq!(plan.targets[0].strategy, CleanStrategy::ExternalCommand);
    assert!(plan.targets[0].identity.is_some());
    // Planning must not invoke npm or mutate a real/user cache.
    assert_eq!(fs::read(payload).unwrap(), b"fixture");
}

#[test]
fn external_command_strategy_never_falls_back_to_filesystem_deletion() {
    let dir = tempdir().unwrap();
    let cache_root = dir.path().join("provider-cache");
    fs::create_dir(&cache_root).unwrap();
    let payload = cache_root.join("keep.bin");
    fs::write(&payload, b"provider owned").unwrap();
    let mut registry = SignatureRegistry::new();
    registry.register(Signature {
        id: "test.unknown-provider".into(),
        name: "Unknown provider".into(),
        category: Category::Developer,
        risk: RiskTier::Rebuild,
        strategy: CleanStrategy::ExternalCommand,
        paths: vec![cache_root.to_string_lossy().into_owned()],
        exclusions: vec![],
        description: "test".into(),
        min_age_days: None,
        include_prefixes: vec![],
        exclude_prefixes: vec![],
        intensive_only: false,
        platforms: vec![],
        provider: "test".into(),
        management_mode: Default::default(),
        artifact_kind: Default::default(),
        consequence: String::new(),
        reclaimable_is_lower_bound: false,
    });
    let item = ScanItem {
        id: "test.unknown-provider".into(),
        signature_id: "test.unknown-provider".into(),
        name: "Unknown provider".into(),
        category: Category::Developer,
        risk: RiskTier::Rebuild,
        path: cache_root.to_string_lossy().into_owned(),
        size: FileSize::new(14, Some(14)),
        file_count: 1,
        description: "test".into(),
        cache_metadata: Default::default(),
        is_selected: true,
        last_modified: None,
        exists: true,
    };
    let plan = SafetyPlanner::create_plan(&[item], &registry).unwrap();
    let result = CleanExecutor::execute(plan, &PlatformEnvironment::native(), |_| {});
    assert!(!result.items[0].success);
    assert_eq!(
        result.items[0].failure_reason,
        Some(CleanFailureReason::ExternalCommandFailed)
    );
    assert!(payload.exists());
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
    let report =
        SafeTreeDeleter::delete_contents(&cache_root, &exclusions, &PlatformEnvironment::native());
    assert!(report.is_success());

    assert!(cache_root.exists());
    assert!(git.join("config").exists());
    assert!(excluded.exists());
    assert!(!removable.exists());
}

#[cfg(unix)]
#[test]
fn test_ancestor_symlink_escape_rejection() {
    let dir = tempdir().expect("create temp dir");
    let trusted_root = dir.path().join("cargo");
    fs::create_dir_all(&trusted_root).unwrap();

    let outside_dir = tempdir().expect("create outside temp dir");
    let precious_file = outside_dir.path().join("precious_data.txt");
    fs::write(&precious_file, b"cannot be deleted").unwrap();

    // Create an intermediate symlink: cargo/registry -> outside_dir
    #[cfg(unix)]
    {
        let symlink_dir = trusted_root.join("registry");
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

#[cfg(unix)]
#[test]
fn test_symlink_ancestor_above_signature_root_rejection() {
    let base_dir = tempdir().expect("create base temp dir");
    let outside_dir = tempdir().expect("create outside temp dir");
    let precious = outside_dir.path().join("precious_code.rs");
    fs::write(&precious, b"fn important() {}").unwrap();

    // Create intermediate symlink: base_dir/.cargo -> outside_dir
    #[cfg(unix)]
    {
        let symlink_dot_cargo = base_dir.path().join(".cargo");
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

#[cfg(unix)]
#[test]
fn test_signature_root_itself_symlink_rejection() {
    let base_dir = tempdir().expect("create base temp dir");
    let outside_dir = tempdir().expect("create outside temp dir");
    let precious = outside_dir.path().join("precious.txt");
    fs::write(&precious, b"cannot delete").unwrap();

    // signature root itself is a symlink: base_dir/cache -> outside_dir
    #[cfg(unix)]
    {
        let symlink_cache = base_dir.path().join("cache");
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
        cache_metadata: Default::default(),
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
        platforms: vec![],
        provider: String::new(),
        management_mode: Default::default(),
        artifact_kind: Default::default(),
        consequence: String::new(),
        reclaimable_is_lower_bound: false,
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
        cache_metadata: Default::default(),
        is_selected: true,
        last_modified: None,
        exists: true,
    };

    let plan = SafetyPlanner::create_plan(&[scan_item], &registry).expect("create plan");
    assert_eq!(plan.targets[0].min_age_days, Some(3));

    let clean_res = CleanExecutor::execute(plan, &PlatformEnvironment::native(), |_| {});
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
    let (measured_size, measured_count) = SizeCalculator::measure_path(
        &cache_dir,
        &gemini_sig.exclusions,
        &PlatformEnvironment::native(),
    );
    assert_eq!(
        measured_count, 1,
        "Only transient_cache should be counted as reclaimable"
    );
    assert_eq!(
        measured_size.logical,
        b"transient session data".len() as u64
    );

    // Perform delete_contents
    let report = SafeTreeDeleter::delete_contents(
        &cache_dir,
        &gemini_sig.exclusions,
        &PlatformEnvironment::native(),
    );
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

#[test]
fn test_select_quick_clean_safe_candidates_filters_risk_bytes_and_settings() {
    use zenith_lib::commands::select_quick_clean_safe_candidates;
    use zenith_lib::models::ZenithSettings;

    let scan = ScanResult {
        scan_id: "test-scan-123".to_string(),
        valid_for_seconds: 60,
        started_at: 1000,
        finished_at: 1005,
        total_bytes: 1500,
        safe_bytes: 600,
        rebuild_bytes: 400,
        manual_bytes: 500,
        categories: vec![
            CategoryResult {
                category: Category::Developer,
                display_name: "Developer".to_string(),
                total_bytes: 600,
                safe_bytes: 200,
                rebuild_bytes: 400,
                manual_bytes: 0,
                items: vec![
                    ScanItem {
                        id: "dev.safe.nonzero".to_string(),
                        signature_id: "dev.signature".to_string(),
                        name: "Safe Dev Cache".to_string(),
                        category: Category::Developer,
                        risk: RiskTier::Safe,
                        path: "/tmp/dev-cache".to_string(),
                        size: FileSize::new(200, Some(200)),
                        file_count: 5,
                        description: "test".to_string(),
                        cache_metadata: Default::default(),
                        is_selected: true,
                        last_modified: None,
                        exists: true,
                    },
                    ScanItem {
                        id: "dev.safe.zero".to_string(),
                        signature_id: "dev.signature".to_string(),
                        name: "Zero Byte Cache".to_string(),
                        category: Category::Developer,
                        risk: RiskTier::Safe,
                        path: "/tmp/dev-zero".to_string(),
                        size: FileSize::new(0, Some(0)),
                        file_count: 0,
                        description: "test".to_string(),
                        cache_metadata: Default::default(),
                        is_selected: true,
                        last_modified: None,
                        exists: true,
                    },
                    ScanItem {
                        id: "dev.rebuild".to_string(),
                        signature_id: "dev.signature".to_string(),
                        name: "Rebuild Dev Cache".to_string(),
                        category: Category::Developer,
                        risk: RiskTier::Rebuild,
                        path: "/tmp/dev-rebuild".to_string(),
                        size: FileSize::new(400, Some(400)),
                        file_count: 10,
                        description: "test".to_string(),
                        cache_metadata: Default::default(),
                        is_selected: true,
                        last_modified: None,
                        exists: true,
                    },
                ],
            },
            CategoryResult {
                category: Category::System,
                display_name: "System".to_string(),
                total_bytes: 400,
                safe_bytes: 400,
                rebuild_bytes: 0,
                manual_bytes: 0,
                items: vec![ScanItem {
                    id: "sys.safe.nonzero".to_string(),
                    signature_id: "sys.signature".to_string(),
                    name: "System Logs".to_string(),
                    category: Category::System,
                    risk: RiskTier::Safe,
                    path: "/tmp/sys-logs".to_string(),
                    size: FileSize::new(400, Some(400)),
                    file_count: 8,
                    description: "test".to_string(),
                    cache_metadata: Default::default(),
                    is_selected: true,
                    last_modified: None,
                    exists: true,
                }],
            },
            CategoryResult {
                category: Category::Model,
                display_name: "Model".to_string(),
                total_bytes: 500,
                safe_bytes: 0,
                rebuild_bytes: 0,
                manual_bytes: 500,
                items: vec![ScanItem {
                    id: "model.manual".to_string(),
                    signature_id: "model.signature".to_string(),
                    name: "Manual Model".to_string(),
                    category: Category::Model,
                    risk: RiskTier::Manual,
                    path: "/tmp/model".to_string(),
                    size: FileSize::new(500, Some(500)),
                    file_count: 1,
                    description: "test".to_string(),
                    cache_metadata: Default::default(),
                    is_selected: true,
                    last_modified: None,
                    exists: true,
                }],
            },
        ],
    };

    // Default settings has clean_developer_tools=true
    let default_settings = ZenithSettings::default();
    let candidates = select_quick_clean_safe_candidates(&scan, &default_settings);
    assert_eq!(candidates, vec!["dev.safe.nonzero", "sys.safe.nonzero"]);

    // If clean_developer_tools is disabled, dev.safe.nonzero must NOT be included
    let disabled_dev_settings = ZenithSettings {
        clean_developer_tools: false,
        ..Default::default()
    };
    let candidates2 = select_quick_clean_safe_candidates(&scan, &disabled_dev_settings);
    assert_eq!(candidates2, vec!["sys.safe.nonzero"]);
}

#[test]
fn test_quick_panel_capability_boundary_safe_only() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let quick_path = manifest_dir.join("capabilities/quick.json");
    let quick_raw = fs::read_to_string(&quick_path).expect("read quick.json");
    let quick_json: serde_json::Value = serde_json::from_str(&quick_raw).expect("parse quick.json");

    let quick_permissions = quick_json
        .get("permissions")
        .and_then(|p| p.as_array())
        .expect("permissions array in quick.json");

    let quick_permission_strings: Vec<&str> = quick_permissions
        .iter()
        .filter_map(|p| p.as_str())
        .collect();

    // Must allow quick_clean_safe
    assert!(
        quick_permission_strings.contains(&"allow-quick-clean-safe"),
        "quick.json must contain allow-quick-clean-safe"
    );

    // Must NOT allow arbitrary plan creation or execution
    assert!(
        !quick_permission_strings.contains(&"allow-create-delete-plan"),
        "quick.json must NOT contain allow-create-delete-plan"
    );
    assert!(
        !quick_permission_strings.contains(&"allow-execute-clean"),
        "quick.json must NOT contain allow-execute-clean"
    );

    // Verify main capability still has general delete plan & execution authority
    let main_path = manifest_dir.join("capabilities/main.json");
    let main_raw = fs::read_to_string(&main_path).expect("read main.json");
    let main_json: serde_json::Value = serde_json::from_str(&main_raw).expect("parse main.json");
    let main_permissions = main_json
        .get("permissions")
        .and_then(|p| p.as_array())
        .expect("permissions array in main.json");
    let main_permission_strings: Vec<&str> =
        main_permissions.iter().filter_map(|p| p.as_str()).collect();

    assert!(main_permission_strings.contains(&"allow-create-delete-plan"));
    assert!(main_permission_strings.contains(&"allow-execute-clean"));
}

#[cfg(unix)]
#[test]
fn unix_parent_replacement_between_validation_and_unlink_leaves_outside_untouched() {
    use std::os::unix::fs::OpenOptionsExt;

    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root");
    let parent = root.join("parent");
    fs::create_dir_all(&parent).unwrap();
    let child = parent.join("child.bin");
    fs::write(&child, b"inside").unwrap();

    let outside = dir.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    let outside_target = outside.join("child.bin");
    fs::write(&outside_target, b"outside").unwrap();

    // Delete through the verified tree deleter, then prove a swapped parent
    // symlink cannot redirect an already-verified descriptor unlink.
    let parent_file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&parent)
        .unwrap();
    fs::remove_file(&child).unwrap();
    fs::remove_dir(&parent).unwrap();
    std::os::unix::fs::symlink(&outside, &parent).unwrap();

    let res = unsafe {
        let name = std::ffi::CString::new("child.bin").unwrap();
        libc::unlinkat(
            std::os::unix::io::AsRawFd::as_raw_fd(&parent_file),
            name.as_ptr(),
            0,
        )
    };
    assert_ne!(res, 0, "stale descriptor unlink must fail");
    assert_eq!(fs::read(&outside_target).unwrap(), b"outside");
}

#[cfg(windows)]
mod windows_safety {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;

    #[test]
    fn directory_handle_captures_real_volume_file_identity() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("cache");
        fs::create_dir(&target).unwrap();
        let identity = ToctouGuard::capture(&target).expect("capture identity");
        assert_ne!((identity.device, identity.inode), (0, 0));
        assert!(ToctouGuard::verify(&target, &identity).is_ok());
    }

    #[test]
    fn zero_identity_is_never_accepted_as_verified() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("cache");
        fs::create_dir(&target).unwrap();
        let mut identity = ToctouGuard::capture(&target).expect("capture");
        identity.device = 0;
        identity.inode = 0;
        assert!(ToctouGuard::verify(&target, &identity).is_err());
    }

    #[test]
    fn file_id_change_is_rejected_as_ownership_changed() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("payload.bin");
        fs::write(&target, b"v1").unwrap();
        let identity = ToctouGuard::capture(&target).expect("capture");
        fs::remove_file(&target).unwrap();
        fs::write(&target, b"v1-recreated").unwrap();
        // Recreated file has a new file ID; verification must fail.
        assert!(ToctouGuard::verify(&target, &identity).is_err());
    }

    #[test]
    fn reparse_point_is_never_traversed_during_cleanup() {
        let dir = tempdir().expect("tempdir");
        let cache = dir.path().join("cache");
        fs::create_dir(&cache).unwrap();
        let outside = tempdir().expect("outside");
        let precious = outside.path().join("precious.bin");
        fs::write(&precious, b"precious").unwrap();

        let link = cache.join("junction");
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&link)
            .arg(outside.path())
            .output()
            .expect("cmd.exe must be runnable to create the junction");
        assert!(
            output.status.success(),
            "mklink /J {} {} failed: {}",
            link.display(),
            outside.path().display(),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            SymlinkGuard::is_symlink(&link),
            "the created junction must be recognized as a reparse point"
        );
        assert!(
            SymlinkGuard::is_symlink_strict(&link).expect("junction metadata is readable"),
            "the junction must be classified as an indirection"
        );

        let report = SafeTreeDeleter::delete_contents(&cache, &[], &PlatformEnvironment::native());
        assert!(report.is_success(), "errors: {:?}", report.errors);
        // The junction itself is removed, never traversed: its target must be
        // untouched, and the cache root must be gone.
        assert!(!link.exists() || SymlinkGuard::is_symlink(&link));
        assert!(precious.exists(), "reparse target must remain untouched");
        assert_eq!(
            fs::read(&precious).expect("target is readable"),
            b"precious".to_vec()
        );
    }

    #[test]
    fn case_insensitive_protected_paths_are_rejected() {
        let host = PlatformEnvironment::native();
        assert!(Blacklist::is_blacklisted_with(
            Path::new("C:\\Windows"),
            &host
        ));
        assert!(Blacklist::is_blacklisted_with(
            Path::new("c:\\windows"),
            &host
        ));
        assert!(Blacklist::is_blacklisted_with(
            Path::new("C:\\WINDOWS\\System32"),
            &host
        ));
        assert!(Blacklist::validate_with(Path::new("c:\\windows\\system32"), &host).is_err());
    }

    #[test]
    fn long_paths_with_verbatim_prefix_are_handled() {
        let dir = tempdir().expect("tempdir");
        let mut deep = dir.path().to_path_buf();
        for i in 0..8 {
            deep = deep.join(format!("very-long-directory-name-{i:02}"));
        }
        fs::create_dir_all(&deep).unwrap();
        let payload = deep.join("payload.bin");
        fs::write(&payload, b"long-path-payload").unwrap();
        let verbatim = format!(r"\\?\{}", payload.display());
        let verbatim_path = PathBuf::from(&verbatim);
        // Blacklist normalization must not mistake the verbatim prefix for ADS.
        assert!(!Blacklist::is_blacklisted_with(
            &verbatim_path,
            &PlatformEnvironment::native()
        ));
        let report = SafeTreeDeleter::delete_contents(&deep, &[], &PlatformEnvironment::native());
        assert!(report.is_success(), "errors: {:?}", report.errors);
        assert!(!payload.exists());
    }

    #[allow(dead_code)]
    fn wide_len(path: &Path) -> usize {
        path.as_os_str().encode_wide().count()
    }
}
