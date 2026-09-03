use crate::models::{Category, RiskTier};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CacheManagementMode {
    #[default]
    Zenith,
    ToolManaged,
    Advisory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CacheArtifactKind {
    #[default]
    Temporary,
    DownloadCache,
    PackageStore,
    BuildArtifact,
    CompiledKernel,
    OptimizedEngine,
    Autotune,
    ModelWeight,
    PromptOrSessionState,
    RuntimeMemory,
    Log,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CacheUsageConfidence {
    Exact,
    Approximate,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CacheSizeSemantics {
    #[default]
    PhysicalReclaimable,
    ConservativeLowerBound,
    Informational,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
pub struct CacheMetadata {
    pub provider: String,
    pub management_mode: CacheManagementMode,
    pub artifact_kind: CacheArtifactKind,
    pub consequence: String,
    pub size_semantics: CacheSizeSemantics,
    #[serde(default)]
    pub last_used_confidence: CacheUsageConfidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
pub struct FileSize {
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub logical: u64,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
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
    #[serde(default)]
    pub cache_metadata: CacheMetadata,
    pub is_selected: bool,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub last_modified: Option<u64>,
    pub exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CategoryResult {
    pub category: Category,
    pub display_name: String,
    pub items: Vec<ScanItem>,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub total_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub safe_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub rebuild_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub manual_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ScanResult {
    pub scan_id: String,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub started_at: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub finished_at: u64,
    pub categories: Vec<CategoryResult>,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub total_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub safe_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub rebuild_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub manual_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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
        #[serde(with = "crate::ipc_numeric::u64")]
        #[specta(type = u64)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_metadata_and_large_numbers_survive_ipc_serialization() {
        const MAX_SAFE: u64 = 9_007_199_254_740_991;
        let item = ScanItem {
            id: "dev.uv.cache".into(),
            signature_id: "dev.uv.cache".into(),
            name: "uv cache".into(),
            category: Category::Developer,
            risk: RiskTier::Rebuild,
            path: "/Users/test/Library/Caches/uv".into(),
            size: FileSize::new(MAX_SAFE, Some(MAX_SAFE - 1)),
            file_count: 1,
            description: "owner managed".into(),
            cache_metadata: CacheMetadata {
                provider: "uv".into(),
                management_mode: CacheManagementMode::ToolManaged,
                artifact_kind: CacheArtifactKind::PackageStore,
                consequence: "re-download".into(),
                size_semantics: CacheSizeSemantics::ConservativeLowerBound,
                last_used_confidence: CacheUsageConfidence::Unknown,
            },
            is_selected: false,
            last_modified: Some(MAX_SAFE - 2),
            exists: true,
        };
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["size"]["logical"], MAX_SAFE);
        assert_eq!(json["size"]["allocated"], MAX_SAFE - 1);
        assert_eq!(json["last_modified"], MAX_SAFE - 2);
        assert_eq!(json["cache_metadata"]["management_mode"], "tool_managed");
        assert_eq!(json["cache_metadata"]["artifact_kind"], "package_store");
    }
}
