use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use tempfile::tempdir;
use zenith_lib::cleaner::CleanExecutor;
use zenith_lib::models::{
    derive_cleanup_disposition, CacheManagementMode, CacheMetadata, CacheSizeSemantics, Category,
    CategoryResult, CleanFailureReason, CleanStrategy, CleanupEligibility, CleanupMode,
    CleanupOwnership, CleanupUnit, CleanupUnitKind, DeletePlan, DeleteTarget, DispositionFacts,
    EligibilityGate, EntryKind, FileSize, ObservationQuality, RiskTier, RunningProcessPolicy,
    ScanItem, ScanResult, Signature, ZenithError,
};
use zenith_lib::safety::blacklist::{classify_windows, BlacklistEnvironment, BlacklistVerdict};
use zenith_lib::safety::{
    Blacklist, RevalidationOutcome, SafeTreeDeleter, SafetyPlanner, SafetyValidator, SymlinkGuard,
    ToctouGuard, ValidatedTarget,
};
use zenith_lib::scanner::SizeCalculator;
use zenith_lib::signatures::SignatureRegistry;
use zenith_platform::path_algebra::PathFlavor;
use zenith_platform::paths::SimulatedPaths;
use zenith_platform::{KnownFolder, NativePlatformPaths, PlatformEnvironment};

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

fn validated_filesystem_target(
    path: &Path,
    strategy: CleanStrategy,
    exclusions: &[String],
    environment: &PlatformEnvironment,
) -> ValidatedTarget {
    let target = DeleteTarget {
        item_id: "test-target".into(),
        signature_id: "test.signature".into(),
        name: "Test target".into(),
        path: path.to_path_buf(),
        strategy,
        expected_bytes: 0,
        risk: RiskTier::Safe,
        identity: ToctouGuard::capture(path),
        exclusions: exclusions.to_vec(),
        min_age_days: None,
        unit: CleanupUnit::fixed_path(path.to_string_lossy().to_string()),
        target_kind: EntryKind::Directory,
        owner: CleanupOwnership::unknown(),
        process_guard: RunningProcessPolicy::none(),
    };

    match SafetyValidator::revalidate(&target, environment) {
        RevalidationOutcome::Validated(validated) => validated,
        RevalidationOutcome::Skipped(result) | RevalidationOutcome::Failed(result) => {
            panic!("test target did not pass the production validator: {result:?}")
        }
    }
}

#[test]
fn present_filesystem_target_without_identity_fails_closed() {
    let fixture = tempdir().expect("create fixture");
    let target_path = fixture.path().join("cache");
    fs::create_dir(&target_path).expect("create target");
    let target = DeleteTarget {
        item_id: "missing-identity".into(),
        signature_id: "test.signature".into(),
        name: "Missing identity".into(),
        path: target_path.clone(),
        strategy: CleanStrategy::DeleteContents,
        expected_bytes: 0,
        risk: RiskTier::Safe,
        identity: None,
        exclusions: vec![],
        min_age_days: None,
        unit: CleanupUnit::fixed_path(target_path.to_string_lossy().to_string()),
        target_kind: EntryKind::Directory,
        owner: CleanupOwnership::unknown(),
        process_guard: RunningProcessPolicy::none(),
    };

    match SafetyValidator::revalidate(&target, &PlatformEnvironment::native()) {
        RevalidationOutcome::Failed(result) => {
            assert_eq!(
                result.failure_reason,
                Some(CleanFailureReason::ChangedSinceScan)
            );
            assert!(result
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("identity is missing")));
        }
        other => panic!("missing filesystem identity must fail closed, got {other:?}"),
    }
    assert!(target_path.exists());
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
    // The spelling is POSIX, so the machine this assertion is about is a POSIX
    // one: the verdict must not depend on the host that runs the test.
    let posix = PlatformEnvironment::simulated(PathFlavor::Posix).with_home("/Users/zenith-tester");
    assert!(
        Blacklist::validate_with(Path::new("/Users/../System"), &posix).is_err(),
        "a POSIX traversal into /System must be rejected"
    );

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
        let measurement =
            SizeCalculator::measure_path_full(dir.path(), &[], &PlatformEnvironment::native());
        assert_eq!(measurement.file_count, 1);
        assert!(measurement.size.logical > 0);
        assert!(
            measurement.complete,
            "a symlink is measured as a link, not as a boundary"
        );
    }
}

#[test]
fn test_safety_planner_rejects_unknown_signatures() {
    let registry = SignatureRegistry::load_embedded().expect("load embedded signatures");

    let fake_item = ScanItem::mock(
        "unknown.signature.123",
        "unknown.signature.123",
        "Fake Cache",
        Category::Ai,
        RiskTier::Safe,
        "/tmp/fake-cache",
        FileSize::new(1024, Some(1024)),
        1,
    );

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

    let mut forged_item = ScanItem::mock(
        "system.developer_temp.0.codex-forged",
        "system.developer_temp",
        "Forged temp item",
        Category::System,
        RiskTier::Safe,
        forged_path.to_string_lossy().into_owned(),
        FileSize::new(1024, Some(1024)),
        1,
    );
    // The unit is spelled the way the signature declares it, so the refusal
    // comes from the scope check rather than from a malformed item.
    forged_item.unit = CleanupUnit::child_namespace(
        dir.path().to_string_lossy().into_owned(),
        forged_path.to_string_lossy().into_owned(),
    );

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
        discovery: Default::default(),
        unit: None,
        owner: String::new(),
        priority: 0,
        fail_if_running: Vec::new(),
        provider: String::new(),
        management_mode: Default::default(),
        artifact_kind: Default::default(),
        consequence: String::new(),
        reclaimable_is_lower_bound: false,
    });

    let scan_item = ScanItem::mock(
        "test.delete-contents",
        "test.delete-contents",
        "Cargo Registry Cache",
        Category::Developer,
        RiskTier::Safe,
        cache_root.to_string_lossy().to_string(),
        FileSize::new(2048, Some(2048)),
        2,
    );

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

    let environment = PlatformEnvironment::native();
    let validated = validated_filesystem_target(
        &cache_root,
        CleanStrategy::DeleteDirectory,
        &[],
        &environment,
    );
    let report = SafeTreeDeleter::delete_path_validated(&validated, &environment);

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

    let environment = PlatformEnvironment::native();
    let validated = validated_filesystem_target(
        &cache_root,
        CleanStrategy::DeleteContents,
        &[],
        &environment,
    );
    let report = SafeTreeDeleter::delete_contents_validated(&validated, &environment);

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

    let environment = PlatformEnvironment::native();
    let validated = validated_filesystem_target(
        &cache_root,
        CleanStrategy::DeleteContents,
        &[],
        &environment,
    );
    let report = SafeTreeDeleter::delete_contents_validated(&validated, &environment);

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
            cleanable_bytes: 0,
            safe_bytes: 0,
            rebuild_bytes: 0,
            manual_bytes: 0,
            quality: ObservationQuality::Fresh,
            skipped_entry_count: 0,
            incomplete_item_count: 0,
            eligibility: Default::default(),
            suppressed_duplicate_count: 0,
            suppressed_duplicate_bytes: 0,
        }],
        total_bytes: 0,
        cleanable_bytes: 0,
        safe_bytes: 0,
        rebuild_bytes: 0,
        manual_bytes: 0,
        quality: ObservationQuality::Fresh,
        incomplete_reasons: vec![],
        skipped_entry_count: 0,
        incomplete_item_count: 0,
        eligibility: Default::default(),
        suppressed_duplicate_count: 0,
        suppressed_duplicate_bytes: 0,
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
        discovery: Default::default(),
        unit: None,
        owner: String::new(),
        priority: 0,
        fail_if_running: Vec::new(),
        provider: String::new(),
        management_mode: Default::default(),
        artifact_kind: Default::default(),
        consequence: String::new(),
        reclaimable_is_lower_bound: false,
    });
    let mut item = ScanItem::mock(
        "test.manual-model",
        "test.manual-model",
        "Manual model",
        Category::Model,
        RiskTier::Manual,
        model_root.to_string_lossy().into_owned(),
        FileSize::new(1, Some(1)),
        1,
    );
    item.is_selected = true;

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
    let mut item = ScanItem::mock(
        "dev.npm.cache",
        "dev.npm.cache",
        "npm Cache",
        Category::Developer,
        RiskTier::Rebuild,
        cache.to_string_lossy().into_owned(),
        FileSize::new(7, Some(7)),
        1,
    );
    // The provider owns the cache: the location is a staleness assertion, not a
    // path generic cleanup may delete.
    item.unit = CleanupUnit::new(
        zenith_lib::models::CleanupUnitKind::ProviderAction,
        cache.to_string_lossy().into_owned(),
        cache.to_string_lossy().into_owned(),
    );
    item.ownership = registry
        .get("dev.npm.cache")
        .expect("the npm entry is in the catalog")
        .ownership();
    item.is_selected = true;
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
        discovery: Default::default(),
        unit: None,
        owner: String::new(),
        priority: 0,
        fail_if_running: Vec::new(),
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
    let mut item = ScanItem::mock(
        "test.unknown-provider",
        "test.unknown-provider",
        "Unknown provider",
        Category::Developer,
        RiskTier::Rebuild,
        cache_root.to_string_lossy().into_owned(),
        FileSize::new(14, Some(14)),
        1,
    );
    item.unit = CleanupUnit::new(
        zenith_lib::models::CleanupUnitKind::ProviderAction,
        cache_root.to_string_lossy().into_owned(),
        cache_root.to_string_lossy().into_owned(),
    );
    item.ownership = registry
        .get("test.unknown-provider")
        .expect("the fixture signature is registered")
        .ownership();
    item.is_selected = true;
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
fn recursive_delete_refuses_a_unit_with_nested_protected_state() {
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
    let environment = PlatformEnvironment::native();
    let target = DeleteTarget {
        item_id: "protected-cache".into(),
        signature_id: "test.protected-cache".into(),
        name: "Protected cache".into(),
        path: cache_root.clone(),
        strategy: CleanStrategy::DeleteContents,
        expected_bytes: 5,
        risk: RiskTier::Safe,
        identity: ToctouGuard::capture(&cache_root),
        exclusions,
        min_age_days: None,
        unit: CleanupUnit::fixed_path(cache_root.to_string_lossy().into_owned()),
        target_kind: EntryKind::Directory,
        owner: CleanupOwnership::unknown(),
        process_guard: RunningProcessPolicy::none(),
    };
    match SafetyValidator::revalidate(&target, &environment) {
        RevalidationOutcome::Skipped(result) => {
            assert_eq!(
                result.failure_reason,
                Some(CleanFailureReason::StructuredStore)
            );
        }
        other => panic!("nested protected state must skip the whole unit: {other:?}"),
    }

    assert!(cache_root.exists());
    assert!(git.join("config").exists());
    assert!(excluded.exists());
    assert!(
        removable.exists(),
        "no sibling may be removed before preflight ends"
    );
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
        let validation_res = SymlinkGuard::validate_no_symlink_ancestors(
            &target_path,
            &trusted_root,
            &PlatformEnvironment::native(),
        );
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
        let validation_res = SymlinkGuard::validate_components_between(
            &signature_target,
            base_dir.path(),
            &PlatformEnvironment::native(),
        );
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

        let validation_res = SymlinkGuard::validate_components_between(
            &symlink_cache,
            base_dir.path(),
            &PlatformEnvironment::native(),
        );
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
    let docker_item = ScanItem::mock(
        "container.docker.builder",
        "container.docker.builder",
        "Docker Build Cache",
        Category::Container,
        RiskTier::Safe,
        "docker://buildkit/cache",
        FileSize::new(1024 * 1024, Some(1024 * 1024)),
        1,
    );

    let mut docker_item = docker_item;
    docker_item.unit = CleanupUnit::new(
        zenith_lib::models::CleanupUnitKind::ContainerResource,
        "docker://buildkit/cache".to_string(),
        "docker://buildkit/cache".to_string(),
    );

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
        discovery: Default::default(),
        unit: None,
        owner: String::new(),
        priority: 0,
        fail_if_running: Vec::new(),
        provider: String::new(),
        management_mode: Default::default(),
        artifact_kind: Default::default(),
        consequence: String::new(),
        reclaimable_is_lower_bound: false,
    });

    let mut scan_item = ScanItem::mock(
        "test.stale_temp.0.active_tool_cache",
        "test.stale_temp",
        "active_tool_cache",
        Category::Developer,
        RiskTier::Safe,
        temp_child.to_string_lossy().into_owned(),
        FileSize::new(1024, Some(1024)),
        1,
    );
    scan_item.unit = CleanupUnit::child_namespace(
        dir.path().to_string_lossy().into_owned(),
        temp_child.to_string_lossy().into_owned(),
    );

    let plan = SafetyPlanner::create_plan(&[scan_item], &registry).expect("create plan");
    assert_eq!(plan.targets[0].min_age_days, Some(3));

    let clean_res = CleanExecutor::execute(plan, &PlatformEnvironment::native(), |_| {});
    assert_eq!(clean_res.items.len(), 1);
    assert_eq!(
        clean_res.items[0].status,
        zenith_lib::models::CleanStatus::Skipped
    );
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
    let measured = SizeCalculator::measure_path_full(
        &cache_dir,
        &gemini_sig.exclusions,
        &PlatformEnvironment::native(),
    );
    assert_eq!(
        measured.file_count, 1,
        "Only transient_cache should be counted as reclaimable"
    );
    assert_eq!(
        measured.size.logical,
        b"transient session data".len() as u64
    );

    // Perform delete_contents
    let environment = PlatformEnvironment::native();
    let validated = validated_filesystem_target(
        &cache_dir,
        CleanStrategy::DeleteContents,
        &gemini_sig.exclusions,
        &environment,
    );
    let report = SafeTreeDeleter::delete_contents_validated(&validated, &environment);
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

    let mut scan = ScanResult {
        scan_id: "test-scan-123".to_string(),
        valid_for_seconds: 60,
        started_at: 1000,
        finished_at: 1005,
        total_bytes: 1500,
        cleanable_bytes: 1000,
        safe_bytes: 600,
        rebuild_bytes: 400,
        manual_bytes: 500,
        categories: vec![
            CategoryResult {
                category: Category::Developer,
                display_name: "Developer".to_string(),
                total_bytes: 600,
                cleanable_bytes: 600,
                safe_bytes: 200,
                rebuild_bytes: 400,
                manual_bytes: 0,
                quality: ObservationQuality::Fresh,
                items: vec![
                    ScanItem::mock(
                        "dev.safe.nonzero",
                        "dev.signature",
                        "Safe Dev Cache",
                        Category::Developer,
                        RiskTier::Safe,
                        "/tmp/dev-cache",
                        FileSize::new(200, Some(200)),
                        5,
                    ),
                    ScanItem::mock(
                        "dev.safe.zero",
                        "dev.signature",
                        "Zero Byte Cache",
                        Category::Developer,
                        RiskTier::Safe,
                        "/tmp/dev-zero",
                        FileSize::new(0, Some(0)),
                        0,
                    ),
                    ScanItem::mock(
                        "dev.rebuild",
                        "dev.signature",
                        "Rebuild Dev Cache",
                        Category::Developer,
                        RiskTier::Rebuild,
                        "/tmp/dev-rebuild",
                        FileSize::new(400, Some(400)),
                        10,
                    ),
                ],
                incomplete_item_count: 0,
                eligibility: Default::default(),
                suppressed_duplicate_count: 0,
                suppressed_duplicate_bytes: 0,
                skipped_entry_count: 0,
            },
            CategoryResult {
                category: Category::System,
                display_name: "System".to_string(),
                total_bytes: 400,
                cleanable_bytes: 400,
                safe_bytes: 400,
                rebuild_bytes: 0,
                manual_bytes: 0,
                quality: ObservationQuality::Fresh,
                items: vec![ScanItem::mock(
                    "sys.safe.nonzero",
                    "sys.signature",
                    "System Logs",
                    Category::System,
                    RiskTier::Safe,
                    "/tmp/sys-logs",
                    FileSize::new(400, Some(400)),
                    8,
                )],
                incomplete_item_count: 0,
                eligibility: Default::default(),
                suppressed_duplicate_count: 0,
                suppressed_duplicate_bytes: 0,
                skipped_entry_count: 0,
            },
            CategoryResult {
                category: Category::Model,
                display_name: "Model".to_string(),
                total_bytes: 500,
                cleanable_bytes: 0,
                safe_bytes: 0,
                rebuild_bytes: 0,
                manual_bytes: 500,
                quality: ObservationQuality::Fresh,
                items: vec![ScanItem::mock(
                    "model.manual",
                    "model.signature",
                    "Manual Model",
                    Category::Model,
                    RiskTier::Manual,
                    "/tmp/model",
                    FileSize::new(500, Some(500)),
                    1,
                )],
                incomplete_item_count: 0,
                eligibility: Default::default(),
                suppressed_duplicate_count: 0,
                suppressed_duplicate_bytes: 0,
                skipped_entry_count: 0,
            },
        ],
        quality: ObservationQuality::Fresh,
        incomplete_reasons: vec![],
        incomplete_item_count: 0,
        eligibility: Default::default(),
        suppressed_duplicate_count: 0,
        suppressed_duplicate_bytes: 0,
        skipped_entry_count: 0,
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

    // A detached permission flag must not survive a change in the captured
    // facts, even if an internally inconsistent scan reaches Quick Clean.
    let stale = &mut scan.categories[0].items[0];
    stale.quality = ObservationQuality::Unavailable;
    stale.incomplete_reason = Some("Access was revoked".into());
    let candidates3 = select_quick_clean_safe_candidates(&scan, &default_settings);
    assert_eq!(candidates3, vec!["sys.safe.nonzero"]);
}

#[test]
fn test_permission_denied_or_inaccessible_measurement_is_captured_with_reason() {
    let dir = tempdir().expect("tempdir");
    let unreadable = dir.path().join("unreadable_dir");
    fs::create_dir(&unreadable).unwrap();
    fs::write(unreadable.join("payload.bin"), b"unreachable content").unwrap();
    let mut deep = dir.path().join("deep");
    for level in 0..=32 {
        deep = deep.join(format!("level-{level}"));
    }
    fs::create_dir_all(&deep).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
    }

    let measurement =
        SizeCalculator::measure_path_full(dir.path(), &[], &PlatformEnvironment::native());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Restore permissions for clean tempdir teardown
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o755)).unwrap();
    }

    assert!(!measurement.complete);
    let reason = measurement
        .incomplete_reason
        .expect("an incomplete measurement carries a reason");
    assert!(
        reason.contains("Failed to read directory")
            || reason.contains("Permission denied")
            || reason.contains("Directory depth limit"),
        "unexpected reason: {reason}"
    );
}

#[test]
fn test_partial_scan_byte_semantics_and_cleanup_gate() {
    use zenith_lib::commands::select_quick_clean_safe_candidates;
    use zenith_lib::models::ZenithSettings;

    let partial_size = FileSize::new(500, Some(500));
    let partial_metadata = CacheMetadata {
        size_semantics: CacheSizeSemantics::ConservativeLowerBound,
        ..Default::default()
    };
    let partial_reason = Some("Permission denied in subtree".to_string());
    let partial_disposition = derive_cleanup_disposition(DispositionFacts::new(
        RiskTier::Safe,
        ObservationQuality::Partial,
        &partial_metadata,
        &partial_size,
        partial_reason.as_deref(),
    ));
    let partial_item = ScanItem {
        id: "dev.partial.item".to_string(),
        signature_id: "dev.signature".to_string(),
        name: "Partial Dev Cache".to_string(),
        category: Category::Developer,
        risk: RiskTier::Safe,
        path: "/tmp/partial-cache".to_string(),
        size: partial_size,
        file_count: 5,
        description: "Partial cache".to_string(),
        cache_metadata: partial_metadata,
        unit: CleanupUnit::fixed_path("/tmp/partial-cache".to_string()),
        ownership: CleanupOwnership::unknown(),
        age: None,
        stale: None,
        structured_state: None,
        entry_kind: EntryKind::Directory,
        gate: EligibilityGate::Open,
        owner_running: false,
        is_selected: partial_disposition.eligibility == CleanupEligibility::AutoCleanable,
        last_modified: None,
        exists: true,
        quality: ObservationQuality::Partial,
        incomplete_reason: partial_reason,
        skipped_entry_count: 0,
        disposition: partial_disposition,
    };

    let unavailable_size = FileSize::new(0, Some(0));
    let unavailable_metadata = CacheMetadata {
        size_semantics: CacheSizeSemantics::Informational,
        ..Default::default()
    };
    let unavailable_reason = Some("Failed to access directory".to_string());
    let unavailable_disposition = derive_cleanup_disposition(DispositionFacts::new(
        RiskTier::Safe,
        ObservationQuality::Unavailable,
        &unavailable_metadata,
        &unavailable_size,
        unavailable_reason.as_deref(),
    ));
    let unavailable_item = ScanItem {
        id: "sys.unavailable.item".to_string(),
        signature_id: "sys.signature".to_string(),
        name: "Inaccessible System Logs".to_string(),
        category: Category::System,
        risk: RiskTier::Safe,
        path: "/tmp/unavailable-logs".to_string(),
        size: unavailable_size,
        file_count: 0,
        description: "Inaccessible".to_string(),
        cache_metadata: unavailable_metadata,
        unit: CleanupUnit::fixed_path("/tmp/unavailable-logs".to_string()),
        ownership: CleanupOwnership::unknown(),
        age: None,
        stale: None,
        structured_state: None,
        entry_kind: EntryKind::Directory,
        gate: EligibilityGate::Open,
        owner_running: false,
        is_selected: unavailable_disposition.eligibility == CleanupEligibility::AutoCleanable,
        last_modified: None,
        exists: true,
        quality: ObservationQuality::Unavailable,
        incomplete_reason: unavailable_reason,
        skipped_entry_count: 0,
        disposition: unavailable_disposition,
    };

    // 1. Cleanup permission invariants
    assert!(partial_item.allows_cleanup());
    assert!(!unavailable_item.allows_cleanup());

    let registry = SignatureRegistry::load_embedded().unwrap();
    let unavailable_plan_res =
        SafetyPlanner::create_plan(std::slice::from_ref(&unavailable_item), &registry);
    assert!(
        matches!(unavailable_plan_res, Err(ZenithError::InvalidPlan(_))),
        "SafetyPlanner must reject unavailable items"
    );

    // 2. Quick clean exclusion
    let scan = ScanResult {
        scan_id: "partial-scan-test".to_string(),
        valid_for_seconds: 60,
        started_at: 1000,
        finished_at: 1005,
        total_bytes: 500,
        cleanable_bytes: 500,
        safe_bytes: 500,
        rebuild_bytes: 0,
        manual_bytes: 0,
        categories: vec![CategoryResult {
            category: Category::Developer,
            display_name: "Developer".to_string(),
            total_bytes: 500,
            cleanable_bytes: 500,
            safe_bytes: 500,
            rebuild_bytes: 0,
            manual_bytes: 0,
            quality: ObservationQuality::Partial,
            items: vec![partial_item.clone(), unavailable_item.clone()],
            skipped_entry_count: 0,
            incomplete_item_count: 0,
            eligibility: Default::default(),
            suppressed_duplicate_count: 0,
            suppressed_duplicate_bytes: 0,
        }],
        quality: ObservationQuality::Partial,
        incomplete_reasons: vec!["Permission denied in subtree".to_string()],
        skipped_entry_count: 0,
        incomplete_item_count: 0,
        eligibility: Default::default(),
        suppressed_duplicate_count: 0,
        suppressed_duplicate_bytes: 0,
    };

    let settings = ZenithSettings::default();
    let candidates = select_quick_clean_safe_candidates(&scan, &settings);
    assert!(
        candidates.is_empty(),
        "Partial and unavailable items must NEVER be selected for quick clean"
    );

    // 3. Freshness & cleanup validation invariants
    assert!(!scan.is_fresh_at(1010)); // Partial scans never report fresh
    assert!(scan.validate_for_cleanup("partial-scan-test", 1010).is_ok()); // Manual cleanup valid for partial

    let unavailable_scan = ScanResult {
        quality: ObservationQuality::Unavailable,
        ..scan
    };
    assert!(unavailable_scan
        .validate_for_cleanup("partial-scan-test", 1010)
        .is_err()); // Unavailable scan fails closed
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

#[test]
fn test_cleanup_eligibility_matrix_and_byte_semantics() {
    struct TestCase {
        risk: RiskTier,
        quality: ObservationQuality,
        management: CacheManagementMode,
        reason: Option<&'static str>,
        expected_eligibility: CleanupEligibility,
        expected_cleanable: Option<u64>,
    }

    let cases = vec![
        TestCase {
            risk: RiskTier::Safe,
            quality: ObservationQuality::Fresh,
            management: CacheManagementMode::Zenith,
            reason: None,
            expected_eligibility: CleanupEligibility::AutoCleanable,
            expected_cleanable: Some(100),
        },
        TestCase {
            risk: RiskTier::Safe,
            quality: ObservationQuality::Partial,
            management: CacheManagementMode::Zenith,
            reason: Some("Permission denied in subtree"),
            expected_eligibility: CleanupEligibility::Reviewable,
            expected_cleanable: Some(100),
        },
        TestCase {
            risk: RiskTier::Safe,
            quality: ObservationQuality::Unavailable,
            management: CacheManagementMode::Zenith,
            reason: Some("Directory inaccessible"),
            expected_eligibility: CleanupEligibility::Blocked,
            expected_cleanable: None,
        },
        TestCase {
            risk: RiskTier::Safe,
            quality: ObservationQuality::Fresh,
            management: CacheManagementMode::Advisory,
            reason: None,
            expected_eligibility: CleanupEligibility::Advisory,
            expected_cleanable: None,
        },
        TestCase {
            risk: RiskTier::Rebuild,
            quality: ObservationQuality::Fresh,
            management: CacheManagementMode::Zenith,
            reason: None,
            expected_eligibility: CleanupEligibility::Reviewable,
            expected_cleanable: Some(100),
        },
        TestCase {
            risk: RiskTier::Rebuild,
            quality: ObservationQuality::Fresh,
            management: CacheManagementMode::ToolManaged,
            reason: None,
            expected_eligibility: CleanupEligibility::Reviewable,
            expected_cleanable: Some(100),
        },
        TestCase {
            risk: RiskTier::Rebuild,
            quality: ObservationQuality::Partial,
            management: CacheManagementMode::ToolManaged,
            reason: Some("Prune warning"),
            expected_eligibility: CleanupEligibility::Reviewable,
            expected_cleanable: Some(100),
        },
        TestCase {
            risk: RiskTier::Rebuild,
            quality: ObservationQuality::Unavailable,
            management: CacheManagementMode::ToolManaged,
            reason: Some("Tool not installed"),
            expected_eligibility: CleanupEligibility::Blocked,
            expected_cleanable: None,
        },
        TestCase {
            risk: RiskTier::Manual,
            quality: ObservationQuality::Fresh,
            management: CacheManagementMode::Zenith,
            reason: None,
            expected_eligibility: CleanupEligibility::Blocked,
            expected_cleanable: None,
        },
        TestCase {
            risk: RiskTier::Manual,
            quality: ObservationQuality::Unavailable,
            management: CacheManagementMode::Zenith,
            reason: Some("Disk unreadable"),
            expected_eligibility: CleanupEligibility::Blocked,
            expected_cleanable: None,
        },
    ];

    for tc in cases {
        let size = FileSize::new(100, Some(100));
        let metadata = CacheMetadata {
            management_mode: tc.management,
            ..Default::default()
        };
        let disposition = derive_cleanup_disposition(DispositionFacts::new(
            tc.risk, tc.quality, &metadata, &size, tc.reason,
        ));
        assert_eq!(
            disposition.eligibility, tc.expected_eligibility,
            "failed eligibility for {:?}/{:?}/{:?}",
            tc.risk, tc.quality, tc.management
        );
        assert_eq!(
            disposition.cleanable_bytes, tc.expected_cleanable,
            "failed cleanable_bytes for {:?}/{:?}/{:?}",
            tc.risk, tc.quality, tc.management
        );

        let item = ScanItem {
            id: "test.item".into(),
            signature_id: "test.sig".into(),
            name: "Test Item".into(),
            category: Category::Developer,
            risk: tc.risk,
            path: "/tmp/test".into(),
            size,
            file_count: 1,
            description: "test".into(),
            cache_metadata: metadata,
            unit: CleanupUnit::fixed_path("/tmp/test".to_string()),
            ownership: CleanupOwnership::unknown(),
            age: None,
            stale: None,
            structured_state: None,
            entry_kind: EntryKind::Directory,
            gate: EligibilityGate::Open,
            owner_running: false,
            is_selected: disposition.eligibility == CleanupEligibility::AutoCleanable,
            last_modified: None,
            exists: true,
            quality: tc.quality,
            incomplete_reason: tc.reason.map(str::to_string),
            skipped_entry_count: 0,
            disposition,
        };

        assert_eq!(
            item.allows_cleanup(),
            matches!(
                tc.expected_eligibility,
                CleanupEligibility::AutoCleanable | CleanupEligibility::Reviewable
            )
        );
        assert_eq!(item.cleanable_bytes(), tc.expected_cleanable.unwrap_or(0));
        assert!(item.cleanable_bytes() <= item.observed_bytes());
    }
}

/// One plan, executed twice: the second run finds every target already gone and
/// skips it rather than deleting whatever occupies the path by then.
#[test]
fn replaying_a_plan_skips_targets_instead_of_deleting_replacements() {
    let fixture = tempdir().expect("fixture");
    let cache = fixture.path().join("replay-cache");
    fs::create_dir(&cache).unwrap();
    let payload = cache.join("payload.bin");
    fs::write(&payload, b"first generation").unwrap();
    let parent = fixture.path().join("parent");
    fs::create_dir(&parent).unwrap();

    let mut registry = SignatureRegistry::new();
    registry.register(Signature {
        id: "test.replay".into(),
        name: "Replay cache".into(),
        category: Category::Developer,
        risk: RiskTier::Safe,
        strategy: CleanStrategy::DeleteContents,
        paths: vec![cache.to_string_lossy().into_owned()],
        exclusions: vec![],
        description: "replay fixture".into(),
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
        management_mode: Default::default(),
        artifact_kind: Default::default(),
        consequence: String::new(),
        reclaimable_is_lower_bound: false,
    });

    let mut item = ScanItem::mock(
        "test.replay",
        "test.replay",
        "Replay cache",
        Category::Developer,
        RiskTier::Safe,
        cache.to_string_lossy().into_owned(),
        FileSize::new(1024, Some(1024)),
        1,
    );
    item.is_selected = true;
    item.entry_kind = EntryKind::Directory;
    item.unit = CleanupUnit::fixed_path(cache.to_string_lossy().into_owned());

    let plan = SafetyPlanner::create_plan(std::slice::from_ref(&item), &registry)
        .expect("the first plan is authorized");
    let first = CleanExecutor::execute(plan.clone(), &PlatformEnvironment::native(), |_| {});
    assert_eq!(
        first.items[0].status,
        zenith_lib::models::CleanStatus::Success
    );
    assert!(!payload.exists());

    // Something else now sits where the plan's target used to be.
    fs::write(&payload, b"second generation").unwrap();

    let second = CleanExecutor::execute(plan, &PlatformEnvironment::native(), |_| {});
    assert_eq!(
        second.items[0].status,
        zenith_lib::models::CleanStatus::Skipped
    );
    assert!(!second.items[0].success);
    assert_eq!(second.total_reclaimed_bytes, 0);
    assert_eq!(second.failed_count, 0);
    assert!(
        payload.exists(),
        "a replayed plan never deletes a replacement object"
    );
}

/// A target whose path is no longer inside the unit that authorized it is
/// refused before any mutation.
#[test]
fn a_target_outside_its_authorizing_unit_is_refused() {
    let fixture = tempdir().expect("fixture");
    let unit_root = fixture.path().join("unit-root");
    let outside = fixture.path().join("elsewhere");
    fs::create_dir(&unit_root).unwrap();
    fs::create_dir(&outside).unwrap();

    let mut target = DeleteTarget {
        item_id: "test.outside".into(),
        signature_id: "test.signature".into(),
        name: "Outside target".into(),
        path: outside.clone(),
        strategy: CleanStrategy::DeleteContents,
        expected_bytes: 0,
        risk: RiskTier::Safe,
        identity: ToctouGuard::capture(&outside),
        exclusions: vec![],
        min_age_days: None,
        unit: CleanupUnit::fixed_path(unit_root.to_string_lossy().into_owned()),
        target_kind: EntryKind::Directory,
        owner: CleanupOwnership::unknown(),
        process_guard: RunningProcessPolicy::none(),
    };
    target.identity = ToctouGuard::capture(&outside);

    match SafetyValidator::revalidate(&target, &PlatformEnvironment::native()) {
        RevalidationOutcome::Failed(result) => {
            assert_eq!(
                result.failure_reason,
                Some(CleanFailureReason::ChangedSinceScan)
            );
            assert!(result
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("cleanup unit")));
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(outside.exists());
}

/// A path whose name already identifies structured state never reaches a
/// filesystem mutation, however it was discovered.
#[test]
fn structured_state_is_refused_by_the_execution_guard() {
    let fixture = tempdir().expect("fixture");
    for (name, expected) in [
        ("session.sqlite", "database file"),
        ("session.sqlite-wal", "database companion file"),
        ("app.lock", "lock or pid file"),
        ("auth.json", "credential or key material"),
    ] {
        let path = fixture.path().join(name);
        fs::write(&path, b"state").unwrap();

        let target = DeleteTarget {
            item_id: format!("test.structured.{name}"),
            signature_id: "test.signature".into(),
            name: name.to_string(),
            path: path.clone(),
            strategy: CleanStrategy::DeleteDirectory,
            expected_bytes: 5,
            risk: RiskTier::Safe,
            identity: ToctouGuard::capture(&path),
            exclusions: vec![],
            min_age_days: None,
            unit: CleanupUnit::fixed_path(path.to_string_lossy().into_owned()),
            target_kind: EntryKind::File,
            owner: CleanupOwnership::unknown(),
            process_guard: RunningProcessPolicy::none(),
        };

        match SafetyValidator::revalidate(&target, &PlatformEnvironment::native()) {
            RevalidationOutcome::Skipped(result) => {
                assert_eq!(
                    result.failure_reason,
                    Some(CleanFailureReason::StructuredStore)
                );
                assert!(
                    result
                        .error_message
                        .as_deref()
                        .is_some_and(|message| message.contains(expected)),
                    "{name} must name its structured kind"
                );
            }
            other => panic!("expected {name} to be refused, got {other:?}"),
        }
        assert!(path.exists(), "{name} must be left in place");
    }
}

/// A plan cannot be built from a candidate an age rule matched but that holds
/// structured state: the refusal happens before the user is offered a
/// confirmation.
#[test]
fn the_planner_refuses_a_structured_target() {
    let fixture = tempdir().expect("fixture");
    let database = fixture.path().join("app-cache.db");
    fs::write(&database, b"state").unwrap();

    let mut registry = SignatureRegistry::new();
    registry.register(Signature {
        id: "test.aged-structured".into(),
        name: "Aged namespace".into(),
        category: Category::System,
        risk: RiskTier::Safe,
        strategy: CleanStrategy::DeleteDirectory,
        paths: vec![fixture.path().to_string_lossy().into_owned()],
        exclusions: vec![],
        description: "aged namespace fixture".into(),
        min_age_days: Some(7),
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
        management_mode: Default::default(),
        artifact_kind: Default::default(),
        consequence: String::new(),
        reclaimable_is_lower_bound: false,
    });

    let mut item = ScanItem::mock(
        "test.aged-structured.0.app-cache.db",
        "test.aged-structured",
        "app-cache.db",
        Category::System,
        RiskTier::Safe,
        database.to_string_lossy().into_owned(),
        FileSize::new(5, Some(4096)),
        1,
    );
    item.is_selected = true;
    item.entry_kind = EntryKind::File;
    item.unit = CleanupUnit::child_namespace(
        fixture.path().to_string_lossy().into_owned(),
        database.to_string_lossy().into_owned(),
    );

    let error = SafetyPlanner::create_plan(&[item], &registry)
        .expect_err("structured state is never plannable");
    assert!(
        matches!(&error, ZenithError::InvalidPlan(message) if message.contains("database")),
        "the refusal names the classification: {error}"
    );
    assert!(database.exists());
}

/// A plan states what it authorizes, and execution refuses anything else: a
/// projection the user reviewed is not a permission to delete.
/// Backdates a file so an age policy sees it as inactive.
fn age_entry(path: &std::path::Path, days: u64) {
    let when = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(days * 86_400))
        .expect("the fixture clock has a past");
    let entry = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open fixture entry");
    entry.set_modified(when).expect("backdate fixture entry");
}

/// A root the scan cannot read stays in the result with its reason, so a
/// smaller total is never presented as a complete one.
#[cfg(unix)]
#[test]
fn an_unreadable_root_is_reported_with_its_reason() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = tempdir().expect("fixture");
    let root = fixture.path().join("cache-root");
    let candidate = root.join("com.example.blocked");
    fs::create_dir_all(&candidate).unwrap();
    fs::write(candidate.join("payload.bin"), vec![7u8; 2_048]).unwrap();

    let mut registry = SignatureRegistry::new();
    registry.register(Signature {
        id: "test.unreadable".into(),
        name: "Unreadable root".into(),
        category: Category::System,
        risk: RiskTier::Safe,
        strategy: CleanStrategy::DeleteStaleContents,
        paths: vec![root.to_string_lossy().into_owned()],
        exclusions: vec![],
        description: "fixture".into(),
        min_age_days: Some(7),
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
        management_mode: Default::default(),
        artifact_kind: Default::default(),
        consequence: String::new(),
        reclaimable_is_lower_bound: false,
    });

    // A directory the process may not read is what macOS withholds without
    // Full Disk Access, and the scan has to say so rather than report nothing.
    fs::set_permissions(&candidate, fs::Permissions::from_mode(0o000)).unwrap();
    let items = zenith_lib::scanner::DirectoryScanner::scan_signature(
        registry.get("test.unreadable").expect("registered"),
        &PlatformEnvironment::native(),
        &zenith_lib::models::NeverCancelled,
    );
    // Restore before the fixture is dropped so it can be removed.
    fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755)).unwrap();

    let item = items
        .iter()
        .find(|item| item.path.ends_with("com.example.blocked"))
        .expect("an unreadable candidate is retained");
    assert_eq!(item.quality, ObservationQuality::Unavailable);
    assert!(
        !item.is_selected,
        "an unreadable candidate is never selected"
    );
    assert_eq!(item.cleanable_bytes(), 0);
    let reason = item
        .incomplete_reason
        .as_deref()
        .expect("the reason travels with the item");
    assert!(
        reason.contains("com.example.blocked"),
        "the reason names the location: {reason}"
    );
}

/// A cache whose owning application is running is reported and never
/// auto-selected, whatever its age, and an explicit selection still works.
#[cfg(unix)]
#[test]
fn a_running_owner_keeps_its_cache_out_of_the_default_selection() {
    let fixture = tempdir().expect("fixture");
    let root = fixture.path().join("cache-root");
    let namespace = root.join("com.example.running");
    fs::create_dir_all(&namespace).unwrap();
    let blob = namespace.join("data.bin");
    fs::write(&blob, vec![1u8; 4_096]).unwrap();
    age_entry(&blob, 30);

    let mut registry = SignatureRegistry::new();
    registry.register(Signature {
        id: "test.running-owner".into(),
        name: "Third-party caches".into(),
        category: Category::System,
        risk: RiskTier::Safe,
        strategy: CleanStrategy::DeleteStaleContents,
        paths: vec![root.to_string_lossy().into_owned()],
        exclusions: vec![],
        description: "fixture".into(),
        min_age_days: Some(7),
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
        management_mode: Default::default(),
        artifact_kind: Default::default(),
        consequence: String::new(),
        reclaimable_is_lower_bound: false,
    });

    let running = zenith_lib::applications::RunningApplications::from_ids(vec![
        "com.example.running".to_string(),
    ]);
    let items = zenith_lib::scanner::DirectoryScanner::scan_signature_with_pool(
        registry.get("test.running-owner").expect("registered"),
        None,
        &PlatformEnvironment::native(),
        &zenith_lib::models::NeverCancelled,
        zenith_lib::models::EligibilityGate::Open,
        &running,
    );

    let item = items
        .iter()
        .find(|item| item.path.ends_with("com.example.running"))
        .expect("the namespace is discovered");
    assert!(item.owner_running);
    assert_eq!(item.disposition.eligibility, CleanupEligibility::Reviewable);
    assert!(
        !item.is_selected,
        "an application that is running keeps its cache out of the default selection"
    );
    assert!(item.cleanable_bytes() > 0, "the bytes are still reported");
    assert!(item
        .disposition
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("running")));

    // The scan without that application running reaches the same namespace
    // with the ordinary verdict, which is what makes the difference the
    // owner's state rather than the fixture.
    let idle = zenith_lib::scanner::DirectoryScanner::scan_signature(
        registry.get("test.running-owner").expect("registered"),
        &PlatformEnvironment::native(),
        &zenith_lib::models::NeverCancelled,
    );
    let idle_item = idle
        .iter()
        .find(|item| item.path.ends_with("com.example.running"))
        .expect("the namespace is discovered");
    assert!(!idle_item.owner_running);
    assert_eq!(
        idle_item.disposition.eligibility,
        CleanupEligibility::AutoCleanable
    );
    assert!(idle_item.is_selected);
}

/// The shipped Explorer cache entry reaches a plan, which the shared
/// structured-state rule would otherwise refuse: the store uses a database
/// extension, and the classifier has to know that this family of containers is
/// regenerated rather than protected.
///
/// The signature under test is the *shipped* one with its root pointed at a
/// fixture, so the test fails if the catalog entry stops being the shape this
/// behaviour needs (a prefix list, a `Safe` tier, and an age policy).
#[test]
fn the_shipped_explorer_cache_entry_scans_and_plans() {
    let fixture = tempdir().expect("fixture");
    let explorer = fixture.path().join("Explorer");
    fs::create_dir_all(&explorer).unwrap();
    let thumbcache = explorer.join("thumbcache_256.db");
    let iconcache = explorer.join("iconcache_48.db");
    fs::write(&thumbcache, b"thumbnails").unwrap();
    fs::write(&iconcache, b"icons").unwrap();
    // The directory's own state is not part of the cache.
    let shell_state = explorer.join("state.bin");
    fs::write(&shell_state, b"shell state").unwrap();
    age_entry(&thumbcache, 30);
    age_entry(&iconcache, 30);

    let shipped = SignatureRegistry::load_embedded()
        .expect("catalog")
        .get("system.windows.explorer_thumbnails")
        .cloned()
        .expect("the Explorer cache entry is in the catalog");
    let mut signature = shipped.clone();
    signature.paths = vec![explorer.to_string_lossy().into_owned()];
    signature.platforms = vec![];

    let mut registry = SignatureRegistry::new();
    registry.register(signature.clone());

    let items = zenith_lib::scanner::DirectoryScanner::scan_signature(
        &signature,
        &PlatformEnvironment::native(),
        &zenith_lib::models::NeverCancelled,
    );
    let cache_item = items
        .iter()
        .find(|item| item.path.ends_with("thumbcache_256.db"))
        .expect("the thumbnail cache is discovered");
    assert_eq!(
        cache_item.disposition.eligibility,
        CleanupEligibility::AutoCleanable,
        "a regenerable cache container is cleanable: {:?}",
        cache_item.disposition
    );
    assert!(cache_item.is_selected);
    assert!(
        !items.iter().any(|item| item.path.ends_with("state.bin")),
        "the shipped prefix list keeps the rest of the directory out of scope"
    );

    eprintln!(
        "DEBUG item ownership={:?} signature ownership={:?} unit={:?}",
        cache_item.ownership,
        signature.ownership(),
        cache_item.unit
    );

    // Discovery is not the claim: the plan must be buildable too.
    let mut selected = cache_item.clone();
    selected.is_selected = true;
    let plan = SafetyPlanner::create_plan(&[selected], &registry)
        .expect("the cache container is plannable");
    assert_eq!(plan.targets.len(), 1);
    assert_eq!(plan.targets[0].path, thumbcache);
    assert!(thumbcache.exists(), "planning does not mutate");
}

/// A cache namespace that is written to while it is being cleaned: the aged
/// remainder is reported and removed, and everything newer — or structured —
/// stays.
#[test]
fn a_mixed_age_cache_namespace_reports_and_prunes_its_stale_remainder() {
    let fixture = tempdir().expect("fixture");
    let root = fixture.path().join("cache-root");
    let namespace = root.join("com.example.client");
    fs::create_dir_all(namespace.join("sub")).unwrap();
    let old_blob = namespace.join("old.bin");
    let fresh_blob = namespace.join("fresh.bin");
    let database = namespace.join("Cache.db");
    let nested_old = namespace.join("sub/old2.bin");
    let outside = fixture.path().join("outside.bin");
    fs::write(&old_blob, vec![1u8; 8_192]).unwrap();
    fs::write(&fresh_blob, vec![2u8; 8_192]).unwrap();
    fs::write(&database, vec![3u8; 4_096]).unwrap();
    fs::write(&nested_old, vec![4u8; 4_096]).unwrap();
    fs::write(&outside, b"precious").unwrap();
    age_entry(&old_blob, 30);
    age_entry(&database, 30);
    age_entry(&nested_old, 30);
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let link = namespace.join("linked.bin");
        symlink(&outside, &link).unwrap();
        // The link's own timestamp is what the policy judges, and a link made
        // now is not stale; the file behind it is never touched either way.
        age_entry(&outside, 30);
    }

    let mut registry = SignatureRegistry::new();
    registry.register(Signature {
        id: "test.stale-namespace".into(),
        name: "App cache".into(),
        category: Category::System,
        risk: RiskTier::Safe,
        strategy: CleanStrategy::DeleteStaleContents,
        paths: vec![root.to_string_lossy().into_owned()],
        exclusions: vec![],
        description: "Third-party cache namespace".into(),
        min_age_days: Some(7),
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
        management_mode: Default::default(),
        artifact_kind: Default::default(),
        consequence: String::new(),
        reclaimable_is_lower_bound: false,
    });

    let items = zenith_lib::scanner::DirectoryScanner::scan_signature(
        registry.get("test.stale-namespace").expect("registered"),
        &PlatformEnvironment::native(),
        &zenith_lib::models::NeverCancelled,
    );
    let item = items
        .iter()
        .find(|item| item.path.ends_with("com.example.client"))
        .expect("the namespace is discovered");
    let observed = item.observed_bytes();
    let cleanable = item.cleanable_bytes();
    assert!(
        cleanable > 0 && cleanable < observed,
        "the aged remainder is reported: {cleanable} of {observed}"
    );
    assert_eq!(
        item.disposition.eligibility,
        CleanupEligibility::AutoCleanable
    );
    let stale = item
        .stale
        .expect("the per-entry verdict travels with the item");
    assert_eq!(stale.min_age_days, 7);
    assert_eq!(stale.stale_bytes, cleanable);
    assert!(item
        .disposition
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("more recently")));

    let mut selected = item.clone();
    selected.is_selected = true;
    let plan = SafetyPlanner::create_plan(&[selected], &registry).expect("plan");
    assert_eq!(plan.expected_reclaim_bytes, cleanable);
    let result = CleanExecutor::execute(plan.clone(), &PlatformEnvironment::native(), |_| {});

    assert!(!old_blob.exists(), "the aged entry is removed");
    assert!(
        !nested_old.exists(),
        "an aged entry below the namespace is removed"
    );
    assert!(
        !namespace.join("sub").exists(),
        "a directory that ends up empty is removed"
    );
    assert!(fresh_blob.exists(), "an entry written today stays");
    assert!(
        database.exists(),
        "structured state stays, however old it is"
    );
    assert!(outside.exists(), "the file behind a link is never touched");
    #[cfg(unix)]
    assert!(
        namespace.join("linked.bin").symlink_metadata().is_ok(),
        "a fresh link is kept like any other fresh entry"
    );
    assert!(result.total_reclaimed_bytes > 0);
    assert_eq!(result.failed_count, 0);

    // A second run has nothing left that satisfies the policy.
    let again = CleanExecutor::execute(plan, &PlatformEnvironment::native(), |_| {});
    assert_eq!(
        again.items[0].status,
        zenith_lib::models::CleanStatus::Skipped
    );
    assert_eq!(again.total_reclaimed_bytes, 0);
    assert!(fresh_blob.exists());
}

#[test]
fn a_non_mutating_plan_is_refused_by_the_executor() {
    let fixture = tempdir().expect("fixture");
    let cache = fixture.path().join("preview-cache");
    fs::create_dir(&cache).unwrap();
    fs::write(cache.join("payload.bin"), b"kept").unwrap();

    let plan = DeletePlan {
        id: uuid::Uuid::new_v4(),
        scan_id: "scan-preview".into(),
        targets: vec![DeleteTarget {
            item_id: "preview-target".into(),
            signature_id: "test.preview".into(),
            name: "Preview cache".into(),
            path: cache.clone(),
            strategy: CleanStrategy::DeleteContents,
            expected_bytes: 4,
            risk: RiskTier::Safe,
            identity: ToctouGuard::capture(&cache),
            exclusions: vec![],
            min_age_days: None,
            unit: CleanupUnit::fixed_path(cache.to_string_lossy().into_owned()),
            target_kind: EntryKind::Directory,
            owner: CleanupOwnership::unknown(),
            process_guard: RunningProcessPolicy::none(),
        }],
        expected_reclaim_bytes: 4,
        risk: zenith_lib::models::RiskSummary::default(),
        created_at: 0,
        mode: CleanupMode::Preview,
    };

    let preview = plan.preview(60);
    assert_eq!(preview.mode, CleanupMode::Preview);
    assert!(!CleanupMode::Preview.is_mutating());
    assert!(CleanupMode::PermanentDelete.is_mutating());
    // The projection cannot carry a path or a strategy.
    let serialized = serde_json::to_value(&preview).expect("the preview is a contract");
    let text = serialized.to_string();
    assert!(!text.contains(&cache.to_string_lossy().into_owned()));
    assert!(!text.contains("strategy"));

    for mode in [CleanupMode::Preview, CleanupMode::Trash] {
        let mut refused_plan = plan.clone();
        refused_plan.mode = mode;
        let result = CleanExecutor::execute(refused_plan, &native_environment(), |_| {});
        assert_eq!(result.failed_count, 1, "{mode:?} must be refused");
        assert_eq!(result.total_reclaimed_bytes, 0);
        assert!(
            cache.join("payload.bin").exists(),
            "{mode:?} mutated the target"
        );
    }
}

#[test]
fn nested_structured_state_skips_the_whole_cleanup_unit_before_mutation() {
    let fixture = tempdir().expect("fixture");
    let cache = fixture.path().join("ordinary-cache");
    fs::create_dir(&cache).unwrap();
    let disposable = cache.join("disposable.bin");
    fs::write(&disposable, b"keep too").unwrap();
    let nested = cache.join("session.sqlite");
    fs::write(&nested, b"database").unwrap();

    let mut registry = SignatureRegistry::new();
    registry.register(Signature {
        id: "test.nested-structured".into(),
        name: "Ordinary cache".into(),
        category: Category::System,
        risk: RiskTier::Safe,
        strategy: CleanStrategy::DeleteContents,
        paths: vec![cache.to_string_lossy().into_owned()],
        exclusions: vec![],
        description: "test fixture".into(),
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
        management_mode: Default::default(),
        artifact_kind: Default::default(),
        consequence: String::new(),
        reclaimable_is_lower_bound: false,
    });
    let mut item = ScanItem::mock(
        "ordinary-cache",
        "test.nested-structured",
        "Ordinary cache",
        Category::System,
        RiskTier::Safe,
        cache.to_string_lossy().into_owned(),
        FileSize::new(16, Some(16)),
        2,
    );
    item.is_selected = true;
    let error = SafetyPlanner::create_plan(&[item], &registry)
        .expect_err("nested structured state must be rejected before confirmation");
    assert!(
        matches!(&error, ZenithError::InvalidPlan(message) if message.contains("session.sqlite"))
    );

    let plan = DeletePlan {
        id: uuid::Uuid::new_v4(),
        scan_id: "scan-nested-structured".into(),
        targets: vec![DeleteTarget {
            item_id: "ordinary-cache".into(),
            signature_id: "test.nested-structured".into(),
            name: "Ordinary cache".into(),
            path: cache.clone(),
            strategy: CleanStrategy::DeleteContents,
            expected_bytes: 16,
            risk: RiskTier::Safe,
            identity: ToctouGuard::capture(&cache),
            exclusions: vec![],
            min_age_days: None,
            unit: CleanupUnit::fixed_path(cache.to_string_lossy().into_owned()),
            target_kind: EntryKind::Directory,
            owner: CleanupOwnership::unknown(),
            process_guard: RunningProcessPolicy::none(),
        }],
        expected_reclaim_bytes: 16,
        risk: zenith_lib::models::RiskSummary::default(),
        created_at: 0,
        mode: CleanupMode::PermanentDelete,
    };

    let result = CleanExecutor::execute(plan, &native_environment(), |_| {});
    assert_eq!(result.skipped_count, 1);
    assert_eq!(
        result.items[0].failure_reason,
        Some(CleanFailureReason::StructuredStore)
    );
    assert_eq!(result.total_reclaimed_bytes, 0);
    assert!(nested.exists());
    assert!(disposable.exists(), "preflight must precede every mutation");
}

#[test]
fn tree_deleter_refuses_structured_state_added_after_validation() {
    let fixture = tempdir().expect("fixture");
    let cache = fixture.path().join("ordinary-cache");
    fs::create_dir(&cache).unwrap();
    let environment = native_environment();
    let validated =
        validated_filesystem_target(&cache, CleanStrategy::DeleteContents, &[], &environment);

    let inserted = cache.join("config.json");
    fs::write(&inserted, b"configuration").unwrap();
    let report = SafeTreeDeleter::delete_contents_validated(&validated, &environment);
    assert!(!report.is_success());
    assert!(report
        .errors
        .iter()
        .any(|error| error.contains("structured state")));
    assert!(inserted.exists());
}

#[cfg(unix)]
#[test]
fn a_file_replaced_by_a_symlink_between_scan_and_clean_is_skipped() {
    use std::os::unix::fs::symlink;

    let fixture = tempdir().expect("fixture");
    let target = fixture.path().join("cached-blob.bin");
    let outside = fixture.path().join("outside.bin");
    fs::write(&target, b"original").unwrap();
    fs::write(&outside, b"precious").unwrap();

    let target_identity = ToctouGuard::capture(&target);
    fs::remove_file(&target).unwrap();
    symlink(&outside, &target).unwrap();

    let plan_target = DeleteTarget {
        item_id: "race-file".into(),
        signature_id: "test.signature".into(),
        name: "Cached blob".into(),
        path: target.clone(),
        strategy: CleanStrategy::DeleteDirectory,
        expected_bytes: 8,
        risk: RiskTier::Safe,
        identity: target_identity,
        exclusions: vec![],
        min_age_days: None,
        unit: CleanupUnit::fixed_path(target.to_string_lossy().into_owned()),
        target_kind: EntryKind::File,
        owner: CleanupOwnership::unknown(),
        process_guard: RunningProcessPolicy::none(),
    };

    match SafetyValidator::revalidate(&plan_target, &PlatformEnvironment::native()) {
        RevalidationOutcome::Skipped(result) => {
            assert_eq!(
                result.failure_reason,
                Some(CleanFailureReason::SafetyBoundary)
            );
        }
        other => panic!("expected the replaced file to be refused, got {other:?}"),
    }
    assert!(outside.exists(), "the link target must be untouched");
    assert!(target.symlink_metadata().is_ok());
}

#[cfg(unix)]
#[test]
fn a_directory_replaced_by_a_symlink_between_scan_and_clean_is_skipped() {
    use std::os::unix::fs::symlink;

    let fixture = tempdir().expect("fixture");
    let target = fixture.path().join("cache-namespace");
    let outside = fixture.path().join("outside-dir");
    fs::create_dir(&target).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("payload.bin"), b"precious").unwrap();

    let identity = ToctouGuard::capture(&target);
    fs::remove_dir(&target).unwrap();
    symlink(&outside, &target).unwrap();

    let plan_target = DeleteTarget {
        item_id: "race-directory".into(),
        signature_id: "test.signature".into(),
        name: "Cache namespace".into(),
        path: target.clone(),
        strategy: CleanStrategy::DeleteDirectory,
        expected_bytes: 0,
        risk: RiskTier::Safe,
        identity,
        exclusions: vec![],
        min_age_days: None,
        unit: CleanupUnit::fixed_path(target.to_string_lossy().into_owned()),
        target_kind: EntryKind::Directory,
        owner: CleanupOwnership::unknown(),
        process_guard: RunningProcessPolicy::none(),
    };

    match SafetyValidator::revalidate(&plan_target, &PlatformEnvironment::native()) {
        RevalidationOutcome::Skipped(result) => {
            assert_eq!(
                result.failure_reason,
                Some(CleanFailureReason::SafetyBoundary)
            );
        }
        other => panic!("expected the replaced directory to be refused, got {other:?}"),
    }
    assert!(
        outside.join("payload.bin").exists(),
        "nothing behind the link is deleted"
    );
}

/// A target that changed kind is no longer the object the plan authorized.
#[test]
fn a_target_that_changed_kind_between_scan_and_clean_is_skipped() {
    let fixture = tempdir().expect("fixture");
    let target = fixture.path().join("cache-entry");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("payload.bin"), b"contents").unwrap();

    let identity = ToctouGuard::capture(&target);
    fs::remove_dir_all(&target).unwrap();
    fs::write(&target, b"now a file").unwrap();

    let plan_target = DeleteTarget {
        item_id: "kind-change".into(),
        signature_id: "test.signature".into(),
        name: "Cache entry".into(),
        path: target.clone(),
        strategy: CleanStrategy::DeleteContents,
        expected_bytes: 8,
        risk: RiskTier::Safe,
        identity,
        exclusions: vec![],
        min_age_days: None,
        unit: CleanupUnit::fixed_path(target.to_string_lossy().into_owned()),
        target_kind: EntryKind::Directory,
        owner: CleanupOwnership::unknown(),
        process_guard: RunningProcessPolicy::none(),
    };

    match SafetyValidator::revalidate(&plan_target, &PlatformEnvironment::native()) {
        RevalidationOutcome::Skipped(result) => {
            assert_eq!(
                result.failure_reason,
                Some(CleanFailureReason::ChangedSinceScan)
            );
        }
        other => panic!("expected the changed kind to be refused, got {other:?}"),
    }
    assert!(target.exists());
}

/// A scan result that carries the given items, taken now, so a test plans
/// through the same entry point production uses.
fn scan_with(items: Vec<ScanItem>) -> ScanResult {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the test clock is after the epoch")
        .as_secs();
    ScanResult {
        scan_id: "scan".into(),
        valid_for_seconds: ScanResult::VALID_FOR_SECONDS,
        started_at: now,
        finished_at: now,
        categories: vec![CategoryResult {
            category: Category::Developer,
            display_name: "Developer".into(),
            items,
            total_bytes: 0,
            cleanable_bytes: 0,
            safe_bytes: 0,
            rebuild_bytes: 0,
            manual_bytes: 0,
            quality: ObservationQuality::Fresh,
            skipped_entry_count: 0,
            incomplete_item_count: 0,
            eligibility: Default::default(),
            suppressed_duplicate_count: 0,
            suppressed_duplicate_bytes: 0,
        }],
        total_bytes: 0,
        cleanable_bytes: 0,
        safe_bytes: 0,
        rebuild_bytes: 0,
        manual_bytes: 0,
        quality: ObservationQuality::Fresh,
        incomplete_reasons: vec![],
        skipped_entry_count: 0,
        incomplete_item_count: 0,
        eligibility: Default::default(),
        suppressed_duplicate_count: 0,
        suppressed_duplicate_bytes: 0,
    }
}

/// The plan carries the scan-time expectations the guard re-asserts, including
/// the owner and the process policy the catalog declared.
#[test]
fn a_plan_carries_the_scan_time_expectations() {
    let fixture = tempdir().expect("fixture");
    // The catalog's cargo signature resolves `~/.cargo/registry/cache`, so the
    // fixture states a profile whose home is the temporary directory.
    let cache = fixture.path().join(".cargo/registry/cache");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("payload.bin"), b"crate").unwrap();

    let environment =
        PlatformEnvironment::simulated(PathFlavor::current()).with_roots(std::sync::Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::current())
                .with_home(fixture.path())
                .with_temp_dir(fixture.path()),
        ));
    let registry = SignatureRegistry::load_embedded_with(&environment).expect("catalog");
    let signature = registry
        .get("dev.cargo.registry.cache")
        .expect("the cargo cache entry is in the catalog");
    let expected_guard = signature.process_guard();
    assert!(!expected_guard.is_empty(), "the catalog declares a guard");
    let expected_owner = signature.ownership();

    let mut item = ScanItem::mock(
        "dev.cargo.registry.cache",
        "dev.cargo.registry.cache",
        "Cargo Registry Cache",
        Category::Developer,
        RiskTier::Rebuild,
        cache.to_string_lossy().into_owned(),
        FileSize::new(1024, Some(4096)),
        1,
    );
    item.is_selected = true;
    item.entry_kind = EntryKind::Directory;
    item.unit = CleanupUnit::fixed_path(cache.to_string_lossy().into_owned());
    // The scan records the ownership the catalog declares; the planner refuses
    // an item that reports anything else.
    item.ownership = expected_owner.clone();

    let scan = scan_with(vec![item.clone()]);
    let plan = SafetyPlanner::create_plan_from_scan(
        &scan,
        "scan",
        &[item.id.clone()],
        &registry,
        &environment,
    )
    .expect("the catalog signature authorizes the fixture path");
    assert_eq!(plan.mode, CleanupMode::PermanentDelete);
    let target = &plan.targets[0];
    assert_eq!(target.target_kind, EntryKind::Directory);
    assert_eq!(target.unit.kind, CleanupUnitKind::FixedPath);
    assert_eq!(target.unit.path, cache.to_string_lossy());
    assert_eq!(target.owner.owner, expected_owner.owner);
    assert_eq!(
        target.process_guard.executables(),
        expected_guard.executables()
    );
}

/// What a plan states about its target is validated, not assumed: an item that
/// cannot name its unit never becomes a target.
#[test]
fn an_item_that_cannot_name_its_unit_is_refused_at_planning() {
    let fixture = tempdir().expect("fixture");
    let cache = fixture.path().join("unnamed-unit");
    fs::create_dir(&cache).unwrap();

    let registry = SignatureRegistry::load_embedded().expect("catalog");
    let mut item = ScanItem::mock(
        "system.intensive.user_app_caches.0.unnamed",
        "system.intensive.user_app_caches",
        "Unnamed",
        Category::System,
        RiskTier::Safe,
        cache.to_string_lossy().into_owned(),
        FileSize::new(1024, Some(1024)),
        1,
    );
    item.is_selected = true;
    item.unit = CleanupUnit::default();

    let error = SafetyPlanner::create_plan(&[item], &registry)
        .expect_err("an undeclared unit is not plannable");
    assert!(matches!(error, ZenithError::InvalidPlan(_)));
}

#[test]
fn test_nested_protected_app_bundle_fails_closed() {
    use zenith_lib::commands::select_quick_clean_safe_candidates;
    use zenith_lib::models::ZenithSettings;

    let size = FileSize::new(5000, Some(5000));
    let reason = Some(
        "Protected system or application bundle detected: /tmp/cache/Payload/Malicious.app"
            .to_string(),
    );
    let metadata = CacheMetadata::default();
    let disposition = derive_cleanup_disposition(DispositionFacts::new(
        RiskTier::Safe,
        ObservationQuality::Partial,
        &metadata,
        &size,
        reason.as_deref(),
    ));

    assert_eq!(disposition.eligibility, CleanupEligibility::Blocked);
    assert_eq!(disposition.cleanable_bytes, None);
    assert!(disposition
        .reason
        .as_deref()
        .unwrap_or_default()
        .contains("Protected"));

    let mut item = ScanItem {
        id: "test.nested_app".into(),
        signature_id: "dev.signature".into(),
        name: "Nested App Item".into(),
        category: Category::Developer,
        risk: RiskTier::Safe,
        path: "/tmp/cache".into(),
        size,
        file_count: 10,
        description: "test".into(),
        cache_metadata: metadata,
        unit: CleanupUnit::fixed_path("/tmp/cache".to_string()),
        ownership: CleanupOwnership::unknown(),
        age: None,
        stale: None,
        structured_state: None,
        entry_kind: EntryKind::Directory,
        gate: EligibilityGate::Open,
        owner_running: false,
        is_selected: false,
        last_modified: None,
        exists: true,
        quality: ObservationQuality::Partial,
        incomplete_reason: reason,
        skipped_entry_count: 0,
        disposition,
    };

    assert!(!item.allows_cleanup());
    assert_eq!(item.cleanable_bytes(), 0);
    assert_eq!(item.observed_bytes(), 5000);

    // Quick clean must NOT include it
    let scan = ScanResult {
        scan_id: "nested-app-scan".into(),
        valid_for_seconds: 60,
        started_at: 1000,
        finished_at: 1005,
        total_bytes: 5000,
        cleanable_bytes: 0,
        safe_bytes: 0,
        rebuild_bytes: 0,
        manual_bytes: 0,
        categories: vec![CategoryResult {
            category: Category::Developer,
            display_name: "Developer".into(),
            total_bytes: 5000,
            cleanable_bytes: 0,
            safe_bytes: 0,
            rebuild_bytes: 0,
            manual_bytes: 0,
            quality: ObservationQuality::Partial,
            items: vec![item.clone()],
            skipped_entry_count: 0,
            incomplete_item_count: 1,
            eligibility: Default::default(),
            suppressed_duplicate_count: 0,
            suppressed_duplicate_bytes: 0,
        }],
        quality: ObservationQuality::Partial,
        incomplete_reasons: vec!["Protected system or application bundle detected".into()],
        skipped_entry_count: 0,
        incomplete_item_count: 1,
        eligibility: Default::default(),
        suppressed_duplicate_count: 0,
        suppressed_duplicate_bytes: 0,
    };

    let settings = ZenithSettings::default();
    let candidates = select_quick_clean_safe_candidates(&scan, &settings);
    assert!(
        candidates.is_empty(),
        "Blocked nested app item must never be quick-cleaned"
    );

    // Planning even if forced selected must fail closed
    item.is_selected = true;
    let registry = SignatureRegistry::load_embedded().unwrap();
    let plan_res = SafetyPlanner::create_plan(&[item], &registry);
    assert!(
        matches!(plan_res, Err(ZenithError::InvalidPlan(_))),
        "Planning must reject items that do not allow cleanup"
    );
}

#[cfg(windows)]
mod windows_safety {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use zenith_core::domain::identity::FileIdentity;
    use zenith_lib::models::CleanupIdentity;

    #[test]
    fn directory_handle_captures_real_volume_file_identity() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("cache");
        fs::create_dir(&target).unwrap();
        let identity = ToctouGuard::capture(&target).expect("capture identity");
        assert_ne!(identity.entity(), FileIdentity::UNKNOWN);
        assert!(ToctouGuard::verify(&target, &identity).is_ok());
    }

    #[test]
    fn zero_identity_is_never_accepted_as_verified() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("cache");
        fs::create_dir(&target).unwrap();
        let identity = ToctouGuard::capture(&target).expect("capture");

        // The legacy Windows capture recorded (0, 0) when it could not open the
        // directory's handle and skipped the identity check. Rebuilding that
        // capture with an unknown entity must fail verification, not pass it.
        let unverified = CleanupIdentity::new(
            FileIdentity::UNKNOWN,
            identity.is_dir(),
            identity.size(),
            identity.modified(),
        );
        assert!(ToctouGuard::verify(&target, &unverified).is_err());
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

        let environment = PlatformEnvironment::native();
        let validated =
            validated_filesystem_target(&cache, CleanStrategy::DeleteContents, &[], &environment);
        let report = SafeTreeDeleter::delete_contents_validated(&validated, &environment);
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
        let environment = PlatformEnvironment::native();
        let validated =
            validated_filesystem_target(&deep, CleanStrategy::DeleteContents, &[], &environment);
        let report = SafeTreeDeleter::delete_contents_validated(&validated, &environment);
        assert!(report.is_success(), "errors: {:?}", report.errors);
        assert!(!payload.exists());
    }

    #[allow(dead_code)]
    fn wide_len(path: &Path) -> usize {
        path.as_os_str().encode_wide().count()
    }
}
