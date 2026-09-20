//! Context-aware protection rules for owner-managed Cargo cache units.
//!
//! Cargo registry sources are rebuildable package artifacts. Their extracted
//! trees legitimately contain names (`Cargo.lock`, `Cargo.toml`, scripts, and
//! executables) that the generic structured-state classifier protects in user
//! data. The exception is therefore scoped to the catalog signature and the
//! environment-resolved registry root; it is never a basename-wide exception.

use std::path::{Path, PathBuf};

use crate::models::{EntryKind, Signature};
use zenith_platform::PlatformEnvironment;

pub const REGISTRY_SOURCE_SIGNATURE_ID: &str = "dev.cargo.registry.src";

/// Returns the trusted Cargo registry source root for the stated environment.
pub fn registry_source_root(environment: &PlatformEnvironment) -> Option<PathBuf> {
    environment
        .user_home()
        .map(|home| home.join(".cargo/registry/src"))
}

/// Whether a filesystem cleanup target is authorized to treat structured names
/// as rebuildable package contents.
///
/// The caller must have already re-derived the signature's authorized roots.
/// Requiring the concrete target root to be the environment-resolved Cargo
/// registry root keeps a forged signature id or a path-prefix lookalike from
/// widening the exception.
pub fn allows_registry_source_contents(
    signature: &Signature,
    target: &Path,
    authorized_roots: &[PathBuf],
    environment: &PlatformEnvironment,
) -> bool {
    if signature.id != REGISTRY_SOURCE_SIGNATURE_ID {
        return false;
    }
    let Some(trusted_root) = registry_source_root(environment) else {
        return false;
    };
    authorized_roots
        .iter()
        .any(|root| root == &trusted_root && target == root)
}

/// Runtime counterpart of [`allows_registry_source_contents`]. The opaque
/// target carries the unit root but not the catalog registry, so execution
/// repeats the exact environment/root check before recursive mutation.
pub fn target_allows_registry_source_contents(
    signature_id: &str,
    target: &Path,
    unit_root: &Path,
    environment: &PlatformEnvironment,
) -> bool {
    if signature_id != REGISTRY_SOURCE_SIGNATURE_ID || target != unit_root {
        return false;
    }
    registry_source_root(environment).is_some_and(|root| unit_root == root)
}

/// Returns whether a structured entry is package metadata that Cargo itself
/// regenerates with the registry source tree.
///
/// The exception is intentionally not a blanket "ignore structured state"
/// switch: databases, credentials, configuration unrelated to Cargo, and
/// executable files inside a crate remain protected by the generic guard.
pub fn allows_registry_source_entry(
    path: &Path,
    entry_kind: EntryKind,
    allow_cargo_registry_contents: bool,
) -> bool {
    if !allow_cargo_registry_contents || entry_kind != EntryKind::File {
        return false;
    }
    path.file_name().is_some_and(|name| {
        name.to_string_lossy().eq_ignore_ascii_case("Cargo.lock")
            || name.to_string_lossy().eq_ignore_ascii_case("Cargo.toml")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Category, CleanStrategy, PlatformKind, RiskTier};
    use std::sync::Arc;
    use tempfile::tempdir;
    use zenith_platform::path_algebra::PathFlavor;
    use zenith_platform::paths::SimulatedPaths;

    fn signature(id: &str) -> Signature {
        Signature {
            id: id.into(),
            name: "Cargo source".into(),
            category: Category::Developer,
            risk: RiskTier::Rebuild,
            strategy: CleanStrategy::DeleteContents,
            paths: vec![],
            exclusions: vec![],
            description: String::new(),
            min_age_days: None,
            include_prefixes: vec![],
            exclude_prefixes: vec![],
            intensive_only: false,
            platforms: vec![PlatformKind::Macos],
            provider: "Cargo".into(),
            provider_id: None,
            management_mode: Default::default(),
            artifact_kind: Default::default(),
            consequence: String::new(),
            reclaimable_is_lower_bound: false,
            discovery: Default::default(),
            unit: None,
            owner: "Cargo".into(),
            priority: 0,
            fail_if_running: vec![],
        }
    }

    fn environment(home: &Path) -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::Posix).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::Posix)
                .with_home(home),
        ))
    }

    #[test]
    fn only_the_registered_cargo_registry_root_gets_the_exception() {
        let dir = tempdir().unwrap();
        let environment = environment(dir.path());
        let root = dir.path().join(".cargo/registry/src");
        let signature = signature(REGISTRY_SOURCE_SIGNATURE_ID);

        assert!(allows_registry_source_contents(
            &signature,
            &root,
            std::slice::from_ref(&root),
            &environment
        ));
        assert!(!allows_registry_source_contents(
            &signature,
            &root.join("index.crates.io-1949cf8c6b5b557f"),
            std::slice::from_ref(&root),
            &environment
        ));
        assert!(!allows_registry_source_contents(
            &signature,
            &dir.path().join("project"),
            std::slice::from_ref(&dir.path().join("project")),
            &environment
        ));
        assert!(!target_allows_registry_source_contents(
            REGISTRY_SOURCE_SIGNATURE_ID,
            &dir.path().join("project"),
            &dir.path().join("project"),
            &environment
        ));
    }

    #[test]
    fn cargo_metadata_exception_only_covers_regular_metadata_files() {
        let dir = tempdir().unwrap();
        let environment = environment(dir.path());
        let project = dir.path().join("project");
        let signature = signature("test.generic");
        assert!(!allows_registry_source_contents(
            &signature,
            &project,
            std::slice::from_ref(&project),
            &environment
        ));
        assert!(allows_registry_source_entry(
            Path::new("/home/me/.cargo/registry/src/pkg/Cargo.lock"),
            EntryKind::File,
            true
        ));
        assert!(allows_registry_source_entry(
            Path::new("/home/me/.cargo/registry/src/pkg/Cargo.toml"),
            EntryKind::File,
            true
        ));
        assert!(!allows_registry_source_entry(
            Path::new("/home/me/.cargo/registry/src/pkg/build.sh"),
            EntryKind::File,
            true
        ));
        assert!(!allows_registry_source_entry(
            Path::new("/home/me/.cargo/registry/src/pkg/Cargo.lock"),
            EntryKind::Other,
            true
        ));
    }
}
