use crate::models::{Category, ObservationQuality, RiskTier};
use serde::{Deserialize, Serialize};

fn unavailable_observation_quality() -> ObservationQuality {
    ObservationQuality::Unavailable
}

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
    #[serde(default = "unavailable_observation_quality")]
    pub quality: ObservationQuality,
    #[serde(default)]
    pub incomplete_reason: Option<String>,
    /// Entries the measurement did not account for (excluded, blacklisted,
    /// protected, unreadable, or beyond the depth limit). Reported so a
    /// partial total is never presented as a complete one.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub skipped_entry_count: u64,
}

impl ScanItem {
    pub fn allows_cleanup(&self) -> bool {
        matches!(
            self.quality,
            ObservationQuality::Fresh | ObservationQuality::Partial
        )
    }

    /// Bytes this item would reclaim when cleaned, and zero when it cannot be
    /// cleaned at all.
    ///
    /// Every total — the risk buckets, the category and scan sums, and the
    /// frontend's selection summary — is derived from this, so an item whose
    /// observation cannot support a cleanup never contributes a byte that only
    /// looks reclaimable.
    pub fn cleanable_bytes(&self) -> u64 {
        if self.allows_cleanup() {
            self.size.reclaimable()
        } else {
            0
        }
    }

    /// Whether this item is a cleanup candidate a user could select.
    pub fn is_cleanable_candidate(&self) -> bool {
        self.allows_cleanup() && self.risk != RiskTier::Manual && self.cleanable_bytes() > 0
    }
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
    #[serde(default = "unavailable_observation_quality")]
    pub quality: ObservationQuality,
    /// Sum of the retained items' skipped-entry counts.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub skipped_entry_count: u64,
    /// Retained items whose observation is not `Fresh`.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub incomplete_item_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ScanResult {
    pub scan_id: String,
    /// Backend-owned lifetime of a cleanup observation, not a deletion lease.
    pub valid_for_seconds: u32,
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
    #[serde(default = "unavailable_observation_quality")]
    pub quality: ObservationQuality,
    #[serde(default)]
    pub incomplete_reasons: Vec<String>,
    /// Sum of the categories' skipped-entry counts.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub skipped_entry_count: u64,
    /// Retained items whose observation is not `Fresh`.
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub incomplete_item_count: u64,
}

impl ScanResult {
    pub const VALID_FOR_SECONDS: u32 = 300;

    pub fn is_fresh_at(&self, now: u64) -> bool {
        self.quality == ObservationQuality::Fresh
            && now
                .checked_sub(self.finished_at)
                .is_some_and(|age| age < u64::from(Self::VALID_FOR_SECONDS))
    }

    pub fn validate_for_cleanup(
        &self,
        scan_id: &str,
        now: u64,
    ) -> Result<(), crate::models::ZenithError> {
        use crate::models::ZenithError;
        if self.scan_id != scan_id {
            return Err(ZenithError::InvalidPlan(
                "The scan is no longer current. Scan again before cleaning.".into(),
            ));
        }
        let is_current = now
            .checked_sub(self.finished_at)
            .is_some_and(|age| age < u64::from(Self::VALID_FOR_SECONDS));
        if !is_current {
            return Err(ZenithError::InvalidPlan(
                "Scan expired. Scan again and review the new results before cleaning.".into(),
            ));
        }
        if !matches!(
            self.quality,
            ObservationQuality::Fresh | ObservationQuality::Partial
        ) {
            return Err(ZenithError::InvalidPlan(
                "The scan failed or is unavailable. Scan again before cleaning.".into(),
            ));
        }
        Ok(())
    }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_observation_expiry_and_clock_rollback_fail_closed() {
        let scan = ScanResult {
            scan_id: "fixture".into(),
            valid_for_seconds: ScanResult::VALID_FOR_SECONDS,
            started_at: 999,
            finished_at: 1000,
            categories: vec![],
            total_bytes: 0,
            safe_bytes: 0,
            rebuild_bytes: 0,
            manual_bytes: 0,
            quality: ObservationQuality::Fresh,
            incomplete_reasons: vec![],
            skipped_entry_count: 0,
            incomplete_item_count: 0,
        };
        assert!(scan.is_fresh_at(1000));
        assert!(scan.is_fresh_at(1299));
        assert!(!scan.is_fresh_at(1300));
        assert!(!scan.is_fresh_at(999));
        assert!(scan.validate_for_cleanup("fixture", 1299).is_ok());
        assert!(scan.validate_for_cleanup("unknown", 1000).is_err());
        assert!(scan.validate_for_cleanup("fixture", 1300).is_err());
        assert!(scan.validate_for_cleanup("fixture", 999).is_err());
        let serialized = serde_json::to_value(&scan).unwrap();
        assert_eq!(serialized["valid_for_seconds"], 300);
        assert_eq!(serialized["quality"], "fresh");
    }

    #[test]
    fn partial_and_unavailable_scans_never_report_fresh() {
        let mut scan = ScanResult {
            scan_id: "fixture".into(),
            valid_for_seconds: ScanResult::VALID_FOR_SECONDS,
            started_at: 999,
            finished_at: 1000,
            categories: vec![],
            total_bytes: 100,
            safe_bytes: 100,
            rebuild_bytes: 0,
            manual_bytes: 0,
            quality: ObservationQuality::Partial,
            incomplete_reasons: vec!["Some directories were unreadable".into()],
            skipped_entry_count: 4,
            incomplete_item_count: 1,
        };
        // A partial scan must NEVER report Fresh, even within the TTL window
        assert!(!scan.is_fresh_at(1000));
        assert!(!scan.is_fresh_at(1200));
        // But validate_for_cleanup allows cleaning inspected items if not expired
        assert!(scan.validate_for_cleanup("fixture", 1200).is_ok());

        // Unavailable scan cannot be cleaned
        scan.quality = ObservationQuality::Unavailable;
        assert!(!scan.is_fresh_at(1000));
        assert!(scan.validate_for_cleanup("fixture", 1200).is_err());
    }

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
            quality: ObservationQuality::Fresh,
            incomplete_reason: None,
            skipped_entry_count: MAX_SAFE - 3,
        };
        let mut json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["size"]["logical"], MAX_SAFE);
        assert_eq!(json["size"]["allocated"], MAX_SAFE - 1);
        assert_eq!(json["last_modified"], MAX_SAFE - 2);
        assert_eq!(json["skipped_entry_count"], MAX_SAFE - 3);
        assert_eq!(json["quality"], "fresh");
        assert_eq!(json["cache_metadata"]["management_mode"], "tool_managed");
        assert_eq!(json["cache_metadata"]["artifact_kind"], "package_store");

        json.as_object_mut().unwrap().remove("quality");
        json.as_object_mut().unwrap().remove("skipped_entry_count");
        let legacy_item: ScanItem = serde_json::from_value(json).unwrap();
        assert_eq!(legacy_item.quality, ObservationQuality::Unavailable);
        assert_eq!(legacy_item.skipped_entry_count, 0);
        assert!(!legacy_item.allows_cleanup());
    }

    /// A count above `Number.MAX_SAFE_INTEGER` must be refused at the IPC
    /// boundary rather than silently rounded in the browser.
    #[test]
    fn an_unsafe_skipped_entry_count_is_refused_by_the_ipc_adapter() {
        let mut item = ScanItem {
            id: "dev.uv.cache".into(),
            signature_id: "dev.uv.cache".into(),
            name: "uv cache".into(),
            category: Category::Developer,
            risk: RiskTier::Rebuild,
            path: "/Users/test/Library/Caches/uv".into(),
            size: FileSize::new(1, Some(1)),
            file_count: 1,
            description: "owner managed".into(),
            cache_metadata: CacheMetadata::default(),
            is_selected: false,
            last_modified: None,
            exists: true,
            quality: ObservationQuality::Fresh,
            incomplete_reason: None,
            skipped_entry_count: crate::ipc_numeric::MAX_SAFE_INTEGER + 1,
        };

        let error = serde_json::to_value(&item).expect_err("unsafe counter must fail closed");
        assert!(error.to_string().contains("MAX_SAFE_INTEGER"));
        item.skipped_entry_count = crate::ipc_numeric::MAX_SAFE_INTEGER;
        assert!(serde_json::to_value(&item).is_ok());
    }
}
