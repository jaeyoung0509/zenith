//! Owner-scoped cleanup: stores another program owns, and the authorization to
//! remove part of one.
//!
//! Some reclaimable storage is a store whose rules only its owner knows. Cargo
//! keeps downloaded archives in `registry/cache`, extracted sources in
//! `registry/src`, and git dependencies under `git/`; a git checkout is a
//! working tree, an extracted crate is a directory a build wrote into, and a
//! downloaded archive is a file Cargo will re-fetch. A generic recursive
//! deletion asked to judge them can only guess from names, and every guess is
//! either a name exception that the next published package defeats or an
//! over-broad permission that removes a live lock.
//!
//! The shape that does not guess is a provider that owns one store: it
//! enumerates the units it is willing to remove, states why each is or is not
//! removable, and re-derives both facts its own way immediately before it
//! mutates. This module is the vocabulary for that exchange. It carries no
//! `serde` or `specta` derives, because [`OwnerProviderAuthorization`] is
//! mutation authority: the interface never sees it, submits no path, and names
//! nothing but an opaque plan id.
//!
//! Three rules the shapes enforce:
//!
//! * a store that could not be read reports no units and no bytes, so an
//!   unreadable store is never presented as an empty one;
//! * a unit states its own state, so an advisory unit stays visible with its
//!   observed bytes instead of disappearing from a total;
//! * an outcome states what the provider's own verification observed
//!   afterwards, so an unverifiable removal is representable as partial rather
//!   than reported as clean.

use std::path::{Path, PathBuf};

use super::plan::{CleanFailureReason, RunningProcessPolicy};
use super::provider::ProviderStatus;
use crate::domain::identity::CleanupIdentity;
use crate::domain::risk::RiskTier;

/// Whether one enumerated unit may be removed by its owner's provider.
///
/// The three states are the three answers the interface has to present
/// differently: a unit that will be removed, a unit whose cleanup belongs to
/// its owner (visible, measured, not removable here), and a unit the provider
/// refuses because something about it cannot be proven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerUnitState {
    /// The provider verified the unit against its own store contract.
    Ready,
    /// The owner, not Zenith, decides when this unit goes. The unit stays
    /// inventoried with its observed bytes and is never selectable.
    Advisory,
    /// The provider found evidence it cannot rule out — an entry its store
    /// contract does not describe, a replaced root, an unreadable directory —
    /// so the unit is refused rather than partially removed.
    Blocked,
}

impl OwnerUnitState {
    /// The phrase a message uses for this state.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Advisory => "advisory",
            Self::Blocked => "blocked",
        }
    }

    /// Whether a plan may authorize this unit.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// One unit an owner-scoped provider is willing (or unwilling) to remove.
///
/// `unit_key` is the provider's own stable identity for the unit inside its
/// store — for Cargo's archive cache, the registry directory name. It is what
/// a scan item id is built from and what a selection is matched against; it is
/// never parsed back into a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerUnitObservation {
    pub unit_key: String,
    /// The absolute path of the unit, as the provider resolved it.
    pub path: PathBuf,
    /// The bytes a removal would reclaim, as the provider measured them.
    pub allocated_bytes: u64,
    /// The bytes the unit's entries occupy as files, for the interface's
    /// accounting when it reports logical sizes.
    pub logical_bytes: u64,
    /// How many entries the measurement covers.
    pub entry_count: u64,
    pub state: OwnerUnitState,
    /// Why the unit is not ready, in the provider's own words.
    pub detail: Option<String>,
}

impl OwnerUnitObservation {
    /// A unit the provider verified and would remove.
    pub fn ready(
        unit_key: impl Into<String>,
        path: PathBuf,
        logical_bytes: u64,
        allocated_bytes: u64,
        entry_count: u64,
    ) -> Self {
        Self {
            unit_key: unit_key.into(),
            path,
            allocated_bytes,
            logical_bytes,
            entry_count,
            state: OwnerUnitState::Ready,
            detail: None,
        }
    }

    /// A unit that stays visible and is never removable here.
    pub fn advisory(
        unit_key: impl Into<String>,
        path: PathBuf,
        logical_bytes: u64,
        allocated_bytes: u64,
        entry_count: u64,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            unit_key: unit_key.into(),
            path,
            allocated_bytes,
            logical_bytes,
            entry_count,
            state: OwnerUnitState::Advisory,
            detail: Some(detail.into()),
        }
    }

    /// A unit the provider refuses, with the reason it observed.
    pub fn blocked(
        unit_key: impl Into<String>,
        path: PathBuf,
        logical_bytes: u64,
        allocated_bytes: u64,
        entry_count: u64,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            unit_key: unit_key.into(),
            path,
            allocated_bytes,
            logical_bytes,
            entry_count,
            state: OwnerUnitState::Blocked,
            detail: Some(detail.into()),
        }
    }

    /// Whether a plan may authorize this unit.
    pub fn is_ready(&self) -> bool {
        self.state.is_ready()
    }
}

/// What one provider read from the store it owns.
///
/// `root` is the resolved store root, so a store that could not be enumerated
/// still reports where it looked — the interface shows the location rather than
/// a synthetic name. `status` is the store-level answer: a provider whose
/// prerequisite does not hold, or whose root could not be read at all, states
/// that once instead of reporting every unit as blocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerStoreObservation {
    pub status: ProviderStatus,
    pub root: Option<PathBuf>,
    pub units: Vec<OwnerUnitObservation>,
    /// The provider's own words about a store-level status.
    pub detail: Option<String>,
}

impl OwnerStoreObservation {
    /// A store the provider read, with the units it found.
    pub fn ready(root: Option<PathBuf>, units: Vec<OwnerUnitObservation>) -> Self {
        Self {
            status: ProviderStatus::Ready,
            root,
            units,
            detail: None,
        }
    }

    /// A store the provider cannot enumerate, and why.
    pub fn refused(
        status: ProviderStatus,
        root: Option<PathBuf>,
        detail: impl Into<String>,
    ) -> Self {
        debug_assert!(
            status.is_probe_state() && !status.is_ready(),
            "a refused store observation states a probe status that is not `Ready`"
        );
        Self {
            status,
            root,
            units: Vec::new(),
            detail: Some(detail.into()),
        }
    }

    /// Whether any unit may be removed.
    pub fn has_ready_units(&self) -> bool {
        self.status.is_ready() && self.units.iter().any(OwnerUnitObservation::is_ready)
    }
}

/// One unit the user selected, as the scan observed it.
///
/// The provider never trusts this as authorization: it re-enumerates its own
/// store and matches the selection against what it found, so a selection that
/// names a path the provider does not own produces a refusal instead of a
/// mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerProviderSelection {
    pub item_id: String,
    pub name: String,
    pub path: PathBuf,
    pub expected_bytes: u64,
}

/// One unit a provider's private plan authorizes removing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerProviderUnit {
    /// The scan item this unit is presented as.
    pub item_id: String,
    pub unit_key: String,
    pub name: String,
    /// The store root that authorized the unit.
    pub root: PathBuf,
    pub path: PathBuf,
    /// The identity the provider captured while preparing, re-verified before
    /// the first mutation.
    pub identity: CleanupIdentity,
    /// What the provider measured at preparation time, which is what the plan's
    /// preview states.
    pub expected_bytes: u64,
    pub entry_count: u64,
}

/// A selected unit the provider declined to authorize, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerUnitRefusal {
    pub item_id: String,
    pub item_name: String,
    pub status: ProviderStatus,
    /// The typed outcome the interface reacts to.
    pub reason: CleanFailureReason,
    pub detail: String,
}

/// What one provider's private plan authorizes.
///
/// This is the provider-owned variant of a cleanup plan: it names units by the
/// provider's own identity and carries the identity the provider captured. It
/// is deliberately not a [`super::plan::DeleteTarget`] — there is no strategy
/// to classify and no generic primitive that could carry it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerProviderAuthorization {
    /// The catalog entry that authorized this provider, so a plan states which
    /// signature its units came from.
    pub signature_id: String,
    pub provider_id: String,
    /// The risk tier the catalog states for the units, which is what a
    /// confirmation preview shows.
    pub risk: RiskTier,
    pub units: Vec<OwnerProviderUnit>,
    /// Selections this provider refused while preparing the plan.
    pub refusals: Vec<OwnerUnitRefusal>,
    /// Executables whose running state refuses the whole authorization.
    pub process_guard: RunningProcessPolicy,
    pub requires_confirmation: bool,
}

impl OwnerProviderAuthorization {
    /// Whether the authorization covers anything.
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// The bytes the authorization expects to reclaim.
    pub fn expected_bytes(&self) -> u64 {
        self.units.iter().fold(0u64, |total, unit| {
            total.saturating_add(unit.expected_bytes)
        })
    }
}

/// Why a provider refused to prepare a plan at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerProviderRefusal {
    pub status: ProviderStatus,
    pub detail: String,
    /// The per-selection refusals that led here, when the provider could say.
    pub refusals: Vec<OwnerUnitRefusal>,
}

impl OwnerProviderRefusal {
    pub fn new(status: ProviderStatus, detail: impl Into<String>) -> Self {
        Self {
            status,
            detail: detail.into(),
            refusals: Vec::new(),
        }
    }

    /// States which selected units were refused.
    pub fn for_selections(
        status: ProviderStatus,
        detail: impl Into<String>,
        refusals: Vec<OwnerUnitRefusal>,
    ) -> Self {
        Self {
            status,
            detail: detail.into(),
            refusals,
        }
    }
}

impl std::fmt::Display for OwnerProviderRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.status.display_name(), self.detail)
    }
}

/// What one provider verified about one unit after acting on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerUnitOutcome {
    pub item_id: String,
    pub unit_key: String,
    pub status: ProviderStatus,
    /// What the provider measured as removed.
    pub reclaimed_bytes: u64,
    /// What the provider observed remaining. `None` means its verification
    /// could not read the unit afterwards, which is reported as partial.
    pub remaining_bytes: Option<u64>,
    pub detail: Option<String>,
}

impl OwnerUnitOutcome {
    /// The unit was removed and verification observed nothing left.
    pub fn cleaned(
        item_id: impl Into<String>,
        unit_key: impl Into<String>,
        reclaimed_bytes: u64,
    ) -> Self {
        Self {
            item_id: item_id.into(),
            unit_key: unit_key.into(),
            status: ProviderStatus::Cleaned,
            reclaimed_bytes,
            remaining_bytes: Some(0),
            detail: None,
        }
    }

    /// Part of the unit remains, or its removal could not be verified.
    pub fn partially_cleaned(
        item_id: impl Into<String>,
        unit_key: impl Into<String>,
        reclaimed_bytes: u64,
        remaining_bytes: Option<u64>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            item_id: item_id.into(),
            unit_key: unit_key.into(),
            status: ProviderStatus::PartiallyCleaned,
            reclaimed_bytes,
            remaining_bytes,
            detail: Some(detail.into()),
        }
    }

    /// Nothing was removed, and why.
    pub fn refused(
        item_id: impl Into<String>,
        unit_key: impl Into<String>,
        status: ProviderStatus,
        detail: impl Into<String>,
    ) -> Self {
        debug_assert!(
            !status.is_cleaned() && status != ProviderStatus::PartiallyCleaned,
            "a refused outcome states neither `Cleaned` nor `PartiallyCleaned`"
        );
        Self {
            item_id: item_id.into(),
            unit_key: unit_key.into(),
            status,
            reclaimed_bytes: 0,
            remaining_bytes: None,
            detail: Some(detail.into()),
        }
    }

    /// Whether this unit's removal succeeded.
    pub fn is_success(&self) -> bool {
        matches!(
            self.status,
            ProviderStatus::Cleaned | ProviderStatus::PartiallyCleaned
        )
    }

    /// The provider's own words, or a phrase for its status.
    pub fn message(&self) -> String {
        self.detail
            .clone()
            .unwrap_or_else(|| self.status.display_name().to_string())
    }
}

/// What one provider action verified, unit by unit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OwnerProviderExecution {
    pub units: Vec<OwnerUnitOutcome>,
}

impl OwnerProviderExecution {
    /// The bytes every unit's own verification measured as reclaimed.
    pub fn reclaimed_bytes(&self) -> u64 {
        self.units.iter().fold(0u64, |total, unit| {
            total.saturating_add(unit.reclaimed_bytes)
        })
    }

    /// The bytes the units that were not removed had been expected to hold.
    pub fn failed_bytes(&self, expected: impl Fn(&str) -> Option<u64>) -> u64 {
        self.units
            .iter()
            .filter(|unit| !unit.is_success())
            .fold(0u64, |total, unit| {
                total.saturating_add(expected(&unit.item_id).unwrap_or(0))
            })
    }
}

/// What a measurement of one path observed.
///
/// The provider reports what its store occupies, and the bytes a filesystem
/// allocates for a file are a platform fact rather than a portable one. The
/// port exists so the measurement stays with the component that already owns
/// it (`scanner::size` on the desktop) instead of being re-derived, slightly
/// differently, inside every provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerUnitMeasurement {
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    pub entry_count: u64,
    /// Whether every entry could be read. An incomplete measurement is a lower
    /// bound, and a provider states it as one rather than as the unit's size.
    pub complete: bool,
    pub detail: Option<String>,
}

impl OwnerUnitMeasurement {
    /// A measurement that read everything it covers.
    pub fn complete(logical_bytes: u64, allocated_bytes: u64, entry_count: u64) -> Self {
        Self {
            logical_bytes,
            allocated_bytes,
            entry_count,
            complete: true,
            detail: None,
        }
    }

    /// A measurement that could not read part of what it covers.
    pub fn partial(
        logical_bytes: u64,
        allocated_bytes: u64,
        entry_count: u64,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            logical_bytes,
            allocated_bytes,
            entry_count,
            complete: false,
            detail: Some(detail.into()),
        }
    }
}

/// Measures one path for an owner-scoped provider.
///
/// A directory is measured as its whole tree, and a file as itself, so a
/// provider can measure a unit and the entries inside it with one port.
pub trait OwnerUnitMeasurer: Send + Sync {
    fn measure(&self, path: &Path) -> OwnerUnitMeasurement;
}

/// Whether any executable of `guard` is running, as the caller observed.
///
/// `None` means the process table could not be read. An owner provider that
/// cannot determine process state must not act, so the port states the
/// unanswerable case rather than folding it into "nothing is running" — the
/// difference is a live Cargo holding its own cache open.
pub trait RunningProcessProbe: Send + Sync {
    /// The executables that are running, or `None` when that cannot be read.
    fn running(&self, guard: &RunningProcessPolicy) -> Option<Vec<String>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authorization(units: Vec<OwnerProviderUnit>) -> OwnerProviderAuthorization {
        OwnerProviderAuthorization {
            signature_id: "test.store".to_string(),
            provider_id: "test.store".to_string(),
            risk: RiskTier::Rebuild,
            units,
            refusals: Vec::new(),
            process_guard: RunningProcessPolicy::none(),
            requires_confirmation: true,
        }
    }

    fn unit(item_id: &str, bytes: u64) -> OwnerProviderUnit {
        OwnerProviderUnit {
            item_id: item_id.to_string(),
            unit_key: item_id.to_string(),
            name: item_id.to_string(),
            root: PathBuf::from("/store"),
            path: PathBuf::from("/store").join(item_id),
            identity: CleanupIdentity::new(
                crate::domain::identity::FileIdentity::new(0, 0),
                true,
                0,
                crate::domain::identity::ModifiedStamp::new(0, 0),
            ),
            expected_bytes: bytes,
            entry_count: 1,
        }
    }

    /// A store that could not be read reports no units and no bytes, while an
    /// advisory unit stays visible with the bytes it occupies.
    #[test]
    fn an_unreadable_store_reports_no_units_and_an_advisory_unit_stays_visible() {
        let refused = OwnerStoreObservation::refused(
            ProviderStatus::Blocked,
            Some(PathBuf::from("/store")),
            "the store root is a link",
        );
        assert!(!refused.has_ready_units());
        assert!(refused.units.is_empty());
        assert_eq!(refused.detail.as_deref(), Some("the store root is a link"));

        let advisory = OwnerUnitObservation::advisory(
            "advisory",
            PathBuf::from("/store/advisory"),
            4_096,
            8_192,
            3,
            "its owner decides",
        );
        assert!(!advisory.is_ready());
        assert_eq!(advisory.allocated_bytes, 8_192);
        let store = OwnerStoreObservation::ready(Some(PathBuf::from("/store")), vec![advisory]);
        assert!(!store.has_ready_units(), "advisory units are not removable");
    }

    /// The authorization states what it covers, and a unit outcome states what
    /// verification observed rather than implying it.
    #[test]
    fn an_authorization_and_its_outcomes_state_what_was_measured() {
        let plan = authorization(vec![unit("a", 100), unit("b", 250)]);
        assert_eq!(plan.expected_bytes(), 350);
        assert!(!plan.is_empty());

        let execution = OwnerProviderExecution {
            units: vec![
                OwnerUnitOutcome::cleaned("a", "a", 100),
                OwnerUnitOutcome::partially_cleaned("b", "b", 50, Some(200), "one archive is busy"),
            ],
        };
        assert_eq!(execution.reclaimed_bytes(), 150);
        assert!(execution.units[0].is_success());
        assert!(execution.units[1].is_success());
        assert_eq!(
            execution.units[1].message(),
            "one archive is busy".to_string()
        );
        assert_eq!(
            execution.failed_bytes(|item_id| (item_id == "b").then_some(250)),
            0,
            "a partial unit reported its own measurement instead of a failure"
        );

        let refused = OwnerUnitOutcome::refused(
            "c",
            "c",
            ProviderStatus::PrerequisiteNotMet,
            "cargo is running",
        );
        assert!(!refused.is_success());
        assert_eq!(refused.reclaimed_bytes, 0);
        let execution = OwnerProviderExecution {
            units: vec![refused],
        };
        assert_eq!(
            execution.failed_bytes(|item_id| (item_id == "c").then_some(400)),
            400
        );
    }
}
