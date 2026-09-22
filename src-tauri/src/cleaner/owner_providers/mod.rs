//! Owner-scoped cleanup providers.
//!
//! A lifecycle-aware provider ([`crate::cleaner::providers`]) performs one
//! whole-store action: it owns the entire object, so a probe and an action are
//! all it can say. An owner-scoped provider owns a *store of units* — Cargo's
//! registry cache is one directory per registry index, each holding downloaded
//! archives — and it enumerates them, states which ones it may remove, and
//! re-derives both facts when it acts.
//!
//! The difference matters because of what a signature is: it identifies a
//! candidate owner and a scope, not a deletion permission. Nothing here turns a
//! registered signature into filesystem authority. A unit reaches a mutation
//! only through the provider that enumerated it, matched against the selection
//! the user made from the scan those very units produced.
//!
//! What every provider must answer, and what none of them may do:
//!
//! * **scan** — resolve its own root from the [`PlatformEnvironment`], read the
//!   store, and report every unit it found with the bytes a measurement
//!   observed. A unit it cannot verify is reported as blocked, never dropped;
//! * **prepare** — re-read the store, match the selection against what it found
//!   now, and capture an identity for each unit it authorizes. A selection it
//!   cannot verify is refused by name;
//! * **execute** — re-verify each authorized unit and each entry inside it
//!   immediately before removing it, and report what its own verification
//!   observed afterwards.
//!
//! None of them falls back: a provider that fails reports its own status, and
//! it cannot degrade into a filesystem delete, because the operation it was
//! classified into carries no path to delete through and the registry refuses
//! an id that no build implements.

pub mod cargo;

use crate::models::{
    derive_cleanup_disposition, CacheManagementMode, CacheSizeSemantics, CleanStrategy,
    CleanupUnit, CleanupUnitKind, DispositionFacts, EligibilityGate, EntryKind, FileSize,
    ObservationQuality, OwnerProviderRefusal, OwnerProviderSelection, OwnerProviderUnit,
    OwnerStoreObservation, OwnerUnitObservation, OwnerUnitState, PlatformKind, ScanItem, Signature,
};
use crate::signatures::SignatureRegistry;
use std::collections::BTreeMap;
use std::sync::Arc;
use zenith_core::domain::cleanup::{
    CleanFailureReason, OwnerProviderAuthorization, OwnerProviderExecution, OwnerUnitRefusal,
};
use zenith_platform::PlatformEnvironment;

/// One reviewed provider of an owner-managed store.
///
/// `scan`, `prepare`, and `execute` all take the environment for the same
/// reason every other layer does: a provider that names a root must resolve it
/// through the environment the scan was produced with, never through the
/// running process's own profile. The process guard travels in as an argument
/// rather than being read from the process table here, because the catalog is
/// what states which executables make a store unsafe to touch.
pub trait OwnerScopedProvider: Send + Sync {
    /// The stable id the catalog names, unique within this build.
    fn id(&self) -> &'static str;

    /// The platforms this build implements the provider for.
    fn platforms(&self) -> &'static [PlatformKind];

    /// What removing one of this provider's units does, in the words the user
    /// is shown before it runs. It states the consequence, not the benefit.
    fn consequence(&self) -> &'static str;

    /// Whether removing one of its units needs explicit confirmation.
    fn requires_confirmation(&self) -> bool;

    /// Reads the store and reports every unit it owns.
    ///
    /// A guard whose executables are running, or whose state cannot be read,
    /// refuses the whole store: a provider that cannot prove its owner is idle
    /// must not enumerate a store its owner may be writing.
    fn scan(
        &self,
        environment: &PlatformEnvironment,
        guard: &zenith_core::domain::cleanup::RunningProcessPolicy,
    ) -> OwnerStoreObservation;

    /// Builds the private authorization for the selected units.
    ///
    /// The selection is a claim about the scan, not authority: the provider
    /// re-reads its store and authorizes only what it can verify now. Selections
    /// it refuses are named in the returned error rather than silently dropped.
    fn prepare(
        &self,
        environment: &PlatformEnvironment,
        guard: &zenith_core::domain::cleanup::RunningProcessPolicy,
        selections: &[OwnerProviderSelection],
    ) -> Result<OwnerProviderAuthorization, OwnerProviderRefusal>;

    /// Performs the authorized removals and verifies each one.
    fn execute(
        &self,
        environment: &PlatformEnvironment,
        authorization: &OwnerProviderAuthorization,
    ) -> OwnerProviderExecution;
}

/// The owner-scoped providers this build implements, keyed by the id the
/// catalog names.
///
/// Built once at the composition root and shared by the scan (which enumerates
/// units) and the executor (which performs the reviewed removal), so the two
/// cannot disagree about which providers exist or what they own.
pub struct OwnerProviderRegistry {
    providers: BTreeMap<&'static str, Arc<dyn OwnerScopedProvider>>,
}

impl OwnerProviderRegistry {
    /// The providers a user's installation actually ships.
    ///
    /// The ports they need are injected rather than reached for: the process
    /// table belongs to `cleaner::process_guard`, the allocated-size
    /// measurement to `scanner::size`, and neither is re-implemented here.
    pub fn native(
        process: Arc<dyn zenith_core::domain::cleanup::RunningProcessProbe>,
        measuring: Arc<dyn zenith_core::domain::cleanup::OwnerUnitMeasurer>,
    ) -> Self {
        Self::new(vec![
            Arc::new(cargo::CargoRegistryArchiveProvider::new(
                process.clone(),
                measuring.clone(),
            )),
            Arc::new(cargo::CargoRegistrySourceProvider::new(
                process.clone(),
                measuring.clone(),
            )),
            Arc::new(cargo::CargoGitProvider::new(process, measuring)),
        ])
    }

    /// A registry over an explicit provider list, for tests and adapters.
    pub fn new(providers: Vec<Arc<dyn OwnerScopedProvider>>) -> Self {
        let mut map = BTreeMap::new();
        for provider in providers {
            let previous = map.insert(provider.id(), provider);
            debug_assert!(
                previous.is_none(),
                "two owner providers registered the same id; the catalog could not tell them apart"
            );
        }
        Self { providers: map }
    }

    /// The ids this build implements, in a deterministic order.
    pub fn implemented_ids(&self) -> Vec<&'static str> {
        self.providers.keys().copied().collect()
    }

    pub fn get(&self, provider_id: &str) -> Option<&Arc<dyn OwnerScopedProvider>> {
        self.providers.get(provider_id)
    }

    /// Enumerates the units the catalog's owner-provider entries cover.
    ///
    /// Every fact on an item comes from a stated source: the catalog owns the
    /// name, category, risk tier, owner, and description; the provider owns the
    /// units, the bytes its measurement observed, and the consequence text. No
    /// unit is ever auto-selected — the disposition decides that, and a
    /// provider's units are never generic cleanup's to remove.
    pub fn scan_items(
        &self,
        registry: &SignatureRegistry,
        category: crate::models::Category,
        intensive_cleanup: bool,
        excluded_signatures: &[String],
        environment: &PlatformEnvironment,
    ) -> Vec<ScanItem> {
        let mut items = Vec::new();
        for signature in registry.by_category_for_mode(category, intensive_cleanup) {
            let Some(provider_id) = signature.provider_id.as_deref() else {
                continue;
            };
            if signature.strategy != CleanStrategy::OwnerProvider {
                continue;
            }
            if excluded_signatures.iter().any(|id| id == &signature.id) {
                continue;
            }
            let Some(provider) = self.providers.get(provider_id) else {
                crate::diagnostics::log_error(
                    "cleanup",
                    &format!(
                        "Signature `{}` names owner provider `{provider_id}`, which this build does not implement; nothing was enumerated",
                        signature.id
                    ),
                );
                continue;
            };
            // The host decides, not the environment's stated flavor: a
            // simulated Windows profile on a machine with no Windows shell
            // cannot act on a Windows store, and the catalog's own platform
            // gate reads the same fact.
            if !provider.platforms().contains(&PlatformKind::current()) {
                continue;
            }
            let guard = signature.process_guard();
            let observation = provider.scan(environment, &guard);
            let gate = signature.eligibility_gate(intensive_cleanup);
            if !observation.status.is_ready() {
                crate::diagnostics::log_error(
                    "cleanup",
                    &format!(
                        "Owner provider `{provider_id}` reported {}: {}",
                        observation.status.display_name(),
                        observation
                            .detail
                            .as_deref()
                            .unwrap_or("the store could not be enumerated")
                    ),
                );
            }
            items.extend(Self::items_for(
                signature,
                provider.as_ref(),
                &observation,
                gate,
            ));
        }
        items
    }

    /// Builds the private authorization for the units the caller selected.
    ///
    /// The signature is looked up again rather than trusted from the item, so a
    /// plan states the catalog entry its authorization came from, and a
    /// provider the catalog does not name is refused instead of being resolved
    /// from whatever is registered.
    pub fn prepare(
        &self,
        registry: &SignatureRegistry,
        signature_id: &str,
        selections: &[OwnerProviderSelection],
        environment: &PlatformEnvironment,
    ) -> Result<OwnerProviderAuthorization, OwnerProviderRefusal> {
        let Some(signature) = registry.get(signature_id) else {
            return Err(OwnerProviderRefusal::new(
                crate::models::ProviderStatus::Unsupported,
                format!("No catalog entry `{signature_id}` authorizes this store"),
            ));
        };
        let Some(provider_id) = signature.provider_id.as_deref() else {
            return Err(OwnerProviderRefusal::new(
                crate::models::ProviderStatus::Unsupported,
                format!("Catalog entry `{signature_id}` names no owner provider"),
            ));
        };
        let Some(provider) = self.providers.get(provider_id) else {
            return Err(OwnerProviderRefusal::new(
                crate::models::ProviderStatus::Unsupported,
                format!(
                    "No provider in this build implements `{provider_id}`; the reviewed action cannot be carried out"
                ),
            ));
        };
        if !provider.platforms().contains(&PlatformKind::current()) {
            return Err(OwnerProviderRefusal::new(
                crate::models::ProviderStatus::Unsupported,
                format!("Provider `{provider_id}` has no adapter on this platform"),
            ));
        }
        let guard = signature.process_guard();
        let mut owner_plan = provider.prepare(environment, &guard, selections)?;
        owner_plan.signature_id = signature.id.clone();
        owner_plan.provider_id = provider_id.to_string();
        owner_plan.risk = signature.risk;
        owner_plan.process_guard = guard;
        owner_plan.requires_confirmation = provider.requires_confirmation();
        Ok(owner_plan)
    }

    /// Performs the action the plan authorized and returns what the provider
    /// verified.
    ///
    /// An id nothing implements is a plan that cannot be carried out, and it is
    /// reported per unit rather than degrading into another operation.
    pub fn execute(
        &self,
        authorization: &OwnerProviderAuthorization,
        environment: &PlatformEnvironment,
    ) -> OwnerProviderExecution {
        let provider_id = authorization.provider_id.as_str();
        let Some(provider) = self.providers.get(provider_id) else {
            return OwnerProviderExecution {
                units: authorization
                    .units
                    .iter()
                    .map(|unit| {
                        zenith_core::domain::cleanup::OwnerUnitOutcome::refused(
                            unit.item_id.clone(),
                            unit.unit_key.clone(),
                            crate::models::ProviderStatus::Unsupported,
                            format!(
                                "No provider in this build implements `{provider_id}`; the reviewed action cannot be carried out"
                            ),
                        )
                    })
                    .collect(),
            };
        };
        provider.execute(environment, authorization)
    }

    /// Builds the scan items one store observation describes.
    fn items_for(
        signature: &Signature,
        provider: &dyn OwnerScopedProvider,
        observation: &OwnerStoreObservation,
        gate: EligibilityGate,
    ) -> Vec<ScanItem> {
        if !observation.status.is_ready() {
            // A store that could not be enumerated is still inventoried: the
            // interface shows the location and the provider's own reason
            // instead of an empty category.
            return vec![Self::item_for_store(signature, provider, observation, gate)];
        }
        // Candidate units are sorted by key so two scans of an unchanged store
        // produce the same item order whatever order the filesystem reports.
        let mut units: Vec<&OwnerUnitObservation> = observation.units.iter().collect();
        units.sort_by(|left, right| left.unit_key.cmp(&right.unit_key));
        units
            .into_iter()
            .map(|unit| Self::item_for_unit(signature, provider, observation, unit, gate))
            .collect()
    }

    /// The item for a store the provider could not enumerate at all.
    fn item_for_store(
        signature: &Signature,
        provider: &dyn OwnerScopedProvider,
        observation: &OwnerStoreObservation,
        gate: EligibilityGate,
    ) -> ScanItem {
        let location = observation
            .root
            .as_ref()
            .map(|root| root.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("provider://{}", provider.id()));
        let reason = observation
            .detail
            .clone()
            .unwrap_or_else(|| observation.status.display_name().to_string());
        Self::item(
            signature,
            provider,
            gate,
            None,
            &location,
            &location,
            &location,
            FileSize::default(),
            0,
            ObservationQuality::Unavailable,
            Some(reason),
        )
    }

    /// The item for one enumerated unit.
    fn item_for_unit(
        signature: &Signature,
        provider: &dyn OwnerScopedProvider,
        observation: &OwnerStoreObservation,
        unit: &OwnerUnitObservation,
        gate: EligibilityGate,
    ) -> ScanItem {
        let root = observation
            .root
            .as_ref()
            .map(|root| root.to_string_lossy().into_owned())
            .unwrap_or_else(|| unit.path.to_string_lossy().into_owned());
        let unit_path = unit.path.to_string_lossy().into_owned();
        let name = format!(
            "{} ({})",
            signature.name,
            unit.path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| unit.unit_key.clone())
        );
        let size = FileSize::new(unit.logical_bytes, Some(unit.allocated_bytes));
        // A unit whose measurement could not read everything is reported with
        // the bytes it did read and the provider's own explanation. Its state
        // still decides whether a plan may touch it.
        let (quality, reason) = match (unit.state, unit.detail.clone()) {
            (OwnerUnitState::Ready, _) => (ObservationQuality::Fresh, None),
            (OwnerUnitState::Advisory, detail) => (ObservationQuality::Fresh, detail),
            (OwnerUnitState::Blocked, detail) => (
                ObservationQuality::Unavailable,
                detail.or_else(|| Some("the provider refused this unit".to_string())),
            ),
        };
        Self::item(
            signature,
            provider,
            gate,
            Some(unit),
            &name,
            &root,
            &unit_path,
            size,
            unit.entry_count as usize,
            quality,
            reason,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn item(
        signature: &Signature,
        provider: &dyn OwnerScopedProvider,
        gate: EligibilityGate,
        unit: Option<&OwnerUnitObservation>,
        name: &str,
        root: &str,
        path: &str,
        size: FileSize,
        file_count: usize,
        quality: ObservationQuality,
        incomplete_reason: Option<String>,
    ) -> ScanItem {
        let mut cache_metadata = signature.cache_metadata();
        cache_metadata.consequence = provider.consequence().to_string();
        if unit.is_some_and(|unit| unit.state != OwnerUnitState::Ready) {
            // Advisory and blocked units report bytes nothing may remove, so
            // the estimate is what was observed rather than what is
            // reclaimable; a partial measurement says so as well.
            cache_metadata.size_semantics = CacheSizeSemantics::Informational;
        }
        // A provider that refuses to remove a unit states that about the unit,
        // and its answer is the one that describes what may be done: an entry
        // whose catalog tier is cleanable still owns only those units its
        // provider is willing to remove.
        if unit.is_some_and(|unit| unit.state == OwnerUnitState::Advisory) {
            cache_metadata.management_mode = CacheManagementMode::Advisory;
        }
        let disposition = derive_cleanup_disposition(
            DispositionFacts::new(
                signature.risk,
                quality,
                &cache_metadata,
                &size,
                incomplete_reason.as_deref(),
            )
            .with_gate(gate)
            .with_lifecycle_provider_action(true)
            .with_confirmation_requirement(provider.requires_confirmation()),
        );
        let id = match unit {
            Some(unit) => format!("{}.{}", signature.id, unit.unit_key),
            None => signature.id.clone(),
        };
        ScanItem {
            id: id.clone(),
            signature_id: signature.id.clone(),
            name: name.to_string(),
            category: signature.category,
            risk: signature.risk,
            path: path.to_string(),
            size,
            file_count,
            description: signature.description.clone(),
            cache_metadata,
            disposition,
            unit: CleanupUnit::new(
                CleanupUnitKind::ProviderAction,
                root.to_string(),
                path.to_string(),
            ),
            ownership: signature.ownership(),
            age: None,
            stale: None,
            structured_state: None,
            entry_kind: EntryKind::Directory,
            gate,
            owner_running: false,
            lifecycle_provider_action: true,
            requires_confirmation: provider.requires_confirmation(),
            overlaps: Vec::new(),
            is_selected: false,
            last_modified: None,
            exists: observation_exists(unit),
            quality,
            incomplete_reason,
            skipped_entry_count: 0,
        }
    }
}

/// Whether an item describes something that exists right now.
///
/// A store that could not be enumerated states nothing about its own
/// existence, so only a unit the provider enumerated claims to exist.
fn observation_exists(unit: Option<&OwnerUnitObservation>) -> bool {
    unit.is_some()
}

/// The typed reason a provider refusal maps to, so the interface derives its
/// copy from the outcome rather than from the provider's prose.
pub fn refusal_reason(status: crate::models::ProviderStatus) -> CleanFailureReason {
    match status {
        crate::models::ProviderStatus::PrerequisiteNotMet => CleanFailureReason::InUse,
        crate::models::ProviderStatus::Unsupported => CleanFailureReason::ProviderUnavailable,
        crate::models::ProviderStatus::Blocked => CleanFailureReason::ProviderRefused,
        crate::models::ProviderStatus::Failed => CleanFailureReason::ProviderRefused,
        crate::models::ProviderStatus::PartiallyCleaned
        | crate::models::ProviderStatus::Cleaned
        | crate::models::ProviderStatus::Ready => CleanFailureReason::Unknown,
    }
}

/// The item-scoped refusal a provider's unit refusal projects to.
pub fn unit_refusal_preview(refusal: &OwnerUnitRefusal) -> crate::models::PlanRefusalPreview {
    crate::models::PlanRefusalPreview {
        item_id: refusal.item_id.clone(),
        name: refusal.item_name.clone(),
        reason: refusal.reason,
        message: refusal.detail.clone(),
    }
}

/// The item a reviewable unit is presented as, for callers that already hold
/// the unit rather than the scan item.
pub fn selection_from_item(item: &ScanItem) -> OwnerProviderSelection {
    OwnerProviderSelection {
        item_id: item.id.clone(),
        name: item.name.clone(),
        path: std::path::PathBuf::from(&item.path),
        expected_bytes: item.cleanable_bytes(),
    }
}

/// The unit a plan authorizes, built from the reviewable observation.
pub fn plan_unit(
    item_id: &str,
    name: &str,
    root: &std::path::Path,
    unit: &OwnerUnitObservation,
    identity: zenith_core::domain::identity::CleanupIdentity,
) -> OwnerProviderUnit {
    OwnerProviderUnit {
        item_id: item_id.to_string(),
        unit_key: unit.unit_key.clone(),
        name: name.to_string(),
        root: root.to_path_buf(),
        path: unit.path.clone(),
        identity,
        expected_bytes: unit.allocated_bytes,
        entry_count: unit.entry_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Category, CleanStrategy, RiskTier, Signature};
    use crate::signatures::SignatureRegistry;
    use std::path::PathBuf;
    use zenith_core::domain::cleanup::{
        OwnerProviderRefusal, OwnerProviderSelection, OwnerStoreObservation, OwnerUnitObservation,
        OwnerUnitOutcome, ProviderStatus, RunningProcessPolicy,
    };
    use zenith_platform::path_algebra::PathFlavor;

    struct StatedProvider {
        id: &'static str,
        store: OwnerStoreObservation,
        prepared: Option<OwnerProviderAuthorization>,
        refusal: Option<OwnerProviderRefusal>,
        executed: OwnerProviderExecution,
    }

    impl StatedProvider {
        fn enumerating(id: &'static str, units: Vec<OwnerUnitObservation>) -> Self {
            Self {
                id,
                store: OwnerStoreObservation::ready(Some(PathBuf::from("/store")), units),
                prepared: None,
                refusal: None,
                executed: OwnerProviderExecution::default(),
            }
        }

        fn refusing(id: &'static str, status: ProviderStatus, detail: &str) -> Self {
            Self {
                id,
                store: OwnerStoreObservation::refused(
                    status,
                    Some(PathBuf::from("/store")),
                    detail.to_string(),
                ),
                prepared: None,
                refusal: None,
                executed: OwnerProviderExecution::default(),
            }
        }

        fn shared(self) -> Arc<dyn OwnerScopedProvider> {
            Arc::new(self)
        }
    }

    impl OwnerScopedProvider for StatedProvider {
        fn id(&self) -> &'static str {
            self.id
        }

        fn platforms(&self) -> &'static [PlatformKind] {
            static PLATFORMS: [PlatformKind; 3] = [
                PlatformKind::Macos,
                PlatformKind::Windows,
                PlatformKind::Linux,
            ];
            &PLATFORMS
        }

        fn consequence(&self) -> &'static str {
            "The stated store's units are removed, and this cannot be undone."
        }

        fn requires_confirmation(&self) -> bool {
            true
        }

        fn scan(
            &self,
            _environment: &PlatformEnvironment,
            _guard: &RunningProcessPolicy,
        ) -> OwnerStoreObservation {
            self.store.clone()
        }

        fn prepare(
            &self,
            _environment: &PlatformEnvironment,
            _guard: &RunningProcessPolicy,
            _selections: &[OwnerProviderSelection],
        ) -> Result<OwnerProviderAuthorization, OwnerProviderRefusal> {
            match (&self.prepared, &self.refusal) {
                (Some(authorization), _) => Ok(authorization.clone()),
                (None, Some(refusal)) => Err(refusal.clone()),
                (None, None) => Ok(OwnerProviderAuthorization {
                    signature_id: String::new(),
                    provider_id: self.id.to_string(),
                    risk: RiskTier::Rebuild,
                    units: Vec::new(),
                    refusals: Vec::new(),
                    process_guard: RunningProcessPolicy::none(),
                    requires_confirmation: true,
                }),
            }
        }

        fn execute(
            &self,
            _environment: &PlatformEnvironment,
            _authorization: &OwnerProviderAuthorization,
        ) -> OwnerProviderExecution {
            self.executed.clone()
        }
    }

    fn catalog_signature() -> Signature {
        Signature {
            id: "test.owner.store".to_string(),
            name: "Owner Store".to_string(),
            category: Category::Developer,
            family: Default::default(),
            risk: RiskTier::Rebuild,
            strategy: CleanStrategy::OwnerProvider,
            paths: Vec::new(),
            exclusions: Vec::new(),
            description: "A store only its owner can enumerate.".to_string(),
            min_age_days: None,
            include_prefixes: Vec::new(),
            exclude_prefixes: Vec::new(),
            intensive_only: false,
            platforms: vec![PlatformKind::current()],
            discovery: Default::default(),
            unit: None,
            owner: "Cargo".to_string(),
            priority: 0,
            fail_if_running: vec!["cargo".to_string()],
            provider: "Cargo".to_string(),
            provider_id: Some("test.owner".to_string()),
            artifact_kind: Default::default(),
            consequence: String::new(),
        }
    }

    fn catalog(
        provider: Arc<dyn OwnerScopedProvider>,
    ) -> (SignatureRegistry, OwnerProviderRegistry) {
        let mut registry = SignatureRegistry::new();
        registry.register(catalog_signature());
        (registry, OwnerProviderRegistry::new(vec![provider]))
    }

    fn environment() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::current())
    }

    fn discover(registry: &SignatureRegistry, providers: &OwnerProviderRegistry) -> Vec<ScanItem> {
        providers.scan_items(registry, Category::Developer, false, &[], &environment())
    }

    fn unit(key: &str, bytes: u64) -> OwnerUnitObservation {
        OwnerUnitObservation::ready(key, PathBuf::from("/store").join(key), bytes, bytes, 2)
    }

    /// Every enumerated unit becomes one item with its own identity and its own
    /// bytes, and none of them is pre-selected: a provider's units are removed
    /// only when the user says which.
    #[test]
    fn a_store_becomes_one_item_per_unit_and_none_is_pre_selected() {
        let (registry, providers) = catalog(
            StatedProvider::enumerating("test.owner", vec![unit("b", 4_096), unit("a", 2_048)])
                .shared(),
        );

        let items = discover(&registry, &providers);

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "test.owner.store.a", "units are sorted by key");
        assert_eq!(items[1].id, "test.owner.store.b");
        assert_eq!(items[0].size.observed_bytes(), 2_048);
        assert_eq!(items[0].file_count, 2);
        assert_eq!(items[0].unit.root, "/store");
        assert_eq!(
            std::path::Path::new(&items[0].unit.path),
            PathBuf::from("/store").join("a"),
            "the provider path uses the separators of the stated platform"
        );
        assert!(items.iter().all(|item| item.disposition.is_cleanable()));
        assert!(
            items.iter().all(|item| !item.is_selected),
            "a provider unit is offered for explicit selection, never pre-approved"
        );
    }

    /// An advisory unit stays visible with the bytes it occupies and is never
    /// cleanable, and a blocked unit stays visible with the provider's reason.
    #[test]
    fn advisory_and_blocked_units_stay_visible_with_their_bytes() {
        let (registry, providers) = catalog(
            StatedProvider::enumerating(
                "test.owner",
                vec![
                    OwnerUnitObservation::advisory(
                        "advised",
                        PathBuf::from("/store/advised"),
                        8_192,
                        16_384,
                        7,
                        "its owner decides",
                    ),
                    OwnerUnitObservation::blocked(
                        "blocked",
                        PathBuf::from("/store/blocked"),
                        1_024,
                        2_048,
                        1,
                        "an entry is not part of the store",
                    ),
                ],
            )
            .shared(),
        );

        let items = discover(&registry, &providers);

        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].disposition.eligibility,
            crate::models::CleanupEligibility::Advisory
        );
        assert!(!items[0].is_selected);
        assert_eq!(
            items[0].size.observed_bytes(),
            16_384,
            "an advisory unit keeps the bytes the measurement observed"
        );
        assert_eq!(
            items[1].disposition.eligibility,
            crate::models::CleanupEligibility::Blocked
        );
        assert_eq!(
            items[1].incomplete_reason.as_deref(),
            Some("an entry is not part of the store")
        );
        assert_eq!(
            items[1].size.observed_bytes(),
            2_048,
            "a refused unit keeps the bytes it was measured at"
        );
    }

    /// A store that could not be read is still inventoried, with the
    /// provider's own reason and no invented bytes.
    #[test]
    fn an_unreadable_store_is_reported_with_its_reason() {
        let (registry, providers) = catalog(
            StatedProvider::refusing(
                "test.owner",
                ProviderStatus::PrerequisiteNotMet,
                "cargo is running",
            )
            .shared(),
        );

        let items = discover(&registry, &providers);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "test.owner.store");
        assert_eq!(items[0].size.observed_bytes(), 0);
        assert_eq!(
            items[0].disposition.eligibility,
            crate::models::CleanupEligibility::Blocked
        );
        assert_eq!(
            items[0].incomplete_reason.as_deref(),
            Some("cargo is running")
        );
    }

    /// A catalog entry naming a provider this build does not implement offers
    /// nothing, and an id nothing implements cannot execute.
    #[test]
    fn an_unimplemented_provider_offers_nothing_and_cannot_execute() {
        let (registry, providers) =
            catalog(StatedProvider::enumerating("test.other", vec![]).shared());
        assert!(discover(&registry, &providers).is_empty());

        let outcome = providers.execute(
            &OwnerProviderAuthorization {
                signature_id: "test.owner.store".to_string(),
                provider_id: "test.owner".to_string(),
                risk: RiskTier::Rebuild,
                units: vec![plan_unit(
                    "test.owner.store.a",
                    "unit a",
                    std::path::Path::new("/store"),
                    &unit("a", 2_048),
                    zenith_core::domain::identity::CleanupIdentity::new(
                        zenith_core::domain::identity::FileIdentity::new(0, 0),
                        true,
                        0,
                        zenith_core::domain::identity::ModifiedStamp::new(0, 0),
                    ),
                )],
                refusals: Vec::new(),
                process_guard: RunningProcessPolicy::none(),
                requires_confirmation: true,
            },
            &environment(),
        );
        assert_eq!(outcome.units.len(), 1);
        assert_eq!(outcome.units[0].status, ProviderStatus::Unsupported);
        assert_eq!(outcome.reclaimed_bytes(), 0);
    }

    /// The authorization a plan carries is stamped from the catalog rather than
    /// from the provider's own answer, so a plan cannot claim a risk tier or a
    /// process guard the catalog does not state.
    #[test]
    fn preparing_stamps_the_catalog_facts_onto_the_authorization() {
        let (registry, providers) =
            catalog(StatedProvider::enumerating("test.owner", vec![]).shared());

        let prepared = providers
            .prepare(
                &registry,
                "test.owner.store",
                &[OwnerProviderSelection {
                    item_id: "test.owner.store.a".to_string(),
                    name: "unit a".to_string(),
                    path: PathBuf::from("/store/a"),
                    expected_bytes: 2_048,
                }],
                &environment(),
            )
            .expect("a registered provider prepares");

        assert_eq!(prepared.signature_id, "test.owner.store");
        assert_eq!(prepared.provider_id, "test.owner");
        assert_eq!(prepared.risk, RiskTier::Rebuild);
        assert_eq!(
            prepared.process_guard.executables(),
            &["cargo".to_string()],
            "the guard comes from the catalog entry"
        );
        assert!(prepared.requires_confirmation);
    }

    /// A provider's own refusal is refused as a value, not converted into
    /// another operation or another provider.
    #[test]
    fn a_provider_refusal_is_returned_as_its_own_value() {
        let mut provider = StatedProvider::enumerating("test.owner", vec![]);
        provider.refusal = Some(OwnerProviderRefusal::new(
            ProviderStatus::Unsupported,
            "this store has no adapter here",
        ));
        let (registry, providers) = catalog(provider.shared());

        let refusal = providers
            .prepare(&registry, "test.owner.store", &[], &environment())
            .expect_err("the provider refused");
        assert_eq!(refusal.status, ProviderStatus::Unsupported);
        assert_eq!(refusal.detail, "this store has no adapter here");
    }

    /// The execution path returns the provider's per-unit outcomes unchanged:
    /// the registry adds no success of its own.
    #[test]
    fn execution_returns_the_providers_own_outcomes() {
        let mut provider = StatedProvider::enumerating("test.owner", vec![]);
        provider.executed = OwnerProviderExecution {
            units: vec![OwnerUnitOutcome::cleaned("test.owner.store.a", "a", 4_096)],
        };
        let (_, providers) = catalog(provider.shared());

        let outcome = providers.execute(
            &OwnerProviderAuthorization {
                signature_id: "test.owner.store".to_string(),
                provider_id: "test.owner".to_string(),
                risk: RiskTier::Rebuild,
                units: Vec::new(),
                refusals: Vec::new(),
                process_guard: RunningProcessPolicy::none(),
                requires_confirmation: true,
            },
            &environment(),
        );

        assert_eq!(outcome.reclaimed_bytes(), 4_096);
    }
}
