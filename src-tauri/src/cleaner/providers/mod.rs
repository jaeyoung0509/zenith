//! Lifecycle-aware cleanup providers.
//!
//! Some reclaimable storage is not a directory under a reviewed root: it is a
//! store an operating system, a shell, or an application owns, and the only
//! safe way to reclaim it is to ask its owner. A generic `delete_contents`
//! against such a store is not a smaller version of the right operation — it is
//! a different, wrong one.
//!
//! A provider is the reviewed implementation of one such operation. It declares
//! a stable id the catalog can name, the platforms this build implements it
//! for, the consequence a user is shown before the action runs, and whether
//! that action needs explicit confirmation. At run time it does three things
//! and nothing else:
//!
//! * **probe** — read the store it owns, or state why it cannot;
//! * **execute** — re-derive its own prerequisites immediately before acting,
//!   never trusting the plan's freshness, then perform the action;
//! * **verify** — read the store again and report what was observed, so the
//!   reclaimed amount is a measurement rather than the interface's return code.
//!
//! What a provider never does is fall back. A failed action reports its own
//! status; it does not become a filesystem delete, and it cannot: the operation
//! it was classified into ([`zenith_core::domain::cleanup::CleanupOperation::LifecycleProvider`])
//! carries no path to delete through.
//!
//! Cancellation is the reviewed plan's lifecycle rather than a channel inside
//! an action. A provider action is one bounded call to an interface that owns
//! its own state; it either runs when the user confirms the reviewed plan, or
//! the plan expires or is dropped and the provider is never reached.

pub mod recycle_bin;

use crate::models::{
    derive_cleanup_disposition, CacheSizeSemantics, Category, CleanupUnit, CleanupUnitKind,
    DispositionFacts, EligibilityGate, EntryKind, FileSize, ObservationQuality, PlatformKind,
    ScanItem, Signature,
};
use crate::signatures::SignatureRegistry;
use std::collections::BTreeMap;
use std::sync::Arc;
use zenith_core::domain::cleanup::{
    LifecycleProviderCleanup, ProviderOutcome, ProviderProbe, ProviderStatus,
};
use zenith_platform::PlatformEnvironment;

/// One reviewed provider of a lifecycle-aware cleanup action.
///
/// `probe` and `execute` take the environment as an argument for the same
/// reason every other layer does: a provider that names a path or a tool must
/// resolve it through the environment the scan was produced with, never through
/// the running process's own profile. A provider whose scope is a shell
/// interface reads no environment fact and says so in its implementation.
pub trait LifecycleProvider: Send + Sync {
    /// The stable id the catalog names, unique within this build.
    fn id(&self) -> &'static str;

    /// The platforms this build implements the action for.
    fn platforms(&self) -> &'static [PlatformKind];

    /// What running this action does, in the words the user is shown before it
    /// runs. It states the consequence, not the benefit.
    fn consequence(&self) -> &'static str;

    /// Whether the action needs explicit confirmation before it runs.
    fn requires_confirmation(&self) -> bool;

    /// The location this action covers, as a stable pseudo path.
    ///
    /// It is presentation and identity — never deletion authority — and it is
    /// what the plan's staleness assertion and the item's row both carry.
    fn location(&self) -> &'static str;

    /// Reads the state the provider owns.
    fn probe(&self, environment: &PlatformEnvironment) -> ProviderProbe;

    /// Performs the action and verifies it.
    fn execute(&self, environment: &PlatformEnvironment) -> ProviderOutcome;
}

/// The reviewed providers this build implements, keyed by the id the catalog
/// names.
///
/// The registry is built once at the composition root and shared by the scan
/// (which probes for candidates) and the executor (which performs the reviewed
/// action), so the two cannot disagree about which providers exist.
pub struct LifecycleProviderRegistry {
    providers: BTreeMap<&'static str, Arc<dyn LifecycleProvider>>,
}

impl LifecycleProviderRegistry {
    /// The providers a user's installation actually ships.
    pub fn native() -> Self {
        Self::new(vec![Arc::new(
            recycle_bin::WindowsRecycleBinProvider::native(),
        )])
    }

    /// A registry over an explicit provider list, for tests and adapters.
    pub fn new(providers: Vec<Arc<dyn LifecycleProvider>>) -> Self {
        let mut map = BTreeMap::new();
        for provider in providers {
            let previous = map.insert(provider.id(), provider);
            debug_assert!(
                previous.is_none(),
                "two providers registered the same id; the catalog could not tell them apart"
            );
        }
        Self { providers: map }
    }

    /// The ids this build implements, in a deterministic order.
    ///
    /// The manifest lint reads this: a catalog entry that names a provider no
    /// build implements is a target nothing can carry out, and it must be
    /// refused by the catalog rather than discovered and then failed at.
    pub fn implemented_ids(&self) -> Vec<&'static str> {
        self.providers.keys().copied().collect()
    }

    pub fn get(&self, provider_id: &str) -> Option<&Arc<dyn LifecycleProvider>> {
        self.providers.get(provider_id)
    }

    /// Discovers the candidates the catalog's provider entries cover.
    ///
    /// Discovery is the probe: a provider that cannot read its store, has no
    /// adapter here, or finds nothing reclaimable produces no item, and the
    /// reason is logged rather than dropped silently. The bytes an item reports
    /// are the provider's own measurement, and the item is never auto-selected
    /// — the risk tier and the disposition decide that, not this function.
    pub fn scan_items(
        &self,
        registry: &SignatureRegistry,
        category: Category,
        intensive_cleanup: bool,
        excluded_signatures: &[String],
        environment: &PlatformEnvironment,
    ) -> Vec<ScanItem> {
        let mut items = Vec::new();
        for signature in registry.by_category_for_mode(category, intensive_cleanup) {
            let Some(provider_id) = signature.provider_id.as_deref() else {
                continue;
            };
            if signature.strategy != crate::models::CleanStrategy::LifecycleProvider {
                continue;
            }
            if excluded_signatures.iter().any(|id| id == &signature.id) {
                continue;
            }
            let Some(provider) = self.providers.get(provider_id) else {
                crate::diagnostics::log_error(
                    "cleanup",
                    &format!(
                        "Signature `{}` names lifecycle provider `{provider_id}`, which this build does not implement; nothing was discovered",
                        signature.id
                    ),
                );
                continue;
            };
            // The host decides, not the environment's stated flavor: a
            // simulated Windows profile on a machine with no Windows shell
            // cannot empty a Recycle Bin, and the catalog's own platform gate
            // (`supports_current_platform`) reads the same fact.
            if !provider.platforms().contains(&PlatformKind::current()) {
                continue;
            }
            let probe = provider.probe(environment);
            if probe.status == ProviderStatus::Ready && !probe.has_reclaimable_bytes() {
                continue;
            }
            if probe.status == ProviderStatus::Unsupported {
                crate::diagnostics::log_error(
                    "cleanup",
                    &format!(
                        "Lifecycle provider `{provider_id}` reported {}: {}",
                        probe.status.display_name(),
                        probe
                            .detail
                            .as_deref()
                            .unwrap_or("the provider is unavailable on this platform")
                    ),
                );
                continue;
            }
            items.push(Self::item_for(
                signature,
                provider.as_ref(),
                &probe,
                signature.eligibility_gate(intensive_cleanup),
            ));
        }
        items
    }

    /// Performs the action the plan authorized and returns what the provider
    /// verified.
    ///
    /// An id nothing implements is a plan that cannot be carried out, and it is
    /// reported as such: it never degrades into another operation.
    pub fn execute(
        &self,
        action: &LifecycleProviderCleanup<'_>,
        environment: &PlatformEnvironment,
    ) -> ProviderOutcome {
        let provider_id = action.provider_id();
        let Some(provider) = self.providers.get(provider_id) else {
            return ProviderOutcome::refused(
                zenith_core::domain::cleanup::ProviderStatus::Unsupported,
                format!(
                    "No provider in this build implements `{provider_id}`; the reviewed action cannot be carried out"
                ),
            );
        };
        provider.execute(environment)
    }

    /// Builds the scan item one probe describes.
    ///
    /// Every fact on the item comes from a stated source: the catalog owns the
    /// name, category, risk tier, owner, and description; the provider owns the
    /// consequence text and the location it covers; the probe owns the bytes.
    /// Selection is derived from the disposition rather than set here, so no
    /// provider can present its action as pre-approved.
    fn item_for(
        signature: &Signature,
        provider: &dyn LifecycleProvider,
        probe: &ProviderProbe,
        gate: EligibilityGate,
    ) -> ScanItem {
        let location = provider.location();
        let size = FileSize::new(probe.estimated_bytes, Some(probe.estimated_bytes));
        let quality = if probe.status == ProviderStatus::Ready {
            ObservationQuality::Fresh
        } else {
            ObservationQuality::Unavailable
        };
        let incomplete_reason = (!probe.status.is_ready()).then(|| {
            probe
                .detail
                .clone()
                .unwrap_or_else(|| probe.status.display_name().to_string())
        });
        let mut cache_metadata = signature.cache_metadata();
        cache_metadata.consequence = provider.consequence().to_string();
        cache_metadata.size_semantics = if probe.bytes_are_lower_bound {
            CacheSizeSemantics::ConservativeLowerBound
        } else {
            CacheSizeSemantics::PhysicalReclaimable
        };
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
        let is_selected = disposition.eligibility.is_auto_cleanable();
        ScanItem {
            id: signature.id.clone(),
            signature_id: signature.id.clone(),
            name: signature.name.clone(),
            category: signature.category,
            risk: signature.risk,
            path: location.to_string(),
            size,
            file_count: usize::try_from(probe.item_count).unwrap_or(usize::MAX),
            description: signature.description.clone(),
            cache_metadata,
            disposition,
            // The provider owns both the discovery and the action; the location
            // travels with the item as presentation and identity only.
            unit: CleanupUnit::new(
                CleanupUnitKind::ProviderAction,
                location.to_string(),
                location.to_string(),
            ),
            ownership: signature.ownership(),
            age: None,
            stale: None,
            structured_state: None,
            entry_kind: EntryKind::Other,
            gate,
            owner_running: false,
            lifecycle_provider_action: true,
            requires_confirmation: provider.requires_confirmation(),
            overlaps: Vec::new(),
            is_selected,
            last_modified: None,
            exists: true,
            quality,
            incomplete_reason,
            skipped_entry_count: 0,
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! A provider whose probe and outcome a test states, so the mechanism —
    //! discovery, item construction, dispatch, result mapping — is exercised
    //! on any host without touching a real store.

    use super::LifecycleProvider;
    use crate::models::PlatformKind;
    use std::sync::Arc;
    use zenith_core::domain::cleanup::{ProviderOutcome, ProviderProbe, ProviderStatus};
    use zenith_platform::PlatformEnvironment;

    /// Every platform, so a stated provider is offered wherever the test runs.
    pub(crate) fn all_platforms() -> &'static [PlatformKind] {
        static PLATFORMS: [PlatformKind; 4] = [
            PlatformKind::Macos,
            PlatformKind::Windows,
            PlatformKind::Linux,
            PlatformKind::Other,
        ];
        &PLATFORMS
    }

    pub(crate) struct StatedProvider {
        pub id: &'static str,
        pub platforms: &'static [PlatformKind],
        pub probe: ProviderProbe,
        pub outcome: ProviderOutcome,
    }

    impl StatedProvider {
        /// A provider holding `bytes` that a run empties cleanly.
        pub(crate) fn holding(bytes: u64, item_count: u64) -> Self {
            Self {
                id: "test.stated",
                platforms: all_platforms(),
                probe: ProviderProbe::ready(bytes, item_count),
                outcome: ProviderOutcome::cleaned(bytes, Some(0)),
            }
        }

        pub(crate) fn with_id(mut self, id: &'static str) -> Self {
            self.id = id;
            self
        }

        pub(crate) fn with_platforms(mut self, platforms: &'static [PlatformKind]) -> Self {
            self.platforms = platforms;
            self
        }

        pub(crate) fn with_probe(mut self, probe: ProviderProbe) -> Self {
            self.probe = probe;
            self
        }

        pub(crate) fn with_outcome(mut self, outcome: ProviderOutcome) -> Self {
            self.outcome = outcome;
            self
        }

        pub(crate) fn shared(self) -> Arc<dyn LifecycleProvider> {
            Arc::new(self)
        }
    }

    impl LifecycleProvider for StatedProvider {
        fn id(&self) -> &'static str {
            self.id
        }

        fn platforms(&self) -> &'static [PlatformKind] {
            self.platforms
        }

        fn consequence(&self) -> &'static str {
            "The stated store is emptied, and this cannot be undone."
        }

        fn requires_confirmation(&self) -> bool {
            true
        }

        fn location(&self) -> &'static str {
            "stated-store://all-volumes"
        }

        fn probe(&self, _environment: &PlatformEnvironment) -> ProviderProbe {
            self.probe.clone()
        }

        fn execute(&self, _environment: &PlatformEnvironment) -> ProviderOutcome {
            self.outcome.clone()
        }
    }

    /// A provider whose outage the caller describes: every probe and every
    /// action reports the same refusal.
    pub(crate) fn refusing_provider(
        status: ProviderStatus,
        detail: &str,
    ) -> Arc<dyn LifecycleProvider> {
        StatedProvider::holding(0, 0)
            .with_probe(ProviderProbe::refused(status, detail.to_string()))
            .with_outcome(ProviderOutcome::refused(status, detail.to_string()))
            .shared()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::StatedProvider;
    use super::LifecycleProviderRegistry;
    use crate::models::{Category, CleanStrategy, DeleteTarget, PlatformKind, RiskTier, Signature};
    use crate::signatures::SignatureRegistry;
    use zenith_core::domain::cleanup::{
        CleanupOperation, ProviderOutcome, ProviderProbe, ProviderStatus,
    };
    use zenith_platform::path_algebra::PathFlavor;
    use zenith_platform::PlatformEnvironment;

    fn catalog_signature() -> Signature {
        Signature {
            id: "test.stated.store".to_string(),
            name: "Stated Store".to_string(),
            category: Category::System,
            family: Default::default(),
            risk: RiskTier::Manual,
            strategy: CleanStrategy::LifecycleProvider,
            paths: Vec::new(),
            exclusions: Vec::new(),
            description: "A store only its owner can empty.".to_string(),
            min_age_days: None,
            include_prefixes: Vec::new(),
            exclude_prefixes: Vec::new(),
            intensive_only: false,
            platforms: vec![PlatformKind::current()],
            discovery: Default::default(),
            unit: None,
            owner: String::new(),
            priority: 0,
            fail_if_running: Vec::new(),
            provider: "Stated Owner".to_string(),
            provider_id: Some("test.stated".to_string()),
            artifact_kind: Default::default(),
            consequence: String::new(),
        }
    }

    /// The plan target the executor would receive for a provider action, so the
    /// registry is exercised through the same classification production uses.
    fn provider_target(provider_id: Option<&str>) -> DeleteTarget {
        DeleteTarget {
            item_id: "test.stated.store".to_string(),
            signature_id: "test.stated.store".to_string(),
            name: "Stated Store".to_string(),
            path: std::path::PathBuf::from("stated-store://all-volumes"),
            strategy: CleanStrategy::LifecycleProvider,
            expected_bytes: 1_024,
            risk: RiskTier::Manual,
            identity: None,
            exclusions: Vec::new(),
            min_age_days: None,
            unit: crate::models::CleanupUnit::new(
                crate::models::CleanupUnitKind::ProviderAction,
                "stated-store://all-volumes",
                "stated-store://all-volumes",
            ),
            target_kind: crate::models::EntryKind::Other,
            owner: crate::models::CleanupOwnership::unknown(),
            process_guard: crate::models::RunningProcessPolicy::none(),
            provider_id: provider_id.map(str::to_string),
            requires_confirmation: true,
        }
    }

    /// Runs the provider action a plan target classifies into, exactly as the
    /// executor does.
    fn run_action(
        providers: &LifecycleProviderRegistry,
        provider_id: Option<&str>,
    ) -> ProviderOutcome {
        let target = provider_target(provider_id);
        match CleanupOperation::of(&target).expect("a named provider action classifies") {
            CleanupOperation::LifecycleProvider(action) => {
                providers.execute(&action, &environment())
            }
            other => panic!("expected a lifecycle provider action, got {other:?}"),
        }
    }

    fn catalog(
        providers: LifecycleProviderRegistry,
    ) -> (SignatureRegistry, LifecycleProviderRegistry) {
        let mut registry = SignatureRegistry::new();
        registry.register(catalog_signature());
        (registry, providers)
    }

    fn environment() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::current())
    }

    fn discover(
        registry: &SignatureRegistry,
        providers: &LifecycleProviderRegistry,
    ) -> Vec<crate::models::ScanItem> {
        providers.scan_items(registry, Category::System, false, &[], &environment())
    }

    /// A provider-backed candidate is reported with the provider's own
    /// measurement and consequence text, and it is never auto-selected: the
    /// item is offered for explicit selection even at the manual risk tier,
    /// which is where a store nothing generic may touch belongs.
    #[test]
    fn a_probe_becomes_an_item_the_user_must_select() {
        let (registry, providers) = catalog(LifecycleProviderRegistry::new(vec![
            StatedProvider::holding(9_000_000_000, 42).shared(),
        ]));

        let items = discover(&registry, &providers);

        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.id, "test.stated.store");
        assert_eq!(item.risk, RiskTier::Manual);
        assert_eq!(item.size.observed_bytes(), 9_000_000_000);
        assert_eq!(item.file_count, 42);
        assert_eq!(item.path, "stated-store://all-volumes");
        assert_eq!(
            item.cache_metadata.consequence,
            "The stated store is emptied, and this cannot be undone."
        );
        assert!(item.disposition.is_cleanable());
        assert!(
            !item.is_selected,
            "a provider action is never pre-selected, whatever the risk tier"
        );
        assert!(
            item.disposition
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("explicit confirmation")),
            "the item states what authorizes the action: {:?}",
            item.disposition
        );
    }

    /// A provider that requires confirmation can never become an automatic
    /// cleanup merely because its catalog risk is Safe.
    #[test]
    fn a_confirmation_required_provider_is_reviewable_even_at_safe_risk() {
        let mut signature = catalog_signature();
        signature.risk = RiskTier::Safe;
        let mut registry = SignatureRegistry::new();
        registry.register(signature);
        let providers =
            LifecycleProviderRegistry::new(vec![StatedProvider::holding(2_048, 2).shared()]);

        let items = discover(&registry, &providers);

        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].disposition.eligibility,
            crate::models::CleanupEligibility::Reviewable
        );
        assert!(!items[0].is_selected);
    }

    /// A probe that cannot run stays visible as a blocked observation. An
    /// unreadable store is not the same thing as an empty store.
    #[test]
    fn a_blocked_probe_remains_visible_with_its_reason() {
        let unreadable = StatedProvider::holding(0, 0).with_probe(ProviderProbe::refused(
            ProviderStatus::Blocked,
            "the store is unreadable",
        ));
        let (registry, providers) =
            catalog(LifecycleProviderRegistry::new(vec![unreadable.shared()]));

        let items = discover(&registry, &providers);

        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].disposition.eligibility,
            crate::models::CleanupEligibility::Blocked
        );
        assert_eq!(
            items[0].incomplete_reason.as_deref(),
            Some("the store is unreadable")
        );
    }

    /// A probe that cannot run, has no adapter here, or finds nothing produces
    /// no item — and a provider the build does not implement is refused rather
    /// than silently skipped.
    #[test]
    fn a_probe_that_finds_nothing_offers_nothing() {
        let empty = StatedProvider::holding(0, 0);
        let (registry, providers) = catalog(LifecycleProviderRegistry::new(vec![empty.shared()]));
        assert!(discover(&registry, &providers).is_empty());

        let foreign = StatedProvider::holding(1_024, 1).with_platforms(&[PlatformKind::Other]);
        let (registry, providers) = catalog(LifecycleProviderRegistry::new(vec![foreign.shared()]));
        assert!(
            discover(&registry, &providers).is_empty(),
            "a provider with no adapter on the running platform offers nothing"
        );

        // A catalog entry naming a provider this build does not implement is
        // its own case: nothing is offered, and the refusal is stated rather
        // than degraded into another operation.
        let (registry, providers) = catalog(LifecycleProviderRegistry::new(Vec::new()));
        assert!(discover(&registry, &providers).is_empty());
        let outcome = run_action(&providers, Some("test.stated"));
        assert_eq!(outcome.status, ProviderStatus::Unsupported);
        assert_eq!(outcome.reclaimed_bytes, 0);
        assert!(outcome
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("test.stated")));
    }

    /// An excluded signature is not discovered through its provider either:
    /// the exclusion the user set applies to the catalog entry, whichever
    /// mechanism would have carried it out.
    #[test]
    fn an_excluded_provider_entry_is_not_discovered() {
        let (registry, providers) = catalog(LifecycleProviderRegistry::new(vec![
            StatedProvider::holding(1_024, 1).shared(),
        ]));

        assert_eq!(discover(&registry, &providers).len(), 1);
        assert!(providers
            .scan_items(
                &registry,
                Category::System,
                false,
                &["test.stated.store".to_string()],
                &environment()
            )
            .is_empty());
    }

    /// The action a plan authorizes is the one that runs: the registry
    /// dispatches by the id the catalog named, and an id nothing implements is
    /// refused instead of being routed to whatever is registered.
    #[test]
    fn the_action_dispatches_to_the_provider_the_catalog_named() {
        let other = StatedProvider::holding(1_024, 1).with_id("test.other");
        let (_, providers) = catalog(LifecycleProviderRegistry::new(vec![other.shared()]));

        let outcome = run_action(&providers, Some("test.stated"));
        assert_eq!(outcome.status, ProviderStatus::Unsupported);
        assert_eq!(outcome.reclaimed_bytes, 0);
    }
}
