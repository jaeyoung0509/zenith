use super::owner::{OwnerProviderAuthorization, OwnerProviderUnit};
use super::structured::EntryKind;
use super::CleanStrategy;
use crate::domain::identity::CleanupIdentity;
use crate::domain::scan::{CleanupOwnership, CleanupUnit};
use crate::domain::{RiskSummary, RiskTier};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// What a plan authorizes doing to its targets.
///
/// Preview and mutation are separate values rather than a flag on a mutation,
/// and permanent deletion is separate from a move to Trash, because "the user
/// looked at a projection of this plan" and "the bytes are gone" are different
/// claims. A plan that reaches a mutation primitive carrying [`Self::Preview`]
/// is a bug in the caller, and the executor refuses it rather than guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CleanupMode {
    /// Nothing is mutated: the plan is a projection the user reviews.
    Preview,
    /// Targets are removed from the filesystem.
    #[default]
    PermanentDelete,
    /// Targets are moved to the platform's recoverable location.
    Trash,
}

impl CleanupMode {
    /// Whether this mode authorizes a destructive filesystem operation.
    pub fn is_mutating(&self) -> bool {
        matches!(self, Self::PermanentDelete | Self::Trash)
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Preview => "Preview only",
            Self::PermanentDelete => "Permanently deletes",
            Self::Trash => "Moves to Trash",
        }
    }
}

/// Executables whose running state makes a cleanup unsafe.
///
/// A cache that a compiler or a runtime is writing right now is not stale
/// storage; removing it mid-write corrupts the tool's state or makes it rebuild
/// from scratch under a lock it still holds. The catalog states which
/// executables matter for a signature, and the platform layer answers whether
/// any of them is running — the policy is data, the probe is an OS call, and
/// neither can decide the other's half.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunningProcessPolicy {
    executables: Vec<String>,
}

impl RunningProcessPolicy {
    /// No process guards this target.
    pub fn none() -> Self {
        Self::default()
    }

    /// A guard that refuses the cleanup while any listed executable runs.
    pub fn guarding(executables: Vec<String>) -> Self {
        Self { executables }
    }

    pub fn executables(&self) -> &[String] {
        &self.executables
    }

    pub fn is_empty(&self) -> bool {
        self.executables.is_empty()
    }

    /// Whether a running process name matches this policy.
    ///
    /// Comparison is ASCII case-insensitive on both platforms: Windows reports
    /// `Python.EXE` for the same binary that POSIX reports as `python`, and a
    /// guard that missed one spelling would be no guard at all.
    pub fn matches(&self, process_name: &str) -> bool {
        self.executables
            .iter()
            .any(|expected| expected.eq_ignore_ascii_case(process_name))
    }

    /// Whether any of the running process names matches this policy.
    pub fn matches_any<'a>(&self, running: impl IntoIterator<Item = &'a str>) -> bool {
        running.into_iter().any(|name| self.matches(name))
    }
}

/// One authorized deletion target.
///
/// `identity` is `None` only for pseudo-path strategies (`DockerPrune`, which
/// runs through a provider CLI and never touches a host path) and for a target
/// that was already gone when the plan was built. Every filesystem strategy
/// that can be revalidated carries a captured [`CleanupIdentity`] so the
/// executor can fail closed when the path changed after planning.
///
/// The remaining fields are the scan-time expectations the execution guard
/// re-asserts immediately before mutating: which unit authorized the path, what
/// kind of entry the scan saw, who owns it, and whether a running process makes
/// the cleanup unsafe. Execution re-derives all of them from the filesystem and
/// the process table rather than trusting the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteTarget {
    pub item_id: String,
    pub signature_id: String,
    pub name: String,
    pub path: PathBuf,
    pub strategy: CleanStrategy,
    pub expected_bytes: u64,
    pub risk: RiskTier,
    pub identity: Option<CleanupIdentity>,
    pub exclusions: Vec<String>,
    pub min_age_days: Option<u32>,
    /// The deletable object this target names, and the root that authorized it.
    pub unit: CleanupUnit,
    /// The entry kind the scan observed, so a type change is detected.
    pub target_kind: EntryKind,
    /// Who the catalog expects to own this location.
    pub owner: CleanupOwnership,
    /// Executables whose running state refuses this cleanup.
    pub process_guard: RunningProcessPolicy,
    /// The lifecycle provider the catalog named for this target's action.
    ///
    /// `None` for every strategy that does not run through one, and for a plan
    /// that names none at all — execution refuses a lifecycle target whose
    /// provider id is absent rather than picking an implementation.
    pub provider_id: Option<String>,
    /// Whether this target's provider contract requires explicit user
    /// confirmation before execution.
    pub requires_confirmation: bool,
}

/// Backend-private authorization state for one cleanup run.
///
/// This type is never serialized. The frontend submits a scan ID, selected
/// item IDs, and an opaque one-shot plan ID; it never supplies a path, a
/// strategy, or an identity. Extending this type with `Serialize` or
/// `specta::Type` would turn mutation authority into a replayable payload, so
/// the projections in [`crate::application::dto::cleanup`] exist instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletePlan {
    pub id: Uuid,
    pub scan_id: String,
    pub targets: Vec<DeleteTarget>,
    /// Selected items this plan did not authorize, each with the reason.
    ///
    /// The selection is not all-or-nothing: an item a current policy refuses
    /// is named here while the rest of the selection is still authorized, so a
    /// refusal never discards the inventory the user was looking at.
    pub refusals: Vec<PlanItemRefusal>,
    /// Owner-scoped provider authorizations, one per provider the selection
    /// reached.
    ///
    /// This is the plan's second variant of authority, and it is a value of its
    /// own rather than a flag or a pseudo target: a provider authorization
    /// names units by the provider's identity and carries the identity that
    /// provider captured, and no filesystem strategy classifies it. It cannot
    /// be a [`DeleteTarget`], because there is nothing generic to carry out —
    /// the registry dispatches it by provider id, and the provider re-derives
    /// every fact it mutates on when it runs.
    pub owner_authorizations: Vec<OwnerProviderAuthorization>,
    pub expected_reclaim_bytes: u64,
    pub risk: RiskSummary,
    pub created_at: u64,
    /// What this plan authorizes; the executor refuses a non-mutating mode.
    pub mode: CleanupMode,
}

impl DeletePlan {
    /// Whether any target or provider authorization in this plan requires an
    /// explicit confirmation token at the destructive boundary.
    pub fn requires_confirmation(&self) -> bool {
        self.targets
            .iter()
            .any(|target| target.requires_confirmation)
            || self
                .owner_authorizations
                .iter()
                .any(|authorization| authorization.requires_confirmation)
    }

    /// Every owner-provider unit this plan authorizes, provider included.
    pub fn owner_units(&self) -> impl Iterator<Item = (&str, &OwnerProviderUnit)> {
        self.owner_authorizations
            .iter()
            .flat_map(|authorization| {
                authorization
                    .units
                    .iter()
                    .map(move |unit| (authorization.provider_id.as_str(), unit))
            })
    }
}


/// Why one cleanup step did not happen, in the closed vocabulary the interface
/// derives its copy from.
///
/// A message is not an outcome: two failures that read the same may need
/// different remedies, and the interface decides which remedy to offer from the
/// kind rather than by parsing prose. The domain owns the vocabulary because
/// planning and execution both produce refusals and both must name them the
/// same way; the projection in [`crate::application::dto::cleanup`] carries it
/// to the interface unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CleanFailureReason {
    PermissionDenied,
    ChangedSinceScan,
    NotFound,
    InUse,
    Blacklisted,
    /// The target matched structured state (a database, its companions, a
    /// lock, a credential, configuration, a bundle, or an executable) that
    /// generic cleanup never removes.
    StructuredStore,
    /// The target is now a link, a reparse point, a junction, or a mount
    /// boundary: traversal and deletion stop there.
    SafetyBoundary,
    ExternalCommandFailed,
    /// A reviewed lifecycle provider was named for the target, and this build
    /// has no adapter that can perform its action here.
    ProviderUnavailable,
    /// The location belongs to a program that maintains it itself, and no
    /// reviewed provider in this build may remove it. The store stays
    /// inventoried and measured; the refusal is about who owns it, not about
    /// what is inside it.
    OwnerManaged,
    /// The provider ran (or re-checked itself) and did not reach the state its
    /// action promises. Its own message states which prerequisite or refusal
    /// applied.
    ProviderRefused,
    Unknown,
}

impl CleanFailureReason {
    pub fn user_message(&self, target_name: &str) -> String {
        match self {
            CleanFailureReason::PermissionDenied => {
                format!("The operating system denied permission to clean {}. Check your system's storage and privacy permissions.", target_name)
            }
            CleanFailureReason::ChangedSinceScan => {
                format!("{} changed on disk since the last scan. Aborted cleaning to prevent data corruption.", target_name)
            }
            CleanFailureReason::NotFound => {
                format!("{} was already removed or does not exist.", target_name)
            }
            CleanFailureReason::InUse => {
                format!(
                    "{} is currently locked or in use by another running process.",
                    target_name
                )
            }
            CleanFailureReason::Blacklisted => {
                format!(
                    "{} matches a protected system security rule and cannot be modified.",
                    target_name
                )
            }
            CleanFailureReason::StructuredStore => {
                format!(
                    "{} holds application state rather than regenerable cache data, so generic cleanup leaves it alone.",
                    target_name
                )
            }
            CleanFailureReason::SafetyBoundary => {
                format!(
                    "{} changed into a link, a mount point, or another indirection. Zenith refuses to delete through it.",
                    target_name
                )
            }
            CleanFailureReason::ExternalCommandFailed => {
                format!(
                    "Failed to execute external clean helper for {}.",
                    target_name
                )
            }
            CleanFailureReason::ProviderUnavailable => {
                format!(
                    "{} is cleaned through a dedicated provider, and no provider adapter for it is available on this platform.",
                    target_name
                )
            }
            CleanFailureReason::ProviderRefused => {
                format!(
                    "The dedicated provider for {} did not complete its action.",
                    target_name
                )
            }
            CleanFailureReason::OwnerManaged => {
                format!(
                    "{} is a store the program that owns it maintains; Zenith inventories it and does not delete it.",
                    target_name
                )
            }
            CleanFailureReason::Unknown => {
                format!(
                    "An unexpected error occurred while cleaning {}.",
                    target_name
                )
            }
        }
    }
}

/// One selected item a plan refused to authorize, and why.
///
/// A refusal is stated per item rather than raised as a failed operation: the
/// rest of a selection is still a plan, and the interface marks the rows this
/// names instead of discarding the inventory the user was looking at. The typed
/// reason travels with the message so a caller never has to read prose to
/// decide whether the item can be retried or the whole scan has to be redone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItemRefusal {
    pub item_id: String,
    pub item_name: String,
    pub reason: CleanFailureReason,
    /// The planner's own words about this item, which name the entry that
    /// caused the refusal.
    pub message: String,
}
