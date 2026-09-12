use crate::models::{Category, PlatformKind, RiskTier, Signature, ZenithError};
use crate::platform::path_algebra::{contains, is_absolute, normalize, PathFlavor};
use crate::platform::PlatformEnvironment;
use crate::safety::Blacklist;
use crate::signatures::SignatureLoader;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

const EMBEDDED_AI_TOML: &str = include_str!("../../../signatures/ai.toml");
const EMBEDDED_DEV_TOML: &str = include_str!("../../../signatures/developer.toml");
const EMBEDDED_CONTAINERS_TOML: &str = include_str!("../../../signatures/containers.toml");
const EMBEDDED_MODELS_TOML: &str = include_str!("../../../signatures/models.toml");
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

    /// Loads built-in embedded signatures for AI, Developer, Container, and Model categories.
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

    /// Loads the embedded catalog without the platform-declaration gate. The
    /// manifest lint and the environment doctor must see an offending
    /// signature in order to report it, so they load through this entry point.
    pub fn load_embedded_catalog() -> Result<Self, ZenithError> {
        let mut registry = Self::new();

        let tomls = [
            EMBEDDED_AI_TOML,
            EMBEDDED_DEV_TOML,
            EMBEDDED_CONTAINERS_TOML,
            EMBEDDED_MODELS_TOML,
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

    /// Loads signatures from a directory containing `.toml` files.
    pub fn load_from_dir<P: AsRef<Path>>(&mut self, dir_path: P) -> Result<usize, ZenithError> {
        self.load_from_dir_with(dir_path, &PlatformEnvironment::native())
    }

    /// Loads signatures from a directory, refusing manifests that resolve under
    /// a platform-specific root without declaring it.
    pub fn load_from_dir_with<P: AsRef<Path>>(
        &mut self,
        dir_path: P,
        environment: &PlatformEnvironment,
    ) -> Result<usize, ZenithError> {
        let dir = dir_path.as_ref();
        if !dir.exists() || !dir.is_dir() {
            return Ok(0);
        }

        let mut count = 0;
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                    if let Ok(sigs) = SignatureLoader::load_file(&path) {
                        for sig in sigs {
                            self.register(sig);
                            count += 1;
                        }
                    }
                }
            }
        }

        self.enforce_platform_declarations(environment);
        Ok(count)
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

    /// Lists signatures available for the selected scan scope.
    pub fn by_category_for_mode(
        &self,
        category: Category,
        intensive_cleanup: bool,
    ) -> Vec<&Signature> {
        self.signatures
            .values()
            .filter(|signature| {
                signature.category == category
                    && signature.supports_current_platform()
                    && (intensive_cleanup || !signature.intensive_only)
            })
            .collect()
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

    /// The pure manifest lint the environment doctor and CI both run.
    ///
    /// Four invariants are checked for every signature, against the described
    /// environment:
    ///
    /// * every path pattern resolves to an absolute path (a literal relative
    ///   pattern is a finding; a dynamic pattern the environment cannot resolve
    ///   is not, since "not present here" is a legitimate answer);
    /// * no resolved path is blacklisted — except for a signature that selects
    ///   aged children inside its root, which never deletes the root itself;
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

    /// Returns which platform (if any) a path pattern or placeholder is specifically tied to.
    pub fn is_platform_specific_path(pattern: &str) -> Option<PlatformKind> {
        let p = pattern.trim();
        if p.starts_with("~/Library")
            || p.starts_with("/Applications")
            || p.starts_with("/Library")
            || p.starts_with("/System")
            || p.starts_with("/private/")
        {
            Some(PlatformKind::Macos)
        } else if p.contains("${LOCAL_APP_DATA}")
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

    for pattern in &signature.paths {
        if pattern.trim().is_empty() {
            continue;
        }
        match SignatureLoader::expand_path(pattern, environment) {
            Some(path) => {
                if !is_absolute(&path.to_string_lossy(), flavor) {
                    findings.push(finding(
                        signature,
                        None,
                        format!("path pattern `{pattern}` did not resolve to an absolute path"),
                    ));
                }
                if signature.min_age_days.is_none() {
                    if let Some(root) = crate::platform::path_algebra::protected_root(
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
                    } else if Blacklist::is_blacklisted(&path) {
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

    if let Some(scope) = common_scope(&resolved, flavor) {
        for exclusion in &signature.exclusions {
            if !is_path_shaped(exclusion) {
                continue;
            }
            let Some(expanded) = SignatureLoader::expand_path(exclusion, environment) else {
                continue;
            };
            let expanded_text = expanded.to_string_lossy().into_owned();
            let within_path = resolved
                .iter()
                .any(|path| contains(&path.to_string_lossy(), &expanded_text, flavor));
            if !within_path && !contains(&scope, &expanded_text, flavor) {
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

/// An exclusion is path-shaped when the walkers treat it as a path rather than
/// as a bare file name.
fn is_path_shaped(exclusion: &str) -> bool {
    exclusion.starts_with('~') || exclusion.starts_with('/') || exclusion.contains("${")
}

fn platform_token(platform: PlatformKind) -> &'static str {
    match platform {
        PlatformKind::Macos => "macos",
        PlatformKind::Windows => "windows",
        PlatformKind::Linux => "linux",
        PlatformKind::Other => "other",
    }
}

/// The deepest directory the resolved paths share, or `None` when they share
/// no root (different drives, a UNC share beside a local path). Containment in
/// the scope is component-boundary checked, so `C:\Program Files (x86)` does
/// not contain `C:\Program Files`.
fn common_scope(paths: &[PathBuf], flavor: PathFlavor) -> Option<String> {
    let mut scope = path_parts(&paths.first()?.to_string_lossy(), flavor);
    for path in &paths[1..] {
        let parts = path_parts(&path.to_string_lossy(), flavor);
        if parts.root != scope.root {
            return None;
        }
        let shared = scope
            .components
            .iter()
            .zip(parts.components.iter())
            .take_while(|(left, right)| {
                crate::platform::path_algebra::fold(left, flavor)
                    == crate::platform::path_algebra::fold(right, flavor)
            })
            .count();
        scope.components.truncate(shared);
    }
    let separator = flavor.separator();
    if scope.components.is_empty() {
        return Some(scope.root);
    }
    let mut text = scope.root;
    if !text.ends_with(separator) {
        text.push(separator);
    }
    text.push_str(&scope.components.join(&separator.to_string()));
    Some(text)
}

struct PathParts {
    root: String,
    components: Vec<String>,
}

fn path_parts(path: &str, flavor: PathFlavor) -> PathParts {
    let normalized = normalize(path, flavor);
    if !flavor.is_windows() {
        return PathParts {
            root: flavor.separator().to_string(),
            components: normalized
                .split('/')
                .filter(|component| !component.is_empty())
                .map(str::to_string)
                .collect(),
        };
    }
    if let Some(rest) = normalized.strip_prefix(r"\\") {
        let mut parts = rest.splitn(3, '\\');
        let server = parts.next().unwrap_or_default();
        let share = parts.next().unwrap_or_default();
        let remainder = parts.next().unwrap_or_default();
        let root = if share.is_empty() {
            format!(r"\\{server}")
        } else {
            format!(r"\\{server}\{share}")
        };
        return PathParts {
            root,
            components: remainder
                .split('\\')
                .filter(|component| !component.is_empty())
                .map(str::to_string)
                .collect(),
        };
    }
    let chars: Vec<char> = normalized.chars().collect();
    if chars.len() >= 2 && chars[1] == ':' {
        return PathParts {
            root: normalized[..2].to_string(),
            components: normalized[2..]
                .split('\\')
                .filter(|component| !component.is_empty())
                .map(str::to_string)
                .collect(),
        };
    }
    PathParts {
        root: String::new(),
        components: normalized
            .split('\\')
            .filter(|component| !component.is_empty())
            .map(str::to_string)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ManifestLintFinding, SignatureRegistry};
    use crate::models::{Category, PlatformKind, RiskTier, Signature};
    use crate::platform::path_algebra::PathFlavor;
    use crate::platform::paths::SimulatedPaths;
    use crate::platform::{KnownFolder, PlatformEnvironment};
    use std::path::PathBuf;
    use std::sync::Arc;

    /// A POSIX environment with stated roots, so the catalog lint below is
    /// checked against paths that exist on any runner.
    fn stated_environment() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::Posix).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::Posix)
                .with_home("/home/tester")
                .with_local_app_data("/home/tester/.local/share")
                .with_roaming_app_data("/home/tester/.config")
                .with_temp_dir("/tmp"),
        ))
    }

    fn test_signature(id: &str, paths: Vec<&str>, platforms: Vec<PlatformKind>) -> Signature {
        Signature {
            id: id.to_string(),
            name: id.to_string(),
            category: Category::System,
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
            provider: String::new(),
            management_mode: Default::default(),
            artifact_kind: Default::default(),
            consequence: String::new(),
            reclaimable_is_lower_bound: false,
        }
    }

    fn audit_one(signature: Signature) -> Vec<ManifestLintFinding> {
        let mut registry = SignatureRegistry::new();
        registry.register(signature);
        SignatureRegistry::audit_signature_platforms(&registry, &stated_environment())
    }

    #[test]
    fn intensive_signatures_are_opt_in() {
        let registry = SignatureRegistry::load_embedded().unwrap();

        let standard = registry.by_category_for_mode(Category::System, false);
        assert!(standard.iter().all(|signature| !signature.intensive_only));

        #[cfg(target_os = "macos")]
        {
            let intensive = registry.by_category_for_mode(Category::System, true);
            assert!(intensive.iter().any(|signature| signature.intensive_only));
            assert!(intensive.len() > standard.len());
        }

        #[cfg(target_os = "windows")]
        {
            let intensive = registry.by_category_for_mode(Category::System, true);
            assert_eq!(intensive.len(), standard.len());
            assert!(intensive.iter().all(|signature| !signature.intensive_only));
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
        for id in [
            "dev.npm.cache",
            "dev.pnpm.store",
            "dev.yarn.cache",
            "dev.bun.cache",
            "dev.pip.cache",
            "dev.uv.cache",
            "dev.gradle.caches",
            "dev.m2.repository",
        ] {
            let signature = registry.get(id).unwrap();
            assert_ne!(signature.risk, RiskTier::Safe, "{id}");
        }
        assert_eq!(
            registry.get("dev.uv.cache").unwrap().strategy,
            crate::models::CleanStrategy::ExternalCommand
        );
        assert_eq!(
            registry.get("dev.pnpm.store").unwrap().strategy,
            crate::models::CleanStrategy::ExternalCommand
        );
        assert_eq!(
            registry.get("dev.npm.cache").unwrap().strategy,
            crate::models::CleanStrategy::ExternalCommand
        );
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

        // A sibling exclusion of a two-path signature stays inside its scope.
        let mut registry = SignatureRegistry::new();
        let mut signature = test_signature(
            "system.test.exclusion",
            vec!["~/.cache/test/logs", "~/.cache/test/tmp"],
            vec![],
        );
        signature.exclusions = vec!["~/.cache/test/settings.json".to_string()];
        registry.register(signature);
        assert!(
            SignatureRegistry::audit_signature_platforms(&registry, &stated_environment())
                .is_empty()
        );
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

    #[test]
    fn intensive_signatures_are_unavailable_on_windows() {
        let registry = SignatureRegistry::load_embedded().unwrap();
        let intensive_sigs: Vec<_> = registry
            .all()
            .into_iter()
            .filter(|s| s.intensive_only)
            .collect();
        assert!(
            !intensive_sigs.is_empty(),
            "Expected at least one intensive signature"
        );
        for sig in intensive_sigs {
            assert!(
                sig.platforms.contains(&crate::models::PlatformKind::Macos)
                    && !sig
                        .platforms
                        .contains(&crate::models::PlatformKind::Windows),
                "Intensive signature {} must not target Windows",
                sig.id
            );
        }
    }

    #[test]
    fn common_scope_uses_component_boundaries() {
        use super::common_scope;
        let paths = vec![
            PathBuf::from(r"C:\Program Files\One\cache"),
            PathBuf::from(r"C:\Program Files (x86)\One\cache"),
        ];
        let scope = common_scope(&paths, PathFlavor::Windows).unwrap();
        assert_eq!(scope, r"C:");
        let shared = vec![
            PathBuf::from(r"C:\Users\me\AppData\Local\x\a"),
            PathBuf::from(r"C:\Users\me\AppData\Local\x\b"),
        ];
        assert_eq!(
            common_scope(&shared, PathFlavor::Windows).unwrap(),
            r"C:\Users\me\AppData\Local\x"
        );
        let unrelated = vec![
            PathBuf::from(r"C:\a"),
            PathBuf::from(r"D:\a"),
            PathBuf::from(r"\\server\share\a"),
        ];
        assert_eq!(common_scope(&unrelated, PathFlavor::Windows), None);
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
