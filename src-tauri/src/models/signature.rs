use crate::models::{
    CacheArtifactKind, CacheManagementMode, CacheMetadata, CacheSizeSemantics,
    CacheUsageConfidence, Category, CleanStrategy, CleanupOwnership, CleanupUnitKind,
    EligibilityGate, PlatformKind, RiskTier, RunningProcessPolicy, ZenithError,
};
use serde::{Deserialize, Serialize};

/// Whether a signature's scope has to be enabled before it is discovered.
///
/// Discovery and deletion permission are decided separately. A signature whose
/// discovery is [`Self::ModeGated`] is only looked at when its scope is on; a
/// signature that opts into [`Self::Always`] is always inventoried, and the
/// scope decides whether the units it finds are eligible. Both states are
/// declared by the catalog, so a wider inventory is a deliberate statement
/// about a signature rather than a side effect of a settings toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryScope {
    /// Discovered only while the signature's scope is enabled.
    #[default]
    ModeGated,
    /// Always discovered; the scope decides only eligibility.
    Always,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    pub id: String,
    pub name: String,
    pub category: Category,
    pub risk: RiskTier,
    pub strategy: CleanStrategy,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub exclusions: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub min_age_days: Option<u32>,
    #[serde(default)]
    pub include_prefixes: Vec<String>,
    #[serde(default)]
    pub exclude_prefixes: Vec<String>,
    #[serde(default)]
    pub intensive_only: bool,
    #[serde(default)]
    pub platforms: Vec<PlatformKind>,
    #[serde(default)]
    pub provider: String,
    /// The lifecycle provider that performs this signature's action.
    ///
    /// Declared only by a signature whose strategy is
    /// [`CleanStrategy::LifecycleProvider`]: the provider is the operation, and
    /// the catalog names it so a manifest cannot reach a code path the catalog
    /// never named. Empty for every other strategy.
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub artifact_kind: CacheArtifactKind,
    #[serde(default)]
    pub consequence: String,
    /// Whether this signature is discovered outside its own scope.
    #[serde(default)]
    pub discovery: DiscoveryScope,
    /// What the signature's deletable object is, when the strategy and the age
    /// policy do not already determine it. Declared only to state something the
    /// derivation cannot: a named cache subtree under an application directory.
    #[serde(default)]
    pub unit: Option<CleanupUnitKind>,
    /// Who the catalog expects to own the locations this signature names.
    ///
    /// Stated rather than guessed: when it is empty, ownership falls back to
    /// the declared provider, and the confidence records which of the two
    /// happened so a message never presents an inference as a fact.
    #[serde(default)]
    pub owner: String,
    /// Specificity: the larger value wins when two signatures claim the same
    /// unit, so the outcome does not depend on map iteration order.
    #[serde(default)]
    pub priority: i32,
    /// Executables whose running state makes cleaning this signature's unit
    /// unsafe (a compiler or runtime holding the cache open).
    #[serde(default)]
    pub fail_if_running: Vec<String>,
}

impl Signature {
    pub fn cache_metadata(&self) -> CacheMetadata {
        CacheMetadata {
            provider: if self.provider.is_empty() {
                "Zenith".to_string()
            } else {
                self.provider.clone()
            },
            management_mode: match self.strategy {
                CleanStrategy::Manual => CacheManagementMode::Advisory,
                CleanStrategy::ExternalCommand
                | CleanStrategy::LifecycleProvider
                | CleanStrategy::OwnerProvider
                | CleanStrategy::DockerPrune => CacheManagementMode::ToolManaged,
                CleanStrategy::DeleteContents
                | CleanStrategy::DeleteDirectory
                | CleanStrategy::DeleteStaleContents => CacheManagementMode::Zenith,
            },
            artifact_kind: self.artifact_kind,
            consequence: self.consequence.clone(),
            size_semantics: match self.strategy {
                CleanStrategy::Manual => CacheSizeSemantics::Informational,
                CleanStrategy::ExternalCommand
                | CleanStrategy::LifecycleProvider
                | CleanStrategy::OwnerProvider
                | CleanStrategy::DockerPrune => CacheSizeSemantics::ConservativeLowerBound,
                CleanStrategy::DeleteContents
                | CleanStrategy::DeleteDirectory
                | CleanStrategy::DeleteStaleContents => CacheSizeSemantics::PhysicalReclaimable,
            },
            last_used_confidence: if self.min_age_days.is_some() {
                CacheUsageConfidence::Approximate
            } else {
                CacheUsageConfidence::Unknown
            },
        }
    }

    /// Who the catalog expects to own this signature's locations.
    pub fn ownership(&self) -> CleanupOwnership {
        if !self.owner.is_empty() {
            CleanupOwnership::declared(self.owner.clone())
        } else if !self.provider.is_empty() {
            CleanupOwnership::inferred(self.provider.clone())
        } else {
            CleanupOwnership::unknown()
        }
    }

    /// The cleanup-unit granularity of this signature.
    ///
    /// A declared granularity wins; otherwise it follows from what the
    /// signature's strategy and age policy already imply. Deriving rather than
    /// requiring keeps the catalog from restating a fact twice and then
    /// disagreeing with itself: a provider signature owns no path, a container
    /// prune owns no path, and an age policy that ages children means the child
    /// is the unit.
    pub fn unit_kind(&self) -> CleanupUnitKind {
        if let Some(declared) = self.unit {
            return declared;
        }
        match self.strategy {
            CleanStrategy::DockerPrune => CleanupUnitKind::ContainerResource,
            CleanStrategy::ExternalCommand
            | CleanStrategy::LifecycleProvider
            | CleanStrategy::OwnerProvider => CleanupUnitKind::ProviderAction,
            CleanStrategy::Manual => CleanupUnitKind::FixedPath,
            CleanStrategy::DeleteContents
            | CleanStrategy::DeleteDirectory
            | CleanStrategy::DeleteStaleContents => {
                if self.min_age_days.is_some() {
                    CleanupUnitKind::ChildNamespace
                } else {
                    CleanupUnitKind::FixedPath
                }
            }
        }
    }

    /// The unit granularity the strategy alone implies, used to reject a
    /// catalog that declares a granularity the strategy cannot honor.
    fn strategy_unit_kind(&self) -> CleanupUnitKind {
        match self.strategy {
            CleanStrategy::DockerPrune => CleanupUnitKind::ContainerResource,
            CleanStrategy::ExternalCommand
            | CleanStrategy::LifecycleProvider
            | CleanStrategy::OwnerProvider => CleanupUnitKind::ProviderAction,
            CleanStrategy::Manual => CleanupUnitKind::FixedPath,
            CleanStrategy::DeleteContents
            | CleanStrategy::DeleteDirectory
            | CleanStrategy::DeleteStaleContents => CleanupUnitKind::FixedPath,
        }
    }

    /// Refuses a signature whose cleanup knowledge contradicts itself.
    ///
    /// These are catalog invariants, not user preferences: a signature that
    /// enumerates children without an age policy would delete active state, and
    /// one that claims a non-filesystem unit while declaring filesystem paths
    /// describes an operation the executor cannot carry out. Both are load-time
    /// errors so a bad manifest cannot reach a scan.
    pub fn validate(&self) -> Result<(), ZenithError> {
        let invalid = |message: String| {
            Err(ZenithError::InvalidPlan(format!(
                "Signature `{}` is invalid: {message}",
                self.id
            )))
        };

        if self.id.trim().is_empty() {
            return invalid("the id is empty".to_string());
        }

        let kind = self.unit_kind();
        if matches!(
            kind,
            CleanupUnitKind::ChildNamespace | CleanupUnitKind::NamedSubtree
        ) && self.min_age_days.is_none()
        {
            return invalid(format!(
                "unit `{}` requires an age policy (`min_age_days`)",
                kind.display_name()
            ));
        }

        if self.min_age_days.is_some() {
            match kind {
                CleanupUnitKind::FixedPath => {
                    return invalid(
                        "an age policy ages enumerated units, but the declared unit is the                          configured path itself"
                            .to_string(),
                    );
                }
                CleanupUnitKind::ProviderAction | CleanupUnitKind::ContainerResource => {
                    return invalid(format!(
                        "unit `{}` cannot carry an age policy",
                        kind.display_name()
                    ));
                }
                CleanupUnitKind::ChildNamespace | CleanupUnitKind::NamedSubtree => {}
            }
        }

        // A strategy that owns no host path and a declared unit that names one
        // describe different operations; either pairing is a catalog error.
        let strategy_kind = self.strategy_unit_kind();
        if (!kind.is_filesystem() || !strategy_kind.is_filesystem()) && kind != strategy_kind {
            return invalid(format!(
                "unit `{}` cannot be carried out by strategy {:?} (which owns `{}`)",
                kind.display_name(),
                self.strategy,
                strategy_kind.display_name()
            ));
        }

        if !kind.is_filesystem() && !self.paths.is_empty() {
            return invalid(format!(
                "unit `{}` owns no host path, but the signature declares paths",
                kind.display_name()
            ));
        }

        // A selector is allowed in `paths` only: the scanner enumerates roots
        // from it, while an exclusion or a child prefix is matched against a
        // concrete name, where `*` would match nothing and silently fail to
        // protect what it names.
        for exclusion in &self.exclusions {
            if zenith_platform::selector::contains_selector_syntax(exclusion) {
                return invalid(format!(
                    "exclusion `{exclusion}` contains selector syntax; name the path or the entry instead"
                ));
            }
        }
        for prefix in self
            .include_prefixes
            .iter()
            .chain(self.exclude_prefixes.iter())
        {
            if zenith_platform::selector::contains_selector_syntax(prefix) {
                return invalid(format!(
                    "prefix `{prefix}` contains selector syntax; a prefix is a literal name fragment"
                ));
            }
        }

        for pattern in &self.paths {
            if pattern.contains('%') {
                return invalid(format!(
                    "path `{pattern}` uses an environment spelling this build does not resolve; \
                     write it as ${{...}} (`${{USER_HOME}}`, `${{LOCAL_APP_DATA}}`, \
                     `${{ROAMING_APP_DATA}}`, `${{PROGRAM_FILES}}`, `${{PROGRAM_DATA}}`, \
                     `${{TEMP}}`, `${{SYSTEM_ROOT}}`, `${{DARWIN_USER_CACHE}}`) or as an absolute path"
                ));
            }
            if let Err(reason) = zenith_platform::selector::validate_pattern_syntax(pattern) {
                return invalid(reason);
            }
        }

        if self.strategy == CleanStrategy::DeleteStaleContents {
            if self.min_age_days.is_none() {
                return invalid(
                    "a stale-entry cleanup needs an age policy: without one it removes every \
                     entry it finds"
                        .to_string(),
                );
            }
            if !matches!(kind, CleanupUnitKind::ChildNamespace) {
                return invalid(format!(
                    "a stale-entry cleanup ages the entries inside a discovered unit, but the \
                     declared unit is `{}`",
                    kind.display_name()
                ));
            }
        }

        for executable in &self.fail_if_running {
            if executable.trim().is_empty() {
                return invalid("`fail_if_running` contains an empty executable name".to_string());
            }
            if executable.contains('/') || executable.contains('\\') {
                return invalid(format!(
                    "`fail_if_running` entry `{executable}` is a path, not an executable name"
                ));
            }
        }

        // A provider action is exactly one reviewed operation, and the catalog
        // is where that operation is named. Declaring the id without the
        // strategy (or the strategy without an id) describes a target whose
        // carrying-out would have to be guessed, so both are refused.
        let names_a_provider = self
            .provider_id
            .as_deref()
            .is_some_and(|provider_id| !provider_id.trim().is_empty());
        match self.strategy {
            CleanStrategy::LifecycleProvider => {
                if !names_a_provider {
                    return invalid(
                        "a lifecycle provider action must name the provider that performs it \
                         (`provider_id`)"
                            .to_string(),
                    );
                }
                if self.platforms.is_empty() {
                    return invalid(
                        "a lifecycle provider action must declare the platforms it runs on"
                            .to_string(),
                    );
                }
            }
            CleanStrategy::OwnerProvider => {
                if !names_a_provider {
                    return invalid(
                        "an owner-scoped provider entry must name the provider that enumerates \
                         and removes its units (`provider_id`)"
                            .to_string(),
                    );
                }
                if self.platforms.is_empty() {
                    return invalid(
                        "an owner-scoped provider entry must declare the platforms it runs on"
                            .to_string(),
                    );
                }
                // The provider consults the process table before it reads or
                // mutates its store, and the executables it consults are the
                // ones the catalog names. An entry that names none would ask a
                // provider to prove its owner is idle with nothing to look for.
                if self.fail_if_running.is_empty() {
                    return invalid(
                        "an owner-scoped provider entry must declare the executables whose \
                         running state makes its store unsafe to touch (`fail_if_running`)"
                            .to_string(),
                    );
                }
            }
            _ => {
                if self.provider_id.is_some() {
                    return invalid(format!(
                        "`provider_id` is declared but strategy {:?} is not a reviewed provider action",
                        self.strategy
                    ));
                }
            }
        }

        Ok(())
    }

    /// Whether this signature is discovered under the current scope.
    pub fn discovery_allows(&self, intensive_cleanup: bool) -> bool {
        match self.discovery {
            DiscoveryScope::Always => true,
            DiscoveryScope::ModeGated => intensive_cleanup || !self.intensive_only,
        }
    }

    /// The scan-policy gate that applies to every unit this signature finds.
    pub fn eligibility_gate(&self, intensive_cleanup: bool) -> EligibilityGate {
        if self.intensive_only && !intensive_cleanup {
            EligibilityGate::IntensiveCleanupDisabled
        } else {
            EligibilityGate::Open
        }
    }

    /// The process guard the execution boundary applies to this signature.
    pub fn process_guard(&self) -> RunningProcessPolicy {
        RunningProcessPolicy::guarding(self.fail_if_running.clone())
    }

    pub fn supports_current_platform(&self) -> bool {
        self.platforms.is_empty() || self.platforms.contains(&PlatformKind::current())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SignatureManifest {
    #[serde(default)]
    pub signatures: Vec<Signature>,
}
