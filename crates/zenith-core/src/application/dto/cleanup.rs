//! Projections of cleanup state onto the interface contract.
//!
//! These types are intentionally serializable. They describe a plan or a run;
//! they cannot reconstruct one. In particular [`PlanPreview`] carries the plan
//! ID, reviewed target paths, item IDs, and byte totals — never a strategy or
//! captured filesystem identity. Paths are backend-derived display evidence;
//! they are not accepted back as mutation authority.

pub use crate::domain::cleanup::CleanFailureReason;

use crate::domain::cleanup::{CleanupMode, DeletePlan};
use crate::domain::{RiskSummary, RiskTier};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PlanTargetPreview {
    pub item_id: String,
    pub name: String,
    /// The backend-resolved path the user is about to affect. Execution still
    /// consumes only the opaque plan id and revalidates its private target.
    pub path: String,
    /// The mutation channel this individual target will use. A mixed plan can
    /// therefore state which reviewed paths are recoverable instead of asking
    /// the frontend to infer policy from a risk badge.
    pub mode: CleanupMode,
    pub requires_confirmation: bool,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub expected_bytes: u64,
    pub risk: RiskTier,
}

/// One selected item a plan did not authorize, with the reason.
///
/// A refusal is stated per item rather than as a failed operation: the rest of
/// the selection is still a plan, and the interface marks the rows this names
/// instead of discarding the inventory the user was looking at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PlanRefusalPreview {
    pub item_id: String,
    pub name: String,
    /// The typed outcome the copy is derived from. The interface must not have
    /// to read [`Self::message`] to decide whether the item can be retried or
    /// the whole scan has to be redone.
    pub reason: CleanFailureReason,
    pub message: String,
}

/// What a refused cleanup operation is about.
///
/// The interface holds state — a scan, a selection — that a failure either
/// invalidates or does not, and only the backend knows which. A single error
/// string cannot say it: "this item is refused under a current policy" and
/// "the inventory this names is gone" need opposite reactions, and treating
/// the first like the second is what makes a correct refusal look like another
/// broken selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CleanupFailureScope {
    /// The scan this operation named is gone, expired, or was replaced.
    /// The inventory it refers to must be rebuilt: an interface that keeps
    /// showing it is showing a measurement of a machine that has changed.
    InventoryStale,
    /// The one-shot plan is gone; a still-current scan can create a new plan.
    PlanUnavailable,
    /// One or more selected items were refused under a current policy. The
    /// inventory and every other selection remain usable, and the refusal is
    /// stated for the items it names.
    Items,
    /// A store's owner is running, so the store is in use right now.
    ProviderBusy,
    /// The platform refused the operation for want of a permission the user
    /// can grant.
    Permission,
    /// The user cancelled the operation.
    Cancelled,
    /// An unexpected backend failure.
    Internal,
}

/// A refused cleanup operation, with the scope the interface reacts to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CleanupFailure {
    pub scope: CleanupFailureScope,
    pub reason: CleanFailureReason,
    pub message: String,
    /// The items the refusal names, when the scope is item-shaped.
    pub items: Vec<PlanRefusalPreview>,
}

impl CleanupFailure {
    /// A failure about the items a policy or a provider refused.
    pub fn items(message: impl Into<String>, items: Vec<PlanRefusalPreview>) -> Self {
        let reason = items
            .first()
            .map(|item| item.reason)
            .unwrap_or(CleanFailureReason::Unknown);
        Self {
            scope: CleanupFailureScope::Items,
            reason,
            message: message.into(),
            items,
        }
    }

    /// A failure that makes the inventory the caller holds unusable.
    pub fn inventory_stale(message: impl Into<String>) -> Self {
        Self {
            scope: CleanupFailureScope::InventoryStale,
            reason: CleanFailureReason::ChangedSinceScan,
            message: message.into(),
            items: Vec::new(),
        }
    }

    /// A failure with a stated scope and no items of its own.
    pub fn new(
        scope: CleanupFailureScope,
        reason: CleanFailureReason,
        message: impl Into<String>,
    ) -> Self {
        Self {
            scope,
            reason,
            message: message.into(),
            items: Vec::new(),
        }
    }

    /// Whether the inventory the interface holds is still usable.
    pub fn invalidates_inventory(&self) -> bool {
        matches!(self.scope, CleanupFailureScope::InventoryStale)
    }

    /// The failure for a worker that never returned its own result.
    ///
    /// A panicked or cancelled worker has no outcome of its own, so it is an
    /// internal failure rather than an answer about the user's items.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(
            CleanupFailureScope::Internal,
            CleanFailureReason::Unknown,
            message,
        )
    }
}

impl std::fmt::Display for CleanupFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}: {}", self.scope, self.message)?;
        for item in &self.items {
            write!(formatter, "; {} `{}`", item.name, item.message)?;
        }
        Ok(())
    }
}

impl From<String> for CleanupFailure {
    fn from(message: String) -> Self {
        Self::internal(message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PlanPreview {
    pub id: Uuid,
    pub targets: Vec<PlanTargetPreview>,
    /// Selected items this plan does not cover, and why. Empty when every
    /// selection was authorized.
    pub refused: Vec<PlanRefusalPreview>,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub expected_reclaim_bytes: u64,
    pub risk: RiskSummary,
    /// True when at least one target must be explicitly confirmed before the
    /// destructive command may execute this plan.
    pub requires_confirmation: bool,
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
    /// show. The projection copies the resolved path as review evidence, but
    /// drops `strategy` and `identity` by construction. Execution accepts only
    /// the opaque plan id, so displaying a path never turns it into authority.
    pub fn preview(&self, ttl_secs: u64) -> PlanPreview {
        let mut targets: Vec<PlanTargetPreview> = self
            .targets
            .iter()
            .map(|target| PlanTargetPreview {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: target.path.to_string_lossy().into_owned(),
                mode: target.execution_mode(),
                requires_confirmation: target.requires_confirmation,
                expected_bytes: target.expected_bytes,
                risk: target.risk,
            })
            .collect();
        let mut refused = Vec::new();
        for authorization in &self.owner_authorizations {
            targets.extend(authorization.units.iter().map(|unit| PlanTargetPreview {
                item_id: unit.item_id.clone(),
                name: unit.name.clone(),
                path: unit.path.to_string_lossy().into_owned(),
                mode: CleanupMode::PermanentDelete,
                requires_confirmation: authorization.requires_confirmation,
                expected_bytes: unit.expected_bytes,
                risk: authorization.risk,
            }));
            refused.extend(
                authorization
                    .refusals
                    .iter()
                    .map(|refusal| PlanRefusalPreview {
                        item_id: refusal.item_id.clone(),
                        name: refusal.item_name.clone(),
                        reason: refusal.reason,
                        message: refusal.detail.clone(),
                    }),
            );
        }
        refused.extend(self.refusals.iter().map(|refusal| PlanRefusalPreview {
            item_id: refusal.item_id.clone(),
            name: refusal.item_name.clone(),
            reason: refusal.reason,
            message: refusal.message.clone(),
        }));
        targets.sort_by(|left, right| left.item_id.cmp(&right.item_id));
        refused.sort_by(|left, right| left.item_id.cmp(&right.item_id));
        PlanPreview {
            id: self.id,
            targets,
            refused,
            expected_reclaim_bytes: self.expected_reclaim_bytes,
            risk: self.risk.clone(),
            requires_confirmation: self.requires_confirmation(),
            expires_at: self.created_at.saturating_add(ttl_secs),
            mode: self.mode,
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
    /// Bytes moved out of their reviewed location into the platform's
    /// recoverable Trash. These bytes are not reclaimed disk space until the
    /// user empties Trash, so they never contribute to `bytes_reclaimed`.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub moved_to_trash_bytes: u64,
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
    /// Bytes moved to the recoverable Trash during this run. Kept separate
    /// from reclaimed bytes so the interface does not claim free space that
    /// the filesystem still occupies.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub total_moved_to_trash_bytes: u64,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_target_preview_serializes_backend_resolved_path() {
        let preview = PlanTargetPreview {
            item_id: "cache".to_string(),
            name: "Build cache".to_string(),
            path: "/Users/example/Library/Caches/build".to_string(),
            mode: CleanupMode::Trash,
            requires_confirmation: true,
            expected_bytes: 4096,
            risk: RiskTier::Rebuild,
        };

        let value = serde_json::to_value(preview).unwrap();
        assert_eq!(
            value["path"],
            serde_json::Value::String("/Users/example/Library/Caches/build".to_string())
        );
        assert_eq!(value["expected_bytes"], 4096);
    }
}
