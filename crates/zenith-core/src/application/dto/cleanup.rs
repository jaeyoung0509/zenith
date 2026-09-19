//! Projections of cleanup state onto the interface contract.
//!
//! These types are intentionally serializable. They describe a plan or a run;
//! they cannot reconstruct one. In particular [`PlanPreview`] carries the plan
//! ID, target item IDs, and byte totals — never a path, a strategy, or a
//! captured filesystem identity.

use crate::domain::cleanup::{CleanupMode, DeletePlan};
use crate::domain::{RiskSummary, RiskTier};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PlanTargetPreview {
    pub item_id: String,
    pub name: String,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub expected_bytes: u64,
    pub risk: RiskTier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PlanPreview {
    pub id: Uuid,
    pub targets: Vec<PlanTargetPreview>,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub expected_reclaim_bytes: u64,
    pub risk: RiskSummary,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub expires_at: u64,
    /// What executing this plan would do, stated rather than implied: a
    /// preview is a projection, and the mode is the difference between "the
    /// bytes are gone" and "the bytes are in the Trash".
    pub mode: CleanupMode,
}

impl DeletePlan {
    /// Reduces authorization state to the disposable facts the interface may
    /// show. The projection drops `path`, `strategy`, and `identity` by
    /// construction rather than by a `skip_serializing` attribute, so a
    /// future field on [`crate::domain::cleanup::DeleteTarget`] cannot leak
    /// through this call by accident.
    pub fn preview(&self, ttl_secs: u64) -> PlanPreview {
        PlanPreview {
            id: self.id,
            targets: self
                .targets
                .iter()
                .map(|target| PlanTargetPreview {
                    item_id: target.item_id.clone(),
                    name: target.name.clone(),
                    expected_bytes: target.expected_bytes,
                    risk: target.risk,
                })
                .collect(),
            expected_reclaim_bytes: self.expected_reclaim_bytes,
            risk: self.risk.clone(),
            expires_at: self.created_at.saturating_add(ttl_secs),
            mode: self.mode,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
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
            CleanFailureReason::Unknown => {
                format!(
                    "An unexpected error occurred while cleaning {}.",
                    target_name
                )
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CleanStatus {
    /// The target's postcondition holds: this run removed what the plan
    /// authorized, or a provider it ran through verified that the state the
    /// action promises already held.
    Success,
    /// The target was not removed, and nothing about it was wrong: it was
    /// already gone (a replayed plan, or a target another process removed),
    /// or executing the plan would have deleted a different object.
    Skipped,
    /// Some of the target was removed.
    Partial,
    #[default]
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CleanItemResult {
    pub item_id: String,
    pub name: String,
    pub path: String,
    pub status: CleanStatus,
    pub success: bool,
    /// What the scan measured for this target. It is an expectation the plan
    /// was built from, never a measurement of what a run removed.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub estimated_bytes: u64,
    /// What this run actually reclaimed, measured after the mutation.
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub bytes_reclaimed: u64,
    pub failure_reason: Option<CleanFailureReason>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CleanResult {
    pub plan_id: Uuid,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub started_at: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub finished_at: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub total_reclaimed_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub total_failed_bytes: u64,
    /// Targets that reclaimed some bytes but were not fully cleaned. They keep
    /// `success = true`, so the count is the only place a partial run is
    /// visible in the summary.
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub partial_count: u64,
    /// Targets that reclaimed nothing.
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub failed_count: u64,
    /// Targets that were not removed because there was nothing to remove:
    /// they were already absent, or the plan was replayed.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub skipped_count: u64,
    pub items: Vec<CleanItemResult>,
    pub actual_disk_free_delta: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type")]
pub enum CleanEvent {
    Started {
        plan_id: Uuid,
        total_targets: usize,
        #[serde(with = "crate::ipc_numeric::u64")]
        #[specta(type = u64)]
        expected_bytes: u64,
    },
    ItemStarted {
        item_id: String,
        name: String,
        index: usize,
        total: usize,
    },
    ItemFinished {
        item_id: String,
        name: String,
        status: CleanStatus,
        success: bool,
        #[serde(with = "crate::ipc_numeric::u64")]
        #[specta(type = u64)]
        reclaimed_bytes: u64,
        error: Option<String>,
    },
    Finished {
        result: CleanResult,
    },
    Error {
        message: String,
    },
}

pub trait CleanupProgressSink: Send + Sync {
    fn emit(&self, event: CleanEvent);
}

impl<F> CleanupProgressSink for F
where
    F: Fn(CleanEvent) + Send + Sync,
{
    fn emit(&self, event: CleanEvent) {
        self(event);
    }
}
