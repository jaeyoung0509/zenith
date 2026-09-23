mod catalog;

use crate::models::{
    derive_cleanup_disposition, AbsolutePath, CacheManagementMode, CacheMetadata,
    CacheSizeSemantics, CancellationProbe, CanonicalPath, Category, CleanupEligibility,
    CleanupUnit, CleanupUnitKind, DispositionFacts, EligibilityGate, EntryKind, ObservationQuality,
    RiskTier, ScanGapKind, ScanItem,
};
use crate::safety::{Blacklist, SymlinkGuard};
use crate::scanner::{
    PathMeasurement, RootProgressSink, ScanLimits, SizeCalculator, TraversalCounters,
};
use crate::signatures::SignatureRegistry;
use crate::tooling;
use catalog::{DiscoveryOutput, ProviderKind};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};
use sysinfo::{ProcessesToUpdate, System};
use zenith_platform::path_algebra::{self, PathFlavor};
use zenith_platform::PlatformEnvironment;

const PROVIDER_TIMEOUT: Duration = Duration::from_secs(15);
/// uv coordinates cache access through its own lock. Its documented lock wait
/// is five minutes, so permit that wait while retaining the same hard deadline.
const UV_PRUNE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MAX_DISCOVERY_OUTPUT: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CacheProviderPruneFailure {
    OwnerRunning(String),
    OwnerStateUnknown(String),
    Failed(String),
}

impl std::fmt::Display for CacheProviderPruneFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OwnerRunning(executable) => {
                write!(formatter, "{executable} is currently running")
            }
            Self::OwnerStateUnknown(executable) => {
                write!(
                    formatter,
                    "{executable} process state could not be verified"
                )
            }
            Self::Failed(reason) => formatter.write_str(reason),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CacheProviderFailure {
    pub kind: ScanGapKind,
    pub reason: String,
}

impl CacheProviderFailure {
    fn new(kind: ScanGapKind, reason: impl Into<String>) -> Self {
        Self {
            kind,
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct CacheProviderScan {
    pub items: Vec<ScanItem>,
    pub failures: Vec<CacheProviderFailure>,
    pub cancelled: bool,
}

pub(crate) trait CacheProviderScanner: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    fn scan_items(
        &self,
        registry: &SignatureRegistry,
        excluded_signatures: &[String],
        environment: &PlatformEnvironment,
        cancellation: &dyn CancellationProbe,
        limits: ScanLimits,
        counters: &TraversalCounters,
        progress: &dyn RootProgressSink,
    ) -> CacheProviderScan;
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProviderCommandResult {
    Absent,
    Output {
        success: bool,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    Failed(String),
    Cancelled,
}

trait ProviderCommandRunner: Send + Sync {
    fn discover(
        &self,
        provider: ProviderKind,
        environment: &PlatformEnvironment,
        cancellation: &dyn CancellationProbe,
    ) -> ProviderCommandResult;

    fn prune(
        &self,
        provider: ProviderKind,
        environment: &PlatformEnvironment,
    ) -> ProviderCommandResult;
}

trait ProviderMeasurer: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    fn measure(
        &self,
        path: &Path,
        environment: &PlatformEnvironment,
        cancellation: &dyn CancellationProbe,
        limits: ScanLimits,
        counters: &TraversalCounters,
    ) -> PathMeasurement;

    fn measure_for_prune(&self, path: &Path, environment: &PlatformEnvironment) -> PathMeasurement {
        let counters = TraversalCounters::default();
        self.measure(
            path,
            environment,
            &crate::models::NeverCancelled,
            ScanLimits::default(),
            &counters,
        )
    }
}

struct NativeProviderCommandRunner;

impl ProviderCommandRunner for NativeProviderCommandRunner {
    fn discover(
        &self,
        provider: ProviderKind,
        environment: &PlatformEnvironment,
        cancellation: &dyn CancellationProbe,
    ) -> ProviderCommandResult {
        let Some(executable) = tooling::resolve_with(provider.executable(), environment) else {
            return ProviderCommandResult::Absent;
        };
        if let Err(error) = validate_executable(&executable, environment) {
            return ProviderCommandResult::Failed(error);
        }
        let mut command = Command::new(executable);
        command.args(provider.discovery_args());
        if let Err(error) = configure_provider_command(provider, environment, &mut command) {
            return ProviderCommandResult::Failed(error);
        }
        match zenith_platform::subprocess::run_with_timeout_cancellable(
            command,
            PROVIDER_TIMEOUT,
            &|| cancellation.is_cancelled(),
        ) {
            Ok(output) => ProviderCommandResult::Output {
                success: output.status.success(),
                stdout: output.stdout,
                stderr: output.stderr,
            },
            Err(zenith_platform::SubprocessError::Cancelled(_)) => ProviderCommandResult::Cancelled,
            Err(error) => ProviderCommandResult::Failed(error.to_string()),
        }
    }

    fn prune(
        &self,
        provider: ProviderKind,
        environment: &PlatformEnvironment,
    ) -> ProviderCommandResult {
        let timeout = if provider == ProviderKind::Uv {
            UV_PRUNE_TIMEOUT
        } else {
            PROVIDER_TIMEOUT
        };
        match run_provider_with_timeout(provider, provider.prune_args(), environment, timeout) {
            Ok(output) => ProviderCommandResult::Output {
                success: output.status.success(),
                stdout: output.stdout,
                stderr: output.stderr,
            },
            Err(error) => ProviderCommandResult::Failed(error),
        }
    }
}

struct NativeProviderMeasurer;

impl ProviderMeasurer for NativeProviderMeasurer {
    fn measure(
        &self,
        path: &Path,
        environment: &PlatformEnvironment,
        cancellation: &dyn CancellationProbe,
        limits: ScanLimits,
        counters: &TraversalCounters,
    ) -> PathMeasurement {
        SizeCalculator::measure_path_bounded(path, &[], environment, cancellation, limits, counters)
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
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn scan_items(
        registry: &SignatureRegistry,
        excluded_signatures: &[String],
        environment: &PlatformEnvironment,
        cancellation: &dyn CancellationProbe,
        limits: ScanLimits,
        counters: &TraversalCounters,
        progress: &dyn RootProgressSink,
    ) -> CacheProviderScan {
        Self::scan_items_with(
            registry,
            excluded_signatures,
            environment,
            cancellation,
            limits,
            counters,
            progress,
            &NativeProviderCommandRunner,
            &NativeProviderMeasurer,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn scan_items_with(
        registry: &SignatureRegistry,
        excluded_signatures: &[String],
        environment: &PlatformEnvironment,
        cancellation: &dyn CancellationProbe,
        limits: ScanLimits,
        counters: &TraversalCounters,
        progress: &dyn RootProgressSink,
        runner: &dyn ProviderCommandRunner,
        measurer: &dyn ProviderMeasurer,
    ) -> CacheProviderScan {
        let mut result = CacheProviderScan::default();
        // Provider discovery is deliberately sequential. Starting every CLI
        // at once would make a user stop incapable of preventing later probes,
        // while these bounded lookups do not justify that lifecycle ambiguity.
        for provider in ProviderKind::ALL {
            if cancellation.is_cancelled() {
                result.cancelled = true;
                break;
            }
            if excluded_signatures
                .iter()
                .any(|id| id == provider.signature_id())
            {
                continue;
            }
            match Self::scan_provider(
                provider,
                registry,
                environment,
                cancellation,
                limits,
                counters,
                progress,
                runner,
                measurer,
            ) {
                Ok(Some(item)) => result.items.push(item),
                Ok(None) => {}
                Err(failure) if failure.kind == ScanGapKind::Cancelled => {
                    result.cancelled = true;
                    break;
                }
                Err(failure) => {
                    crate::diagnostics::log_error(
                        "cache_providers",
                        &format!(
                            "{} scan incomplete: {}",
                            provider.executable(),
                            failure.reason
                        ),
                    );
                    result.failures.push(failure);
                }
            }
            if cancellation.is_cancelled() {
                result.cancelled = true;
                break;
            }
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn scan_provider(
        provider: ProviderKind,
        registry: &SignatureRegistry,
        environment: &PlatformEnvironment,
        cancellation: &dyn CancellationProbe,
        limits: ScanLimits,
        counters: &TraversalCounters,
        progress: &dyn RootProgressSink,
        runner: &dyn ProviderCommandRunner,
        measurer: &dyn ProviderMeasurer,
    ) -> Result<Option<ScanItem>, CacheProviderFailure> {
        let signature_id = provider.signature_id();
        let Some(signature) = registry.get(signature_id) else {
            return Ok(None);
        };
        if signature.family != provider.family() {
            return Err(CacheProviderFailure::new(
                ScanGapKind::IoError,
                format!(
                    "{} catalog ownership does not match its provider contract",
                    provider.display_name()
                ),
            ));
        }
        if !signature.supports_current_platform() {
            return Ok(None);
        }
        let command = runner.discover(provider, environment, cancellation);
        let path = match command {
            ProviderCommandResult::Absent => return Ok(None),
            ProviderCommandResult::Cancelled => {
                return Err(CacheProviderFailure::new(
                    ScanGapKind::Cancelled,
                    format!("{} cache inspection was cancelled", provider.executable()),
                ));
            }
            ProviderCommandResult::Failed(reason) => {
                return Err(CacheProviderFailure::new(
                    classify_provider_failure(&reason),
                    format!(
                        "{} cache inspection failed: {reason}",
                        provider.executable()
                    ),
                ));
            }
            ProviderCommandResult::Output {
                success,
                stdout,
                stderr,
            } => {
                if !success {
                    let reason = bounded_message(&stderr);
                    return Err(CacheProviderFailure::new(
                        classify_provider_failure(&reason),
                        format!(
                            "{} cache discovery failed: {}",
                            provider.executable(),
                            reason
                        ),
                    ));
                }
                let parsed = parse_provider_path(provider, &stdout).map_err(|reason| {
                    CacheProviderFailure::new(
                        ScanGapKind::IoError,
                        format!("{}: {reason}", provider.executable()),
                    )
                })?;
                validate_cache_path(parsed, environment).map_err(|reason| {
                    CacheProviderFailure::new(
                        classify_provider_failure(&reason),
                        format!("{}: {reason}", provider.executable()),
                    )
                })?
            }
        };
        progress.root_started(signature, &path);
        let measurement = measurer.measure(&path, environment, cancellation, limits, counters);
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
        let cache_metadata = CacheMetadata {
            provider: provider.executable().to_string(),
            management_mode: CacheManagementMode::ToolManaged,
            artifact_kind: provider.artifact_kind(),
            consequence: provider.consequence().to_string(),
            size_semantics,
            last_used_confidence: Default::default(),
        };
        let disposition = derive_cleanup_disposition(DispositionFacts::new(
            RiskTier::Rebuild,
            quality,
            &cache_metadata,
            &measurement.size,
            measurement.incomplete_reason.as_deref(),
        ));
        let is_selected = disposition.eligibility == CleanupEligibility::AutoCleanable;
        let path_text = path.to_string_lossy().into_owned();
        Ok(Some(ScanItem {
            id: signature_id.to_string(),
            signature_id: signature_id.to_string(),
            name: provider.display_name().to_string(),
            category: Category::Developer,
            risk: RiskTier::Rebuild,
            path: path_text.clone(),
            size: measurement.size,
            file_count: measurement.file_count,
            description: "Inspected and pruned by the owning tool.".to_string(),
            cache_metadata,
            disposition,
            // The provider owns both the discovery and the invalidation; the
            // location travels with the item as a staleness assertion only.
            unit: CleanupUnit::new(
                CleanupUnitKind::ProviderAction,
                path_text.clone(),
                path_text,
            ),
            // The planner compares this claim with the catalog. Reuse the
            // catalog projection instead of manufacturing a stronger
            // executable-name claim (for example `go` versus provider `Go`).
            ownership: signature.ownership(),
            age: None,
            stale: None,
            structured_state: None,
            entry_kind: EntryKind::Directory,
            gate: EligibilityGate::Open,
            owner_running: false,
            lifecycle_provider_action: false,
            requires_confirmation: false,
            overlaps: Vec::new(),
            is_selected,
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
    pub(crate) fn prune(
        signature_id: &str,
        planned_path: &Path,
        environment: &PlatformEnvironment,
    ) -> Result<Option<u64>, CacheProviderPruneFailure> {
        let provider = ProviderKind::for_signature(signature_id).ok_or_else(|| {
            CacheProviderPruneFailure::Failed("Unknown external cache provider".to_string())
        })?;
        Self::prune_with(
            provider,
            planned_path,
            environment,
            &NativeProviderCommandRunner,
            &NativeProviderMeasurer,
        )
    }

    fn prune_with(
        provider: ProviderKind,
        planned_path: &Path,
        environment: &PlatformEnvironment,
        runner: &dyn ProviderCommandRunner,
        measurer: &dyn ProviderMeasurer,
    ) -> Result<Option<u64>, CacheProviderPruneFailure> {
        if !provider.active_processes().is_empty() {
            if let Some(refusal) =
                owner_process_refusal(provider, matching_process_is_active(provider))
            {
                return Err(refusal);
            }
        }
        let fresh_path = discover_path_with(provider, environment, runner)
            .map_err(CacheProviderPruneFailure::Failed)?;
        if !paths_match(&fresh_path, planned_path) {
            return Err(CacheProviderPruneFailure::Failed(
                "The provider cache location changed since the scan. Scan again.".to_string(),
            ));
        }
        let before = measurer.measure_for_prune(&fresh_path, environment);
        match runner.prune(provider, environment) {
            ProviderCommandResult::Output { success: true, .. } => {}
            ProviderCommandResult::Output {
                success: false,
                stderr,
                ..
            } => {
                return Err(CacheProviderPruneFailure::Failed(format!(
                    "{} prune failed: {}",
                    provider.executable(),
                    bounded_message(&stderr)
                )));
            }
            ProviderCommandResult::Failed(reason) => {
                return Err(CacheProviderPruneFailure::Failed(format!(
                    "{} prune failed: {reason}",
                    provider.executable()
                )));
            }
            ProviderCommandResult::Absent => {
                return Err(CacheProviderPruneFailure::Failed(format!(
                    "{} was not detected in trusted tool locations",
                    provider.executable()
                )));
            }
            ProviderCommandResult::Cancelled => {
                return Err(CacheProviderPruneFailure::Failed(format!(
                    "{} prune was cancelled",
                    provider.executable()
                )));
            }
        }
        let rediscovered = discover_path_with(provider, environment, runner)
            .map_err(CacheProviderPruneFailure::Failed)?;
        if !paths_match(&rediscovered, &fresh_path) {
            return Err(CacheProviderPruneFailure::Failed(
                "The provider cache location changed during cleanup.".to_string(),
            ));
        }
        let after = measurer.measure_for_prune(&rediscovered, environment);
        Ok(crate::scanner::size::reclaimed_between(&before, &after))
    }
}

impl CacheProviderScanner for CacheProviderRegistry {
    fn scan_items(
        &self,
        registry: &SignatureRegistry,
        excluded_signatures: &[String],
        environment: &PlatformEnvironment,
        cancellation: &dyn CancellationProbe,
        limits: ScanLimits,
        counters: &TraversalCounters,
        progress: &dyn RootProgressSink,
    ) -> CacheProviderScan {
        Self::scan_items(
            registry,
            excluded_signatures,
            environment,
            cancellation,
            limits,
            counters,
            progress,
        )
    }
}

fn classify_provider_failure(reason: &str) -> ScanGapKind {
    let reason = reason.to_ascii_lowercase();
    if reason.contains("cancel") {
        ScanGapKind::Cancelled
    } else if reason.contains("permission denied")
        || reason.contains("operation not permitted")
        || reason.contains("access denied")
    {
        ScanGapKind::PermissionDenied
    } else {
        ScanGapKind::IoError
    }
}

fn run_provider_with_timeout(
    provider: ProviderKind,
    args: &[&str],
    environment: &PlatformEnvironment,
    timeout: Duration,
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
    configure_provider_command(provider, environment, &mut command)?;
    zenith_platform::subprocess::run_with_timeout(command, timeout)
        .map_err(|error| error.to_string())
}

// Discovery and mutation must see the same cache configuration. Explicitly
// remove both npm spellings: environment keys are case-sensitive on Unix.
const CACHE_PATH_ENVIRONMENT: &[&str] = &[
    "GOCACHE",
    "GOMODCACHE",
    "GOPATH",
    "GOENV",
    "UV_CACHE_DIR",
    "PIP_CACHE_DIR",
    "PIP_CONFIG_FILE",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "PNPM_HOME",
    "npm_config_store_dir",
    "NPM_CONFIG_STORE_DIR",
    "npm_config_cache",
    "NPM_CONFIG_CACHE",
    "npm_config_userconfig",
    "NPM_CONFIG_USERCONFIG",
    "npm_config_globalconfig",
    "NPM_CONFIG_GLOBALCONFIG",
    "BUN_INSTALL_CACHE_DIR",
    "BUN_RUNTIME_TRANSPILER_CACHE_PATH",
    "COMPOSER_CACHE_DIR",
    "COMPOSER_HOME",
    "NUGET_PACKAGES",
    "NUGET_HTTP_CACHE_PATH",
    "NUGET_PLUGINS_CACHE_PATH",
];

fn strip_cache_environment(command: &mut Command) {
    for variable in CACHE_PATH_ENVIRONMENT {
        command.env_remove(variable);
    }
}

fn configure_provider_command(
    provider: ProviderKind,
    environment: &PlatformEnvironment,
    command: &mut Command,
) -> Result<(), String> {
    strip_cache_environment(command);
    let mut dependency_dirs = Vec::new();
    for dependency in provider.runtime_dependencies() {
        let executable = tooling::resolve_with(dependency, environment).ok_or_else(|| {
            format!(
                "{} requires {}, which was not detected in trusted tool locations",
                provider.executable(),
                dependency
            )
        })?;
        validate_executable(&executable, environment)?;
        let canonical = std::fs::canonicalize(executable)
            .map_err(|_| format!("Could not validate the {dependency} runtime"))?;
        let parent = canonical
            .parent()
            .ok_or_else(|| format!("Could not resolve the {dependency} runtime directory"))?;
        dependency_dirs.push(parent.to_path_buf());
    }
    if !dependency_dirs.is_empty() {
        // Script shims commonly use `/usr/bin/env node` or `/usr/bin/env php`.
        // Give them only the verified runtime directories plus the OS command
        // roots needed by `env`; never forward an arbitrary inherited PATH.
        dependency_dirs.extend([PathBuf::from("/usr/bin"), PathBuf::from("/bin")]);
        let path = std::env::join_paths(dependency_dirs)
            .map_err(|_| "Could not construct the provider runtime PATH".to_string())?;
        command.env("PATH", path);
    }
    if provider.local_toolchain_only() {
        // Go's default `auto` toolchain mode may download a different toolchain
        // merely to answer `go env` or perform `go clean`. Inspection and
        // cleanup must not create the cache they are measuring or access the
        // network as a side effect.
        command.env("GOTOOLCHAIN", "local");
    }
    Ok(())
}

fn discover_path_with(
    provider: ProviderKind,
    environment: &PlatformEnvironment,
    runner: &dyn ProviderCommandRunner,
) -> Result<PathBuf, String> {
    let output = runner.discover(provider, environment, &crate::models::NeverCancelled);
    let bytes = match output {
        ProviderCommandResult::Output {
            success: true,
            stdout,
            ..
        } => stdout,
        ProviderCommandResult::Output {
            success: false,
            stderr,
            ..
        } => {
            return Err(format!(
                "{} cache discovery failed: {}",
                provider.executable(),
                bounded_message(&stderr)
            ));
        }
        ProviderCommandResult::Absent => {
            return Err(format!(
                "{} was not detected in trusted tool locations",
                provider.executable()
            ));
        }
        ProviderCommandResult::Failed(reason) => {
            return Err(format!(
                "{} cache discovery failed: {reason}",
                provider.executable()
            ));
        }
        ProviderCommandResult::Cancelled => {
            return Err(format!(
                "{} cache discovery was cancelled",
                provider.executable()
            ));
        }
    };
    parse_provider_path(provider, &bytes).and_then(|path| validate_cache_path(path, environment))
}

fn parse_provider_path(provider: ProviderKind, output: &[u8]) -> Result<AbsolutePath, String> {
    match provider.discovery_output() {
        DiscoveryOutput::BarePath => parse_discovered_path(output),
        DiscoveryOutput::LabeledPath(expected_label) => {
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
            if lines.len() != 1 {
                return Err("Cache discovery returned an ambiguous path".to_string());
            }
            let (label, path) = lines[0]
                .split_once(':')
                .ok_or_else(|| "Cache discovery did not return a labeled path".to_string())?;
            if !label.trim().eq_ignore_ascii_case(expected_label) {
                return Err("Cache discovery returned an unexpected resource label".to_string());
            }
            parse_discovered_path(path.trim().as_bytes())
        }
    }
}

fn parse_discovered_path(output: &[u8]) -> Result<AbsolutePath, String> {
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
    AbsolutePath::new(lines[0]).map_err(|_| "Cache discovery returned a relative path".to_string())
}

fn validate_cache_path(
    path: AbsolutePath,
    environment: &PlatformEnvironment,
) -> Result<PathBuf, String> {
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|_| "The discovered cache directory is unavailable".to_string())?;
    if !metadata.is_dir() || SymlinkGuard::is_symlink(path.as_path()) {
        return Err("The discovered cache must be a real directory".to_string());
    }
    // The stated profile is the authority: the runner's own home must not
    // decide whether a cache belongs to this user.
    let home = environment
        .user_home()
        .ok_or_else(|| "Could not resolve the current user profile".to_string())?;
    let canonical_home = std::fs::canonicalize(home)
        .map_err(|_| "Could not validate the current user profile".to_string())?;
    // Every check below runs on the resolved location: an approved cache
    // reached through a link is the link's target, and that is what the
    // profile, blacklist, and symlink rules must be applied to.
    let canonical = CanonicalPath::resolve(&path)
        .map_err(|_| "Could not canonicalize the discovered cache".to_string())?;

    let flavor = environment.flavor();
    let in_profile = path_is_within(canonical.as_path(), &canonical_home, flavor);
    let mut approved =
        in_profile && cache_location_approved(canonical.as_path(), &canonical_home, flavor);
    // Relocated caches (PNPM_HOME, UV_CACHE_DIR, a configured npm cache) stay
    // supported under the same blacklist and symlink validation as in-profile
    // locations instead of being refused for being outside the profile.
    for root in relocated_cache_roots(environment) {
        approved |= path_is_same(canonical.as_path(), &root, flavor)
            || path_is_within(canonical.as_path(), &root, flavor);
    }

    if !approved {
        return Err(
            "The provider cache override is outside approved user cache locations".to_string(),
        );
    }
    Blacklist::validate_with(canonical.as_path(), environment)
        .map_err(|error| error.to_string())?;
    if in_profile {
        SymlinkGuard::validate_no_symlink_ancestors(
            canonical.as_path(),
            &canonical_home,
            environment,
        )
        .map_err(|error| error.to_string())?;
    } else {
        SymlinkGuard::validate_anchored_path(canonical.as_path(), environment)
            .map_err(|error| error.to_string())?;
    }
    Ok(canonical.into_path_buf())
}

fn relocated_cache_roots(environment: &PlatformEnvironment) -> Vec<PathBuf> {
    // The application-data containers come from the described environment;
    // the per-tool overrides (`UV_CACHE_DIR`, `PNPM_HOME`, `NPM_CONFIG_CACHE`)
    // are process environment variables with no environment representation, so
    // they stay host-derived like the tool search itself.
    let stated = [environment.local_app_data(), environment.roaming_app_data()];
    let overrides = [
        "UV_CACHE_DIR",
        "PIP_CACHE_DIR",
        "PNPM_HOME",
        "NPM_CONFIG_CACHE",
        "NUGET_PACKAGES",
        "NUGET_HTTP_CACHE_PATH",
        "NUGET_PLUGINS_CACHE_PATH",
    ]
    .into_iter()
    .filter_map(|variable| std::env::var_os(variable).map(PathBuf::from));
    stated
        .into_iter()
        .flatten()
        .chain(overrides)
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .map(|root| zenith_platform::NativePlatformPaths::normalize_verbatim_path(&root))
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
        canonical_home.join(".bun/install/cache"),
        canonical_home.join(".composer/cache"),
        canonical_home.join(".nuget/packages"),
        canonical_home.join(".local/share/NuGet"),
        canonical_home.join("go/pkg/mod"),
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
    let canonical = zenith_platform::NativePlatformPaths::normalize_verbatim_path(&canonical);
    let mut roots = vec![
        PathBuf::from("/usr/bin"),
        PathBuf::from("/usr/local/bin"),
        // Microsoft's macOS package places the real host here and commonly
        // exposes `/usr/local/bin/dotnet` as a symlink. Validation is applied
        // to the canonical executable, so the documented target must be a
        // trust root as well as the symlink directory.
        PathBuf::from("/usr/local/share/dotnet"),
        PathBuf::from("/opt/homebrew"),
    ];
    roots.extend(zenith_platform::NativePlatformPaths::trusted_tool_roots(
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

    // Compare canonical locations when a root exists. This matters on macOS,
    // where `/var` resolves to `/private/var`, and for package-manager shims
    // whose documented root may itself be reached through a link.
    let roots = roots
        .into_iter()
        .map(|root| {
            std::fs::canonicalize(&root)
                .map(|path| zenith_platform::NativePlatformPaths::normalize_verbatim_path(&path))
                .unwrap_or(root)
        })
        .collect::<Vec<_>>();

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
        let norm_root = zenith_platform::NativePlatformPaths::normalize_verbatim_path(root);
        if looks_like_windows_path(canonical) || looks_like_windows_path(&norm_root) {
            zenith_platform::NativePlatformPaths::windows_path_starts_with(canonical, &norm_root)
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

fn matching_process_is_active(provider: ProviderKind) -> Option<bool> {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let processes: Vec<_> = system.processes().values().collect();
    process_table_activity(&processes, |process| {
        process_matches_provider(provider, process.name(), process.cmd())
    })
}

fn owner_process_refusal(
    provider: ProviderKind,
    active: Option<bool>,
) -> Option<CacheProviderPruneFailure> {
    match active {
        Some(false) => None,
        Some(true) => Some(CacheProviderPruneFailure::OwnerRunning(
            provider.executable().to_string(),
        )),
        None => Some(CacheProviderPruneFailure::OwnerStateUnknown(
            provider.executable().to_string(),
        )),
    }
}

fn process_table_activity<T>(
    processes: &[T],
    mut matches_owner: impl FnMut(&T) -> bool,
) -> Option<bool> {
    if processes.is_empty() {
        None
    } else {
        Some(processes.iter().any(&mut matches_owner))
    }
}

fn process_matches_provider(
    provider: ProviderKind,
    process_name: &std::ffi::OsStr,
    command: &[std::ffi::OsString],
) -> bool {
    let name = process_name.to_string_lossy();
    provider.active_processes().iter().any(|expected| {
        name.eq_ignore_ascii_case(expected)
            || name.eq_ignore_ascii_case(&format!("{expected}.exe"))
            || command.iter().take(3).any(|argument| {
                Path::new(argument)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| {
                        value.eq_ignore_ascii_case(expected)
                            || value.eq_ignore_ascii_case(&format!("{expected}.exe"))
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
    use super::{
        cache_location_approved, node_manager_roots, owner_process_refusal, parse_discovered_path,
        parse_provider_path, process_matches_provider, process_table_activity, AbsolutePath,
        CacheProviderPruneFailure, CacheProviderRegistry, UV_PRUNE_TIMEOUT,
    };
    use super::{
        configure_provider_command, strip_cache_environment, NativeProviderMeasurer,
        ProviderCommandResult, ProviderCommandRunner, ProviderKind, ProviderMeasurer,
        CACHE_PATH_ENVIRONMENT,
    };
    use crate::models::{
        CancellationProbe, CleanStrategy, NeverCancelled, ObservationQuality, ScanGapKind,
    };
    use crate::scanner::{NoRootProgress, PathMeasurement, ScanLimits, TraversalCounters};
    use crate::signatures::SignatureRegistry;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::{Arc, Mutex};
    use zenith_platform::path_algebra::PathFlavor;
    use zenith_platform::paths::SimulatedPaths;
    use zenith_platform::PlatformEnvironment;

    #[derive(Default)]
    struct FakeRunner {
        responses: std::collections::HashMap<ProviderKind, ProviderCommandResult>,
        prune_responses: std::collections::HashMap<ProviderKind, ProviderCommandResult>,
        prune_files: std::collections::HashMap<ProviderKind, PathBuf>,
        calls: Mutex<Vec<ProviderKind>>,
        prune_calls: Mutex<Vec<ProviderKind>>,
    }

    impl FakeRunner {
        fn with(mut self, provider: ProviderKind, result: ProviderCommandResult) -> Self {
            self.responses.insert(provider, result);
            self
        }

        fn with_prune(mut self, provider: ProviderKind, result: ProviderCommandResult) -> Self {
            self.prune_responses.insert(provider, result);
            self
        }

        fn delete_file_on_prune(mut self, provider: ProviderKind, path: PathBuf) -> Self {
            self.prune_files.insert(provider, path);
            self
        }

        fn absent() -> Self {
            Self::default()
        }

        fn calls(&self) -> Vec<ProviderKind> {
            self.calls.lock().unwrap().clone()
        }

        fn prune_calls(&self) -> Vec<ProviderKind> {
            self.prune_calls.lock().unwrap().clone()
        }
    }

    impl ProviderCommandRunner for FakeRunner {
        fn discover(
            &self,
            provider: ProviderKind,
            _environment: &PlatformEnvironment,
            _cancellation: &dyn CancellationProbe,
        ) -> ProviderCommandResult {
            self.calls.lock().unwrap().push(provider);
            self.responses
                .get(&provider)
                .cloned()
                .unwrap_or(ProviderCommandResult::Absent)
        }

        fn prune(
            &self,
            provider: ProviderKind,
            _environment: &PlatformEnvironment,
        ) -> ProviderCommandResult {
            self.prune_calls.lock().unwrap().push(provider);
            let response = self
                .prune_responses
                .get(&provider)
                .cloned()
                .unwrap_or(ProviderCommandResult::Absent);
            if matches!(
                response,
                ProviderCommandResult::Output { success: true, .. }
            ) {
                if let Some(path) = self.prune_files.get(&provider) {
                    let _ = std::fs::remove_file(path);
                }
            }
            response
        }
    }

    struct FixedMeasurer(PathMeasurement);

    impl ProviderMeasurer for FixedMeasurer {
        fn measure(
            &self,
            _path: &std::path::Path,
            _environment: &PlatformEnvironment,
            _cancellation: &dyn CancellationProbe,
            _limits: ScanLimits,
            _counters: &TraversalCounters,
        ) -> PathMeasurement {
            self.0.clone()
        }
    }

    struct CancelAfterVisits<'a> {
        counters: &'a TraversalCounters,
        limit: u64,
    }

    impl CancellationProbe for CancelAfterVisits<'_> {
        fn is_cancelled(&self) -> bool {
            self.counters.visited_entries() >= self.limit
        }
    }

    #[derive(Default)]
    struct RecordingProgress(Mutex<Vec<(String, PathBuf)>>);

    impl crate::scanner::RootProgressSink for RecordingProgress {
        fn root_started(&self, signature: &crate::models::Signature, root: &std::path::Path) {
            self.0
                .lock()
                .unwrap()
                .push((signature.id.clone(), root.to_path_buf()));
        }
    }

    fn fixture_environment(home: &std::path::Path) -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::current()).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::current())
                .with_home(home),
        ))
    }

    fn successful_discovery(path: &std::path::Path) -> ProviderCommandResult {
        ProviderCommandResult::Output {
            success: true,
            stdout: format!("{}\n", path.display()).into_bytes(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn uv_prune_uses_its_lock_wait_and_bounded_owner_command() {
        assert_eq!(ProviderKind::Uv.prune_args(), &["cache", "prune"]);
        assert!(ProviderKind::Uv.active_processes().is_empty());
        assert_eq!(UV_PRUNE_TIMEOUT, std::time::Duration::from_secs(300));
    }

    #[test]
    fn uv_owner_prune_succeeds_without_generic_tree_deletion() {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path().join("home");
        let cache = home.join(".cache/uv");
        std::fs::create_dir_all(&cache).unwrap();
        let unused = cache.join("unused.whl");
        let retained = cache.join("retained.whl");
        std::fs::write(&unused, vec![1u8; 4_096]).unwrap();
        std::fs::write(&retained, vec![2u8; 2_048]).unwrap();
        let environment = fixture_environment(&home);
        let runner = FakeRunner::default()
            .with(ProviderKind::Uv, successful_discovery(&cache))
            .with_prune(
                ProviderKind::Uv,
                ProviderCommandResult::Output {
                    success: true,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                },
            )
            .delete_file_on_prune(ProviderKind::Uv, unused.clone());

        let outcome = CacheProviderRegistry::prune_with(
            ProviderKind::Uv,
            &cache,
            &environment,
            &runner,
            &NativeProviderMeasurer,
        )
        .expect("the owner command succeeds");

        assert_eq!(outcome, Some(4_096));
        assert_eq!(runner.prune_calls(), vec![ProviderKind::Uv]);
        assert!(!unused.exists(), "the fake owner pruned its unused fixture");
        assert!(retained.exists(), "the owner retained referenced data");
        assert!(
            cache.is_dir(),
            "the store root was never recursively removed"
        );
    }

    #[test]
    fn failed_uv_owner_prune_leaves_the_store_untouched_without_fallback() {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path().join("home");
        let cache = home.join(".cache/uv");
        std::fs::create_dir_all(&cache).unwrap();
        let existing = cache.join("package.whl");
        std::fs::write(&existing, vec![3u8; 1_024]).unwrap();
        let environment = fixture_environment(&home);
        let runner = FakeRunner::default()
            .with(ProviderKind::Uv, successful_discovery(&cache))
            .with_prune(
                ProviderKind::Uv,
                ProviderCommandResult::Output {
                    success: false,
                    stdout: Vec::new(),
                    stderr: b"fixture refusal".to_vec(),
                },
            )
            .delete_file_on_prune(ProviderKind::Uv, existing.clone());

        let error = CacheProviderRegistry::prune_with(
            ProviderKind::Uv,
            &cache,
            &environment,
            &runner,
            &NativeProviderMeasurer,
        )
        .expect_err("a failed owner command is reported honestly");

        assert!(error
            .to_string()
            .contains("uv prune failed: fixture refusal"));
        assert_eq!(runner.prune_calls(), vec![ProviderKind::Uv]);
        assert!(
            existing.exists(),
            "failure does not fall back to file deletion"
        );
        assert!(
            cache.is_dir(),
            "failure does not fall back to root deletion"
        );
    }

    #[test]
    fn provider_command_removes_cache_path_overrides() {
        let mut command = Command::new("provider-fixture");
        for variable in CACHE_PATH_ENVIRONMENT {
            command.env(variable, "/untrusted/cache");
        }
        command.env("ZENITH_UNRELATED_FIXTURE", "preserved");
        strip_cache_environment(&mut command);
        let variables: std::collections::HashMap<_, _> = command
            .get_envs()
            .map(|(key, value)| (key.to_string_lossy().to_ascii_lowercase(), value))
            .collect();
        for variable in CACHE_PATH_ENVIRONMENT {
            assert_eq!(
                variables.get(&variable.to_ascii_lowercase()),
                Some(&None),
                "{variable}"
            );
        }
        assert_eq!(
            variables.get("zenith_unrelated_fixture"),
            Some(&Some(std::ffi::OsStr::new("preserved")))
        );
    }

    #[test]
    fn go_commands_disable_automatic_toolchain_downloads() {
        let mut command = Command::new("go-fixture");
        command.env("GOTOOLCHAIN", "auto");
        command.env("GOMODCACHE", "/untrusted/mod-cache");
        configure_provider_command(
            ProviderKind::GoModule,
            &PlatformEnvironment::native(),
            &mut command,
        )
        .unwrap();
        let variables: std::collections::HashMap<_, _> = command
            .get_envs()
            .map(|(key, value)| (key.to_string_lossy().into_owned(), value))
            .collect();

        assert_eq!(
            variables.get("GOTOOLCHAIN"),
            Some(&Some(std::ffi::OsStr::new("local")))
        );
        assert_eq!(variables.get("GOMODCACHE"), Some(&None));
    }

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
        assert_eq!(ProviderKind::Npm.prune_args(), &["cache", "verify"]);
        assert_eq!(ProviderKind::Npm.active_processes(), &["npm", "npm-cli"]);
        assert_eq!(ProviderKind::for_signature("dev.yarn.cache"), None);
    }

    #[test]
    fn cache_owner_process_checks_ignore_unrelated_node_apps() {
        let unrelated_node_command = vec![
            std::ffi::OsString::from("node"),
            std::ffi::OsString::from("/workspace/server.js"),
        ];
        assert!(!process_matches_provider(
            ProviderKind::Npm,
            std::ffi::OsStr::new("node"),
            &unrelated_node_command,
        ));
        assert!(!process_matches_provider(
            ProviderKind::Pnpm,
            std::ffi::OsStr::new("node"),
            &unrelated_node_command,
        ));

        let npm_command = vec![
            std::ffi::OsString::from("node"),
            std::ffi::OsString::from("/usr/local/lib/node_modules/npm/bin/npm-cli.js"),
            std::ffi::OsString::from("cache"),
        ];
        assert!(process_matches_provider(
            ProviderKind::Npm,
            std::ffi::OsStr::new("node"),
            &npm_command,
        ));

        let pnpm_command = vec![
            std::ffi::OsString::from("node"),
            std::ffi::OsString::from("/usr/local/lib/node_modules/pnpm/bin/pnpm.cjs"),
            std::ffi::OsString::from("store"),
        ];
        assert!(process_matches_provider(
            ProviderKind::Pnpm,
            std::ffi::OsStr::new("node"),
            &pnpm_command,
        ));

        assert_eq!(
            process_table_activity::<&str>(&[], |_| true),
            None,
            "an unreadable process table is not an idle-owner signal"
        );
        assert_eq!(
            process_table_activity(&["node"], |_| false),
            Some(false),
            "a readable unrelated Node process does not block the owner command"
        );
        assert_eq!(
            owner_process_refusal(ProviderKind::Npm, None),
            Some(CacheProviderPruneFailure::OwnerStateUnknown("npm".into()))
        );
        assert_eq!(
            owner_process_refusal(ProviderKind::Npm, Some(true)),
            Some(CacheProviderPruneFailure::OwnerRunning("npm".into()))
        );
        assert_eq!(owner_process_refusal(ProviderKind::Npm, Some(false)), None);
    }

    #[test]
    fn language_cache_providers_use_fixed_owner_commands() {
        assert_eq!(ProviderKind::GoBuild.discovery_args(), &["env", "GOCACHE"]);
        assert_eq!(ProviderKind::GoBuild.prune_args(), &["clean", "-cache"]);
        assert_eq!(
            ProviderKind::GoModule.discovery_args(),
            &["env", "GOMODCACHE"]
        );
        assert_eq!(ProviderKind::GoModule.prune_args(), &["clean", "-modcache"]);
        assert_eq!(ProviderKind::Pip.discovery_args(), &["cache", "dir"]);
        assert_eq!(ProviderKind::Pip.prune_args(), &["cache", "purge"]);
        assert_eq!(
            ProviderKind::NugetHttp.discovery_args(),
            &[
                "nuget",
                "locals",
                "http-cache",
                "--list",
                "--force-english-output"
            ]
        );
        assert_eq!(
            ProviderKind::NugetGlobalPackages.prune_args(),
            &[
                "nuget",
                "locals",
                "global-packages",
                "--clear",
                "--force-english-output"
            ]
        );
        assert_eq!(
            ProviderKind::Composer.prune_args(),
            &["--no-interaction", "--no-plugins", "clear-cache"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn script_providers_receive_only_a_validated_runtime_path() {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let node = bin.join("node");
        std::fs::write(&node, "#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = std::fs::metadata(&node).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&node, permissions).unwrap();

        let environment = fixture_environment(home.path()).with_tool("node", &node);
        let mut command = Command::new("npm-fixture");
        command.env("PATH", "/untrusted/bin");
        configure_provider_command(ProviderKind::Npm, &environment, &mut command).unwrap();
        let path = command
            .get_envs()
            .find(|(key, _)| *key == std::ffi::OsStr::new("PATH"))
            .and_then(|(_, value)| value)
            .unwrap()
            .to_string_lossy();

        assert!(path.contains(".local/bin"));
        assert!(path.contains("/usr/bin"));
        assert!(!path.contains("/untrusted/bin"));
    }

    #[test]
    fn nuget_discovery_accepts_one_expected_labeled_path() {
        let path = if cfg!(windows) {
            r"C:\Users\tester\AppData\Local\NuGet\v3-cache"
        } else {
            "/Users/tester/Library/Caches/NuGet/v3-cache"
        };
        let output = format!("http-cache: {path}\n");
        let parsed = parse_provider_path(ProviderKind::NugetHttp, output.as_bytes()).unwrap();
        assert_eq!(parsed.as_path(), std::path::Path::new(path));
        assert!(parse_provider_path(
            ProviderKind::NugetHttp,
            b"global-packages: /Users/tester/.nuget/packages\n",
        )
        .is_err());
        assert!(parse_provider_path(
            ProviderKind::NugetHttp,
            b"http-cache: /one\nhttp-cache: /two\n",
        )
        .is_err());
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
        assert!(cache_location_approved(
            &home.join(".nuget/packages"),
            &home,
            flavor
        ));
        assert!(cache_location_approved(
            &home.join(".local/share/NuGet/v3-cache"),
            &home,
            flavor
        ));
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
        let expected = zenith_platform::NativePlatformPaths::normalize_verbatim_path(
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
                crate::safety::Blacklist::validate_with(&root.join("uv/cache"), &environment)
                    .is_ok(),
                "a cache below the relocated container {} must stay cleanable",
                root.display()
            );
        }
        assert!(
            crate::safety::Blacklist::validate_with(local_app_data.path(), &environment).is_err(),
            "the relocated application-data root itself stays protected"
        );
    }

    /// Fixture paths are rooted by construction; the type only makes the
    /// assumption explicit at the boundary the test is exercising.
    fn rooted(path: PathBuf) -> AbsolutePath {
        AbsolutePath::new(path).expect("fixture path is absolute")
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
        assert!(super::validate_cache_path(rooted(cache), &environment).is_ok());

        // An in-profile directory that is not an approved cache root is refused
        // even though it exists, so the approval step still does work.
        let unapproved = stated_home.path().join("random-override");
        std::fs::create_dir_all(&unapproved).unwrap();
        assert!(super::validate_cache_path(rooted(unapproved), &environment).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn nuget_provider_executes_fixed_commands_against_an_isolated_store() {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".local/bin");
        let cache = home.path().join(".nuget/packages");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("package.bin"), vec![1_u8; 4096]).unwrap();

        let executable = bin.join("dotnet");
        std::fs::write(
            &executable,
            r#"#!/bin/sh
set -eu
fixture_home=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
cache="$fixture_home/.nuget/packages"
case "$*" in
  "nuget locals global-packages --list --force-english-output")
    printf 'global-packages: %s\n' "$cache"
    ;;
  "nuget locals global-packages --clear --force-english-output")
    find "$cache" -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +
    ;;
  *)
    exit 64
    ;;
esac
"#,
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).unwrap();

        let environment = fixture_environment(home.path()).with_tool("dotnet", &executable);
        let reclaimed =
            super::CacheProviderRegistry::prune("dev.nuget.global_packages", &cache, &environment)
                .unwrap();

        assert!(reclaimed.is_some_and(|bytes| bytes >= 4096));
        assert!(cache.is_dir());
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
    }

    #[test]
    fn a_provider_with_a_stated_missing_tool_is_skipped() {
        // npm/pnpm/uv may be installed on this host; the environment states
        // they are absent, and a stated absence is never re-discovered.
        let environment = PlatformEnvironment::simulated(PathFlavor::current())
            .with_missing_tool("go")
            .with_missing_tool("npm")
            .with_missing_tool("pnpm")
            .with_missing_tool("uv")
            .with_missing_tool("pip3")
            .with_missing_tool("composer")
            .with_missing_tool("dotnet");
        let registry = SignatureRegistry::load_embedded().unwrap();
        let counters = TraversalCounters::default();
        let result = super::CacheProviderRegistry::scan_items(
            &registry,
            &[],
            &environment,
            &NeverCancelled,
            ScanLimits::default(),
            &counters,
            &NoRootProgress,
        );
        assert!(result.items.is_empty());
        assert!(result.failures.is_empty());
        assert!(!result.cancelled);
    }

    #[test]
    fn a_go_module_cache_with_toolchain_locks_is_plannable_through_go() {
        let home = tempfile::tempdir().unwrap();
        let cache = home.path().join("go/pkg/mod");
        let download = cache.join("cache/download/golang.org/toolchain/@v");
        std::fs::create_dir_all(&download).unwrap();
        std::fs::write(
            download.join("v0.0.1-go1.24.0.darwin-arm64.lock"),
            b"owner lock fixture",
        )
        .unwrap();

        let environment = fixture_environment(home.path());
        let registry = SignatureRegistry::load_embedded().unwrap();
        let runner =
            FakeRunner::absent().with(ProviderKind::GoModule, successful_discovery(&cache));
        let counters = TraversalCounters::default();
        let result = super::CacheProviderRegistry::scan_items_with(
            &registry,
            &[],
            &environment,
            &NeverCancelled,
            ScanLimits::default(),
            &counters,
            &NoRootProgress,
            &runner,
            &NativeProviderMeasurer,
        );

        assert!(result.failures.is_empty());
        assert_eq!(result.items.len(), 1);
        let mut item = result.items.into_iter().next().unwrap();
        assert_eq!(item.signature_id, "dev.go.mod");
        item.is_selected = true;

        let owner_providers = crate::cleaner::OwnerProviderRegistry::new(Vec::new());
        let plan = crate::safety::SafetyPlanner::create_plan_with_environment(
            &[item],
            &registry,
            &environment,
            &owner_providers,
        )
        .expect("Go owns its valid lockfiles; generic structured-state checks do not apply");
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].strategy, CleanStrategy::ExternalCommand);
    }

    #[test]
    fn an_excluded_provider_is_not_invoked() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        let environment = PlatformEnvironment::simulated(PathFlavor::current());
        let runner = FakeRunner::absent();
        let counters = TraversalCounters::default();
        let result = super::CacheProviderRegistry::scan_items_with(
            &registry,
            &["dev.uv.cache".to_string()],
            &environment,
            &NeverCancelled,
            ScanLimits::default(),
            &counters,
            &NoRootProgress,
            &runner,
            &NativeProviderMeasurer,
        );

        assert!(result.items.is_empty());
        assert_eq!(
            runner.calls(),
            vec![
                ProviderKind::GoBuild,
                ProviderKind::GoModule,
                ProviderKind::Pip,
                ProviderKind::Pnpm,
                ProviderKind::Npm,
                ProviderKind::Composer,
                ProviderKind::NugetHttp,
                ProviderKind::NugetTemp,
                ProviderKind::NugetPlugins,
                ProviderKind::NugetGlobalPackages,
            ]
        );
    }

    #[test]
    fn command_cancellation_stops_before_later_providers() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        let runner =
            FakeRunner::absent().with(ProviderKind::GoBuild, ProviderCommandResult::Cancelled);
        let counters = TraversalCounters::default();
        let progress = RecordingProgress::default();
        let result = super::CacheProviderRegistry::scan_items_with(
            &registry,
            &[],
            &PlatformEnvironment::simulated(PathFlavor::current()),
            &NeverCancelled,
            ScanLimits::default(),
            &counters,
            &progress,
            &runner,
            &NativeProviderMeasurer,
        );

        assert!(result.cancelled);
        assert!(result.items.is_empty());
        assert_eq!(runner.calls(), vec![ProviderKind::GoBuild]);
    }

    #[test]
    fn malformed_and_timed_out_discovery_are_typed_scan_failures() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        let runner = FakeRunner::absent()
            .with(
                ProviderKind::GoBuild,
                ProviderCommandResult::Output {
                    success: true,
                    stdout: b"relative/cache\n".to_vec(),
                    stderr: Vec::new(),
                },
            )
            .with(
                ProviderKind::GoModule,
                ProviderCommandResult::Failed("command timed out".to_string()),
            );
        let counters = TraversalCounters::default();
        let result = super::CacheProviderRegistry::scan_items_with(
            &registry,
            &[],
            &PlatformEnvironment::simulated(PathFlavor::current()),
            &NeverCancelled,
            ScanLimits::default(),
            &counters,
            &NoRootProgress,
            &runner,
            &NativeProviderMeasurer,
        );

        assert!(result.items.is_empty());
        assert_eq!(result.failures.len(), 2);
        assert!(result
            .failures
            .iter()
            .all(|failure| failure.kind == ScanGapKind::IoError));
    }

    #[test]
    fn permission_refused_measurement_remains_a_visible_unavailable_row() {
        let home = tempfile::tempdir().unwrap();
        let cache = home.path().join(".cache/uv/store");
        std::fs::create_dir_all(&cache).unwrap();
        let environment = fixture_environment(home.path());
        let runner = FakeRunner::absent().with(ProviderKind::Uv, successful_discovery(&cache));
        let counters = TraversalCounters::default();
        let progress = RecordingProgress::default();
        let result = super::CacheProviderRegistry::scan_items_with(
            &SignatureRegistry::load_embedded().unwrap(),
            &[],
            &environment,
            &NeverCancelled,
            ScanLimits::default(),
            &counters,
            &progress,
            &runner,
            &FixedMeasurer(PathMeasurement::unavailable("Permission denied")),
        );

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].quality, ObservationQuality::Unavailable);
        assert_eq!(
            result.items[0].incomplete_reason.as_deref(),
            Some("Permission denied")
        );
        assert_eq!(
            progress.0.lock().unwrap().as_slice(),
            &[("dev.uv.cache".to_string(), cache.canonicalize().unwrap())]
        );
        assert!(result.failures.is_empty());
    }

    #[test]
    fn cancellation_inside_cache_measurement_stops_the_walk_and_later_providers() {
        let home = tempfile::tempdir().unwrap();
        let cache = home.path().join(".cache/uv/store");
        std::fs::create_dir_all(&cache).unwrap();
        for index in 0..32 {
            std::fs::write(cache.join(format!("{index}.bin")), b"fixture").unwrap();
        }
        let environment = fixture_environment(home.path());
        let runner = FakeRunner::absent().with(ProviderKind::Uv, successful_discovery(&cache));
        let counters = TraversalCounters::default();
        let cancellation = CancelAfterVisits {
            counters: &counters,
            limit: 5,
        };
        let result = super::CacheProviderRegistry::scan_items_with(
            &SignatureRegistry::load_embedded().unwrap(),
            &[],
            &environment,
            &cancellation,
            ScanLimits::default(),
            &counters,
            &NoRootProgress,
            &runner,
            &NativeProviderMeasurer,
        );

        assert!(result.cancelled);
        assert_eq!(result.items.len(), 1);
        assert_ne!(result.items[0].quality, ObservationQuality::Fresh);
        assert!(result.items[0]
            .incomplete_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("cancelled")));
        assert!(counters.visited_entries() < 33);
        assert!(counters.directories_read() > 0);
        assert_eq!(
            runner.calls(),
            vec![
                ProviderKind::GoBuild,
                ProviderKind::GoModule,
                ProviderKind::Pip,
                ProviderKind::Uv,
            ]
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
        use zenith_platform::NativePlatformPaths;

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
        let app_data = zenith_platform::PlatformEnvironment::native()
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
