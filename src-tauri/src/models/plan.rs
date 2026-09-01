use crate::models::{CleanStrategy, RiskSummary, RiskTier};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
    pub is_dir: bool,
    pub size: u64,
    pub mtime_secs: u64,
    pub mtime_nanos: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteTarget {
    pub item_id: String,
    pub signature_id: String,
    pub name: String,
    pub path: PathBuf,
    pub strategy: CleanStrategy,
    pub expected_bytes: u64,
    pub risk: RiskTier,
    pub identity: Option<FileIdentity>,
    pub exclusions: Vec<String>,
    pub min_age_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletePlan {
    pub id: Uuid,
    pub scan_id: String,
    pub targets: Vec<DeleteTarget>,
    pub expected_reclaim_bytes: u64,
    pub risk: RiskSummary,
    pub created_at: u64,
}

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
                format!("macOS denied permission to clean {}. Check full disk access in System Settings.", target_name)
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
