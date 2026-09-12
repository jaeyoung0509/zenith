use crate::models::{
    CacheArtifactKind, CacheManagementMode, CacheMetadata, CacheSizeSemantics, Category,
    ObservationQuality, RiskTier, ScanItem,
};
use crate::platform::path_algebra::{self, PathFlavor};
use crate::platform::PlatformEnvironment;
use crate::safety::{Blacklist, SymlinkGuard};
use crate::scanner::SizeCalculator;
use crate::signatures::SignatureRegistry;
use crate::tooling;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};
use sysinfo::{ProcessesToUpdate, System};

const PROVIDER_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_DISCOVERY_OUTPUT: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderKind {
    Uv,
    Pnpm,
    Npm,
}

impl ProviderKind {
    fn for_signature(id: &str) -> Option<Self> {
        match id {
            "dev.uv.cache" => Some(Self::Uv),
            "dev.pnpm.store" => Some(Self::Pnpm),
            "dev.npm.cache" => Some(Self::Npm),
            _ => None,
        }
    }

    fn executable(self) -> &'static str {
        match self {
            Self::Uv => "uv",
            Self::Pnpm => "pnpm",
            Self::Npm => "npm",
        }
    }

    fn discovery_args(self) -> &'static [&'static str] {
        match self {
            Self::Uv => &["cache", "dir"],
            Self::Pnpm => &["store", "path"],
            // Prints the single configured cache directory.
            Self::Npm => &["config", "get", "cache"],
        }
    }

    fn prune_args(self) -> &'static [&'static str] {
        match self {
            Self::Uv => &["cache", "prune"],
            Self::Pnpm => &["store", "prune"],
            Self::Npm => &["cache", "clean", "--force"],
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Uv => "uv Package Cache",
            Self::Pnpm => "pnpm Content-Addressable Store",
            Self::Npm => "npm Cache",
        }
    }

    fn consequence(self) -> &'static str {
        match self {
            Self::Uv => "Unused archives are pruned; future environments may re-download packages.",
            Self::Pnpm => {
                "Unreferenced packages are pruned; future installs may download them again."
            }
            Self::Npm => "A full cleanup can force package downloads on later installs.",
        }
    }
}

/// Backend-owned cache providers. The frontend can select only the ScanItem ID;
/// executable names, arguments, and cache paths are all rediscovered here.
pub struct CacheProviderRegistry;

impl CacheProviderRegistry {
    /// Scans every external cache provider through the described environment.
    /// The provider executables are resolved with the environment's stated tool
    /// facts and the discovered caches are approved against the environment's
    /// home, so a redirected or simulated environment cannot be answered by the
    /// runner's own profile.
    pub fn scan_items(
        registry: &SignatureRegistry,
        environment: &PlatformEnvironment,
    ) -> Vec<ScanItem> {
        // Three tiny provider lookups; route through the shared bounded scan
        // pool intentionally instead of the unbounded global Rayon pool.
        crate::execution_budget::install_shared(
            || {
                [ProviderKind::Uv, ProviderKind::Pnpm, ProviderKind::Npm]
                    .into_par_iter()
                    .filter_map(|provider| {
                        Self::scan_provider_logged(provider, registry, environment)
                    })
                    .collect()
            },
            || {
                [ProviderKind::Uv, ProviderKind::Pnpm, ProviderKind::Npm]
                    .into_iter()
                    .filter_map(|provider| {
                        Self::scan_provider_logged(provider, registry, environment)
                    })
                    .collect()
            },
        )
    }

    fn scan_provider_logged(
        provider: ProviderKind,
        registry: &SignatureRegistry,
        environment: &PlatformEnvironment,
    ) -> Option<ScanItem> {
        match Self::scan_provider(provider, registry, environment) {
            Ok(item) => item,
            Err(error) => {
                // A rejected provider must be visible in diagnostics instead of
                // silently disappearing from the scan results.
                crate::diagnostics::log_error(
                    "cache_providers",
                    &format!("{} scan skipped: {error}", provider.executable()),
                );
                None
            }
        }
    }

    fn scan_provider(
        provider: ProviderKind,
        registry: &SignatureRegistry,
        environment: &PlatformEnvironment,
    ) -> Result<Option<ScanItem>, String> {
        let signature_id = match provider {
            ProviderKind::Uv => "dev.uv.cache",
            ProviderKind::Pnpm => "dev.pnpm.store",
            ProviderKind::Npm => "dev.npm.cache",
        };
        let Some(signature) = registry.get(signature_id) else {
            return Ok(None);
        };
        if !signature.supports_current_platform() {
            return Ok(None);
        }
        let path = discover_path(provider, environment)?;
        let measurement = SizeCalculator::measure_path_full(&path, &[], environment);
        if measurement.size.reclaimable() == 0 && measurement.complete {
            return Ok(None);
        }
        let (quality, size_semantics) = if !measurement.complete {
            let q = if measurement.size.reclaimable() == 0 {
                ObservationQuality::Unavailable
            } else {
                ObservationQuality::Partial
            };
            (q, CacheSizeSemantics::ConservativeLowerBound)
        } else {
            (ObservationQuality::Fresh, CacheSizeSemantics::Informational)
        };
        let last_modified = std::fs::metadata(&path)
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs());
        Ok(Some(ScanItem {
            id: signature_id.to_string(),
            signature_id: signature_id.to_string(),
            name: provider.display_name().to_string(),
            category: Category::Developer,
            risk: RiskTier::Rebuild,
            path: path.to_string_lossy().into_owned(),
            size: measurement.size,
            file_count: measurement.file_count,
            description: "Inspected and pruned by the owning package manager.".to_string(),
            cache_metadata: CacheMetadata {
                provider: provider.executable().to_string(),
                management_mode: CacheManagementMode::ToolManaged,
                artifact_kind: CacheArtifactKind::PackageStore,
                consequence: provider.consequence().to_string(),
                size_semantics,
                last_used_confidence: Default::default(),
            },
            is_selected: false,
            last_modified,
            exists: true,
            quality,
            incomplete_reason: measurement.incomplete_reason,
            skipped_entry_count: measurement.skipped_entries,
        }))
    }

    /// Runs the provider's own prune command.
    ///
    /// The returned amount is `None` when either measurement was incomplete:
    /// the provider still pruned, and the caller reports the target as partial
    /// instead of showing a difference between two partial numbers as exact.
    pub fn prune(
        signature_id: &str,
        planned_path: &Path,
        environment: &PlatformEnvironment,
    ) -> Result<Option<u64>, String> {
        let provider = ProviderKind::for_signature(signature_id)
            .ok_or_else(|| "Unknown external cache provider".to_string())?;
        if matching_process_is_active(provider) {
            return Err(format!(
                "{} is currently running. Close it and try again.",
                provider.executable()
            ));
        }
        let fresh_path = discover_path(provider, environment)?;
        if !paths_match(&fresh_path, planned_path) {
            return Err(
                "The provider cache location changed since the scan. Scan again.".to_string(),
            );
        }
        let before = SizeCalculator::measure_path_logged(&fresh_path, &[], environment);
        let output = run_provider(provider, provider.prune_args(), environment)?;
        if !output.status.success() {
            return Err(format!(
                "{} prune failed: {}",
                provider.executable(),
                bounded_message(&output.stderr)
            ));
        }
        let rediscovered = discover_path(provider, environment)?;
        if !paths_match(&rediscovered, &fresh_path) {
            return Err("The provider cache location changed during cleanup.".to_string());
        }
        let after = SizeCalculator::measure_path_logged(&rediscovered, &[], environment);
        Ok(crate::scanner::size::reclaimed_between(&before, &after))
    }
}

pub fn mutation_blocked_by_active_runtime(signature_id: &str) -> bool {
    let protected: &[&str] = match signature_id {
        "ai.torchinductor.temp" => &["python", "python3", "python.exe", "vllm", "sglang"],
        "ai.llamacpp.opencl.windows" | "ai.llamacpp.opencl.macos" => &[
            "llama-cli",
            "llama-server",
            "llama-cli.exe",
            "llama-server.exe",
        ],
        id if id.starts_with("dev.cargo.") => &["cargo", "cargo.exe", "rustc", "rustc.exe"],
        "dev.rustup.downloads" => &["rustup", "rustup.exe"],
        id if id.starts_with("dev.go.") => &["go", "go.exe"],
        id if id.starts_with("dev.xcode.") => &["xcodebuild"],
        _ => return false,
    };
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system.processes().values().any(|process| {
        let name = process.name().to_string_lossy();
        protected
            .iter()
            .any(|expected| name.eq_ignore_ascii_case(expected))
    })
}

fn run_provider(
    provider: ProviderKind,
    args: &[&str],
    environment: &PlatformEnvironment,
) -> Result<std::process::Output, String> {
    let executable =
        tooling::resolve_with(provider.executable(), environment).ok_or_else(|| {
            format!(
                "{} was not detected in PATH or known tool locations",
                provider.executable()
            )
        })?;
    validate_executable(&executable, environment)?;
    let mut command = Command::new(executable);
    command.args(args);
    tooling::run_with_timeout(command, PROVIDER_TIMEOUT).map_err(|error| error.to_string())
}

fn discover_path(
    provider: ProviderKind,
    environment: &PlatformEnvironment,
) -> Result<PathBuf, String> {
    let output = run_provider(provider, provider.discovery_args(), environment)?;
    if !output.status.success() {
        return Err(format!(
            "{} cache discovery failed: {}",
            provider.executable(),
            bounded_message(&output.stderr)
        ));
    }
    parse_discovered_path(&output.stdout).and_then(|path| validate_cache_path(path, environment))
}

fn parse_discovered_path(output: &[u8]) -> Result<PathBuf, String> {
    if output.len() > MAX_DISCOVERY_OUTPUT {
        return Err("Cache discovery output exceeded the safety limit".to_string());
    }
    let text = std::str::from_utf8(output)
        .map_err(|_| "Cache discovery did not return UTF-8".to_string())?;
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() != 1 || lines[0].chars().any(char::is_control) {
        return Err("Cache discovery returned an ambiguous path".to_string());
    }
    let path = PathBuf::from(lines[0]);
    if !path.is_absolute() {
        return Err("Cache discovery returned a relative path".to_string());
    }
    Ok(path)
}

fn validate_cache_path(
    path: PathBuf,
    environment: &PlatformEnvironment,
) -> Result<PathBuf, String> {
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|_| "The discovered cache directory is unavailable".to_string())?;
    if !metadata.is_dir() || SymlinkGuard::is_symlink(&path) {
        return Err("The discovered cache must be a real directory".to_string());
    }
    // The stated profile is the authority: the runner's own home must not
    // decide whether a cache belongs to this user.
    let home = environment
        .user_home()
        .ok_or_else(|| "Could not resolve the current user profile".to_string())?;
    let canonical_home = std::fs::canonicalize(home)
        .map_err(|_| "Could not validate the current user profile".to_string())?;
    let canonical = std::fs::canonicalize(&path)
        .map_err(|_| "Could not canonicalize the discovered cache".to_string())?;

    let flavor = environment.flavor();
    let in_profile = path_is_within(&canonical, &canonical_home, flavor);
    let mut approved = in_profile && cache_location_approved(&canonical, &canonical_home, flavor);
    // Relocated caches (PNPM_HOME, UV_CACHE_DIR, a configured npm cache) stay
    // supported under the same blacklist and symlink validation as in-profile
    // locations instead of being refused for being outside the profile.
    for root in relocated_cache_roots(environment) {
        approved |=
            path_is_same(&canonical, &root, flavor) || path_is_within(&canonical, &root, flavor);
    }

    if !approved {
        return Err(
            "The provider cache override is outside approved user cache locations".to_string(),
        );
    }
    Blacklist::validate_with(&canonical, environment).map_err(|error| error.to_string())?;
    if in_profile {
        SymlinkGuard::validate_no_symlink_ancestors(&canonical, &canonical_home, environment)
            .map_err(|error| error.to_string())?;
    } else {
        SymlinkGuard::validate_anchored_path(&canonical, environment)
            .map_err(|error| error.to_string())?;
    }
    Ok(canonical)
}

fn relocated_cache_roots(environment: &PlatformEnvironment) -> Vec<PathBuf> {
    // The application-data containers come from the described environment;
    // the per-tool overrides (`UV_CACHE_DIR`, `PNPM_HOME`, `NPM_CONFIG_CACHE`)
    // are process environment variables with no environment representation, so
    // they stay host-derived like the tool search itself.
    let stated = [environment.local_app_data(), environment.roaming_app_data()];
    let overrides = ["UV_CACHE_DIR", "PNPM_HOME", "NPM_CONFIG_CACHE"]
        .into_iter()
        .filter_map(|variable| std::env::var_os(variable).map(PathBuf::from));
    stated
        .into_iter()
        .flatten()
        .chain(overrides)
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .map(|root| crate::platform::NativePlatformPaths::normalize_verbatim_path(&root))
        .collect()
}

/// Same location under the environment's own flavor: case folded and
/// separator-agnostic on Windows, byte exact on POSIX. Using the flavor rather
/// than the host keeps a simulated Windows environment comparable on any
/// runner.
fn path_is_same(left: &Path, right: &Path, flavor: PathFlavor) -> bool {
    path_algebra::equal(&left.to_string_lossy(), &right.to_string_lossy(), flavor)
}

fn path_is_within(path: &Path, base: &Path, flavor: PathFlavor) -> bool {
    !path_is_same(path, base, flavor)
        && path_algebra::contains(&base.to_string_lossy(), &path.to_string_lossy(), flavor)
}

/// Pure approval check over already-canonicalized paths (no filesystem access),
/// so the trust boundary is directly unit-testable. Broad cache roots match
/// strict subdirectories; specific store roots (including the npm home
/// directory) match themselves and their contents.
fn cache_location_approved(canonical: &Path, canonical_home: &Path, flavor: PathFlavor) -> bool {
    let broad_cache_roots = [
        canonical_home.join(".cache"),
        canonical_home.join("Library/Caches"),
    ];
    let specific_store_roots = [
        canonical_home.join(".local/share/pnpm"),
        canonical_home.join("Library/pnpm"),
        canonical_home.join(".pnpm-store"),
        canonical_home.join(".npm"),
    ];
    broad_cache_roots.iter().any(|root| {
        !path_is_same(canonical, root, flavor) && path_is_within(canonical, root, flavor)
    }) || specific_store_roots.iter().any(|root| {
        path_is_same(canonical, root, flavor) || path_is_within(canonical, root, flavor)
    })
}

/// Home-relative install locations of version-manager-owned toolchains
/// (nvm, Volta, asdf, fnm). Narrow to each manager's own directory; the
/// caller still requires the canonical executable to live under one of these
/// roots, and execution stays limited to fixed provider arguments.
fn node_manager_roots(home: &Path) -> Vec<PathBuf> {
    [".nvm", ".volta", ".asdf", ".fnm"]
        .iter()
        .map(|dir| home.join(dir))
        .collect()
}

fn validate_executable(path: &Path, environment: &PlatformEnvironment) -> Result<(), String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|_| "Could not validate the provider executable".to_string())?;
    let canonical = crate::platform::NativePlatformPaths::normalize_verbatim_path(&canonical);
    let mut roots = vec![
        PathBuf::from("/usr/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/homebrew"),
    ];
    roots.extend(crate::platform::NativePlatformPaths::trusted_tool_roots(
        environment.user_home().as_deref(),
    ));
    if let Some(home) = environment.user_home() {
        roots.extend([
            home.join(".local/bin"),
            home.join(".local/share/uv"),
            home.join(".local/share/pnpm"),
            home.join(".cargo/bin"),
            home.join(".npm-global/bin"),
            home.join("Library/pnpm"),
        ]);
        roots.extend(node_manager_roots(&home));
    }

    if executable_under_roots(&canonical, &roots) {
        Ok(())
    } else {
        Err("The provider executable is outside trusted install locations".to_string())
    }
}

/// Pure matcher over an already-canonicalized executable and its trust roots.
/// Kept separate so the Windows trust boundary can be tested without touching
/// the real filesystem. Windows-style paths always use the Windows comparison
/// so the fixtures remain meaningful on Unix hosts.
fn executable_under_roots(canonical: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| {
        let norm_root = crate::platform::NativePlatformPaths::normalize_verbatim_path(root);
        if looks_like_windows_path(canonical) || looks_like_windows_path(&norm_root) {
            crate::platform::NativePlatformPaths::windows_path_starts_with(canonical, &norm_root)
        } else {
            canonical.starts_with(&norm_root)
        }
    })
}

fn looks_like_windows_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.starts_with("\\\\")
        || text.starts_with("//")
        || matches!(
            text.as_bytes(),
            [drive, b':', ..] if drive.is_ascii_alphabetic()
        )
}

fn paths_match(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn matching_process_is_active(provider: ProviderKind) -> bool {
    let expected = provider.executable();
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system.processes().values().any(|process| {
        let name = process.name().to_string_lossy();
        name.eq_ignore_ascii_case(expected)
            || name.eq_ignore_ascii_case(&format!("{expected}.exe"))
            || process.cmd().iter().take(3).any(|argument| {
                Path::new(argument)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| {
                        value.eq_ignore_ascii_case(expected)
                            || value.starts_with(&format!("{expected}."))
                    })
            })
    })
}

fn bounded_message(bytes: &[u8]) -> String {
    crate::diagnostics::sanitize_log(
        String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).trim(),
    )
}

#[cfg(test)]
mod tests {
    use super::ProviderKind;
    use super::{cache_location_approved, node_manager_roots, parse_discovered_path};
    use crate::platform::path_algebra::PathFlavor;
    use crate::platform::paths::SimulatedPaths;
    use crate::platform::PlatformEnvironment;
    use std::path::PathBuf;
    use std::sync::Arc;

    #[test]
    fn npm_provider_is_wired_to_its_signature_and_commands() {
        assert_eq!(
            ProviderKind::for_signature("dev.npm.cache"),
            Some(ProviderKind::Npm)
        );
        assert_eq!(ProviderKind::Npm.executable(), "npm");
        assert_eq!(
            ProviderKind::Npm.discovery_args(),
            &["config", "get", "cache"]
        );
        assert_eq!(
            ProviderKind::Npm.prune_args(),
            &["cache", "clean", "--force"]
        );
        assert_eq!(ProviderKind::for_signature("dev.yarn.cache"), None);
    }

    #[test]
    fn npm_home_cache_is_an_approved_location() {
        let home = if cfg!(target_os = "windows") {
            PathBuf::from("C:\\Users\\tester")
        } else {
            PathBuf::from("/Users/tester")
        };
        // The default npm cache directory and the npm home itself.
        let flavor = if cfg!(target_os = "windows") {
            PathFlavor::Windows
        } else {
            PathFlavor::Posix
        };
        assert!(cache_location_approved(
            &home.join(".npm/_cacache"),
            &home,
            flavor
        ));
        assert!(cache_location_approved(&home.join(".npm"), &home, flavor));
        // Broad roots still match strict subdirectories only.
        assert!(cache_location_approved(
            &home.join(".cache/npm/_cacache"),
            &home,
            flavor
        ));
        // Unrelated home children and the home root itself stay rejected.
        assert!(!cache_location_approved(
            &home.join("random-override"),
            &home,
            flavor
        ));
        assert!(!cache_location_approved(&home, &home, flavor));
    }

    #[test]
    fn approved_cache_roots_follow_the_stated_profile() {
        // A cache on a stated non-system drive is approved against that stated
        // home, while a cache under the runner's literal profile is not: the
        // provider's approval boundary is the environment's, not the host's.
        let environment = PlatformEnvironment::simulated(PathFlavor::Windows).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::Windows)
                .with_home(r"D:\Cache"),
        ));
        let stated_home = environment.user_home().unwrap();

        assert!(cache_location_approved(
            &PathBuf::from(r"D:\Cache\.npm\_cacache"),
            &stated_home,
            PathFlavor::Windows
        ));
        assert!(cache_location_approved(
            &PathBuf::from(r"D:\Cache\Library\Caches\uv"),
            &stated_home,
            PathFlavor::Windows
        ));
        // The literal profile spelling of another account is not the profile.
        assert!(!cache_location_approved(
            &PathBuf::from(r"C:\Users\other\.npm\_cacache"),
            &stated_home,
            PathFlavor::Windows
        ));

        // Relocated roots come from the stated application-data container, and
        // relocation is only trusted once the root resolves.
        let local_app_data = tempfile::tempdir().unwrap();
        let environment = environment.with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::Windows)
                .with_home(r"D:\Cache")
                .with_local_app_data(local_app_data.path()),
        ));
        let relocated = super::relocated_cache_roots(&environment);
        // `canonicalize` returns a verbatim path on Windows; the relocation
        // decision is made on the normalized spelling, so the expectation is
        // normalized the same way instead of encoding the host's prefix.
        let expected = crate::platform::NativePlatformPaths::normalize_verbatim_path(
            &local_app_data.path().canonicalize().unwrap(),
        );
        assert_eq!(
            relocated,
            vec![expected],
            "the stated application-data container is the only relocated root"
        );
        // An approved relocation must not be refused by the blacklist: a
        // redirected application-data root that protects its whole tree would
        // make every cache under it cleanable in one place and forbidden in
        // the other.
        for root in &relocated {
            assert!(
                crate::safety::Blacklist::validate_with(root, &environment).is_ok(),
                "the approved relocated root {} must stay cleanable",
                root.display()
            );
        }
        assert!(
            crate::safety::Blacklist::validate_with(local_app_data.path(), &environment).is_err(),
            "the relocated application-data root itself stays protected"
        );
    }

    #[test]
    fn validate_cache_path_approves_the_stated_home_and_refuses_elsewhere() {
        let stated_home = tempfile::tempdir().unwrap();
        let cache = stated_home.path().join("Library/Caches/npm/_cacache");
        std::fs::create_dir_all(&cache).unwrap();

        let environment =
            PlatformEnvironment::simulated(PathFlavor::current()).with_roots(Arc::new(
                SimulatedPaths::new()
                    .with_flavor(PathFlavor::current())
                    .with_home(stated_home.path()),
            ));
        assert!(super::validate_cache_path(cache.clone(), &environment).is_ok());

        // An in-profile directory that is not an approved cache root is refused
        // even though it exists, so the approval step still does work.
        let unapproved = stated_home.path().join("random-override");
        std::fs::create_dir_all(&unapproved).unwrap();
        assert!(super::validate_cache_path(unapproved, &environment).is_err());
    }

    #[test]
    fn a_provider_with_a_stated_missing_tool_is_skipped() {
        use crate::signatures::SignatureRegistry;

        // npm/pnpm/uv may be installed on this host; the environment states
        // they are absent, and a stated absence is never re-discovered.
        let environment = PlatformEnvironment::simulated(PathFlavor::current())
            .with_missing_tool("npm")
            .with_missing_tool("pnpm")
            .with_missing_tool("uv");
        let registry = SignatureRegistry::load_embedded().unwrap();
        assert!(
            super::CacheProviderRegistry::scan_items(&registry, &environment).is_empty(),
            "a stated missing tool must not be re-discovered from the host"
        );
    }

    #[test]
    fn provider_executable_validation_uses_trusted_tool_roots() {
        #[cfg(target_os = "macos")]
        {
            use super::validate_executable;
            use std::path::Path;
            let environment = PlatformEnvironment::native();
            assert!(validate_executable(Path::new("/usr/bin/env"), &environment).is_ok());
            let dir = tempfile::tempdir().unwrap();
            let stray = dir.path().join("npm");
            std::fs::write(&stray, b"#!/bin/sh\n").unwrap();
            assert!(validate_executable(&stray, &environment).is_err());
        }
    }

    #[test]
    fn trusted_paths_require_a_documented_tool_root() {
        use super::executable_under_roots;
        use std::path::{Path, PathBuf};

        // Windows-style fixtures stay lexically comparable on Unix hosts.
        let roots: Vec<PathBuf> = vec![
            PathBuf::from(r"C:\Users\tester\AppData\Roaming\npm"),
            PathBuf::from(r"C:\Users\tester\AppData\Local\pnpm"),
            PathBuf::from(r"C:\Program Files\nodejs"),
        ];
        assert!(executable_under_roots(
            Path::new(r"C:\Users\tester\AppData\Roaming\npm\npm.cmd"),
            &roots
        ));
        assert!(executable_under_roots(
            Path::new(r"C:\Users\tester\AppData\Local\pnpm\pnpm.cmd"),
            &roots
        ));
        // A random directory under a user-writable container is not trusted.
        assert!(!executable_under_roots(
            Path::new(r"C:\Users\tester\AppData\Local\random\npm.cmd"),
            &roots
        ));
        assert!(!executable_under_roots(
            Path::new(r"C:\Users\tester\AppData\Roaming\random\npm.cmd"),
            &roots
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn trusted_tool_roots_exclude_bare_user_writable_containers() {
        use crate::platform::NativePlatformPaths;

        let roots = NativePlatformPaths::trusted_tool_roots(
            PlatformEnvironment::native().user_home().as_deref(),
        );

        // A bare user-writable container is never a trusted executable root:
        // trust comes from a named install directory below it, so no root may be
        // the container itself.
        for root in &roots {
            let text = root
                .to_string_lossy()
                .replace('/', "\\")
                .to_ascii_lowercase();
            let bare = [
                r"\appdata",
                r"\appdata\local",
                r"\appdata\roaming",
                r"\programdata",
            ]
            .iter()
            .any(|tail| text.ends_with(tail));
            assert!(!bare, "bare user-writable container is trusted: {root:?}");
        }

        // The documented %APPDATA%\npm install root stays trusted.
        let app_data = crate::platform::PlatformEnvironment::native()
            .roaming_app_data()
            .expect("a Windows session exposes the roaming application data root");
        let npm = app_data.join("npm");
        assert!(
            roots
                .iter()
                .any(|candidate| NativePlatformPaths::windows_path_eq(candidate, &npm)),
            "the documented %APPDATA%\\npm install root must stay trusted"
        );
    }

    #[test]
    fn node_manager_roots_stay_inside_their_own_directories() {
        let home = PathBuf::from("/Users/tester");
        let roots = node_manager_roots(&home);
        assert!(roots.contains(&home.join(".nvm")));
        assert!(roots.contains(&home.join(".volta")));
        for root in &roots {
            assert!(root.starts_with(&home));
        }
    }

    #[test]
    fn discovery_requires_one_absolute_utf8_path() {
        #[cfg(target_os = "windows")]
        let absolute = "C:\\Users\\테스트 사용자\\AppData\\Local\\uv\\cache\r\n";
        #[cfg(not(target_os = "windows"))]
        let absolute = "/Users/테스트 사용자/Library/Caches/uv\n";

        assert!(parse_discovered_path(absolute.as_bytes()).is_ok());
        assert!(parse_discovered_path(b"relative/cache\n").is_err());
        assert!(parse_discovered_path(b"/one\n/two\n").is_err());
        assert!(parse_discovered_path(&[0xff]).is_err());
    }
}
