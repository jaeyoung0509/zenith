use std::fs;
use std::sync::Arc;
use zenith_lib::cleaner::{LifecycleProviderRegistry, OwnerProviderRegistry};
use zenith_lib::models::{
    CacheManagementMode, CacheSizeSemantics, Category, CleanStrategy, CleanerFamily,
    NeverCancelled, RiskTier, ZenithSettings,
};
use zenith_lib::scanner::{DirectoryScanner, ScanEngine};
use zenith_lib::signatures::{SignatureLoader, SignatureRegistry};
use zenith_platform::paths::SimulatedPaths;
use zenith_platform::{PathFlavor, PlatformEnvironment};

#[test]
fn temp_aliases_produce_one_cleanup_unit() {
    let fixture = tempfile::tempdir().unwrap();
    let temp = fixture.path().join("temp");
    let unit = temp.join("codex-fixture");
    fs::create_dir_all(&unit).unwrap();
    fs::write(unit.join("payload.bin"), vec![1u8; 4096]).unwrap();
    let environment = PlatformEnvironment::simulated(PathFlavor::current())
        .with_home(fixture.path())
        .with_temp_dir(&temp);
    let registry = SignatureRegistry::load_embedded_catalog().unwrap();
    let mut signature = registry.get("system.developer_temp").unwrap().clone();
    signature.min_age_days = Some(0);
    let items = DirectoryScanner::scan_signature(&signature, &environment, &NeverCancelled);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].size.logical, 4096);
    assert_eq!(items[0].path, unit.to_string_lossy());
}

#[test]
fn browser_profiles_and_offline_state_are_outside_cleanup_patterns() {
    let registry = SignatureRegistry::load_embedded_catalog().unwrap();
    for signature_id in [
        "system.app_support_cache_segments",
        "system.windows.roaming_cache_segments",
        "system.intensive.windows_browser_caches",
    ] {
        let signature = registry.get(signature_id).unwrap();
        let patterns = signature.paths.join("\n").to_ascii_lowercase();
        for protected in [
            "service worker",
            "cachestorage",
            "cookies",
            "history",
            "bookmarks",
            "login data",
            "local storage",
            "extensions",
            "downloads",
        ] {
            assert!(
                !patterns.contains(protected),
                "{signature_id} must not select browser state `{protected}`"
            );
        }
    }

    let saved_state = registry.get("system.saved_application_state").unwrap();
    assert_eq!(saved_state.strategy, CleanStrategy::Manual);
    assert_eq!(saved_state.risk, RiskTier::Manual);
}

#[test]
fn new_user_space_providers_never_claim_system_ownership() {
    let registry = SignatureRegistry::load_embedded_catalog().unwrap();
    for signature_id in [
        "dev.pip.cache",
        "dev.nuget.http",
        "dev.nuget.temp",
        "dev.nuget.plugins",
        "dev.nuget.global_packages",
    ] {
        let signature = registry.get(signature_id).unwrap();
        assert_eq!(signature.strategy, CleanStrategy::ExternalCommand);
        assert_eq!(signature.family, CleanerFamily::PackageManagers);
        assert!(signature.paths.is_empty());
    }

    let playwright = registry.get("dev.playwright.browsers").unwrap();
    assert_eq!(playwright.strategy, CleanStrategy::Manual);
    assert_eq!(playwright.risk, RiskTier::Manual);
    assert_eq!(playwright.family, CleanerFamily::PackageManagers);
}

#[test]
fn mixed_ai_tool_roots_are_advisory() {
    let registry = SignatureRegistry::load_embedded_catalog().unwrap();
    for signature_id in [
        "ai.claude.logs",
        "ai.claude.cache",
        "ai.cursor.logs",
        "ai.cursor.extensions.cache",
        "ai.gemini.cache",
        "ai.gemini.logs",
        "ai.codex.cache",
        "ai.aider.cache",
        "ai.opencode.cache",
    ] {
        let signature = registry.get(signature_id).unwrap();
        assert_eq!(signature.strategy, CleanStrategy::Manual, "{signature_id}");
        assert_eq!(signature.risk, RiskTier::Manual, "{signature_id}");
    }
}

#[test]
fn profile_and_app_data_aliases_measure_cursor_cache_once() {
    let fixture = tempfile::tempdir().unwrap();
    let support = fixture.path().join("Library/Application Support");
    let cache = support.join("Cursor/Cache");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("payload.bin"), vec![1u8; 8192]).unwrap();
    let environment = PlatformEnvironment::simulated(PathFlavor::current()).with_roots(Arc::new(
        SimulatedPaths::new()
            .with_flavor(PathFlavor::current())
            .with_home(fixture.path())
            .with_roaming_app_data(&support),
    ));
    let registry = SignatureRegistry::load_embedded_catalog().unwrap();
    let mut signature = registry.get("ai.cursor.cache").unwrap().clone();
    signature.paths = vec![
        "~/Library/Application Support/Cursor/Cache".into(),
        "${ROAMING_APP_DATA}/Cursor/Cache".into(),
    ];
    let items = DirectoryScanner::scan_signature(&signature, &environment, &NeverCancelled);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].size.logical, 8192);
}

#[test]
fn strategy_is_the_only_catalog_source_of_management_and_size_semantics() {
    let registry = SignatureRegistry::load_embedded_catalog().unwrap();
    let mut signature = registry.get("ai.claude.cache").unwrap().clone();
    for (strategy, management, size) in [
        (
            CleanStrategy::DeleteContents,
            CacheManagementMode::Zenith,
            CacheSizeSemantics::PhysicalReclaimable,
        ),
        (
            CleanStrategy::DeleteDirectory,
            CacheManagementMode::Zenith,
            CacheSizeSemantics::PhysicalReclaimable,
        ),
        (
            CleanStrategy::DeleteStaleContents,
            CacheManagementMode::Zenith,
            CacheSizeSemantics::PhysicalReclaimable,
        ),
        (
            CleanStrategy::Manual,
            CacheManagementMode::Advisory,
            CacheSizeSemantics::Informational,
        ),
        (
            CleanStrategy::ExternalCommand,
            CacheManagementMode::ToolManaged,
            CacheSizeSemantics::ConservativeLowerBound,
        ),
        (
            CleanStrategy::OwnerProvider,
            CacheManagementMode::ToolManaged,
            CacheSizeSemantics::ConservativeLowerBound,
        ),
        (
            CleanStrategy::LifecycleProvider,
            CacheManagementMode::ToolManaged,
            CacheSizeSemantics::ConservativeLowerBound,
        ),
        (
            CleanStrategy::DockerPrune,
            CacheManagementMode::ToolManaged,
            CacheSizeSemantics::ConservativeLowerBound,
        ),
    ] {
        signature.strategy = strategy;
        let metadata = signature.cache_metadata();
        assert_eq!(metadata.management_mode, management);
        assert_eq!(metadata.size_semantics, size);
    }
    assert_eq!(
        registry.get("dev.xcode.archives").unwrap().strategy,
        CleanStrategy::Manual
    );
    let text = r#"
[[signatures]]
id = "test.retired-knob"
name = "Retired knob"
category = "system"
risk = "safe"
strategy = "delete_contents"
paths = ["~/.cache/test"]
management_mode = "advisory"
"#;
    assert!(SignatureLoader::load_str(text)
        .unwrap_err()
        .to_string()
        .contains("unknown field"));
}

#[test]
fn model_weights_are_owned_by_model_inventory_and_old_settings_still_load() {
    let fixture = tempfile::tempdir().unwrap();
    let weights = fixture.path().join(".ollama/models/blobs/weights.bin");
    fs::create_dir_all(weights.parent().unwrap()).unwrap();
    fs::write(&weights, vec![2u8; 8192]).unwrap();
    let registry = SignatureRegistry::load_embedded_catalog().unwrap();
    assert!(registry.by_category(Category::Model).is_empty());
    let environment = PlatformEnvironment::simulated(PathFlavor::current())
        .with_home(fixture.path())
        .with_missing_tool("docker")
        .with_missing_tool("npm")
        .with_missing_tool("pnpm")
        .with_missing_tool("uv");
    let result = ScanEngine::scan(
        &registry,
        &LifecycleProviderRegistry::new(vec![]),
        &OwnerProviderRegistry::new(vec![]),
        None,
        &[],
        false,
        &environment,
        &NeverCancelled,
        |_| {},
    );
    assert!(result
        .categories
        .iter()
        .all(|category| category.category != Category::Model));
    assert!(result
        .categories
        .iter()
        .flat_map(|category| &category.items)
        .all(|item| !item.path.contains(".ollama")));
    assert!(weights.is_file());
    let settings: ZenithSettings = serde_json::from_str(
        r#"{"clean_local_models":true,"theme":"dark","clean_ai_tools":false}"#,
    )
    .unwrap();
    assert_eq!(settings.theme, "dark");
    assert!(!settings.clean_ai_tools);
    assert!(!settings.is_category_clean_enabled(Category::Model));
    assert!(serde_json::to_value(settings)
        .unwrap()
        .get("clean_local_models")
        .is_none());
}
