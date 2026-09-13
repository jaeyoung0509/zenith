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

    pub fn observed_bytes(&self) -> u64 {
        self.allocated.unwrap_or(self.logical)
    }

    pub fn reclaimable(&self) -> u64 {
        self.observed_bytes()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CleanupEligibility {
    AutoCleanable,
    Reviewable,
    #[default]
    Blocked,
    Advisory,
}

impl CleanupEligibility {
    pub fn is_cleanable(&self) -> bool {
        matches!(self, Self::AutoCleanable | Self::Reviewable)
    }

    pub fn is_auto_cleanable(&self) -> bool {
        matches!(self, Self::AutoCleanable)
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Blocked)
    }

    pub fn is_advisory(&self) -> bool {
        matches!(self, Self::Advisory)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
pub struct CleanupDisposition {
    pub eligibility: CleanupEligibility,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default, with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub cleanable_bytes: Option<u64>,
}

impl CleanupDisposition {
    pub fn new(
        eligibility: CleanupEligibility,
        reason: Option<String>,
        cleanable_bytes: Option<u64>,
    ) -> Self {
        Self {
            eligibility,
            reason,
            cleanable_bytes,
        }
    }

    pub fn auto_cleanable(bytes: u64) -> Self {
        Self {
            eligibility: CleanupEligibility::AutoCleanable,
            reason: None,
            cleanable_bytes: Some(bytes),
        }
    }

    pub fn reviewable(bytes: u64, reason: Option<String>) -> Self {
        Self {
            eligibility: CleanupEligibility::Reviewable,
            reason,
            cleanable_bytes: Some(bytes),
        }
    }

    pub fn blocked(reason: impl Into<String>) -> Self {
        Self {
            eligibility: CleanupEligibility::Blocked,
            reason: Some(reason.into()),
            cleanable_bytes: None,
        }
    }

    pub fn advisory(reason: impl Into<String>) -> Self {
        Self {
            eligibility: CleanupEligibility::Advisory,
            reason: Some(reason.into()),
            cleanable_bytes: None,
        }
    }

    pub fn is_cleanable(&self) -> bool {
        self.eligibility.is_cleanable() && self.cleanable_bytes.unwrap_or(0) > 0
    }
}

pub fn is_safety_blocked_reason(reason: &str) -> bool {
    let lower = reason.to_lowercase();
    (lower.contains("protected") && (lower.contains("bundle") || lower.contains(".app")))
        || lower.contains("symlink")
        || lower.contains("blacklist")
}

pub fn derive_cleanup_disposition(
    risk: RiskTier,
    quality: ObservationQuality,
    cache_metadata: &CacheMetadata,
    size: &FileSize,
    incomplete_reason: Option<&str>,
) -> CleanupDisposition {
    // 1. Safety violations (protected .app bundle, symlink escape, blacklist) fail closed
    if incomplete_reason.is_some_and(is_safety_blocked_reason) {
        return CleanupDisposition::blocked(
            incomplete_reason.unwrap_or("Protected path encountered; cleanup blocked"),
        );
    }

    // 2. Unavailable observations are always blocked from cleanup
    if quality == ObservationQuality::Unavailable {
        return CleanupDisposition::blocked(
            incomplete_reason.unwrap_or("Inaccessible path; inspection failed"),
        );
    }

    // 3. Advisory caches cannot enter generic cleanup
    if cache_metadata.management_mode == CacheManagementMode::Advisory {
        return CleanupDisposition::advisory(
            incomplete_reason.unwrap_or("Advisory cache: managed manually or outside Zenith"),
        );
    }

    // 4. Manual risk tiers cannot be cleaned generically
    if risk == RiskTier::Manual {
        return CleanupDisposition::blocked(
            incomplete_reason.unwrap_or("Manual cleanup only; generic cleanup is unsupported"),
        );
    }

    let observed = size.observed_bytes();

    // 5. Tool-managed caches: provider policy decides, never AutoCleanable
    if cache_metadata.management_mode == CacheManagementMode::ToolManaged {
        if observed == 0 {
            return CleanupDisposition::blocked("No cleanable data found");
        }
        if quality == ObservationQuality::Partial {
            return CleanupDisposition::reviewable(
                observed,
                incomplete_reason.map(Into::into).or_else(|| {
                    Some("Incomplete scan; review before pruning with provider".into())
                }),
            );
        }
        return CleanupDisposition::reviewable(observed, None);
    }

    // 6. Partial observation quality: reviewable, never auto-selected or quick-cleanable
    if quality == ObservationQuality::Partial {
        if observed == 0 {
            return CleanupDisposition::blocked("No cleanable data found");
        }
        return CleanupDisposition::reviewable(
            observed,
            incomplete_reason
                .map(Into::into)
                .or_else(|| Some("Incomplete scan; review before cleaning".into())),
        );
    }

    // 7. Fresh observation with Zenith management
    if observed == 0 {
        return CleanupDisposition::blocked("No cleanable data found");
    }
    match risk {
        RiskTier::Safe => CleanupDisposition::auto_cleanable(observed),
        RiskTier::Rebuild => CleanupDisposition::reviewable(observed, None),
        RiskTier::Manual => CleanupDisposition::blocked("Manual cleanup only"),
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
    #[serde(default)]
    pub disposition: CleanupDisposition,
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
    pub fn observed_bytes(&self) -> u64 {
        self.size.observed_bytes()
    }

    pub fn derive_disposition(&self) -> CleanupDisposition {
        derive_cleanup_disposition(
            self.risk,
            self.quality,
            &self.cache_metadata,
            &self.size,
            self.incomplete_reason.as_deref(),
        )
    }

    pub fn with_derived_disposition(mut self) -> Self {
        self.disposition = self.derive_disposition();
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mock(
        id: impl Into<String>,
        signature_id: impl Into<String>,
        name: impl Into<String>,
        category: Category,
        risk: RiskTier,
        path: impl Into<String>,
        size: FileSize,
        file_count: usize,
    ) -> Self {
        let disposition = derive_cleanup_disposition(
            risk,
            ObservationQuality::Fresh,
            &Default::default(),
            &size,
            None,
        );
        let is_selected = disposition.eligibility == CleanupEligibility::AutoCleanable;
        Self {
            id: id.into(),
            signature_id: signature_id.into(),
            name: name.into(),
            category,
            risk,
            path: path.into(),
            size,
            file_count,
            description: String::new(),
            cache_metadata: Default::default(),
            is_selected,
            last_modified: None,
            exists: true,
            quality: ObservationQuality::Fresh,
            incomplete_reason: None,
            skipped_entry_count: 0,
            disposition,
        }
    }

    pub fn allows_cleanup(&self) -> bool {
        self.disposition.is_cleanable()
    }

    /// Bytes this item would reclaim when cleaned, and zero when it cannot be
    /// cleaned at all.
    ///
    /// Every total — the risk buckets, the category and scan sums, and the
    /// frontend's selection summary — is derived from this, so an item whose
    /// observation cannot support a cleanup never contributes a byte that only
    /// looks reclaimable.
    pub fn cleanable_bytes(&self) -> u64 {
        if !self.disposition.is_cleanable() {
            return 0;
        }
        self.disposition
            .cleanable_bytes
            .unwrap_or(0)
            .min(self.observed_bytes())
    }

    /// Whether the serialized disposition still matches the facts captured by
    /// this scan item. Mutation paths use this to reject stale or internally
    /// inconsistent scan data instead of trusting a detached permission flag.
    pub fn has_current_disposition(&self) -> bool {
        self.disposition == self.derive_disposition()
    }

    /// Whether this item is a cleanup candidate a user could select.
    pub fn is_cleanable_candidate(&self) -> bool {
        self.allows_cleanup()
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
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub cleanable_bytes: u64,
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
    #[serde(default, with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub cleanable_bytes: u64,
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
            cleanable_bytes: 0,
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
            cleanable_bytes: 100,
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
        let mut item = ScanItem {
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
            disposition: CleanupDisposition::reviewable(MAX_SAFE - 1, None),
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
        assert_eq!(json["disposition"]["eligibility"], "reviewable");
        assert_eq!(json["disposition"]["cleanable_bytes"], MAX_SAFE - 1);

        json.as_object_mut().unwrap().remove("quality");
        json.as_object_mut().unwrap().remove("skipped_entry_count");
        json.as_object_mut().unwrap().remove("disposition");
        let legacy_item: ScanItem = serde_json::from_value(json).unwrap();
        assert_eq!(legacy_item.quality, ObservationQuality::Unavailable);
        assert_eq!(legacy_item.skipped_entry_count, 0);
        assert_eq!(
            legacy_item.disposition.eligibility,
            CleanupEligibility::Blocked
        );
        assert!(!legacy_item.allows_cleanup());

        // Test unsafe number in cleanable_bytes fails closed
        item.disposition.cleanable_bytes = Some(crate::ipc_numeric::MAX_SAFE_INTEGER + 1);
        let error =
            serde_json::to_value(&item).expect_err("unsafe cleanable_bytes must fail closed");
        assert!(error.to_string().contains("MAX_SAFE_INTEGER"));
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
            disposition: CleanupDisposition::default(),
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

    #[test]
    fn test_derive_cleanup_disposition_matrix() {
        let size = FileSize::new(1000, Some(1000));
        let zenith_meta = CacheMetadata {
            management_mode: CacheManagementMode::Zenith,
            ..Default::default()
        };
        let tool_meta = CacheMetadata {
            management_mode: CacheManagementMode::ToolManaged,
            ..Default::default()
        };
        let advisory_meta = CacheMetadata {
            management_mode: CacheManagementMode::Advisory,
            ..Default::default()
        };

        // 1. Safe + Fresh + Zenith => AutoCleanable
        let d1 = derive_cleanup_disposition(
            RiskTier::Safe,
            ObservationQuality::Fresh,
            &zenith_meta,
            &size,
            None,
        );
        assert_eq!(d1.eligibility, CleanupEligibility::AutoCleanable);
        assert_eq!(d1.cleanable_bytes, Some(1000));
        assert!(d1.is_cleanable());

        // 2. Safe + Partial + Zenith => Reviewable (never auto-cleanable)
        let d2 = derive_cleanup_disposition(
            RiskTier::Safe,
            ObservationQuality::Partial,
            &zenith_meta,
            &size,
            None,
        );
        assert_eq!(d2.eligibility, CleanupEligibility::Reviewable);
        assert_eq!(d2.cleanable_bytes, Some(1000));
        assert!(d2.is_cleanable());

        // 3. Safe + Unavailable + Zenith => Blocked
        let d3 = derive_cleanup_disposition(
            RiskTier::Safe,
            ObservationQuality::Unavailable,
            &zenith_meta,
            &size,
            None,
        );
        assert_eq!(d3.eligibility, CleanupEligibility::Blocked);
        assert_eq!(d3.cleanable_bytes, None);
        assert!(!d3.is_cleanable());

        // 4. Rebuild + Fresh + Zenith => Reviewable
        let d4 = derive_cleanup_disposition(
            RiskTier::Rebuild,
            ObservationQuality::Fresh,
            &zenith_meta,
            &size,
            None,
        );
        assert_eq!(d4.eligibility, CleanupEligibility::Reviewable);
        assert_eq!(d4.cleanable_bytes, Some(1000));
        assert!(d4.is_cleanable());

        // 5. Rebuild + Partial + Zenith => Reviewable (never quick clean)
        let d5 = derive_cleanup_disposition(
            RiskTier::Rebuild,
            ObservationQuality::Partial,
            &zenith_meta,
            &size,
            None,
        );
        assert_eq!(d5.eligibility, CleanupEligibility::Reviewable);
        assert_eq!(d5.cleanable_bytes, Some(1000));
        assert!(d5.is_cleanable());

        // 6. Manual + Fresh + Zenith => Blocked from generic cleanup
        let d6 = derive_cleanup_disposition(
            RiskTier::Manual,
            ObservationQuality::Fresh,
            &zenith_meta,
            &size,
            None,
        );
        assert_eq!(d6.eligibility, CleanupEligibility::Blocked);
        assert_eq!(d6.cleanable_bytes, None);
        assert!(!d6.is_cleanable());

        // 7. Safe + Fresh + ToolManaged => Reviewable (provider decides, never AutoCleanable)
        let d7 = derive_cleanup_disposition(
            RiskTier::Safe,
            ObservationQuality::Fresh,
            &tool_meta,
            &size,
            None,
        );
        assert_eq!(d7.eligibility, CleanupEligibility::Reviewable);
        assert_eq!(d7.cleanable_bytes, Some(1000));
        assert!(d7.is_cleanable());

        // 8. Safe + Fresh + Advisory => Advisory
        let d8 = derive_cleanup_disposition(
            RiskTier::Safe,
            ObservationQuality::Fresh,
            &advisory_meta,
            &size,
            None,
        );
        assert_eq!(d8.eligibility, CleanupEligibility::Advisory);
        assert_eq!(d8.cleanable_bytes, None);
        assert!(!d8.is_cleanable());

        // 9. Nested protected .app => Blocked (even if Safe + Fresh/Partial)
        let d9 = derive_cleanup_disposition(
            RiskTier::Safe,
            ObservationQuality::Partial,
            &zenith_meta,
            &size,
            Some("Protected application bundle encountered in /Library/Caches/something.app"),
        );
        assert_eq!(d9.eligibility, CleanupEligibility::Blocked);
        assert_eq!(d9.cleanable_bytes, None);
        assert!(!d9.is_cleanable());
    }

    #[test]
    fn cleanable_bytes_fail_closed_for_inconsistent_dispositions() {
        let mut item = ScanItem::mock(
            "test.item",
            "test.signature",
            "Test item",
            Category::Developer,
            RiskTier::Safe,
            "/tmp/test-item",
            FileSize::new(100, Some(100)),
            1,
        );

        item.disposition = CleanupDisposition::new(
            CleanupEligibility::Blocked,
            Some("blocked".into()),
            Some(100),
        );
        assert_eq!(item.cleanable_bytes(), 0);

        item.disposition = CleanupDisposition::reviewable(500, None);
        assert_eq!(item.cleanable_bytes(), 100);
    }
}
