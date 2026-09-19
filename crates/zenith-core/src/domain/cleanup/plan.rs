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
    pub expected_reclaim_bytes: u64,
    pub risk: RiskSummary,
    pub created_at: u64,
    /// What this plan authorizes; the executor refuses a non-mutating mode.
    pub mode: CleanupMode,
}

impl DeletePlan {
    /// Whether any target in this plan requires an explicit confirmation token
    /// at the destructive boundary.
    pub fn requires_confirmation(&self) -> bool {
        self.targets.iter().any(|target| target.requires_confirmation)
    }
}
