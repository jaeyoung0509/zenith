use crate::models::{
    Category, CleanStrategy, CleanerFamily, PlatformKind, RiskTier, Signature, ZenithError,
};
use crate::safety::Blacklist;
use crate::signatures::SignatureLoader;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use zenith_platform::path_algebra::{is_absolute, PathFlavor};
use zenith_platform::PlatformEnvironment;

const EMBEDDED_AI_TOML: &str = include_str!("../../../signatures/ai.toml");
const EMBEDDED_DEV_TOML: &str = include_str!("../../../signatures/developer.toml");
const EMBEDDED_CONTAINERS_TOML: &str = include_str!("../../../signatures/containers.toml");
const EMBEDDED_SYSTEM_TOML: &str = include_str!("../../../signatures/system.toml");

/// One manifest lint result. `platform` is set when the finding is that a path
/// resolves under a platform-specific root without the signature declaring it;
/// the other invariants are platform independent and leave it `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestLintFinding {
    pub signature_id: String,
    pub platform: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct SignatureRegistry {
    signatures: HashMap<String, Signature>,
}

impl Default for SignatureRegistry {
    fn default() -> Self {
        Self::load_embedded().unwrap_or_else(|_| Self {
            signatures: HashMap::new(),
        })
    }
}

impl SignatureRegistry {
    pub fn new() -> Self {
        Self {
            signatures: HashMap::new(),
        }
    }

    /// Loads built-in embedded signatures for AI, Developer, Container, and System categories.
    ///
    /// The catalog declares nothing environment-dependent, so this is the
    /// native wrapper; the platform-declaration gate is applied against the
    /// running environment.
    pub fn load_embedded() -> Result<Self, ZenithError> {
        Self::load_embedded_with(&PlatformEnvironment::native())
    }

    /// Loads the embedded catalog and refuses signatures that resolve under a
    /// platform-specific root without declaring it. A refused signature is not
    /// registered and the refusal is recorded, so an undeclared platform root
    /// can never be silently accepted and then offered by a scan.
    pub fn load_embedded_with(environment: &PlatformEnvironment) -> Result<Self, ZenithError> {
        let mut registry = Self::load_embedded_catalog()?;
        registry.enforce_platform_declarations(environment);
        Ok(registry)
    }

    /// Loads the catalog for startup, recording a load failure instead of
    /// presenting an empty catalog as a healthy one.
    ///
    /// A failed catalog used to reach `unwrap_or_default()` at the composition
    /// root, where the failure was invisible in the log and the fallback was
    /// unreachable from a test. The fallback is an empty registry rather than
    /// [`Self::default`]: the catalog that just failed to load cannot be
    /// replaced by one validated against a different environment, and nothing
    /// should be scanned or cleaned from a catalog that was never verified.
    ///
    /// The failure is returned as well as logged. An empty catalog produces an
    /// empty scan, and an empty scan is byte-for-byte what a clean machine
    /// reports; the caller is the only place that can tell the two apart.
    pub(crate) fn load_or_default<E: std::fmt::Display>(
        environment: &PlatformEnvironment,
        load: impl FnOnce(&PlatformEnvironment) -> Result<SignatureRegistry, E>,
    ) -> (SignatureRegistry, Option<String>) {
        match load(environment) {
            Ok(registry) => (registry, None),
            Err(error) => {
                let message = format!("Signature catalog could not be loaded: {error}");
                crate::diagnostics::log_error("startup", &message);
                (Self::new(), Some(message))
            }
        }
    }

    /// Loads the embedded catalog without the platform-declaration gate. The
    /// manifest lint and the environment doctor must see an offending
    /// signature in order to report it, so they load through this entry point.
    pub fn load_embedded_catalog() -> Result<Self, ZenithError> {
        let mut registry = Self::new();

        let tomls = [
            EMBEDDED_AI_TOML,
            EMBEDDED_DEV_TOML,
            EMBEDDED_CONTAINERS_TOML,
            EMBEDDED_SYSTEM_TOML,
        ];

        for toml_str in &tomls {
            let sigs = SignatureLoader::load_str(toml_str)?;
            for sig in sigs {
                registry.register(sig);
            }
        }

        Ok(registry)
    }

    /// Drops every signature the platform-declaration rule refuses and records
    /// the refusal. The registry itself stores nothing environment-dependent.
    fn enforce_platform_declarations(&mut self, environment: &PlatformEnvironment) {
        let refused: Vec<String> = Self::audit_signature_platforms(self, environment)
            .into_iter()
            .filter(|finding| finding.platform.is_some())
            .map(|finding| finding.signature_id)
            .collect();
        let mut recorded = BTreeSet::new();
        for signature_id in refused {
            if !recorded.insert(signature_id.clone()) {
                continue;
            }
            self.signatures.remove(&signature_id);
            crate::diagnostics::log_error(
                "signatures",
                &format!(
                    "Signature `{signature_id}` was refused: it resolves under a \
                     platform-specific root without declaring `platforms`"
                ),
            );
        }
    }

    /// Registers a single signature.
    pub fn register(&mut self, signature: Signature) {
        self.signatures.insert(signature.id.clone(), signature);
    }

    /// Gets a signature by ID.
    pub fn get(&self, id: &str) -> Option<&Signature> {
        self.signatures.get(id)
    }

    /// Lists all signatures.
    pub fn all(&self) -> Vec<&Signature> {
        self.signatures.values().collect()
    }

    /// Lists signatures by category.
    pub fn by_category(&self, category: Category) -> Vec<&Signature> {
        self.signatures
            .values()
            .filter(|s| s.category == category)
            .collect()
    }

    /// Lists catalog entries owned by one cleaner family.
    ///
    /// Categories are presentation groups; families are the implementation
    /// boundary used when a workflow chooses discovery and mutation adapters.
    pub fn by_family(&self, family: CleanerFamily) -> Vec<&Signature> {
        let mut signatures: Vec<&Signature> = self
            .signatures
            .values()
            .filter(|signature| signature.family == family)
            .collect();
        signatures.sort_by(|left, right| left.id.cmp(&right.id));
        signatures
    }

    /// Lists the signatures a scan discovers for a category and scope.
    ///
    /// Discovery and eligibility are separate: this returns what the scan
    /// looks at, and [`Signature::eligibility_gate`] states what the current
    /// scope permits for each of them. A signature whose scope is off is not
    /// discovered at all; a signature that opted into always-on discovery is
    /// returned in either mode, and the walker records the gate on every unit
    /// it finds.
    ///
    /// The order is deterministic — most specific first, then by id — because
    /// two signatures can describe the same location, and which one wins must
    /// not depend on the hash order of a map.
    pub fn by_category_for_mode(
        &self,
        category: Category,
        intensive_cleanup: bool,
    ) -> Vec<&Signature> {
        let mut discovered: Vec<&Signature> = self
            .signatures
            .values()
            .filter(|signature| {
                signature.category == category
                    && signature.supports_current_platform()
                    && signature.discovery_allows(intensive_cleanup)
            })
            .collect();
        discovered.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.id.cmp(&right.id))
        });
        discovered
    }

    /// Lists signatures by risk tier.
    pub fn by_risk(&self, risk: RiskTier) -> Vec<&Signature> {
        self.signatures
            .values()
            .filter(|s| s.risk == risk)
            .collect()
    }

    /// Resolves expanded paths for a given signature against the described
    /// environment. Because the registry stores nothing environment-dependent,
    /// the environment is a parameter of the lookup, not of the load.
    pub fn resolve_paths(
        &self,
        signature: &Signature,
        environment: &PlatformEnvironment,
    ) -> Vec<PathBuf> {
        signature
            .paths
            .iter()
            .filter_map(|p| SignatureLoader::expand_path(p, environment))
            .collect()
    }

    /// The concrete roots that authorize a path, in pattern order.
    ///
    /// A literal pattern authorizes its resolved path, and — for a signature
    /// that enumerates children — each direct child of it. A selector pattern
    /// authorizes every path it matches, with the same child rule.
    ///
    /// Matching is textual and flavor-correct: it reads no metadata, so the
    /// scope a plan is checked against cannot be widened by a link, and a
    /// selector that matches nothing simply authorizes nothing. An empty result
    /// means the path is outside the signature's scope.
    pub fn authorizing_roots(
        &self,
        signature: &Signature,
        path: &Path,
        environment: &PlatformEnvironment,
    ) -> Vec<PathBuf> {
        let flavor = environment.flavor();
        let enumerates_children = signature.unit_kind().is_enumerated_child();
        let path_text = path.to_string_lossy();
        let parent_text = path
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned());
        let mut roots = Vec::new();

        for pattern in &signature.paths {
            let Some(expanded) = SignatureLoader::expand_path(pattern, environment) else {
                continue;
            };
            let expanded_text = expanded.to_string_lossy().into_owned();

            if zenith_platform::selector::PathSelector::is_pattern(&expanded_text) {
                let Ok(selector) =
                    zenith_platform::selector::PathSelector::parse(&expanded_text, flavor)
                else {
                    continue;
                };
                if selector.matches(&path_text, flavor) {
                    roots.push(PathBuf::from(path_text.to_string()));
                    continue;
                }
                if enumerates_children {
                    if let Some(parent) = parent_text.as_deref() {
                        if selector.matches(parent, flavor) {
                            roots.push(PathBuf::from(parent.to_string()));
                        }
                    }
                }
                continue;
            }

            if path == expanded.as_path() {
                roots.push(expanded);
                continue;
            }
            if enumerates_children && path.parent() == Some(expanded.as_path()) {
                roots.push(expanded);
            }
        }

        roots
    }

    /// Whether a signature authorizes a concrete path.
    pub fn path_is_in_scope(
        &self,
        signature: &Signature,
        path: &Path,
        environment: &PlatformEnvironment,
    ) -> bool {
        !self
            .authorizing_roots(signature, path, environment)
            .is_empty()
    }

    /// The pure manifest lint the environment doctor and CI both run.
    ///
    /// Four invariants are checked for every signature, against the described
    /// environment:
    ///
    /// * every path pattern resolves to an absolute path (a literal relative
    ///   pattern is a finding; a dynamic pattern the environment cannot resolve
    ///   is not, since "not present here" is a legitimate answer);
    /// * no resolved path is blacklisted — except for a signature that selects
    ///   aged children inside its root, which never deletes the root itself,
    ///   and except for an observation-only (`Manual`) entry, which reports
    ///   bytes the generic cleaner may not remove;
    /// * every path-shaped exclusion resolves inside its own signature's scope,
    ///   so an exclusion cannot protect something the signature never touches;
    /// * a signature whose paths resolve under a platform-specific root
    ///   declares `platforms`. An empty `platforms` means "offered everywhere",
    ///   which is only truthful when at least one path is not platform
    ///   specific; a declared list must cover every platform the paths name.
    ///
    /// Findings are sorted so the lint and the doctor are deterministic.
    pub fn audit_signature_platforms(
        registry: &SignatureRegistry,
        environment: &PlatformEnvironment,
    ) -> Vec<ManifestLintFinding> {
        let mut findings = Vec::new();
        for signature in registry.all() {
            findings.extend(audit_signature(signature, environment));
        }
        findings.sort_by(|left, right| {
            left.signature_id
                .cmp(&right.signature_id)
                .then_with(|| left.message.cmp(&right.message))
        });
        findings
    }

    /// Returns which platform (if any) a path pattern or placeholder is
    /// specifically tied to.
    ///
    /// Patterns arrive normalized by the loader, so a manifest written with a
    /// Windows `%VAR%` spelling is judged by the placeholder it became. The
    /// `%VAR%` arms below stay for signatures built by hand in tests and
    /// adapters, where treating a Windows spelling as platform-neutral would
    /// skip the platform gate in the fail-open direction.
    pub fn is_platform_specific_path(pattern: &str) -> Option<PlatformKind> {
        let p = pattern.trim();
        if p.starts_with("~/Library")
            || p.starts_with("/Applications")
            || p.starts_with("/Library")
            || p.starts_with("/System")
            || p.starts_with("/Volumes")
            || p.starts_with("/private/")
            || p.contains("${DARWIN_USER_CACHE}")
        {
            Some(PlatformKind::Macos)
        } else if p.contains("${SYSTEM_ROOT}")
            || p.contains("${LOCAL_APP_DATA}")
            || p.contains("${ROAMING_APP_DATA}")
            || p.contains("${PROGRAM_DATA}")
            || p.contains("${PROGRAM_FILES}")
            || p.starts_with("C:\\")
            || p.starts_with("c:\\")
            || p.starts_with("%USERPROFILE%")
            || p.starts_with("%APPDATA%")
            || p.starts_with("%LOCALAPPDATA%")
            || p.starts_with("%PROGRAMDATA%")
            || p.starts_with("%PROGRAMFILES%")
            || p.starts_with("%SYSTEMROOT%")
            || p.starts_with("%WINDIR%")
        {
            Some(PlatformKind::Windows)
        } else {
            None
        }
    }
}

fn audit_signature(
    signature: &Signature,
    environment: &PlatformEnvironment,
) -> Vec<ManifestLintFinding> {
    let flavor = environment.flavor();
    let mut findings = Vec::new();
    let mut resolved: Vec<PathBuf> = Vec::new();

    // A pattern that names another platform is not this machine's path: it is
    // checked when the lint runs against that platform, and resolving it here
    // would report a `~/Library` root as a relative path on Windows.
    let stated_platform = match flavor {
        PathFlavor::Windows => PlatformKind::Windows,
        PathFlavor::Posix => PlatformKind::Macos,
    };

    for pattern in &signature.paths {
        if pattern.trim().is_empty() {
            continue;
        }
        if SignatureRegistry::is_platform_specific_path(pattern)
            .is_some_and(|target| target != stated_platform)
        {
            continue;
        }
        match SignatureLoader::expand_path(pattern, environment) {
            Some(path) => {
                let expanded_text = path.to_string_lossy().into_owned();
                if zenith_platform::selector::PathSelector::is_pattern(&expanded_text) {
                    // A selector names roots the catalog cannot spell out, so
                    // the rule is about what it *can*: a literal prefix to scan
                    // from. A pattern that begins with a selector would be
                    // resolved against `/` or a drive root on every run.
                    match zenith_platform::selector::PathSelector::parse(&expanded_text, flavor) {
                        Ok(selector) => match selector.static_root() {
                            Some(static_root) if is_absolute(&static_root, flavor) => {}
                            _ => findings.push(finding(
                                signature,
                                None,
                                format!(
                                    "pattern `{pattern}` begins with a selector; it needs an absolute literal prefix"
                                ),
                            )),
                        },
                        Err(error) => findings.push(finding(
                            signature,
                            None,
                            format!("pattern `{pattern}` is not a usable selector: {error}"),
                        )),
                    }
                }
                if !is_absolute(&path.to_string_lossy(), flavor) {
                    findings.push(finding(
                        signature,
                        None,
                        format!("path pattern `{pattern}` did not resolve to an absolute path"),
                    ));
                }
                // An observation-only entry does not need a deletion boundary
                // checked: the planner refuses a `Manual` target and the
                // executor has no operation for it, so what the entry covers is
                // reported and never removed.
                let may_delete = signature.strategy != CleanStrategy::Manual;
                if may_delete && signature.min_age_days.is_none() {
                    if let Some(root) = zenith_platform::path_algebra::protected_root(
                        &path.to_string_lossy(),
                        flavor,
                    ) {
                        findings.push(finding(
                            signature,
                            None,
                            format!(
                                "path pattern `{pattern}` resolves under a protected {}",
                                root.reason()
                            ),
                        ));
                    } else if Blacklist::is_blacklisted_with(&path, environment) {
                        findings.push(finding(
                            signature,
                            None,
                            format!("path pattern `{pattern}` resolves to a blacklisted location"),
                        ));
                    }
                }
                resolved.push(path);
            }
            None => {
                if !is_dynamic_pattern(pattern) {
                    findings.push(finding(
                        signature,
                        None,
                        format!("path pattern `{pattern}` is not an absolute path"),
                    ));
                }
            }
        }
    }

    if !resolved.is_empty() {
        for exclusion in &signature.exclusions {
            // The same rule as a path: an exclusion written for another
            // platform is checked when the lint runs against that platform,
            // and comparing it with this machine's scope would report a
            // correct manifest as inconsistent.
            if SignatureRegistry::is_platform_specific_path(exclusion)
                .is_some_and(|target| target != stated_platform)
            {
                continue;
            }
            let Some(expanded) = SignatureLoader::expand_exclusion(exclusion, environment) else {
                continue;
            };
            let expanded_text = expanded.to_string_lossy().into_owned();
            let within_path = resolved.iter().any(|path| {
                super::exclusions::reachable_from(&path.to_string_lossy(), &expanded_text, flavor)
            });
            if !within_path {
                findings.push(finding(
                    signature,
                    None,
                    format!("exclusion `{exclusion}` resolves outside the signature's scope"),
                ));
            }
        }
    }

    let inferred = inferred_platforms(&signature.paths);
    if !inferred.is_empty() {
        let declared: BTreeSet<PlatformKind> = signature.platforms.iter().copied().collect();
        let undeclared: Vec<PlatformKind> = if declared.is_empty() {
            // Empty `platforms` means "offered on every platform". That is only
            // truthful when at least one path serves every platform; when all
            // inferred paths name a single platform, offering it elsewhere
            // presents paths that resolve to nothing.
            if inferred.len() == 1 {
                inferred.iter().copied().collect()
            } else {
                Vec::new()
            }
        } else {
            inferred
                .iter()
                .copied()
                .filter(|platform| !declared.contains(platform))
                .collect()
        };
        for platform in undeclared {
            let token = platform_token(platform);
            findings.push(finding(
                signature,
                Some(token.to_string()),
                format!("resolves under a {token}-specific root but does not declare `platforms`"),
            ));
        }
    }

    // A provider action is carried out by the implementation the manifest
    // names, so a catalog entry that names one this build does not have is a
    // target nothing can complete. The check reads the same registries the scan
    // and the executor dispatch through, so the two cannot drift apart — and it
    // reads the registry the declared strategy dispatches into, so a lifecycle
    // action cannot pass itself off as an owner-scoped one or the reverse.
    if let Some(provider_id) = signature
        .provider_id
        .as_deref()
        .filter(|provider_id| !provider_id.trim().is_empty())
    {
        let (implemented, kind) = match signature.strategy {
            CleanStrategy::OwnerProvider => (
                crate::cleaner::OwnerProviderRegistry::native(
                    std::sync::Arc::new(crate::cleaner::SysinfoProcessProbe),
                    std::sync::Arc::new(crate::scanner::SizeCalculatorMeasurement),
                )
                .implemented_ids(),
                "owner-scoped provider",
            ),
            _ => (
                crate::cleaner::LifecycleProviderRegistry::native().implemented_ids(),
                "lifecycle provider",
            ),
        };
        if !implemented.contains(&provider_id) {
            findings.push(finding(
                signature,
                None,
                format!(
                    "names {kind} `{provider_id}`, which this build does not implement (implemented: {})",
                    implemented.join(", ")
                ),
            ));
        }
    }

    findings
}

fn finding(
    signature: &Signature,
    platform: Option<String>,
    message: String,
) -> ManifestLintFinding {
    ManifestLintFinding {
        signature_id: signature.id.clone(),
        platform,
        message,
    }
}

/// Platforms named by a signature's path patterns. The vocabulary is a
/// property of the manifest (a placeholder or a literal platform prefix), not
/// of the host, which is exactly why it must be declared.
fn inferred_platforms(patterns: &[String]) -> BTreeSet<PlatformKind> {
    patterns
        .iter()
        .filter_map(|pattern| SignatureRegistry::is_platform_specific_path(pattern))
        .collect()
}

/// A pattern the environment may legitimately fail to resolve: it names a
/// placeholder (`~`, `${...}`, `%VAR%`, `$TMPDIR`) rather than a literal path.
fn is_dynamic_pattern(pattern: &str) -> bool {
    let trimmed = pattern.trim();
    trimmed.contains("${") || trimmed.starts_with(['~', '$', '%']) || trimmed.contains('%')
}

fn platform_token(platform: PlatformKind) -> &'static str {
    match platform {
        PlatformKind::Macos => "macos",
        PlatformKind::Windows => "windows",
        PlatformKind::Linux => "linux",
        PlatformKind::Other => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::{ManifestLintFinding, SignatureRegistry};
    use crate::models::{
        CacheArtifactKind, Category, CleanStrategy, CleanerFamily, CleanupUnitKind, DiscoveryScope,
        EligibilityGate, PlatformKind, RiskTier, Signature,
    };
    use std::path::PathBuf;
    use std::sync::Arc;
    use zenith_platform::path_algebra::PathFlavor;
    use zenith_platform::paths::SimulatedPaths;
    use zenith_platform::{KnownFolder, PlatformEnvironment};

    /// A POSIX environment with stated roots, so the catalog lint below is
    /// checked against paths that exist on any runner.
    fn stated_environment() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::Posix).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::Posix)
                .with_home("/home/tester")
                .with_local_app_data("/home/tester/.local/share")
                .with_roaming_app_data("/home/tester/.config")
                .with_user_cache_dir("/var/folders/ab/cdefgh/C")
                .with_temp_dir("/tmp"),
        ))
    }

    use crate::signatures::SignatureLoader;

    /// A stated Windows machine: the profile is on `D:`, and the catalog
    /// assertions below are about that machine rather than about the host.
    fn stated_windows_environment() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::Windows).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::Windows)
                .with_home(r"D:\Users\tester")
                .with_system_root(r"D:\Windows")
                .with_temp_dir(r"D:\Users\tester\AppData\Local\Temp")
                .with_local_app_data(r"D:\Users\tester\AppData\Local")
                .with_roaming_app_data(r"D:\Users\tester\AppData\Roaming")
                .with_program_files(r"D:\Program Files")
                .with_program_data(r"D:\ProgramData"),
        ))
    }

    #[test]
    fn signature_catalog_failure_falls_back_to_an_empty_registry() {
        let environment = stated_environment();

        let (registry, failure) = SignatureRegistry::load_or_default(&environment, |_| {
            Err(crate::models::ZenithError::SignatureMismatch(
                "stated catalog defect".to_string(),
            ))
        });

        // Fail closed: a catalog that could not be loaded must not leave the
        // process scanning and cleaning from signatures nobody verified.
        assert!(registry.all().is_empty());
        let failure = failure.expect("the failure is reported, not only logged");
        assert!(failure.contains("stated catalog defect"), "{failure}");

        // A successful load is passed through untouched, so the fallback is
        // not a second implementation of the loader.
        let (loaded, failure) =
            SignatureRegistry::load_or_default(&environment, SignatureRegistry::load_embedded_with);
        assert!(!loaded.all().is_empty());
        assert_eq!(failure, None);
    }

    #[test]
    fn a_drive_relative_signature_path_is_refused() {
        // `C:cache` names a location relative to the current directory on `C:`
        // and must never be treated as an absolute cleanup target.
        assert_eq!(
            SignatureLoader::expand_path(r"C:cache\logs", &stated_windows_environment()),
            None
        );
        assert_eq!(
            SignatureLoader::expand_path(r"C:..\Windows\Temp", &stated_windows_environment()),
            None
        );
    }

    #[test]
    fn the_stated_environment_decides_the_blacklist_verdict() {
        let mut registry = SignatureRegistry::new();
        let mut signature = test_signature(
            "developer.test.host-decides",
            vec![r"C:\Windows\Temp\zenith"],
            vec![PlatformKind::Windows],
        );
        signature.min_age_days = None;
        registry.register(signature);

        // Windows rules refuse the system directory, and they are reached
        // through the stated environment rather than through the host's.
        let windows_findings =
            SignatureRegistry::audit_signature_platforms(&registry, &stated_windows_environment());
        assert_eq!(windows_findings.len(), 1, "{windows_findings:?}");
        assert!(
            windows_findings[0]
                .message
                .contains("protected Windows directory"),
            "{windows_findings:?}"
        );

        // The same literal is a Windows spelling, so the POSIX audit does not
        // judge it at all: a signature that declares its platform is checked
        // where it runs, and reporting a Windows path as relative on a POSIX
        // machine would make every cross-platform manifest a false finding.
        let posix_findings =
            SignatureRegistry::audit_signature_platforms(&registry, &stated_environment());
        assert!(
            posix_findings.is_empty(),
            "a foreign-platform pattern is not this machine's path: {posix_findings:?}"
        );

        // A pattern that names no platform is judged on every machine, which is
        // what keeps a typo such as a relative path a finding.
        let mut neutral = SignatureRegistry::new();
        let mut relative = test_signature("developer.test.relative", vec!["data/cache"], vec![]);
        relative.min_age_days = None;
        neutral.register(relative);
        for environment in [stated_environment(), stated_windows_environment()] {
            let findings = SignatureRegistry::audit_signature_platforms(&neutral, &environment);
            assert_eq!(findings.len(), 1, "{findings:?}");
            assert!(
                findings[0].message.contains("not an absolute path"),
                "{findings:?}"
            );
        }
    }

    #[test]
    fn a_windows_absolute_exclusion_is_scope_checked() {
        let mut registry = SignatureRegistry::new();
        let mut signature = test_signature(
            "developer.test.windows-exclusion",
            vec![r"${LOCAL_APP_DATA}\Zenith\cache"],
            vec![PlatformKind::Windows],
        );
        signature.exclusions = vec![r"D:\Documents\do-not-delete".to_string()];
        registry.register(signature);

        // The exclusion is absolute in its own spelling, so the scope audit has
        // to run: on a Windows runner the host `Path` check would have skipped
        // it and the escape would have gone unreported.
        let findings =
            SignatureRegistry::audit_signature_platforms(&registry, &stated_windows_environment());
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].message.contains("scope"), "{findings:?}");
    }

    fn test_signature(id: &str, paths: Vec<&str>, platforms: Vec<PlatformKind>) -> Signature {
        Signature {
            id: id.to_string(),
            name: id.to_string(),
            category: Category::System,
            family: CleanerFamily::System,
            risk: RiskTier::Safe,
            strategy: crate::models::CleanStrategy::DeleteDirectory,
            paths: paths.into_iter().map(str::to_string).collect(),
            exclusions: vec![],
            description: String::new(),
            min_age_days: None,
            include_prefixes: vec![],
            exclude_prefixes: vec![],
            intensive_only: false,
            platforms,
            discovery: Default::default(),
            unit: None,
            owner: String::new(),
            priority: 0,
            fail_if_running: Vec::new(),
            provider: String::new(),
            provider_id: None,
            artifact_kind: Default::default(),
            consequence: String::new(),
        }
    }

    fn audit_one(signature: Signature) -> Vec<ManifestLintFinding> {
        let mut registry = SignatureRegistry::new();
        registry.register(signature);
        SignatureRegistry::audit_signature_platforms(&registry, &stated_environment())
    }

    /// The macOS entries resolve against a stated macOS machine, including the
    /// root the system owns outside the profile.
    #[test]
    fn the_macos_entries_resolve_on_a_stated_macos_machine() {
        let environment = stated_environment();
        let registry = SignatureRegistry::load_embedded_with(&environment).expect("catalog");

        let findings = SignatureRegistry::audit_signature_platforms(&registry, &environment);
        assert!(findings.is_empty(), "{findings:?}");

        let system_cache = registry
            .get("system.darwin_user_cache")
            .expect("the system cache entry is in the catalog");
        let resolved = registry.resolve_paths(system_cache, &environment);
        assert_eq!(
            resolved,
            vec![PathBuf::from("/var/folders/ab/cdefgh/C")],
            "the per-user cache root resolves through the system, not through the profile"
        );

        // A selector entry names a literal prefix, and matches the shapes a
        // machine actually has.
        let segments = registry
            .get("system.app_support_cache_segments")
            .expect("the segment entry is in the catalog");
        let pattern = segments
            .paths
            .iter()
            .find(|pattern| pattern.contains("GPUCache"))
            .expect("the entry names a GPU cache segment");
        let expanded = SignatureLoader::expand_path(pattern, &environment)
            .expect("the pattern resolves to its literal prefix");
        let selector = zenith_platform::selector::PathSelector::parse(
            &expanded.to_string_lossy(),
            PathFlavor::Posix,
        )
        .expect("the selector parses");
        assert!(selector.matches(
            "/home/tester/Library/Application Support/Slack/GPUCache",
            PathFlavor::Posix
        ));
        assert!(!selector.matches(
            "/home/tester/Library/Application Support/Slack/Local Storage",
            PathFlavor::Posix
        ));
    }

    /// The Windows entries resolve against a stated Windows machine, including
    /// the roots the catalog now names there.
    #[test]
    fn the_windows_entries_resolve_on_a_stated_windows_machine() {
        let environment = stated_windows_environment();
        let registry = SignatureRegistry::load_embedded_with(&environment).expect("catalog");

        let findings = SignatureRegistry::audit_signature_platforms(&registry, &environment);
        assert!(findings.is_empty(), "{findings:?}");

        let maintenance = registry
            .get("system.advisory.windows_maintenance")
            .expect("the maintenance entry is in the catalog");
        let resolved = registry.resolve_paths(maintenance, &environment);
        assert!(
            resolved.iter().any(|path| path
                .to_string_lossy()
                .ends_with(r"D:\Windows\SoftwareDistribution\Download")),
            "the installation root resolves from the stated machine: {resolved:?}"
        );

        let browsers = registry
            .get("system.intensive.windows_browser_caches")
            .expect("the browser entry is in the catalog");
        let expanded = SignatureLoader::expand_path(&browsers.paths[0], &environment)
            .expect("the pattern resolves to its literal prefix");
        let selector = zenith_platform::selector::PathSelector::parse(
            &expanded.to_string_lossy(),
            PathFlavor::Windows,
        )
        .expect("the selector parses");
        assert!(selector.matches(
            r"D:\Users\tester\AppData\Local\Google\Chrome\User Data\Profile 3\Cache",
            PathFlavor::Windows
        ));
        assert!(
            !selector.matches(
                r"D:\Users\tester\AppData\Local\Google\Chrome\User Data\Default\Local Storage",
                PathFlavor::Windows
            ),
            "a sibling directory under the profile is not a cache"
        );
    }

    /// A store that holds browsing artifacts is never a generic delete target,
    /// on any platform: the entries that name it must be observation-only.
    #[test]
    fn no_deletable_entry_names_a_browsing_artifact_store() {
        let registry = SignatureRegistry::load_embedded_catalog().expect("catalog");
        let named: Vec<&Signature> = registry
            .all()
            .into_iter()
            .filter(|signature| {
                signature
                    .paths
                    .iter()
                    .any(|pattern| pattern.contains("WebCache"))
            })
            .collect();
        assert!(
            !named.is_empty(),
            "the store is still reported, so its bytes are not invisible"
        );
        for signature in named {
            assert_eq!(
                signature.strategy,
                CleanStrategy::Manual,
                "{} names a store that holds user artifacts and must not delete it",
                signature.id
            );
            assert_eq!(signature.risk, RiskTier::Manual);
        }
    }

    /// The stores Windows itself maintains are never generic delete targets:
    /// every entry that names a path under the installation root is
    /// observation-only, and the one Windows-owned store Zenith may act on is
    /// reached through the provider the catalog names rather than through a
    /// path. This is the #230 boundary, pinned as a class so a future entry
    /// cannot quietly turn update payloads or kernel dumps into deletable
    /// paths.
    #[test]
    fn no_deletable_entry_names_an_os_owned_maintenance_store() {
        let registry = SignatureRegistry::load_embedded_catalog().expect("catalog");

        let mut named_roots = 0;
        for signature in registry.all() {
            if !signature
                .paths
                .iter()
                .any(|pattern| pattern.contains("${SYSTEM_ROOT}"))
            {
                continue;
            }
            named_roots += 1;
            assert_eq!(
                signature.strategy,
                CleanStrategy::Manual,
                "{} names a store Windows maintains and must not delete it",
                signature.id
            );
            assert_eq!(signature.risk, RiskTier::Manual, "{}", signature.id);
        }
        assert!(
            named_roots > 0,
            "the installation root's stores are still reported, so their bytes are not invisible"
        );

        // The Recycle Bin is the one Windows-owned store that is executable,
        // and only through the lifecycle provider it names: no path, a manual
        // tier, and an id the manifest lint checks against this build.
        let recycle_bin = registry
            .get("system.windows.recycle_bin")
            .expect("the Recycle Bin entry is in the catalog");
        assert_eq!(recycle_bin.strategy, CleanStrategy::LifecycleProvider);
        assert_eq!(
            recycle_bin.provider_id.as_deref(),
            Some("windows.recycle_bin")
        );
        assert_eq!(recycle_bin.risk, RiskTier::Manual);
        assert!(
            recycle_bin.paths.is_empty(),
            "a provider action owns no host path"
        );
    }

    /// Every shipped entry satisfies the catalog schema, and the granularity it
    /// implies matches the strategy it declares. A manifest that contradicts
    /// itself fails the load instead of reaching a scan.
    #[test]
    fn the_embedded_catalog_satisfies_the_catalog_schema() {
        let registry = SignatureRegistry::load_embedded_catalog()
            .expect("the embedded catalog loads and validates");
        assert!(!registry.all().is_empty(), "the catalog is not empty");

        for signature in registry.all() {
            signature
                .validate()
                .unwrap_or_else(|error| panic!("{} does not validate: {error}", signature.id));
            let kind = signature.unit_kind();
            match signature.strategy {
                CleanStrategy::DockerPrune => assert_eq!(
                    kind,
                    CleanupUnitKind::ContainerResource,
                    "{} prunes a runtime-owned resource",
                    signature.id
                ),
                CleanStrategy::ExternalCommand => assert_eq!(
                    kind,
                    CleanupUnitKind::ProviderAction,
                    "{} is performed by its provider",
                    signature.id
                ),
                CleanStrategy::LifecycleProvider => {
                    assert_eq!(
                        kind,
                        CleanupUnitKind::ProviderAction,
                        "{} is performed by the lifecycle provider it names",
                        signature.id
                    );
                    assert!(
                        signature
                            .provider_id
                            .as_deref()
                            .is_some_and(|provider_id| !provider_id.trim().is_empty()),
                        "{} names the provider that performs its action",
                        signature.id
                    );
                }
                CleanStrategy::OwnerProvider => {
                    assert_eq!(
                        kind,
                        CleanupUnitKind::ProviderAction,
                        "{} is enumerated and removed by the provider it names",
                        signature.id
                    );
                    assert!(
                        signature
                            .provider_id
                            .as_deref()
                            .is_some_and(|provider_id| !provider_id.trim().is_empty()),
                        "{} names the provider that owns the store",
                        signature.id
                    );
                    assert!(
                        !signature.fail_if_running.is_empty(),
                        "{} states the executables whose running state makes its store unsafe",
                        signature.id
                    );
                }
                CleanStrategy::DeleteContents
                | CleanStrategy::DeleteDirectory
                | CleanStrategy::DeleteStaleContents => {
                    assert!(
                        kind.is_filesystem(),
                        "{} deletes a host path and must own a filesystem unit",
                        signature.id
                    );
                    if signature.min_age_days.is_some() {
                        // An age policy belongs to a unit the scanner either
                        // enumerates (each child) or treats whole (a named
                        // subtree); it never ages a fixed path in place.
                        let ages_a_unit = matches!(
                            kind,
                            CleanupUnitKind::ChildNamespace | CleanupUnitKind::NamedSubtree
                        );
                        assert!(
                            ages_a_unit,
                            "{} ages an enumerated or named unit, not `{}`",
                            signature.id,
                            kind.display_name()
                        );
                    }
                }
                CleanStrategy::Manual => {}
            }
        }
    }

    /// The schema rules refuse a catalog entry that contradicts itself.
    #[test]
    fn a_contradictory_signature_is_refused() {
        let base = |id: &str| Signature {
            id: id.to_string(),
            name: id.to_string(),
            category: Category::System,
            family: CleanerFamily::System,
            risk: RiskTier::Safe,
            strategy: CleanStrategy::DeleteDirectory,
            paths: vec!["/tmp/example".to_string()],
            exclusions: vec![],
            description: String::new(),
            min_age_days: None,
            include_prefixes: vec![],
            exclude_prefixes: vec![],
            intensive_only: false,
            platforms: vec![],
            discovery: DiscoveryScope::ModeGated,
            unit: None,
            owner: String::new(),
            priority: 0,
            fail_if_running: Vec::new(),
            provider: String::new(),
            provider_id: None,
            artifact_kind: Default::default(),
            consequence: String::new(),
        };

        // An enumerated or named unit without an age policy would delete state
        // that is still in use.
        let mut child_without_age = base("test.invalid.child");
        child_without_age.unit = Some(CleanupUnitKind::ChildNamespace);
        assert!(child_without_age.validate().is_err());

        let mut named_without_age = base("test.invalid.named");
        named_without_age.unit = Some(CleanupUnitKind::NamedSubtree);
        assert!(named_without_age.validate().is_err());

        // An age policy ages children, so a signature that declares the fixed
        // path itself as its unit contradicts it.
        let mut fixed_with_age = base("test.invalid.fixed");
        fixed_with_age.min_age_days = Some(7);
        fixed_with_age.unit = Some(CleanupUnitKind::FixedPath);
        assert!(fixed_with_age.validate().is_err());

        // Stating nothing derives the unit from the age policy instead.
        let mut derived = base("test.valid.derived");
        derived.min_age_days = Some(7);
        assert!(derived.validate().is_ok());
        assert_eq!(derived.unit_kind(), CleanupUnitKind::ChildNamespace);

        // A path-owning strategy cannot be paired with a unit that owns none.
        let mut provider_paths = base("test.invalid.provider");
        provider_paths.strategy = CleanStrategy::ExternalCommand;
        provider_paths.unit = Some(CleanupUnitKind::ProviderAction);
        provider_paths.paths = vec!["/tmp/example".to_string()];
        assert!(provider_paths.validate().is_err());

        // A process guard names an executable, never a path.
        let mut path_guard = base("test.invalid.guard");
        path_guard.fail_if_running = vec!["/usr/bin/cargo".to_string()];
        assert!(path_guard.validate().is_err());

        let mut empty_guard = base("test.invalid.empty-guard");
        empty_guard.fail_if_running = vec!["   ".to_string()];
        assert!(empty_guard.validate().is_err());

        // The same facts without the contradiction are accepted.
        let mut child = base("test.valid.child");
        child.min_age_days = Some(7);
        assert!(child.validate().is_ok());
        assert_eq!(child.unit_kind(), CleanupUnitKind::ChildNamespace);

        let mut provider = base("test.valid.provider");
        provider.strategy = CleanStrategy::ExternalCommand;
        provider.paths = vec![];
        provider.provider = "example".to_string();
        assert!(provider.validate().is_ok());
        assert_eq!(provider.unit_kind(), CleanupUnitKind::ProviderAction);
        assert!(provider.ownership().is_known());
    }

    /// Discovery and eligibility are separate decisions, and the registry order
    /// is deterministic: two signatures that describe the same location must
    /// not depend on a map's iteration order.
    #[test]
    fn discovery_and_eligibility_are_decided_separately_and_in_order() {
        let mut registry = SignatureRegistry::new();
        let mut broad = test_signature(
            "test.scope.broad",
            vec!["/tmp/cache"],
            vec![PlatformKind::Macos, PlatformKind::Windows],
        );
        broad.intensive_only = true;
        broad.priority = 1;
        let mut specific = test_signature(
            "test.scope.specific",
            vec!["/tmp/cache"],
            vec![PlatformKind::Macos, PlatformKind::Windows],
        );
        specific.intensive_only = true;
        specific.priority = 9;
        let mut always = test_signature(
            "test.scope.always",
            vec!["/tmp/cache"],
            vec![PlatformKind::Macos, PlatformKind::Windows],
        );
        always.intensive_only = true;
        always.discovery = DiscoveryScope::Always;
        let always_gate = always.eligibility_gate(false);
        registry.register(broad);
        registry.register(specific);
        registry.register(always);

        let standard = registry.by_category_for_mode(Category::System, false);
        let ids: Vec<&str> = standard.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["test.scope.always"],
            "a mode-gated signature is not discovered, an always-on one is"
        );
        assert_eq!(
            always_gate,
            EligibilityGate::IntensiveCleanupDisabled,
            "the discovered unit reports the gate that kept it out of scope"
        );

        let intensive = registry.by_category_for_mode(Category::System, true);
        let ids: Vec<&str> = intensive.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "test.scope.specific",
                "test.scope.broad",
                "test.scope.always"
            ],
            "most specific first, then by id"
        );
        for signature in &intensive {
            assert_eq!(signature.eligibility_gate(true), EligibilityGate::Open);
        }
    }

    /// The scope decides discovery for a mode-gated signature and eligibility
    /// for one that opted into always-on discovery. A signature that is
    /// discovered outside its scope is reported with a gate, never silently
    /// cleaned.
    #[test]
    fn intensive_signatures_are_opt_in_or_discovered_with_a_gate() {
        let registry = SignatureRegistry::load_embedded().unwrap();

        let standard = registry.by_category_for_mode(Category::System, false);
        for signature in &standard {
            if signature.intensive_only {
                assert_eq!(
                    signature.discovery,
                    DiscoveryScope::Always,
                    "{} is discovered in a standard scan only because it says so",
                    signature.id
                );
                assert_eq!(
                    signature.eligibility_gate(false),
                    EligibilityGate::IntensiveCleanupDisabled,
                    "{} reports why it is out of scope",
                    signature.id
                );
            }
        }

        #[cfg(target_os = "macos")]
        {
            // The two generic user roots are the reason a default scan no
            // longer reports zero bytes for `~/Library/Caches`.
            let discovered_in_standard: Vec<&str> = standard
                .iter()
                .filter(|signature| signature.intensive_only)
                .map(|signature| signature.id.as_str())
                .collect();
            assert!(
                discovered_in_standard.contains(&"system.intensive.user_app_caches"),
                "the generic user cache root is inventoried in a standard scan: {discovered_in_standard:?}"
            );
            assert!(discovered_in_standard.contains(&"system.intensive.application_logs"));

            let intensive = registry.by_category_for_mode(Category::System, true);
            assert!(intensive.iter().any(|signature| signature.intensive_only));
            assert!(intensive.len() >= standard.len());
            for signature in &intensive {
                assert_eq!(
                    signature.eligibility_gate(true),
                    EligibilityGate::Open,
                    "{} is eligible once its scope is on",
                    signature.id
                );
            }
        }

        #[cfg(target_os = "windows")]
        {
            let intensive = registry.by_category_for_mode(Category::System, true);
            assert!(intensive.len() >= standard.len());
            for signature in intensive.iter().filter(|s| s.intensive_only) {
                assert_eq!(
                    signature.eligibility_gate(true),
                    EligibilityGate::Open,
                    "the opt-in scope is what makes {} eligible",
                    signature.id
                );
            }
        }
    }

    #[test]
    fn developer_temp_prefixes_keep_the_three_day_age_gate() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        let signature = registry.get("system.developer_temp").unwrap();

        for prefix in [
            "agent-browser-chrome-",
            "metro-cache",
            "metro-file-map-",
            "node-compile-cache",
            "openai-docs-cache",
            "pytest-of-",
            "v8-compile-cache-",
        ] {
            assert!(
                signature
                    .include_prefixes
                    .iter()
                    .any(|entry| entry == prefix),
                "missing reviewed prefix: {prefix}"
            );
        }
        assert_eq!(signature.min_age_days, Some(3));
        assert_eq!(signature.family, CleanerFamily::Developer);
        assert_eq!(signature.risk, RiskTier::Manual);
        assert_eq!(signature.strategy, CleanStrategy::Manual);
        assert_eq!(signature.unit_kind(), CleanupUnitKind::ChildNamespace);
    }

    #[test]
    fn embedded_catalog_is_partitioned_into_owned_families() {
        let registry = SignatureRegistry::load_embedded_catalog().unwrap();

        assert!(registry
            .all()
            .iter()
            .all(|signature| signature.family != CleanerFamily::Unclassified));
        assert!(registry
            .by_family(CleanerFamily::PackageManagers)
            .iter()
            .all(|signature| signature.category == Category::Developer));
        assert!(registry
            .by_family(CleanerFamily::Containers)
            .iter()
            .all(|signature| signature.category == Category::Container));
    }

    #[test]
    fn user_app_caches_exclude_tool_managed_dotslash_cache() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        let signature = registry.get("system.intensive.user_app_caches").unwrap();

        assert!(
            signature
                .exclude_prefixes
                .iter()
                .any(|prefix| prefix == "dotslash"),
            "dotslash cache must stay excluded: it is content-addressed and managed by the dotslash CLI"
        );
    }

    /// The shipped signature claims Apple/system/tool-managed namespaces are
    /// excluded; this pins every namespace it names, so the policy cannot drift
    /// away from the description the user reads.
    #[test]
    fn user_app_caches_exclude_tool_managed_and_apple_namespaces() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        let signature = registry.get("system.intensive.user_app_caches").unwrap();

        for prefix in [
            "com.apple.",
            "com.apple.CloudKit",
            "com.apple.FamilyCircle",
            "com.apple.GeoServices",
            "com.apple.HomeKit",
            "com.apple.Safari",
            "ms-playwright",
        ] {
            assert!(
                signature
                    .exclude_prefixes
                    .iter()
                    .any(|entry| prefix.starts_with(entry)),
                "missing excluded namespace: {prefix}"
            );
        }

        assert!(
            signature.description.contains("ms-playwright"),
            "the description must name the tool-managed Playwright cache it excludes"
        );
    }

    #[test]
    fn platform_specific_gpu_signatures_do_not_cross_platforms() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        let ai = registry.by_category_for_mode(Category::Ai, false);
        #[cfg(target_os = "macos")]
        {
            assert!(ai
                .iter()
                .any(|signature| signature.id == "ai.llamacpp.opencl.macos"));
            assert!(ai
                .iter()
                .all(|signature| signature.id != "ai.gpu.directx_shader"));
        }
        #[cfg(target_os = "windows")]
        {
            assert!(ai
                .iter()
                .any(|signature| signature.id == "ai.gpu.directx_shader"));
            assert!(ai
                .iter()
                .all(|signature| signature.id != "ai.llamacpp.opencl.macos"));
        }
    }

    #[test]
    fn shared_package_stores_are_never_generic_safe_deletions() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        for signature in registry
            .all()
            .into_iter()
            .filter(|signature| signature.artifact_kind == CacheArtifactKind::PackageStore)
        {
            assert_ne!(signature.risk, RiskTier::Safe, "{}", signature.id);
            assert!(
                matches!(
                    signature.strategy,
                    CleanStrategy::ExternalCommand
                        | CleanStrategy::LifecycleProvider
                        | CleanStrategy::OwnerProvider
                        | CleanStrategy::Manual
                ),
                "{} must be provider-owned or advisory, not generic filesystem cleanup",
                signature.id
            );
        }
        for id in [
            "dev.go.build",
            "dev.go.mod",
            "dev.uv.cache",
            "dev.pnpm.store",
            "dev.npm.cache",
            "dev.bun.cache",
            "dev.composer.cache",
        ] {
            assert_eq!(
                registry.get(id).unwrap().strategy,
                CleanStrategy::ExternalCommand,
                "{id}"
            );
        }
        // Yarn has no version-aware adapter yet, so it stays a manual target:
        // a `rebuild` risk with a `manual` strategy would be a plan the planner
        // refuses and the interface can still select.
        let yarn = registry.get("dev.yarn.cache").unwrap();
        assert_eq!(yarn.risk, RiskTier::Manual);
        assert_eq!(yarn.strategy, CleanStrategy::Manual);
        let rustup = registry.get("dev.rustup.downloads").unwrap();
        assert_eq!(rustup.risk, RiskTier::Manual);
        assert_eq!(rustup.strategy, CleanStrategy::Manual);
    }

    #[test]
    fn the_embedded_catalog_lints_clean() {
        let registry = SignatureRegistry::load_embedded_catalog().unwrap();
        let findings =
            SignatureRegistry::audit_signature_platforms(&registry, &stated_environment());
        assert!(
            findings.is_empty(),
            "embedded catalog lint findings: {findings:?}"
        );
    }

    #[test]
    fn the_embedded_catalog_survives_the_platform_gate() {
        // The gate must refuse nothing in the reviewed catalog, so the native
        // and simulated loads see the same signatures.
        let catalog = SignatureRegistry::load_embedded_catalog().unwrap();
        let gated = SignatureRegistry::load_embedded_with(&stated_environment()).unwrap();
        assert_eq!(gated.all().len(), catalog.all().len());
    }

    #[test]
    fn a_signature_resolving_to_platform_specific_root_must_declare_platforms() {
        let findings = audit_one(test_signature(
            "system.test.mac_only",
            vec!["~/Library/Caches/test", "~/.cache/test"],
            vec![],
        ));
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].platform.as_deref(), Some("macos"));

        // Declaring the platform is accepted.
        let declared = audit_one(test_signature(
            "system.test.mac_only",
            vec!["~/Library/Caches/test"],
            vec![PlatformKind::Macos],
        ));
        assert!(declared.is_empty(), "{declared:?}");

        // Declaring the wrong platform is not.
        let wrong = audit_one(test_signature(
            "system.test.mac_only",
            vec!["~/Library/Caches/test"],
            vec![PlatformKind::Windows],
        ));
        assert_eq!(wrong.len(), 1, "{wrong:?}");
        assert_eq!(wrong[0].platform.as_deref(), Some("macos"));
    }

    #[test]
    fn the_platform_gate_refuses_an_undeclared_signature() {
        let environment = stated_environment();
        let mut registry = SignatureRegistry::new();
        registry.register(test_signature(
            "system.test.mac_only",
            vec!["~/Library/Caches/test"],
            vec![],
        ));
        registry.register(test_signature(
            "system.test.shared",
            vec!["~/.cache/test"],
            vec![],
        ));

        let findings = SignatureRegistry::audit_signature_platforms(&registry, &environment);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].signature_id, "system.test.mac_only");

        registry.enforce_platform_declarations(&environment);
        // The refusal applies to the offending signature only: an undeclared
        // platform root is never silently accepted, and nothing else is lost.
        assert!(registry.get("system.test.mac_only").is_none());
        assert!(registry.get("system.test.shared").is_some());
        assert!(SignatureRegistry::audit_signature_platforms(&registry, &environment).is_empty());
    }

    #[test]
    fn a_literal_relative_path_is_a_lint_finding() {
        let findings = audit_one(test_signature(
            "system.test.relative",
            vec!["relative/cache"],
            vec![],
        ));
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].platform, None);
        assert!(findings[0].message.contains("absolute"), "{findings:?}");
    }

    #[test]
    fn an_unresolvable_placeholder_is_not_a_lint_finding() {
        // The environment states no program files root, so the pattern simply
        // does not exist here; that is not a manifest defect.
        let findings = audit_one(test_signature(
            "system.test.program_files",
            vec!["${PROGRAM_FILES}/test"],
            vec![PlatformKind::Windows],
        ));
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn an_exclusion_outside_the_signature_scope_is_a_lint_finding() {
        let mut registry = SignatureRegistry::new();
        let mut signature =
            test_signature("system.test.exclusion", vec!["~/.cache/test/logs"], vec![]);
        signature.exclusions = vec!["~/Documents/keep".to_string()];
        registry.register(signature);

        let findings =
            SignatureRegistry::audit_signature_platforms(&registry, &stated_environment());
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].message.contains("scope"), "{findings:?}");

        // A common ancestor does not make a sibling reachable from either root.
        let mut registry = SignatureRegistry::new();
        let mut signature = test_signature(
            "system.test.exclusion",
            vec!["~/.cache/test/logs", "~/.cache/test/tmp"],
            vec![],
        );
        signature.exclusions = vec!["~/.cache/test/settings.json".to_string()];
        registry.register(signature);
        let findings =
            SignatureRegistry::audit_signature_platforms(&registry, &stated_environment());
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("scope"));
    }

    #[test]
    fn resolved_paths_follow_the_stated_environment() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Windows)
            .with_roots(Arc::new(
                SimulatedPaths::new()
                    .with_flavor(PathFlavor::Windows)
                    .with_home(r"C:\Users\me")
                    .with_local_app_data(r"C:\Users\me\AppData\Local"),
            ))
            .with_known_folder(KnownFolder::Documents, r"D:\Cache\Documents");
        let registry = SignatureRegistry::load_embedded().unwrap();
        let signature = test_signature(
            "system.test.paths",
            vec!["~/Documents/cache", "${LOCAL_APP_DATA}/test"],
            vec![],
        );
        // The registry stores nothing environment-dependent: the same instance
        // resolves differently for a different environment.
        assert_eq!(
            registry.resolve_paths(&signature, &environment),
            vec![
                PathBuf::from(r"D:\Cache\Documents\cache"),
                PathBuf::from(r"C:\Users\me\AppData\Local\test"),
            ]
        );
    }

    /// An opt-in signature is only offered where an adapter exists, and every
    /// one of them declares the platform it runs on.
    #[test]
    fn intensive_signatures_declare_a_supported_platform() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        let intensive: Vec<_> = registry
            .all()
            .into_iter()
            .filter(|signature| signature.intensive_only)
            .collect();
        assert!(
            !intensive.is_empty(),
            "the broader scope has at least one signature to widen"
        );
        for signature in &intensive {
            assert!(
                signature.platforms.contains(&PlatformKind::Macos)
                    || signature.platforms.contains(&PlatformKind::Windows),
                "{} is offered on a platform Zenith ships",
                signature.id
            );
        }
        // Both shipped platforms now have intensive coverage: enabling the
        // scope changes what a `%TEMP%`-side scan reports on Windows, and what
        // a `~/Library/Caches`-side scan reports on macOS.
        for platform in [PlatformKind::Macos, PlatformKind::Windows] {
            assert!(
                intensive
                    .iter()
                    .any(|signature| signature.platforms.contains(&platform)),
                "{platform:?} has at least one intensive signature"
            );
        }
    }

    #[test]
    fn the_lint_output_is_sorted_and_deterministic() {
        // HashMap iteration order must not leak into the lint output.
        let environment = stated_environment();
        let mut registry = SignatureRegistry::new();
        for (id, paths) in [
            ("system.test.a", vec!["~/Library/Caches/a"]),
            ("system.test.b", vec!["relative/b"]),
            ("system.test.c", vec!["~/Library/Caches/c"]),
        ] {
            registry.register(test_signature(id, paths, vec![]));
        }
        let ids: Vec<String> =
            SignatureRegistry::audit_signature_platforms(&registry, &environment)
                .into_iter()
                .map(|finding| finding.signature_id)
                .collect();
        assert_eq!(
            ids,
            vec![
                "system.test.a".to_string(),
                "system.test.b".to_string(),
                "system.test.c".to_string(),
            ]
        );
    }
}
