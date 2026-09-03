use crate::models::{
    CacheArtifactKind, CacheManagementMode, CacheMetadata, CacheSizeSemantics, Category, RiskTier,
    ScanItem,
};
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
}

impl ProviderKind {
    fn for_signature(id: &str) -> Option<Self> {
        match id {
            "dev.uv.cache" => Some(Self::Uv),
            "dev.pnpm.store" => Some(Self::Pnpm),
            _ => None,
        }
    }

    fn executable(self) -> &'static str {
        match self {
            Self::Uv => "uv",
            Self::Pnpm => "pnpm",
        }
    }

    fn discovery_args(self) -> &'static [&'static str] {
        match self {
            Self::Uv => &["cache", "dir"],
            Self::Pnpm => &["store", "path"],
        }
    }

    fn prune_args(self) -> &'static [&'static str] {
        match self {
            Self::Uv => &["cache", "prune"],
            Self::Pnpm => &["store", "prune"],
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Uv => "uv Package Cache",
            Self::Pnpm => "pnpm Content-Addressable Store",
        }
    }

    fn consequence(self) -> &'static str {
        match self {
            Self::Uv => "Unused archives are pruned; future environments may re-download packages.",
            Self::Pnpm => {
                "Unreferenced packages are pruned; future installs may download them again."
            }
        }
    }
}

/// Backend-owned cache providers. The frontend can select only the ScanItem ID;
/// executable names, arguments, and cache paths are all rediscovered here.
pub struct CacheProviderRegistry;

impl CacheProviderRegistry {
    pub fn scan_items(registry: &SignatureRegistry) -> Vec<ScanItem> {
        [ProviderKind::Uv, ProviderKind::Pnpm]
            .into_par_iter()
            .filter_map(|provider| Self::scan_provider(provider, registry).ok().flatten())
            .collect()
    }

    fn scan_provider(
        provider: ProviderKind,
        registry: &SignatureRegistry,
    ) -> Result<Option<ScanItem>, String> {
        let signature_id = match provider {
            ProviderKind::Uv => "dev.uv.cache",
            ProviderKind::Pnpm => "dev.pnpm.store",
        };
        let Some(signature) = registry.get(signature_id) else {
            return Ok(None);
        };
        if !signature.supports_current_platform() {
            return Ok(None);
        }
        let path = discover_path(provider)?;
        let (size, file_count) = SizeCalculator::measure_path(&path, &[]);
        if size.reclaimable() == 0 {
            return Ok(None);
        }
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
            size,
            file_count,
            description: "Inspected and pruned by the owning package manager.".to_string(),
            cache_metadata: CacheMetadata {
                provider: provider.executable().to_string(),
                management_mode: CacheManagementMode::ToolManaged,
                artifact_kind: CacheArtifactKind::PackageStore,
                consequence: provider.consequence().to_string(),
                size_semantics: CacheSizeSemantics::Informational,
                last_used_confidence: Default::default(),
            },
            is_selected: false,
            last_modified,
            exists: true,
        }))
    }

    pub fn prune(signature_id: &str, planned_path: &Path) -> Result<u64, String> {
        let provider = ProviderKind::for_signature(signature_id)
            .ok_or_else(|| "Unknown external cache provider".to_string())?;
        if matching_process_is_active(provider) {
            return Err(format!(
                "{} is currently running. Close it and try again.",
                provider.executable()
            ));
        }
        let fresh_path = discover_path(provider)?;
        if !paths_match(&fresh_path, planned_path) {
            return Err(
                "The provider cache location changed since the scan. Scan again.".to_string(),
            );
        }
        let before = SizeCalculator::measure_path(&fresh_path, &[])
            .0
            .reclaimable();
        let output = run_provider(provider, provider.prune_args())?;
        if !output.status.success() {
            return Err(format!(
                "{} prune failed: {}",
                provider.executable(),
                bounded_message(&output.stderr)
            ));
        }
        let rediscovered = discover_path(provider)?;
        if !paths_match(&rediscovered, &fresh_path) {
            return Err("The provider cache location changed during cleanup.".to_string());
        }
        let after = SizeCalculator::measure_path(&rediscovered, &[])
            .0
            .reclaimable();
        Ok(before.saturating_sub(after))
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

fn run_provider(provider: ProviderKind, args: &[&str]) -> Result<std::process::Output, String> {
    let executable = tooling::resolve(provider.executable())
        .ok_or_else(|| format!("{} is not installed", provider.executable()))?;
    validate_executable(&executable)?;
    let mut command = Command::new(executable);
    command.args(args);
    tooling::run_with_timeout(command, PROVIDER_TIMEOUT).map_err(|error| error.to_string())
}

fn discover_path(provider: ProviderKind) -> Result<PathBuf, String> {
    let output = run_provider(provider, provider.discovery_args())?;
    if !output.status.success() {
        return Err(format!(
            "{} cache discovery failed: {}",
            provider.executable(),
            bounded_message(&output.stderr)
        ));
    }
    parse_discovered_path(&output.stdout).and_then(validate_cache_path)
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

fn validate_cache_path(path: PathBuf) -> Result<PathBuf, String> {
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|_| "The discovered cache directory is unavailable".to_string())?;
    if !metadata.is_dir() || SymlinkGuard::is_symlink(&path) {
        return Err("The discovered cache must be a real directory".to_string());
    }
    let home = crate::platform::paths::NativePlatformPaths::new()
        .home()
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .ok_or_else(|| "Could not resolve the current user profile".to_string())?;
    let canonical_home = std::fs::canonicalize(home)
        .map_err(|_| "Could not validate the current user profile".to_string())?;
    let canonical = std::fs::canonicalize(&path)
        .map_err(|_| "Could not canonicalize the discovered cache".to_string())?;
    if canonical == canonical_home || !canonical.starts_with(&canonical_home) {
        return Err("The discovered cache is outside the current user profile".to_string());
    }
    let broad_cache_roots = [
        canonical_home.join(".cache"),
        canonical_home.join("Library/Caches"),
    ];
    let specific_store_roots = [
        canonical_home.join(".local/share/pnpm"),
        canonical_home.join("Library/pnpm"),
        canonical_home.join(".pnpm-store"),
    ];
    let mut approved = broad_cache_roots
        .iter()
        .any(|root| canonical != *root && canonical.starts_with(root))
        || specific_store_roots
            .iter()
            .any(|root| canonical == *root || canonical.starts_with(root));
    for variable in ["LOCALAPPDATA", "APPDATA"] {
        if let Some(root) = std::env::var_os(variable)
            .map(PathBuf::from)
            .and_then(|root| std::fs::canonicalize(root).ok())
        {
            approved |= canonical != root && canonical.starts_with(root);
        }
    }
    if !approved {
        return Err(
            "The provider cache override is outside approved user cache locations".to_string(),
        );
    }
    Blacklist::validate(&canonical).map_err(|error| error.to_string())?;
    SymlinkGuard::validate_no_symlink_ancestors(&canonical, &canonical_home)
        .map_err(|error| error.to_string())?;
    Ok(canonical)
}

fn validate_executable(path: &Path) -> Result<(), String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|_| "Could not validate the provider executable".to_string())?;
    let mut roots = vec![
        PathBuf::from("/usr/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/homebrew"),
    ];
    if let Some(home) = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
    {
        roots.extend([
            home.join(".local/bin"),
            home.join(".local/share/uv"),
            home.join(".local/share/pnpm"),
            home.join(".cargo/bin"),
            home.join(".npm-global/bin"),
            home.join("Library/pnpm"),
        ]);
    }
    for variable in ["LOCALAPPDATA", "APPDATA"] {
        if let Some(root) = std::env::var_os(variable).map(PathBuf::from) {
            roots.extend([root.join("Programs"), root.join("npm"), root.join("pnpm")]);
        }
    }
    if let Some(program_files) = std::env::var_os("ProgramFiles").map(PathBuf::from) {
        roots.push(program_files);
    }
    if roots.iter().any(|root| canonical.starts_with(root)) {
        Ok(())
    } else {
        Err("The provider executable is outside trusted install locations".to_string())
    }
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
    String::from_utf8_lossy(&bytes[..bytes.len().min(1024)])
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::parse_discovered_path;

    #[test]
    fn discovery_requires_one_absolute_utf8_path() {
        assert!(parse_discovered_path(b"/Users/test/Library/Caches/uv\n").is_ok());
        assert!(parse_discovered_path(b"relative/cache\n").is_err());
        assert!(parse_discovered_path(b"/one\n/two\n").is_err());
        assert!(parse_discovered_path(&[0xff]).is_err());
    }
}
