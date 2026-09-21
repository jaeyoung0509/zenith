use std::fs;
use std::path::Path;
use zenith_lib::models::{
    CleanStrategy, CleanupOwnership, CleanupUnit, DeleteTarget, EntryKind, NeverCancelled,
    RiskTier, RunningProcessPolicy,
};
use zenith_lib::safety::{
    RevalidationOutcome, SafeTreeDeleter, SafetyValidator, ToctouGuard, ValidatedTarget,
};
use zenith_lib::scanner::{
    DirectoryScanner, NoRootProgress, ScanLimits, SizeCalculator, TraversalCounters, WalkContext,
};
use zenith_platform::{PathFlavor, PlatformEnvironment};

fn validate(
    root: &Path,
    strategy: CleanStrategy,
    exclusions: Vec<String>,
    environment: &PlatformEnvironment,
) -> ValidatedTarget {
    let target = DeleteTarget {
        item_id: "exclusion-fixture".into(),
        signature_id: "test.exclusion".into(),
        name: "Exclusion fixture".into(),
        path: root.into(),
        strategy,
        expected_bytes: 0,
        risk: RiskTier::Safe,
        identity: ToctouGuard::capture(root),
        exclusions,
        min_age_days: None,
        unit: CleanupUnit::fixed_path(root.to_string_lossy().into_owned()),
        target_kind: EntryKind::Directory,
        owner: CleanupOwnership::unknown(),
        process_guard: RunningProcessPolicy::none(),
        provider_id: None,
        requires_confirmation: false,
    };
    match SafetyValidator::revalidate(&target, environment) {
        RevalidationOutcome::Validated(target) => target,
        other => panic!("fixture must pass the production guard: {other:?}"),
    }
}

#[test]
fn all_three_walkers_preserve_the_same_exclusions_and_count_the_same_payload() {
    for path_exclusion in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("cache");
        fs::create_dir(&root).unwrap();
        let kept = root.join("keep.bin");
        let payload = root.join("keep.bin.bak");
        fs::write(&kept, b"keep").unwrap();
        fs::write(&payload, b"disposable-payload").unwrap();
        let environment =
            PlatformEnvironment::simulated(PathFlavor::current()).with_home(fixture.path());
        let exclusions = vec![if path_exclusion {
            "~/cache/keep.bin".into()
        } else {
            "keep.bin".into()
        }];
        let size = SizeCalculator::measure_path_full(&root, &exclusions, &environment);
        let counters = TraversalCounters::default();
        let context = WalkContext::new(
            &environment,
            &NeverCancelled,
            ScanLimits::default(),
            &counters,
            &NoRootProgress,
        );
        let aged = DirectoryScanner::measure_tree_stats(&context, &root, &exclusions, 0, None);
        assert_eq!(size.file_count, 1);
        assert_eq!(size.size.logical, 18);
        assert_eq!(aged.file_count, 1);
        assert_eq!(aged.logical, size.size.logical);
        let target = validate(
            &root,
            CleanStrategy::DeleteContents,
            exclusions,
            &environment,
        );
        let report = SafeTreeDeleter::delete_contents_validated(&target, &environment);
        assert!(report.is_success(), "{:?}", report.errors);
        assert_eq!(report.deleted_files, 1);
        assert_eq!(report.skipped_files, 1);
        assert_eq!(fs::read(&kept).unwrap(), b"keep");
        assert!(!payload.exists());
    }
}

#[test]
fn git_state_added_after_validation_keeps_the_parent_and_reports_incomplete_deletion() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("cache");
    fs::create_dir_all(root.join("nested")).unwrap();
    fs::write(root.join("payload.bin"), b"disposable").unwrap();
    let environment =
        PlatformEnvironment::simulated(PathFlavor::current()).with_home(fixture.path());
    let target = validate(&root, CleanStrategy::DeleteDirectory, vec![], &environment);
    let git = root.join("nested/.git");
    fs::create_dir(&git).unwrap();
    fs::write(git.join("config"), b"protected").unwrap();
    let report = SafeTreeDeleter::delete_path_validated(&target, &environment);
    assert!(!report.is_success());
    assert_eq!(report.deleted_files, 1);
    assert!(!root.join("payload.bin").exists());
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("directory remains")),
        "{:?}",
        report.errors
    );
    assert_eq!(fs::read(git.join("config")).unwrap(), b"protected");
    assert!(root.join("nested").is_dir());
    assert!(root.is_dir());
}
