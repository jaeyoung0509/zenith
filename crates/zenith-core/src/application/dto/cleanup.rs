//! Projections of cleanup state onto the interface contract.
//!
//! These types are intentionally serializable. They describe a plan or a run;
//! they cannot reconstruct one. In particular [`PlanPreview`] carries the plan
//! ID, target item IDs, and byte totals — never a path, a strategy, or a
//! captured filesystem identity.

use crate::domain::cleanup::DeletePlan;
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
    ExternalCommandFailed,
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
            CleanFailureReason::ExternalCommandFailed => {
                format!(
                    "Failed to execute external clean helper for {}.",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CleanStatus {
    Success,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CleanItemResult {
    pub item_id: String,
    pub name: String,
    pub path: String,
    pub status: CleanStatus,
    pub success: bool,
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
