//! What the storage workflows report across the interface boundary.
//!
//! Every structure here is an observation: an inventory, a scan result, a
//! preview, or a progress step. The taxonomy those observations are described
//! with — what kind of large file this is, what a filter means, how strong a
//! relationship is — lives in [`crate::domain::storage`]. The authority that
//! turns a selection into a mutation never appears here: a `TrashPlanPreview`
//! carries a plan ID and byte totals, not the plan.

use crate::domain::storage::{
    AppInstallSource, AppRelatedConfidence, AppRelatedKind, LargeFileFilter, LargeFileKind,
};
use crate::domain::ObservationQuality;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LargeFileItem {
    pub id: String,
    pub name: String,
    pub display_parent: String,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub logical_size: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub allocated_size: u64,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub modified_at: Option<u64>,
    pub kind: LargeFileKind,
    pub extension: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LargeFileScanRequest {
    pub roots: Vec<String>,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub min_size_bytes: u64,
    #[serde(default)]
    pub filter: LargeFileFilter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LargeFileScanResult {
    pub scan_id: String,
    pub items: Vec<LargeFileItem>,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub entries_scanned: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub skipped_entries: u64,
    pub cancelled: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LargeFileScanEvent {
    Started {
        scan_id: String,
    },
    RootStarted {
        root: String,
    },
    Progress {
        root: String,
        #[serde(with = "crate::ipc_numeric::u64")]
        #[specta(type = u64)]
        entries_scanned: u64,
        #[serde(with = "crate::ipc_numeric::u64")]
        #[specta(type = u64)]
        matches_found: u64,
    },
    ItemFound {
        item: LargeFileItem,
    },
    RootFinished {
        root: String,
    },
    Finished {
        result: LargeFileScanResult,
    },
    Cancelled {
        scan_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct InstalledApp {
    pub id: String,
    pub name: String,
    pub bundle_id: Option<String>,
    pub version: Option<String>,
    pub display_path: String,
    pub executable_name: Option<String>,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub logical_size: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub allocated_size: u64,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub modified_at: Option<u64>,
    pub install_source: AppInstallSource,
    pub is_running: bool,
    pub is_system_protected: bool,
    #[serde(default)]
    pub quality: ObservationQuality,
    #[serde(default)]
    pub size_quality: ObservationQuality,
    #[serde(default)]
    pub incomplete_reason: Option<String>,
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub skipped_entries: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct InstalledAppInventory {
    pub apps: Vec<InstalledApp>,
    pub quality: ObservationQuality,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub skipped_entry_count: u64,
    pub incomplete_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AppRelatedItem {
    pub id: String,
    pub name: String,
    pub display_path: String,
    pub kind: AppRelatedKind,
    pub confidence: AppRelatedConfidence,
    pub evidence: String,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub logical_size: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub allocated_size: u64,
    pub selected_by_default: bool,
    #[serde(default)]
    pub quality: ObservationQuality,
    #[serde(default)]
    pub incomplete_reason: Option<String>,
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub skipped_entries: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AppUninstallInspection {
    pub inspection_id: String,
    pub app: InstalledApp,
    pub related_items: Vec<AppRelatedItem>,
    pub incomplete: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct TrashPlanPreview {
    pub id: uuid::Uuid,
    pub item_count: usize,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub logical_size: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub allocated_size: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub expires_at: u64,
    #[serde(default)]
    pub size_is_lower_bound: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct TrashItemResult {
    pub item_id: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct TrashResult {
    pub moved_count: usize,
    pub failed_count: usize,
    pub skipped_count: usize,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub moved_allocated_size: u64,
    pub items: Vec<TrashItemResult>,
    #[serde(default)]
    pub size_is_lower_bound: bool,
}
