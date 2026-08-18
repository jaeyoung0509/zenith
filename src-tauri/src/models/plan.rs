use crate::models::{CleanStrategy, RiskSummary, RiskTier};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
    pub is_dir: bool,
    pub size: u64,
    pub mtime_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteTarget {
    pub item_id: String,
    pub signature_id: String,
    pub name: String,
    pub path: PathBuf,
    pub strategy: CleanStrategy,
    pub expected_bytes: u64,
    pub risk: RiskTier,
    pub identity: Option<FileIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletePlan {
    pub id: Uuid,
    pub targets: Vec<DeleteTarget>,
    pub expected_reclaim_bytes: u64,
    pub risk: RiskSummary,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanItemResult {
    pub item_id: String,
    pub name: String,
    pub path: String,
    pub success: bool,
    pub bytes_reclaimed: u64,
    pub failure_reason: Option<CleanFailureReason>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanResult {
    pub plan_id: Uuid,
    pub started_at: u64,
    pub finished_at: u64,
    pub total_reclaimed_bytes: u64,
    pub total_failed_bytes: u64,
    pub items: Vec<CleanItemResult>,
    pub actual_disk_free_delta: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CleanEvent {
    Started {
        plan_id: Uuid,
        total_targets: usize,
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
