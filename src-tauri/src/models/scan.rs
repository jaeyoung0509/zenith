use crate::models::{Category, RiskTier};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FileSize {
    pub logical: u64,
    pub allocated: Option<u64>,
}

impl FileSize {
    pub fn new(logical: u64, allocated: Option<u64>) -> Self {
        Self { logical, allocated }
    }

    pub fn reclaimable(&self) -> u64 {
        self.allocated.unwrap_or(self.logical)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub is_selected: bool,
    pub last_modified: Option<u64>,
    pub exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryResult {
    pub category: Category,
    pub display_name: String,
    pub items: Vec<ScanItem>,
    pub total_bytes: u64,
    pub safe_bytes: u64,
    pub rebuild_bytes: u64,
    pub manual_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanResult {
    pub scan_id: String,
    pub started_at: u64,
    pub finished_at: u64,
    pub categories: Vec<CategoryResult>,
    pub total_bytes: u64,
    pub safe_bytes: u64,
    pub rebuild_bytes: u64,
    pub manual_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ScanEvent {
    Started {
        scan_id: String,
    },
    CategoryStarted {
        category: Category,
    },
    ItemFound {
        item: ScanItem,
    },
    CategoryFinished {
        category: Category,
        bytes: u64,
        item_count: usize,
    },
    Finished {
        result: ScanResult,
    },
    Error {
        message: String,
    },
}
