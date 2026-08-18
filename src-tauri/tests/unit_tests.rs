use std::fs::File;
use std::io::Write;
use tempfile::tempdir;
use zenith_lib::docker::DockerAdapter;
use zenith_lib::models::{AwakeBehavior, Category, RiskTier};
use zenith_lib::power::{KeepAwakeManager, PowerAssertion};
use zenith_lib::scanner::SizeCalculator;
use zenith_lib::signatures::SignatureRegistry;

#[test]
fn test_signature_registry_categories_and_risk_counts() {
    let registry = SignatureRegistry::load_embedded().expect("load embedded");

    // All categories must have valid signatures
    let ai_sigs = registry.by_category(Category::Ai);
    let dev_sigs = registry.by_category(Category::Developer);
    let container_sigs = registry.by_category(Category::Container);
    let model_sigs = registry.by_category(Category::Model);

    assert!(!ai_sigs.is_empty(), "AI signatures must not be empty");
    assert!(!dev_sigs.is_empty(), "Developer signatures must not be empty");
    assert!(!container_sigs.is_empty(), "Container signatures must not be empty");
    assert!(!model_sigs.is_empty(), "Model signatures must not be empty");

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
fn test_docker_size_parser() {
    assert_eq!(DockerAdapter::parse_docker_size("0B"), 0);
    assert_eq!(DockerAdapter::parse_docker_size("512B"), 512);
    assert_eq!(DockerAdapter::parse_docker_size("1KB"), 1024);
    assert_eq!(DockerAdapter::parse_docker_size("1.5MB"), (1.5 * 1024.0 * 1024.0) as u64);
    assert_eq!(DockerAdapter::parse_docker_size("10GB"), 10 * 1024 * 1024 * 1024);
    assert_eq!(DockerAdapter::parse_docker_size("2.5TB"), (2.5 * 1024.0 * 1024.0 * 1024.0 * 1024.0) as u64);
}

#[test]
fn test_size_calculator_recursive_and_exclusions() {
    let dir = tempdir().expect("tempdir");

    let keep_file = dir.path().join("keep.log");
    File::create(&keep_file).unwrap().write_all(&vec![0u8; 10000]).unwrap();

    let exclude_dir = dir.path().join("excluded_folder");
    std::fs::create_dir(&exclude_dir).unwrap();
    File::create(exclude_dir.join("excluded.dat")).unwrap().write_all(&vec![0u8; 50000]).unwrap();

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
