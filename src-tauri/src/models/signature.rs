use crate::models::{
    CacheArtifactKind, CacheManagementMode, CacheMetadata, CacheSizeSemantics,
    CacheUsageConfidence, Category, CleanStrategy, PlatformKind, RiskTier,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Signature {
    pub id: String,
    pub name: String,
    pub category: Category,
    pub risk: RiskTier,
    #[serde(default = "default_strategy")]
    pub strategy: CleanStrategy,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub exclusions: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub min_age_days: Option<u32>,
    #[serde(default)]
    pub include_prefixes: Vec<String>,
    #[serde(default)]
    pub exclude_prefixes: Vec<String>,
    #[serde(default)]
    pub intensive_only: bool,
    #[serde(default)]
    pub platforms: Vec<PlatformKind>,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub management_mode: CacheManagementMode,
    #[serde(default)]
    pub artifact_kind: CacheArtifactKind,
    #[serde(default)]
    pub consequence: String,
    #[serde(default)]
    pub reclaimable_is_lower_bound: bool,
}

impl Signature {
    pub fn cache_metadata(&self) -> CacheMetadata {
        CacheMetadata {
            provider: if self.provider.is_empty() {
                "Zenith".to_string()
            } else {
                self.provider.clone()
            },
            management_mode: self.management_mode,
            artifact_kind: self.artifact_kind,
            consequence: self.consequence.clone(),
            size_semantics: if self.reclaimable_is_lower_bound {
                CacheSizeSemantics::ConservativeLowerBound
            } else {
                CacheSizeSemantics::PhysicalReclaimable
            },
            last_used_confidence: if self.min_age_days.is_some() {
                CacheUsageConfidence::Approximate
            } else {
                CacheUsageConfidence::Unknown
            },
        }
    }

    pub fn supports_current_platform(&self) -> bool {
        self.platforms.is_empty() || self.platforms.contains(&PlatformKind::current())
    }
}

fn default_strategy() -> CleanStrategy {
    CleanStrategy::DeleteContents
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SignatureManifest {
    #[serde(default)]
    pub signatures: Vec<Signature>,
}
