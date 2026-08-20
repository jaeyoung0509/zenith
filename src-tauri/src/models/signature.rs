use crate::models::{Category, CleanStrategy, RiskTier};
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
}

fn default_strategy() -> CleanStrategy {
    CleanStrategy::DeleteContents
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SignatureManifest {
    #[serde(default)]
    pub signatures: Vec<Signature>,
}
