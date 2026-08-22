use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum LargeFileKind {
    Video,
    Archive,
    DiskImage,
    VmImage,
    AiModel,
    Database,
    DeveloperArtifact,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LargeFileItem {
    pub id: String,
    pub name: String,
    pub display_parent: String,
    pub logical_size: u64,
    pub allocated_size: u64,
    pub modified_at: Option<u64>,
    pub kind: LargeFileKind,
    pub extension: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LargeFileScanRequest {
    pub roots: Vec<String>,
    pub min_size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LargeFileScanResult {
    pub scan_id: String,
    pub items: Vec<LargeFileItem>,
    pub entries_scanned: u64,
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
        entries_scanned: u64,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AppInstallSource {
    ApplicationBundle,
    HomebrewCask,
    InstallerPackage,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct InstalledApp {
    pub id: String,
    pub name: String,
    pub bundle_id: Option<String>,
    pub version: Option<String>,
    pub display_path: String,
    pub executable_name: Option<String>,
    pub logical_size: u64,
    pub allocated_size: u64,
    pub modified_at: Option<u64>,
    pub install_source: AppInstallSource,
    pub is_running: bool,
    pub is_system_protected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AppRelatedConfidence {
    High,
    Medium,
    Shared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AppRelatedKind {
    AppBundle,
    ApplicationSupport,
    Cache,
    Log,
    Preference,
    SavedState,
    Container,
    GroupContainer,
    ApplicationScripts,
    HttpStorage,
    WebKit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AppRelatedItem {
    pub id: String,
    pub name: String,
    pub display_path: String,
    pub kind: AppRelatedKind,
    pub confidence: AppRelatedConfidence,
    pub evidence: String,
    pub logical_size: u64,
    pub allocated_size: u64,
    pub selected_by_default: bool,
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
    pub logical_size: u64,
    pub allocated_size: u64,
    pub expires_at: u64,
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
    pub moved_allocated_size: u64,
    pub items: Vec<TrashItemResult>,
}
