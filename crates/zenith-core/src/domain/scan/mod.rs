//! What a scan discovered, what it measured, and whether any of it may be
//! cleaned.
//!
//! Three questions are answered separately here, because conflating them is how
//! a cleaner ends up either lying about a number or deleting something it was
//! never authorized to touch:
//!
//! 1. **Discovery** — a unit was found, and its bytes were measured.
//! 2. **Eligibility** — the current settings, the unit's age, and the catalog
//!    entry's risk tier together decide whether it may be cleaned, and the
//!    answer is a [`CleanupDisposition`] with a reason rather than an absence.
//! 3. **Selection** — the interface asks for a subset of eligible items, and
//!    the planner re-derives eligibility from the item's own facts before a
//!    plan exists.
//!
//! Every item therefore carries its own observation: size, quality, incomplete
//! reason, unit, age, and gate. That is what makes the totals explainable —
//! "12.4 GB discovered, 3.1 GB cleanable" is only truthful if the other 9.3 GB
//! is still on the page with a reason attached.

use crate::domain::cleanup::{EntryKind, StructuredStateKind};
use crate::domain::{Category, ObservationQuality, RiskTier};
use serde::{Deserialize, Serialize};

pub mod overlap;
pub mod unit;

pub use overlap::{
    resolve_unit_overlaps, resolve_unit_overlaps_with, CleanupOverlap, OverlapReport,
    OverlappedDiscovery, UnitRelationship,
};
pub use unit::{
    AgeObservation, CleanupOwnership, CleanupUnit, CleanupUnitIdentity, CleanupUnitKind,
    EligibilityGate, OwnershipConfidence, PathIdentity, StaleEntryObservation,
};

fn unavailable_observation_quality() -> ObservationQuality {
    ObservationQuality::Unavailable
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CacheManagementMode {
    #[default]
    Zenith,
    ToolManaged,
    Advisory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CacheArtifactKind {
    #[default]
    Temporary,
    DownloadCache,
    PackageStore,
    BuildArtifact,
    CompiledKernel,
    OptimizedEngine,
    Autotune,
    ModelWeight,
    PromptOrSessionState,
    RuntimeMemory,
    Log,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CacheUsageConfidence {
    Exact,
    Approximate,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CacheSizeSemantics {
    #[default]
    PhysicalReclaimable,
    ConservativeLowerBound,
    Informational,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
pub struct CacheMetadata {
    pub provider: String,
    pub management_mode: CacheManagementMode,
    pub artifact_kind: CacheArtifactKind,
    pub consequence: String,
    pub size_semantics: CacheSizeSemantics,
    #[serde(default)]
    pub last_used_confidence: CacheUsageConfidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
pub struct FileSize {
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub logical: u64,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub allocated: Option<u64>,
}

impl FileSize {
    pub fn new(logical: u64, allocated: Option<u64>) -> Self {
        Self { logical, allocated }
    }

    pub fn observed_bytes(&self) -> u64 {
        self.allocated.unwrap_or(self.logical)
    }

    pub fn reclaimable(&self) -> u64 {
        self.observed_bytes()
    }
}

/// Whether a discovered unit may be cleaned, and by which action.
///
/// A discovered unit is always representable; this type says what may be done
/// with it. `AutoCleanable` and `Reviewable` are the two cleanable states —
/// the first may be selected automatically, the second only by explicit user
/// selection. Everything else is inventory: visible, measured, and explained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CleanupEligibility {
    /// Safe to remove without asking: the catalog says so, the observation is
    /// complete, and the age policy is satisfied.
    AutoCleanable,
    /// Removable only by explicit selection: a rebuild cost, a provider-owned
    /// cache, or an incomplete observation.
    Reviewable,
    /// Discovered, but not old enough yet. The bytes are real and reported;
    /// the age policy that would authorize removal is not met.
    Recent,
    /// Discovered, but the current settings refuse to clean a unit this
    /// signature found (an opt-in scope that is switched off).
    PolicyGated,
    /// Not Zenith's operation: an external manager or provider owns the
    /// invalidation.
    Advisory,
    #[default]
    Blocked,
}

impl CleanupEligibility {
    /// Every state, in the order the aggregates present them.
    ///
    /// The order is part of the contract: a breakdown that reordered itself
    /// between runs would make two scans of the same disk incomparable.
    pub const ALL: [CleanupEligibility; 6] = [
        CleanupEligibility::AutoCleanable,
        CleanupEligibility::Reviewable,
        CleanupEligibility::Recent,
        CleanupEligibility::PolicyGated,
        CleanupEligibility::Advisory,
        CleanupEligibility::Blocked,
    ];

    pub fn is_cleanable(&self) -> bool {
        matches!(self, Self::AutoCleanable | Self::Reviewable)
    }

    pub fn is_auto_cleanable(&self) -> bool {
        matches!(self, Self::AutoCleanable)
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Blocked)
    }

    pub fn is_advisory(&self) -> bool {
        matches!(self, Self::Advisory)
    }

    /// Whether the unit was found but the age policy is not satisfied yet.
    pub fn is_recent(&self) -> bool {
        matches!(self, Self::Recent)
    }

    /// Whether the unit was found but the current settings do not clean it.
    pub fn is_policy_gated(&self) -> bool {
        matches!(self, Self::PolicyGated)
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::AutoCleanable => "Safe to clean now",
            Self::Reviewable => "Review before cleaning",
            Self::Recent => "Recently used",
            Self::PolicyGated => "Outside the current scope",
            Self::Advisory => "Managed outside Zenith",
            Self::Blocked => "Blocked or inaccessible",
        }
    }

    /// How restrictive the state is, least reversible first.
    ///
    /// The order is part of the accounting contract: when two rules describe
    /// one location, the scan states the stricter verdict, and "stricter" has
    /// to mean the same thing on every run and every platform. A state that
    /// can never be cleaned outranks one that needs a settings change, which
    /// outranks one that needs time, which outranks one that needs the user to
    /// look at it.
    pub fn strictness(self) -> u8 {
        match self {
            Self::Blocked => 0,
            Self::Advisory => 1,
            Self::PolicyGated => 2,
            Self::Recent => 3,
            Self::Reviewable => 4,
            Self::AutoCleanable => 5,
        }
    }

    /// The stricter of two states, with ties keeping `self`.
    pub fn strictest(self, other: Self) -> Self {
        if other.strictness() < self.strictness() {
            other
        } else {
            self
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
pub struct CleanupDisposition {
    pub eligibility: CleanupEligibility,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default, with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub cleanable_bytes: Option<u64>,
}

impl CleanupDisposition {
    pub fn new(
        eligibility: CleanupEligibility,
        reason: Option<String>,
        cleanable_bytes: Option<u64>,
    ) -> Self {
        Self {
            eligibility,
            reason,
            cleanable_bytes,
        }
    }

    pub fn auto_cleanable(bytes: u64) -> Self {
        Self {
            eligibility: CleanupEligibility::AutoCleanable,
            reason: None,
            cleanable_bytes: Some(bytes),
        }
    }

    pub fn reviewable(bytes: u64, reason: Option<String>) -> Self {
        Self {
            eligibility: CleanupEligibility::Reviewable,
            reason,
            cleanable_bytes: Some(bytes),
        }
    }

    /// The unit was measured, and the age policy that would authorize removal
    /// is not satisfied. Nothing may be cleaned now, so it contributes no
    /// cleanable bytes — its observed bytes still reach the totals.
    pub fn recent(reason: impl Into<String>) -> Self {
        Self {
            eligibility: CleanupEligibility::Recent,
            reason: Some(reason.into()),
            cleanable_bytes: None,
        }
    }

    /// The unit was measured, and the settings that would authorize removal are
    /// switched off.
    pub fn policy_gated(reason: impl Into<String>) -> Self {
        Self {
            eligibility: CleanupEligibility::PolicyGated,
            reason: Some(reason.into()),
            cleanable_bytes: None,
        }
    }

    pub fn blocked(reason: impl Into<String>) -> Self {
        Self {
            eligibility: CleanupEligibility::Blocked,
            reason: Some(reason.into()),
            cleanable_bytes: None,
        }
    }

    pub fn advisory(reason: impl Into<String>) -> Self {
        Self {
            eligibility: CleanupEligibility::Advisory,
            reason: Some(reason.into()),
            cleanable_bytes: None,
        }
    }

    pub fn is_cleanable(&self) -> bool {
        self.eligibility.is_cleanable() && self.cleanable_bytes.unwrap_or(0) > 0
    }
}

/// Observed and cleanable bytes for one eligibility state.
///
/// A breakdown is how a scan explains itself: the states sum to the observed
/// total, so a large "discovered" number can always be traced to the states
/// that hold the bytes and the reasons attached to them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
pub struct EligibilityBucket {
    pub eligibility: CleanupEligibility,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub observed_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub cleanable_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub items: u64,
}

impl EligibilityBucket {
    pub fn new(eligibility: CleanupEligibility) -> Self {
        Self {
            eligibility,
            ..Default::default()
        }
    }

    /// Accounts one retained item. The cleanable amount is the item's own
    /// answer, so a state that is not cleanable contributes zero there no
    /// matter what the caller passes.
    pub fn add(&mut self, item: &ScanItem) {
        self.observed_bytes += item.observed_bytes();
        self.cleanable_bytes += if self.eligibility.is_cleanable() {
            item.cleanable_bytes()
        } else {
            0
        };
        self.items += 1;
    }
}

/// The complete, ordered breakdown every aggregate carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EligibilitySummary {
    pub buckets: Vec<EligibilityBucket>,
}

impl Default for EligibilitySummary {
    fn default() -> Self {
        Self {
            buckets: CleanupEligibility::ALL
                .into_iter()
                .map(EligibilityBucket::new)
                .collect(),
        }
    }
}

impl EligibilitySummary {
    /// Accounts one retained item into its own state.
    pub fn add(&mut self, item: &ScanItem) {
        let eligibility = item.disposition.eligibility;
        if let Some(bucket) = self
            .buckets
            .iter_mut()
            .find(|bucket| bucket.eligibility == eligibility)
        {
            bucket.add(item);
        }
    }

    /// Merges another summary into this one, bucket by bucket.
    pub fn merge(&mut self, other: &EligibilitySummary) {
        for incoming in &other.buckets {
            if let Some(bucket) = self
                .buckets
                .iter_mut()
                .find(|bucket| bucket.eligibility == incoming.eligibility)
            {
                bucket.observed_bytes += incoming.observed_bytes;
                bucket.cleanable_bytes += incoming.cleanable_bytes;
                bucket.items += incoming.items;
            }
        }
    }

    /// The bucket for one state.
    ///
    /// `None` only for a payload that carried a breakdown without this state:
    /// the aggregates only ever read what a summary holds, so a missing bucket
    /// reads as zero bytes rather than as an index into an empty list.
    pub fn bucket(&self, eligibility: CleanupEligibility) -> Option<&EligibilityBucket> {
        self.buckets
            .iter()
            .find(|bucket| bucket.eligibility == eligibility)
    }

    pub fn observed_bytes(&self, eligibility: CleanupEligibility) -> u64 {
        self.bucket(eligibility)
            .map(|bucket| bucket.observed_bytes)
            .unwrap_or(0)
    }

    pub fn cleanable_bytes(&self, eligibility: CleanupEligibility) -> u64 {
        self.bucket(eligibility)
            .map(|bucket| bucket.cleanable_bytes)
            .unwrap_or(0)
    }

    pub fn items(&self, eligibility: CleanupEligibility) -> u64 {
        self.bucket(eligibility)
            .map(|bucket| bucket.items)
            .unwrap_or(0)
    }
}

pub fn is_safety_blocked_reason(reason: &str) -> bool {
    let lower = reason.to_lowercase();
    (lower.contains("protected") && (lower.contains("bundle") || lower.contains(".app")))
        || lower.contains("symlink")
        || lower.contains("blacklist")
}

/// Everything the eligibility decision is derived from.
///
/// The facts live on the item that carries the disposition, so the decision can
/// be re-derived later without the scan that produced it — which is exactly what
/// [`ScanItem::has_current_disposition`] and the planner's precondition check
/// rely on. Passing them as one struct keeps the derivation a pure function of
/// stated facts instead of a positional argument list that grows a silent
/// default every time a policy is added.
#[derive(Debug, Clone, Copy)]
pub struct DispositionFacts<'a> {
    pub risk: RiskTier,
    pub quality: ObservationQuality,
    pub cache_metadata: &'a CacheMetadata,
    pub size: &'a FileSize,
    pub incomplete_reason: Option<&'a str>,
    /// The scan-policy gate that applied when the unit was discovered.
    pub gate: EligibilityGate,
    /// Whether the owning application is running.
    pub owner_running: bool,
    /// The age policy's verdict, when the unit carries one.
    pub age: Option<&'a AgeObservation>,
    /// The per-entry verdict for a unit whose policy ages its entries.
    pub stale: Option<&'a StaleEntryObservation>,
    /// The structured state the path was classified as, when it is one.
    pub structured_state: Option<StructuredStateKind>,
    /// Whether a reviewed provider — not generic cleanup — performs this
    /// unit's cleanup.
    ///
    /// A provider action owns no host path, so the manual risk tier means
    /// something different for it: the unit is not generic cleanup's to remove,
    /// and it is still executable, through the one operation the catalog named.
    pub lifecycle_provider_action: bool,
    /// Whether this action must be explicitly confirmed before execution.
    pub requires_confirmation: bool,
    /// The strictest verdict another rule reached about the same location, and
    /// the rule that reached it.
    pub overlap: Option<(CleanupEligibility, &'a str, bool)>,
}

impl<'a> DispositionFacts<'a> {
    /// Facts for a unit discovered with an open gate and no age policy: what a
    /// fixed-path signature with no `min_age_days` produces.
    pub fn new(
        risk: RiskTier,
        quality: ObservationQuality,
        cache_metadata: &'a CacheMetadata,
        size: &'a FileSize,
        incomplete_reason: Option<&'a str>,
    ) -> Self {
        Self {
            risk,
            quality,
            cache_metadata,
            size,
            incomplete_reason,
            gate: EligibilityGate::Open,
            owner_running: false,
            age: None,
            stale: None,
            structured_state: None,
            lifecycle_provider_action: false,
            requires_confirmation: false,
            overlap: None,
        }
    }

    pub fn with_gate(mut self, gate: EligibilityGate) -> Self {
        self.gate = gate;
        self
    }

    pub fn with_running_owner(mut self, owner_running: bool) -> Self {
        self.owner_running = owner_running;
        self
    }

    pub fn with_age(mut self, age: Option<&'a AgeObservation>) -> Self {
        self.age = age;
        self
    }

    pub fn with_stale_entries(mut self, stale: Option<&'a StaleEntryObservation>) -> Self {
        self.stale = stale;
        self
    }

    pub fn with_structured_state(mut self, state: Option<StructuredStateKind>) -> Self {
        self.structured_state = state;
        self
    }

    /// States the strictest verdict another rule reached about this location.
    pub fn with_overlap(mut self, overlap: Option<(CleanupEligibility, &'a str, bool)>) -> Self {
        self.overlap = overlap;
        self
    }

    /// States that a reviewed provider, not generic cleanup, performs this
    /// unit's cleanup.
    pub fn with_lifecycle_provider_action(mut self, lifecycle_provider_action: bool) -> Self {
        self.lifecycle_provider_action = lifecycle_provider_action;
        self
    }

    pub fn with_confirmation_requirement(mut self, requires_confirmation: bool) -> Self {
        self.requires_confirmation = requires_confirmation;
        self
    }
}

pub fn derive_cleanup_disposition(facts: DispositionFacts<'_>) -> CleanupDisposition {
    let disposition = derive_own_disposition(facts);
    let Some((verdict, source, explain_equal)) = facts.overlap else {
        return disposition;
    };
    if verdict.strictness() > disposition.eligibility.strictness()
        || (verdict == disposition.eligibility && !explain_equal)
    {
        return disposition;
    }
    // The item states the stricter verdict, and the reason names the rule that
    // reached it, so the number on the screen can be traced to a rule rather
    // than to an unexplained downgrade.
    CleanupDisposition::new(
        verdict,
        Some(format!(
            "Another rule for this location ({}), classifies it as {}; the stricter verdict applies",
            source,
            verdict.display_name().to_lowercase()
        )),
        verdict
            .is_cleanable()
            .then(|| disposition.cleanable_bytes.unwrap_or(0)),
    )
}

/// The verdict this item's own facts support, before other rules are folded in.
fn derive_own_disposition(facts: DispositionFacts<'_>) -> CleanupDisposition {
    let DispositionFacts {
        risk,
        quality,
        cache_metadata,
        size,
        incomplete_reason,
        gate,
        owner_running,
        age,
        stale,
        structured_state,
        lifecycle_provider_action,
        requires_confirmation,
        // The overlap verdict is applied by the caller, after these facts have
        // produced their own answer.
        overlap: _,
    } = facts;

    // 1. Safety violations (protected .app bundle, symlink escape, blacklist) fail closed
    if incomplete_reason.is_some_and(is_safety_blocked_reason) {
        return CleanupDisposition::blocked(
            incomplete_reason.unwrap_or("Protected path encountered; cleanup blocked"),
        );
    }

    // 2. Unavailable observations are always blocked from cleanup
    if quality == ObservationQuality::Unavailable {
        return CleanupDisposition::blocked(
            incomplete_reason.unwrap_or("Inaccessible path; inspection failed"),
        );
    }

    // 3. A path that holds application state rather than regenerable cache
    //    data is never generic cleanup's to remove, whatever an age rule says
    //    about it. This is the rule that keeps a discovery heuristic from
    //    reaching a database or a credential store.
    if let Some(state) = structured_state {
        return CleanupDisposition::blocked(format!(
            "This location is {}; generic cleanup does not remove structured state",
            state.display_name()
        ));
    }

    // 4. Advisory caches cannot enter generic cleanup
    if cache_metadata.management_mode == CacheManagementMode::Advisory {
        return CleanupDisposition::advisory(
            incomplete_reason.unwrap_or("Advisory cache: managed manually or outside Zenith"),
        );
    }

    // 5. Manual risk tiers cannot be cleaned generically. A unit a reviewed
    //    provider performs is the exception the tier describes rather than
    //    forbids: nothing generic may touch it, and the one operation the
    //    catalog named may, so it is offered for explicit selection with the
    //    reason that confirmation is what authorizes it.
    if risk == RiskTier::Manual {
        if lifecycle_provider_action {
            return CleanupDisposition::reviewable(
                size.observed_bytes(),
                incomplete_reason.map(Into::into).or_else(|| {
                    Some(
                        "A dedicated provider performs this cleanup and runs only on explicit confirmation"
                            .to_string(),
                    )
                }),
            );
        }
        return CleanupDisposition::blocked(
            incomplete_reason.unwrap_or("Manual cleanup only; generic cleanup is unsupported"),
        );
    }

    // 6. A unit the current settings deliberately do not clean is still
    //    discovered and measured; it is never made cleanable by discovery.
    if let Some(reason) = gate.reason() {
        return CleanupDisposition::policy_gated(reason);
    }

    // 6b. A policy that ages the entries inside the unit reports how much of it
    //     satisfies the policy: nothing old enough is the `recent` state, and a
    //     partial remainder is what the unit could actually reclaim.
    if let Some(stale) = stale {
        if stale.nothing_is_stale() {
            return CleanupDisposition::recent(format!(
                "Nothing in this location has been inactive for {} days",
                stale.min_age_days
            ));
        }
    }

    // 7. An age policy that is not satisfied blocks removal. The measurement is
    //    real and reported; the authorization is not there yet.
    if let Some(age) = age {
        if !age.satisfied {
            let days = age.min_age_days;
            let detail = match age.newest_modified {
                Some(_) => format!(
                    "Modified within the last {days} days; the age policy needs {days} days of inactivity"
                ),
                None => format!(
                    "Directory freshness could not be proven; the age policy needs {days} days of inactivity"
                ),
            };
            return CleanupDisposition::recent(detail);
        }
    }

    let observed = size.observed_bytes();
    let reclaimable = stale.map(|stale| stale.stale_bytes).unwrap_or(observed);

    // 8. Actions requiring explicit confirmation are never automatic,
    //    regardless of their risk tier. This prevents a future Safe provider
    //    from entering Quick Clean merely because its bytes are reclaimable.
    if requires_confirmation {
        if observed == 0 {
            return CleanupDisposition::blocked("No cleanable data found");
        }
        return CleanupDisposition::reviewable(
            reclaimable,
            incomplete_reason.map(Into::into).or_else(|| {
                Some("This action requires explicit confirmation before it can run".to_string())
            }),
        );
    }

    // 8. Tool-managed caches: provider policy decides, never AutoCleanable
    if cache_metadata.management_mode == CacheManagementMode::ToolManaged {
        if observed == 0 {
            return CleanupDisposition::blocked("No cleanable data found");
        }
        if quality == ObservationQuality::Partial {
            return CleanupDisposition::reviewable(
                reclaimable,
                incomplete_reason.map(Into::into).or_else(|| {
                    Some("Incomplete scan; review before pruning with provider".into())
                }),
            );
        }
        return CleanupDisposition::reviewable(reclaimable, None);
    }

    // 9. Partial observation quality: reviewable, never auto-selected or quick-cleanable
    if quality == ObservationQuality::Partial {
        if observed == 0 {
            return CleanupDisposition::blocked("No cleanable data found");
        }
        return CleanupDisposition::reviewable(
            reclaimable,
            incomplete_reason
                .map(Into::into)
                .or_else(|| Some("Incomplete scan; review before cleaning".into())),
        );
    }

    // 9b. An application that is running keeps its own cache: the unit stays
    //     selectable, because the user may know better, but it is never
    //     removed without that decision.
    if owner_running {
        return CleanupDisposition::reviewable(
            reclaimable,
            Some(
                "The application that owns this location is running; close it before cleaning"
                    .to_string(),
            ),
        );
    }

    // 10. Fresh observation with Zenith management
    if observed == 0 {
        return CleanupDisposition::blocked("No cleanable data found");
    }
    // A stale-entry policy that only part of the unit satisfies says so: the
    // estimate is the part that would go, and the reason names the rest.
    let partial_reason = stale
        .filter(|stale| stale.stale_bytes < observed)
        .map(|stale| {
            format!(
                "Removing what has been inactive for {} days; the rest was modified more recently",
                stale.min_age_days
            )
        });

    match risk {
        RiskTier::Safe => {
            let disposition = CleanupDisposition::auto_cleanable(reclaimable);
            match partial_reason {
                Some(reason) => CleanupDisposition::new(
                    disposition.eligibility,
                    Some(reason),
                    disposition.cleanable_bytes,
                ),
                None => disposition,
            }
        }
        RiskTier::Rebuild => CleanupDisposition::reviewable(reclaimable, partial_reason),
        RiskTier::Manual => CleanupDisposition::blocked("Manual cleanup only"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ScanItem {
    pub id: String,
    pub signature_id: String,
    pub name: String,
    pub category: Category,
    pub risk: RiskTier,
    pub path: String,
    pub size: FileSize,
    pub file_count: usize,
    pub description: String,
    #[serde(default)]
    pub cache_metadata: CacheMetadata,
    #[serde(default)]
    pub disposition: CleanupDisposition,
    /// The deletable object this item describes, and the root it was found
    /// under.
    ///
    /// Defaulted rather than required so a scan persisted by an earlier build
    /// still deserializes; a unit with an empty path is not a target, and the
    /// planner refuses it instead of treating absence as permission.
    #[serde(default)]
    pub unit: CleanupUnit,
    /// Who the catalog expects to own this location, and how strongly.
    #[serde(default)]
    pub ownership: CleanupOwnership,
    /// The age policy's verdict, when the unit carries one.
    #[serde(default)]
    pub age: Option<AgeObservation>,
    /// The per-entry verdict, when the unit's policy ages the entries inside it
    /// rather than the unit as a whole.
    #[serde(default)]
    pub stale: Option<StaleEntryObservation>,
    /// The structured state the path was classified as, when it is one.
    ///
    /// Recorded at discovery so the disposition derivation and the execution
    /// guard agree about a path neither of them may delete, and so the reason
    /// reaches the interface instead of the item disappearing.
    #[serde(default)]
    pub structured_state: Option<StructuredStateKind>,
    /// What the scan found at the path: a file, a directory, or neither.
    ///
    /// The plan carries this observation forward so execution can tell "the
    /// approved object is still there" from "something else is at that path
    /// now", which is the difference between deleting and skipping.
    #[serde(default)]
    pub entry_kind: EntryKind,
    /// The scan-policy gate that applied when this unit was discovered.
    #[serde(default)]
    pub gate: EligibilityGate,
    /// Whether the application that owns this location is running right now.
    ///
    /// A cache an application is using is not abandoned storage, however old
    /// its bytes are: removing it while the application holds it open is a
    /// race the user did not ask for. The scan records the fact and the
    /// disposition keeps the unit selectable but never automatic.
    #[serde(default)]
    pub owner_running: bool,
    /// Whether this unit is executed by the lifecycle-provider contract rather
    /// than by a generic provider/external-command unit.
    #[serde(default)]
    pub lifecycle_provider_action: bool,
    /// Whether execution requires an explicit confirmation token from the
    /// reviewed UI path.
    #[serde(default)]
    pub requires_confirmation: bool,
    /// The other catalog rules that described the same location.
    ///
    /// Two rules can name one cache directory, or a broad rule can name a
    /// directory that contains a unit a narrower rule enumerated. The bytes are
    /// counted once — by this item — and the rules that did not count them are
    /// carried here, so the strictest verdict still applies and the interface
    /// can say which rule imposed it.
    #[serde(default)]
    pub overlaps: Vec<CleanupOverlap>,
    pub is_selected: bool,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub last_modified: Option<u64>,
    pub exists: bool,
    #[serde(default = "unavailable_observation_quality")]
    pub quality: ObservationQuality,
    #[serde(default)]
    pub incomplete_reason: Option<String>,
    /// Entries the measurement did not account for (excluded, blacklisted,
    /// protected, unreadable, or beyond the depth limit). Reported so a
    /// partial total is never presented as a complete one.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub skipped_entry_count: u64,
}

impl ScanItem {
    pub fn observed_bytes(&self) -> u64 {
        self.size.observed_bytes()
    }

    /// Everything the eligibility decision is derived from, taken from this
    /// item's own facts.
    ///
    /// The rules that described the same location are part of those facts: a
    /// unit another rule classifies more strictly is never made cleaner by the
    /// fact that a second rule matched it. A rule whose scope is switched off
    /// is the exception — it was discovered for visibility, so it cannot
    /// withhold a permission the current settings grant.
    pub fn disposition_facts(&self) -> DispositionFacts<'_> {
        let overlap = self
            .overlaps
            .iter()
            .filter(|overlap| overlap.gate.is_open())
            .map(|overlap| {
                (
                    if overlap.authority_conflict {
                        CleanupEligibility::Blocked
                    } else {
                        overlap.eligibility
                    },
                    overlap.name.as_str(),
                    overlap.authority_conflict
                        || (overlap.risk == self.risk && self.risk != RiskTier::Safe),
                )
            })
            .min_by_key(|(eligibility, _, _)| eligibility.strictness());
        DispositionFacts::new(
            self.risk,
            self.quality,
            &self.cache_metadata,
            &self.size,
            self.incomplete_reason.as_deref(),
        )
        .with_gate(self.gate)
        .with_running_owner(self.owner_running)
        .with_age(self.age.as_ref())
        .with_stale_entries(self.stale.as_ref())
        .with_structured_state(self.structured_state)
        .with_lifecycle_provider_action(self.lifecycle_provider_action)
        .with_confirmation_requirement(self.requires_confirmation)
        .with_overlap(overlap)
    }

    pub fn derive_disposition(&self) -> CleanupDisposition {
        derive_cleanup_disposition(self.disposition_facts())
    }

    /// Whether a scan may pre-select this item for cleaning.
    ///
    /// Only an auto-cleanable item with a positive cleanable amount qualifies:
    /// a discovered-but-ineligible unit is inventory, and selecting it would
    /// put bytes in a total no plan may act on.
    pub fn is_pre_selectable(&self) -> bool {
        self.disposition.eligibility == CleanupEligibility::AutoCleanable
            && self.cleanable_bytes() > 0
    }

    /// Re-derives the disposition from this item's own facts and drops a
    /// selection the new facts no longer support.
    ///
    /// Every path that changes an item's facts goes through this, so the
    /// serialized disposition and the selection flag can never describe two
    /// different states of the same item — the planner re-derives the
    /// disposition and refuses the mismatch, but the interface would already
    /// have shown the item as selected.
    pub fn rederive_disposition(&mut self) {
        self.disposition = self.derive_disposition();
        self.is_selected = self.is_selected && self.is_pre_selectable();
    }

    /// The same, for a caller that owns the item.
    pub fn with_derived_disposition(mut self) -> Self {
        self.rederive_disposition();
        self
    }

    /// The normalized identity two discoveries of the same unit share.
    ///
    /// The caller states whether the filesystem folds case, because that is a
    /// property of the machine the scan ran against and not of the path text.
    pub fn unit_identity(&self, identity: PathIdentity) -> CleanupUnitIdentity {
        self.unit.identity(identity)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mock(
        id: impl Into<String>,
        signature_id: impl Into<String>,
        name: impl Into<String>,
        category: Category,
        risk: RiskTier,
        path: impl Into<String>,
        size: FileSize,
        file_count: usize,
    ) -> Self {
        let path = path.into();
        let disposition = derive_cleanup_disposition(DispositionFacts::new(
            risk,
            ObservationQuality::Fresh,
            &Default::default(),
            &size,
            None,
        ));
        let is_selected = disposition.eligibility == CleanupEligibility::AutoCleanable
            && size.observed_bytes() > 0;
        Self {
            id: id.into(),
            signature_id: signature_id.into(),
            name: name.into(),
            category,
            risk,
            path: path.clone(),
            size,
            file_count,
            description: String::new(),
            cache_metadata: Default::default(),
            disposition,
            unit: CleanupUnit::fixed_path(path),
            ownership: CleanupOwnership::unknown(),
            age: None,
            stale: None,
            structured_state: None,
            owner_running: false,
            lifecycle_provider_action: false,
            requires_confirmation: false,
            overlaps: Vec::new(),
            entry_kind: EntryKind::Directory,
            gate: EligibilityGate::Open,
            is_selected,
            last_modified: None,
            exists: true,
            quality: ObservationQuality::Fresh,
            incomplete_reason: None,
            skipped_entry_count: 0,
        }
    }

    pub fn allows_cleanup(&self) -> bool {
        self.disposition.is_cleanable()
    }

    /// Bytes this item would reclaim when cleaned, and zero when it cannot be
    /// cleaned at all.
    ///
    /// Every total — the risk buckets, the category and scan sums, and the
    /// frontend's selection summary — is derived from this, so an item whose
    /// observation cannot support a cleanup never contributes a byte that only
    /// looks reclaimable.
    pub fn cleanable_bytes(&self) -> u64 {
        if !self.disposition.is_cleanable() {
            return 0;
        }
        self.disposition
            .cleanable_bytes
            .unwrap_or(0)
            .min(self.observed_bytes())
    }

    /// Whether the serialized disposition still matches the facts captured by
    /// this scan item. Mutation paths use this to reject stale or internally
    /// inconsistent scan data instead of trusting a detached permission flag.
    pub fn has_current_disposition(&self) -> bool {
        self.disposition == self.derive_disposition()
    }

    /// Whether this item is a cleanup candidate a user could select.
    pub fn is_cleanable_candidate(&self) -> bool {
        self.allows_cleanup()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CategoryResult {
    pub category: Category,
    pub display_name: String,
    pub items: Vec<ScanItem>,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub total_bytes: u64,
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub cleanable_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub safe_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub rebuild_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub manual_bytes: u64,
    #[serde(default = "unavailable_observation_quality")]
    pub quality: ObservationQuality,
    /// Sum of the retained items' skipped-entry counts.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub skipped_entry_count: u64,
    /// Retained items whose observation is not `Fresh`.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub incomplete_item_count: u64,
    /// Observed and cleanable bytes per eligibility state.
    ///
    /// The buckets sum to `total_bytes` over the observed column, so a category
    /// whose cleanable total is small can always be explained by the states
    /// that hold the rest of the bytes.
    #[serde(default)]
    pub eligibility: EligibilitySummary,
    /// Units this category discovered but did not count again because an
    /// identical unit had already been counted.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub suppressed_duplicate_count: u64,
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub suppressed_duplicate_bytes: u64,
    /// Units inside a broader unit this category reported, so their bytes are
    /// already in `total_bytes` and are not counted a second time.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub suppressed_overlap_count: u64,
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub suppressed_overlap_bytes: u64,
    /// Units that may be inside a broader unit's observation without proof.
    ///
    /// A partial walk cannot say which entries it measured and a gated
    /// container must not absorb a running rule, so the pair stays as two
    /// rows and the bytes are stated here: the observed union is between
    /// `total_bytes - ambiguous_overlap_bytes` and `total_bytes`, rather than
    /// a sum presented as exact.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub ambiguous_overlap_count: u64,
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub ambiguous_overlap_bytes: u64,
}

impl CategoryResult {
    /// Observed bytes in one eligibility state.
    pub fn eligibility_observed_bytes(&self, eligibility: CleanupEligibility) -> u64 {
        self.eligibility.observed_bytes(eligibility)
    }

    /// Retained items in one eligibility state.
    pub fn eligibility_items(&self, eligibility: CleanupEligibility) -> u64 {
        self.eligibility.items(eligibility)
    }

    /// States the aggregates again from the items the category still retains.
    ///
    /// Removing a unit that another unit already accounted for changes every
    /// byte total and the eligibility breakdown. The category restates them
    /// from what is left instead of subtracting the removed unit from each
    /// field, so the totals stay a function of the retained items rather than
    /// of the order the removals happened in.
    ///
    /// `quality` is not recomputed: it may have been stated by the scan (a
    /// cancelled category is `Partial` whatever its items look like).
    pub(super) fn recompute_accounting(&mut self) {
        let mut total_bytes = 0u64;
        let mut cleanable_bytes = 0u64;
        let mut safe_bytes = 0u64;
        let mut rebuild_bytes = 0u64;
        let mut manual_bytes = 0u64;
        let mut eligibility = EligibilitySummary::default();
        for item in &self.items {
            let observed = item.observed_bytes();
            let cleanable = item.cleanable_bytes();
            total_bytes += observed;
            cleanable_bytes += cleanable;
            eligibility.add(item);
            if item.disposition.is_cleanable() {
                match item.risk {
                    RiskTier::Safe => safe_bytes += cleanable,
                    RiskTier::Rebuild => rebuild_bytes += cleanable,
                    RiskTier::Manual => {}
                }
            } else if item.risk == RiskTier::Manual {
                manual_bytes += observed;
            }
        }
        self.total_bytes = total_bytes;
        self.cleanable_bytes = cleanable_bytes;
        self.safe_bytes = safe_bytes;
        self.rebuild_bytes = rebuild_bytes;
        self.manual_bytes = manual_bytes;
        self.eligibility = eligibility;
        self.skipped_entry_count = self.items.iter().map(|item| item.skipped_entry_count).sum();
        self.incomplete_item_count = self
            .items
            .iter()
            .filter(|item| item.quality != ObservationQuality::Fresh)
            .count() as u64;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ScanResult {
    pub scan_id: String,
    /// Backend-owned lifetime of a cleanup observation, not a deletion lease.
    pub valid_for_seconds: u32,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub started_at: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub finished_at: u64,
    pub categories: Vec<CategoryResult>,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub total_bytes: u64,
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub cleanable_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub safe_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub rebuild_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub manual_bytes: u64,
    #[serde(default = "unavailable_observation_quality")]
    pub quality: ObservationQuality,
    #[serde(default)]
    pub incomplete_reasons: Vec<String>,
    /// Sum of the categories' skipped-entry counts.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub skipped_entry_count: u64,
    /// Retained items whose observation is not `Fresh`.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub incomplete_item_count: u64,
    /// Observed and cleanable bytes per eligibility state, summed over the
    /// categories.
    #[serde(default)]
    pub eligibility: EligibilitySummary,
    /// Units suppressed as duplicates of an already-counted unit.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub suppressed_duplicate_count: u64,
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub suppressed_duplicate_bytes: u64,
    /// Units whose bytes a broader unit already accounts for.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub suppressed_overlap_count: u64,
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub suppressed_overlap_bytes: u64,
    /// Units that may be inside a broader unit's observation without proof:
    /// `total_bytes` is an upper bound of the observed union, and the union is
    /// at least `total_bytes - ambiguous_overlap_bytes`.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub ambiguous_overlap_count: u64,
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub ambiguous_overlap_bytes: u64,
    /// Whether this scan stopped because it was cancelled.
    ///
    /// A cancelled scan is incomplete for a stated reason, and the interface
    /// must be able to say *which* reason without reading prose: a user who
    /// pressed Stop sees a cancelled scan, not a scan that failed. Every other
    /// incompleteness leaves this `false` and keeps its own reason.
    #[serde(default)]
    pub cancelled: bool,
    /// What the scan observed about its own work.
    #[serde(default)]
    pub metrics: ScanMetrics,
}

/// What one scan observed about the work it did.
///
/// These are the numbers a benchmark compares and a user can be shown; they are
/// measurements of the run, not estimates of the machine. Item counts and bytes
/// stay on the result — this carries only what the result did not already
/// state.
///
/// Two of the fields describe the tree and repeat exactly across scans
/// (`visited_entries`, `directories_read`); two describe one run and are
/// reported because a bound is only real if it is measured (`duration_ms`,
/// `peak_outstanding_directory_tasks`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
pub struct ScanMetrics {
    /// Wall-clock duration of the scan.
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub duration_ms: u64,
    /// Entries the traversal visited and accounted for.
    ///
    /// Counted where the walk actually looks at an entry: a file it measured, a
    /// directory it read, or an entry it refused or could not read. A scan that
    /// silently stopped visiting entries therefore cannot look like one that
    /// visited fewer.
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub visited_entries: u64,
    /// Directories whose contents were read.
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub directories_read: u64,
    /// The highest number of directory tasks that were outstanding at once.
    ///
    /// Reported so the traversal's stated bound is a measurement rather than a
    /// claim: a run whose peak exceeds the bound is a bug the interface shows
    /// instead of a property the reader has to trust.
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub peak_outstanding_directory_tasks: u64,
}

impl ScanResult {
    pub const VALID_FOR_SECONDS: u32 = 300;

    /// Observed bytes in one eligibility state, across every category.
    pub fn eligibility_observed_bytes(&self, eligibility: CleanupEligibility) -> u64 {
        self.eligibility.observed_bytes(eligibility)
    }

    /// Retained items in one eligibility state, across every category.
    pub fn eligibility_items(&self, eligibility: CleanupEligibility) -> u64 {
        self.eligibility.items(eligibility)
    }

    pub fn is_fresh_at(&self, now: u64) -> bool {
        self.quality == ObservationQuality::Fresh
            && crate::domain::is_within_window(
                self.finished_at,
                now,
                u64::from(Self::VALID_FOR_SECONDS),
            )
    }

    pub fn validate_for_cleanup(
        &self,
        scan_id: &str,
        now: u64,
    ) -> Result<(), crate::domain::ZenithError> {
        use crate::domain::ZenithError;
        if self.scan_id != scan_id {
            return Err(ZenithError::InvalidPlan(
                "The scan is no longer current. Scan again before cleaning.".into(),
            ));
        }
        let is_current = crate::domain::is_within_window(
            self.finished_at,
            now,
            u64::from(Self::VALID_FOR_SECONDS),
        );
        if !is_current {
            return Err(ZenithError::InvalidPlan(
                "Scan expired. Scan again and review the new results before cleaning.".into(),
            ));
        }
        if !matches!(
            self.quality,
            ObservationQuality::Fresh | ObservationQuality::Partial
        ) {
            return Err(ZenithError::InvalidPlan(
                "The scan failed or is unavailable. Scan again before cleaning.".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_observation_expiry_and_clock_rollback_fail_closed() {
        let scan = ScanResult {
            cancelled: false,
            metrics: ScanMetrics::default(),
            scan_id: "fixture".into(),
            valid_for_seconds: ScanResult::VALID_FOR_SECONDS,
            started_at: 999,
            finished_at: 1000,
            categories: vec![],
            total_bytes: 0,
            cleanable_bytes: 0,
            safe_bytes: 0,
            rebuild_bytes: 0,
            manual_bytes: 0,
            quality: ObservationQuality::Fresh,
            incomplete_reasons: vec![],
            skipped_entry_count: 0,
            incomplete_item_count: 0,
            eligibility: EligibilitySummary::default(),
            suppressed_duplicate_count: 0,
            suppressed_duplicate_bytes: 0,
            suppressed_overlap_count: 0,
            suppressed_overlap_bytes: 0,
            ambiguous_overlap_count: 0,
            ambiguous_overlap_bytes: 0,
        };
        assert!(scan.is_fresh_at(1000));
        assert!(scan.is_fresh_at(1299));
        assert!(!scan.is_fresh_at(1300));
        assert!(!scan.is_fresh_at(999));
        assert!(scan.validate_for_cleanup("fixture", 1299).is_ok());
        assert!(scan.validate_for_cleanup("unknown", 1000).is_err());
        assert!(scan.validate_for_cleanup("fixture", 1300).is_err());
        assert!(scan.validate_for_cleanup("fixture", 999).is_err());
        let serialized = serde_json::to_value(&scan).unwrap();
        assert_eq!(serialized["valid_for_seconds"], 300);
        assert_eq!(serialized["quality"], "fresh");
    }

    #[test]
    fn partial_and_unavailable_scans_never_report_fresh() {
        let mut scan = ScanResult {
            cancelled: false,
            metrics: ScanMetrics::default(),
            scan_id: "fixture".into(),
            valid_for_seconds: ScanResult::VALID_FOR_SECONDS,
            started_at: 999,
            finished_at: 1000,
            categories: vec![],
            total_bytes: 100,
            cleanable_bytes: 100,
            safe_bytes: 100,
            rebuild_bytes: 0,
            manual_bytes: 0,
            quality: ObservationQuality::Partial,
            incomplete_reasons: vec!["Some directories were unreadable".into()],
            skipped_entry_count: 4,
            incomplete_item_count: 1,
            eligibility: EligibilitySummary::default(),
            suppressed_duplicate_count: 0,
            suppressed_duplicate_bytes: 0,
            suppressed_overlap_count: 0,
            suppressed_overlap_bytes: 0,
            ambiguous_overlap_count: 0,
            ambiguous_overlap_bytes: 0,
        };
        // A partial scan must NEVER report Fresh, even within the TTL window
        assert!(!scan.is_fresh_at(1000));
        assert!(!scan.is_fresh_at(1200));
        // But validate_for_cleanup allows cleaning inspected items if not expired
        assert!(scan.validate_for_cleanup("fixture", 1200).is_ok());

        // Unavailable scan cannot be cleaned
        scan.quality = ObservationQuality::Unavailable;
        assert!(!scan.is_fresh_at(1000));
        assert!(scan.validate_for_cleanup("fixture", 1200).is_err());
    }

    #[test]
    fn cache_metadata_and_large_numbers_survive_ipc_serialization() {
        const MAX_SAFE: u64 = 9_007_199_254_740_991;
        let mut item = ScanItem {
            id: "dev.uv.cache".into(),
            signature_id: "dev.uv.cache".into(),
            name: "uv cache".into(),
            category: Category::Developer,
            risk: RiskTier::Rebuild,
            path: "/Users/test/Library/Caches/uv".into(),
            size: FileSize::new(MAX_SAFE, Some(MAX_SAFE - 1)),
            file_count: 1,
            description: "owner managed".into(),
            cache_metadata: CacheMetadata {
                provider: "uv".into(),
                management_mode: CacheManagementMode::ToolManaged,
                artifact_kind: CacheArtifactKind::PackageStore,
                consequence: "re-download".into(),
                size_semantics: CacheSizeSemantics::ConservativeLowerBound,
                last_used_confidence: CacheUsageConfidence::Unknown,
            },
            disposition: CleanupDisposition::reviewable(MAX_SAFE - 1, None),
            unit: CleanupUnit::fixed_path("/Users/test/Library/Caches/uv"),
            ownership: CleanupOwnership::declared("Astral"),
            age: None,
            stale: None,
            structured_state: None,
            entry_kind: EntryKind::Directory,
            gate: EligibilityGate::Open,
            owner_running: false,
            lifecycle_provider_action: false,
            requires_confirmation: false,
            overlaps: Vec::new(),
            is_selected: false,
            last_modified: Some(MAX_SAFE - 2),
            exists: true,
            quality: ObservationQuality::Fresh,
            incomplete_reason: None,
            skipped_entry_count: MAX_SAFE - 3,
        };
        let mut json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["size"]["logical"], MAX_SAFE);
        assert_eq!(json["size"]["allocated"], MAX_SAFE - 1);
        assert_eq!(json["last_modified"], MAX_SAFE - 2);
        assert_eq!(json["skipped_entry_count"], MAX_SAFE - 3);
        assert_eq!(json["quality"], "fresh");
        assert_eq!(json["cache_metadata"]["management_mode"], "tool_managed");
        assert_eq!(json["cache_metadata"]["artifact_kind"], "package_store");
        assert_eq!(json["disposition"]["eligibility"], "reviewable");
        assert_eq!(json["disposition"]["cleanable_bytes"], MAX_SAFE - 1);

        json.as_object_mut().unwrap().remove("quality");
        json.as_object_mut().unwrap().remove("skipped_entry_count");
        json.as_object_mut().unwrap().remove("disposition");
        let legacy_item: ScanItem = serde_json::from_value(json).unwrap();
        assert_eq!(legacy_item.quality, ObservationQuality::Unavailable);
        assert_eq!(legacy_item.skipped_entry_count, 0);
        assert_eq!(
            legacy_item.disposition.eligibility,
            CleanupEligibility::Blocked
        );
        assert!(!legacy_item.allows_cleanup());

        // Test unsafe number in cleanable_bytes fails closed
        item.disposition.cleanable_bytes = Some(crate::ipc_numeric::MAX_SAFE_INTEGER + 1);
        let error =
            serde_json::to_value(&item).expect_err("unsafe cleanable_bytes must fail closed");
        assert!(error.to_string().contains("MAX_SAFE_INTEGER"));
    }
    /// A count above `Number.MAX_SAFE_INTEGER` must be refused at the IPC
    /// boundary rather than silently rounded in the browser.
    #[test]
    fn an_unsafe_skipped_entry_count_is_refused_by_the_ipc_adapter() {
        let mut item = ScanItem::mock(
            "dev.uv.cache",
            "dev.uv.cache",
            "uv cache",
            Category::Developer,
            RiskTier::Rebuild,
            "/Users/test/Library/Caches/uv",
            FileSize::new(1, Some(1)),
            1,
        );
        item.skipped_entry_count = crate::ipc_numeric::MAX_SAFE_INTEGER + 1;

        let error = serde_json::to_value(&item).expect_err("unsafe counter must fail closed");
        assert!(error.to_string().contains("MAX_SAFE_INTEGER"));
        item.skipped_entry_count = crate::ipc_numeric::MAX_SAFE_INTEGER;
        assert!(serde_json::to_value(&item).is_ok());
    }

    fn zenith_metadata() -> CacheMetadata {
        CacheMetadata {
            management_mode: CacheManagementMode::Zenith,
            ..Default::default()
        }
    }

    #[test]
    fn test_derive_cleanup_disposition_matrix() {
        let size = FileSize::new(1000, Some(1000));
        let zenith_meta = zenith_metadata();
        let tool_meta = CacheMetadata {
            management_mode: CacheManagementMode::ToolManaged,
            ..Default::default()
        };
        let advisory_meta = CacheMetadata {
            management_mode: CacheManagementMode::Advisory,
            ..Default::default()
        };
        // 1. Safe + Fresh + Zenith => AutoCleanable
        let d1 = derive_cleanup_disposition(DispositionFacts::new(
            RiskTier::Safe,
            ObservationQuality::Fresh,
            &zenith_meta,
            &size,
            None,
        ));
        assert_eq!(d1.eligibility, CleanupEligibility::AutoCleanable);
        assert_eq!(d1.cleanable_bytes, Some(1000));
        assert!(d1.is_cleanable());

        // 2. Safe + Partial + Zenith => Reviewable (never auto-cleanable)
        let d2 = derive_cleanup_disposition(DispositionFacts::new(
            RiskTier::Safe,
            ObservationQuality::Partial,
            &zenith_meta,
            &size,
            None,
        ));
        assert_eq!(d2.eligibility, CleanupEligibility::Reviewable);
        assert_eq!(d2.cleanable_bytes, Some(1000));
        assert!(d2.is_cleanable());

        // 3. Safe + Unavailable + Zenith => Blocked
        let d3 = derive_cleanup_disposition(DispositionFacts::new(
            RiskTier::Safe,
            ObservationQuality::Unavailable,
            &zenith_meta,
            &size,
            None,
        ));
        assert_eq!(d3.eligibility, CleanupEligibility::Blocked);
        assert_eq!(d3.cleanable_bytes, None);
        assert!(!d3.is_cleanable());

        // 4. Rebuild + Fresh + Zenith => Reviewable
        let d4 = derive_cleanup_disposition(DispositionFacts::new(
            RiskTier::Rebuild,
            ObservationQuality::Fresh,
            &zenith_meta,
            &size,
            None,
        ));
        assert_eq!(d4.eligibility, CleanupEligibility::Reviewable);
        assert_eq!(d4.cleanable_bytes, Some(1000));
        assert!(d4.is_cleanable());

        // 5. Rebuild + Partial + Zenith => Reviewable (never quick clean)
        let d5 = derive_cleanup_disposition(DispositionFacts::new(
            RiskTier::Rebuild,
            ObservationQuality::Partial,
            &zenith_meta,
            &size,
            None,
        ));
        assert_eq!(d5.eligibility, CleanupEligibility::Reviewable);
        assert_eq!(d5.cleanable_bytes, Some(1000));
        assert!(d5.is_cleanable());

        // 6. Manual + Fresh + Zenith => Blocked from generic cleanup
        let d6 = derive_cleanup_disposition(DispositionFacts::new(
            RiskTier::Manual,
            ObservationQuality::Fresh,
            &zenith_meta,
            &size,
            None,
        ));
        assert_eq!(d6.eligibility, CleanupEligibility::Blocked);
        assert_eq!(d6.cleanable_bytes, None);
        assert!(!d6.is_cleanable());

        // 6b. The same manual tier with a reviewed provider action behind it is
        //     offered for explicit selection: nothing generic may touch the
        //     unit, and the operation the catalog named may.
        let d6b = derive_cleanup_disposition(
            DispositionFacts::new(
                RiskTier::Manual,
                ObservationQuality::Fresh,
                &zenith_meta,
                &size,
                None,
            )
            .with_lifecycle_provider_action(true),
        );
        assert_eq!(d6b.eligibility, CleanupEligibility::Reviewable);
        assert_eq!(d6b.cleanable_bytes, Some(1000));
        assert!(d6b.is_cleanable());
        assert!(!d6b.eligibility.is_auto_cleanable());
        assert!(
            d6b.reason
                .as_deref()
                .is_some_and(|reason| reason.contains("explicit confirmation")),
            "the reason states what authorizes the action: {d6b:?}"
        );

        // 6c. A generic ProviderAction unit kind is not itself lifecycle-provider
        //     authority. Existing external-command providers use the same unit
        //     kind and must not make Manual cleanup executable by accident.
        let mut generic_provider = ScanItem::mock(
            "generic-provider",
            "dev.external",
            "External provider",
            Category::Developer,
            RiskTier::Manual,
            "provider://external",
            size,
            1,
        );
        generic_provider.unit.kind = CleanupUnitKind::ProviderAction;
        generic_provider.lifecycle_provider_action = false;
        generic_provider.rederive_disposition();
        assert_eq!(
            generic_provider.disposition.eligibility,
            CleanupEligibility::Blocked
        );

        // 7. Safe + Fresh + ToolManaged => Reviewable (provider decides, never AutoCleanable)
        let d7 = derive_cleanup_disposition(DispositionFacts::new(
            RiskTier::Safe,
            ObservationQuality::Fresh,
            &tool_meta,
            &size,
            None,
        ));
        assert_eq!(d7.eligibility, CleanupEligibility::Reviewable);
        assert_eq!(d7.cleanable_bytes, Some(1000));
        assert!(d7.is_cleanable());

        // 8. Safe + Fresh + Advisory => Advisory
        let d8 = derive_cleanup_disposition(DispositionFacts::new(
            RiskTier::Safe,
            ObservationQuality::Fresh,
            &advisory_meta,
            &size,
            None,
        ));
        assert_eq!(d8.eligibility, CleanupEligibility::Advisory);
        assert_eq!(d8.cleanable_bytes, None);
        assert!(!d8.is_cleanable());

        // 9. Nested protected .app => Blocked (even if Safe + Fresh/Partial)
        let d9 = derive_cleanup_disposition(DispositionFacts::new(
            RiskTier::Safe,
            ObservationQuality::Partial,
            &zenith_meta,
            &size,
            Some("Protected application bundle encountered in /Library/Caches/something.app"),
        ));
        assert_eq!(d9.eligibility, CleanupEligibility::Blocked);
        assert_eq!(d9.cleanable_bytes, None);
        assert!(!d9.is_cleanable());
    }

    /// A discovered unit that is not old enough is inventory, not a candidate:
    /// it keeps its observed bytes and gains a reason, and it contributes no
    /// cleanable byte.
    #[test]
    fn a_unit_that_is_too_recent_is_inventoried_but_not_cleanable() {
        let size = FileSize::new(4_000_000_000, Some(4_000_000_000));
        let now = 1_800_000_000;
        let meta = zenith_metadata();

        let fresh = AgeObservation::evaluate(7, Some(now - 3_600), now);
        let recent = derive_cleanup_disposition(
            DispositionFacts::new(
                RiskTier::Safe,
                ObservationQuality::Fresh,
                &meta,
                &size,
                None,
            )
            .with_age(Some(&fresh)),
        );
        assert_eq!(recent.eligibility, CleanupEligibility::Recent);
        assert_eq!(recent.cleanable_bytes, None);
        assert!(!recent.is_cleanable());
        assert!(recent.reason.unwrap().contains("7 days"));

        let stale = AgeObservation::evaluate(7, Some(now - 30 * 86_400), now);
        let cleanable = derive_cleanup_disposition(
            DispositionFacts::new(
                RiskTier::Safe,
                ObservationQuality::Fresh,
                &meta,
                &size,
                None,
            )
            .with_age(Some(&stale)),
        );
        assert_eq!(cleanable.eligibility, CleanupEligibility::AutoCleanable);

        // An age policy never rescues a unit that is otherwise not cleanable:
        // the safety and advisory rules are decided first.
        let advisory = CacheMetadata {
            management_mode: CacheManagementMode::Advisory,
            ..Default::default()
        };
        let gated = derive_cleanup_disposition(
            DispositionFacts::new(
                RiskTier::Safe,
                ObservationQuality::Fresh,
                &advisory,
                &size,
                None,
            )
            .with_age(Some(&stale)),
        );
        assert_eq!(gated.eligibility, CleanupEligibility::Advisory);
    }

    /// Discovery and deletion permission are separate: a unit the settings do
    /// not clean is still discovered, still measured, and never selectable.
    #[test]
    fn a_policy_gated_unit_is_discovered_without_becoming_cleanable() {
        let size = FileSize::new(2_500_000_000, Some(2_500_000_000));
        let meta = zenith_metadata();
        let disposition = derive_cleanup_disposition(
            DispositionFacts::new(
                RiskTier::Safe,
                ObservationQuality::Fresh,
                &meta,
                &size,
                None,
            )
            .with_gate(EligibilityGate::IntensiveCleanupDisabled),
        );

        assert_eq!(disposition.eligibility, CleanupEligibility::PolicyGated);
        assert_eq!(disposition.cleanable_bytes, None);
        assert!(!disposition.is_cleanable());
        assert!(disposition.reason.unwrap().contains("intensive cleanup"));

        let mut item = ScanItem::mock(
            "system.intensive.user_app_caches.0.third.party",
            "system.intensive.user_app_caches",
            "third.party",
            Category::System,
            RiskTier::Safe,
            "/Users/test/Library/Caches/third.party",
            size,
            10,
        );
        item.gate = EligibilityGate::IntensiveCleanupDisabled;
        item.unit = CleanupUnit::child_namespace(
            "/Users/test/Library/Caches",
            "/Users/test/Library/Caches/third.party",
        );
        item = item.with_derived_disposition();

        assert!(item.has_current_disposition());
        assert_eq!(item.observed_bytes(), 2_500_000_000);
        assert_eq!(item.cleanable_bytes(), 0);
        assert!(!item.is_selected);

        // The gate is the only thing standing between this unit and a plan:
        // with the opt-in on, the same facts produce an eligible unit.
        item.gate = EligibilityGate::Open;
        item = item.with_derived_disposition();
        assert_eq!(
            item.disposition.eligibility,
            CleanupEligibility::AutoCleanable
        );
        assert_eq!(item.cleanable_bytes(), 2_500_000_000);
    }

    /// A cache whose owner is running is never automatic, whatever its age.
    #[test]
    fn a_running_owner_keeps_its_cache_selectable_but_not_automatic() {
        let size = FileSize::new(4_000_000, Some(4_000_000));
        let meta = zenith_metadata();
        let disposition = derive_cleanup_disposition(
            DispositionFacts::new(
                RiskTier::Safe,
                ObservationQuality::Fresh,
                &meta,
                &size,
                None,
            )
            .with_running_owner(true),
        );
        assert_eq!(disposition.eligibility, CleanupEligibility::Reviewable);
        assert_eq!(disposition.cleanable_bytes, Some(4_000_000));
        assert!(disposition.is_cleanable(), "the user may still choose it");
        assert!(disposition
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("running")));

        let mut item = ScanItem::mock(
            "system.intensive.user_app_caches.0.com.example.app",
            "system.intensive.user_app_caches",
            "com.example.app",
            Category::System,
            RiskTier::Safe,
            "/Users/test/Library/Caches/com.example.app",
            size,
            12,
        );
        item.owner_running = true;
        item.unit = CleanupUnit::child_namespace(
            "/Users/test/Library/Caches",
            "/Users/test/Library/Caches/com.example.app",
        );
        let item = item.with_derived_disposition();
        assert!(item.has_current_disposition());
        assert_eq!(item.disposition.eligibility, CleanupEligibility::Reviewable);
        assert!(
            !item.is_selected,
            "an application that is running keeps its cache out of the default selection"
        );
        assert!(
            item.allows_cleanup(),
            "an explicit selection is still allowed"
        );
    }

    /// The breakdown is the explanation: discovered bytes partition into the
    /// states, and only the cleanable states carry cleanable bytes.
    #[test]
    fn the_eligibility_breakdown_partitions_discovered_bytes() {
        let recent_size = FileSize::new(600, Some(600));
        let stale_size = FileSize::new(400, Some(400));
        let blocked_size = FileSize::new(200, Some(200));
        let now = 1_800_000_000;

        let mut stale = ScanItem::mock(
            "sig.stale",
            "sig",
            "stale cache",
            Category::System,
            RiskTier::Safe,
            "/tmp/stale",
            stale_size,
            2,
        );
        let age = AgeObservation::evaluate(7, Some(now - 30 * 86_400), now);
        stale.age = Some(age);
        stale.unit = CleanupUnit::child_namespace("/tmp", "/tmp/stale");
        let stale = stale.with_derived_disposition();

        let mut recent = ScanItem::mock(
            "sig.recent",
            "sig",
            "recent cache",
            Category::System,
            RiskTier::Safe,
            "/tmp/recent",
            recent_size,
            3,
        );
        let age = AgeObservation::evaluate(7, Some(now - 3_600), now);
        recent.age = Some(age);
        recent.unit = CleanupUnit::child_namespace("/tmp", "/tmp/recent");
        let recent = recent.with_derived_disposition();

        let mut blocked = ScanItem::mock(
            "sig.blocked",
            "sig",
            "blocked cache",
            Category::System,
            RiskTier::Safe,
            "/tmp/blocked",
            blocked_size,
            1,
        );
        blocked.quality = ObservationQuality::Unavailable;
        blocked.incomplete_reason = Some("Permission denied".into());
        let blocked = blocked.with_derived_disposition();

        assert_eq!(
            stale.disposition.eligibility,
            CleanupEligibility::AutoCleanable
        );
        assert_eq!(recent.disposition.eligibility, CleanupEligibility::Recent);
        assert_eq!(blocked.disposition.eligibility, CleanupEligibility::Blocked);

        let mut summary = EligibilitySummary::default();
        for item in [&stale, &recent, &blocked] {
            summary.add(item);
        }

        assert_eq!(summary.buckets.len(), CleanupEligibility::ALL.len());
        assert_eq!(
            summary.observed_bytes(CleanupEligibility::AutoCleanable),
            400
        );
        assert_eq!(summary.observed_bytes(CleanupEligibility::Recent), 600);
        assert_eq!(summary.observed_bytes(CleanupEligibility::Blocked), 200);
        assert_eq!(summary.items(CleanupEligibility::Recent), 1);
        // A state that is not cleanable never reports cleanable bytes.
        assert_eq!(summary.cleanable_bytes(CleanupEligibility::Recent), 0);
        assert_eq!(summary.cleanable_bytes(CleanupEligibility::Blocked), 0);
        assert_eq!(
            summary.cleanable_bytes(CleanupEligibility::AutoCleanable),
            400
        );

        // One recent child neither erases nor inflates its stale sibling.
        let observed_total: u64 = summary.buckets.iter().map(|b| b.observed_bytes).sum();
        assert_eq!(
            observed_total,
            stale.observed_bytes() + recent.observed_bytes() + blocked.observed_bytes()
        );
    }

    /// Merging summaries is how categories roll up: bucket by bucket, with no
    /// state dropped and no double count.
    #[test]
    fn summaries_merge_without_losing_a_state() {
        let mut left = EligibilitySummary::default();
        left.add(&ScanItem::mock(
            "sig.left",
            "sig",
            "left",
            Category::System,
            RiskTier::Safe,
            "/tmp/left",
            FileSize::new(10, Some(10)),
            1,
        ));
        let mut right = EligibilitySummary::default();
        right.add(&ScanItem::mock(
            "sig.right",
            "sig",
            "right",
            Category::System,
            RiskTier::Safe,
            "/tmp/right",
            FileSize::new(25, Some(25)),
            1,
        ));

        left.merge(&right);
        assert_eq!(left.observed_bytes(CleanupEligibility::AutoCleanable), 35);
        assert_eq!(left.items(CleanupEligibility::AutoCleanable), 2);
        for bucket in &left.buckets {
            if bucket.eligibility != CleanupEligibility::AutoCleanable {
                assert_eq!(bucket.observed_bytes, 0);
                assert_eq!(bucket.items, 0);
            }
        }
    }

    #[test]
    fn cleanable_bytes_fail_closed_for_inconsistent_dispositions() {
        let mut item = ScanItem::mock(
            "test.item",
            "test.signature",
            "Test item",
            Category::Developer,
            RiskTier::Safe,
            "/tmp/test-item",
            FileSize::new(100, Some(100)),
            1,
        );

        item.disposition = CleanupDisposition::new(
            CleanupEligibility::Blocked,
            Some("blocked".into()),
            Some(100),
        );
        assert_eq!(item.cleanable_bytes(), 0);

        item.disposition = CleanupDisposition::reviewable(500, None);
        assert_eq!(item.cleanable_bytes(), 100);

        // The three byte populations every total depends on stay ordered.
        item.disposition = CleanupDisposition::recent("too new");
        assert_eq!(item.cleanable_bytes(), 0);
        assert_eq!(item.observed_bytes(), 100);
        assert!(item.cleanable_bytes() <= item.observed_bytes());
    }
}
