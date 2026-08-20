use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, specta::Type,
)]
#[serde(rename_all = "lowercase")]
pub enum RiskTier {
    Safe,
    Rebuild,
    Manual,
}

impl RiskTier {
    pub fn display_name(&self) -> &'static str {
        match self {
            RiskTier::Safe => "Safe",
            RiskTier::Rebuild => "Rebuild Required",
            RiskTier::Manual => "Manual Review",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            RiskTier::Safe => "Temporary caches and diagnostic logs that can be safely removed without affecting workflows.",
            RiskTier::Rebuild => "Build artifacts or dependency caches that will be automatically re-downloaded or recompiled on next use.",
            RiskTier::Manual => "Stateful assets such as model weights or persistent volumes requiring explicit user review.",
        }
    }

    pub fn is_auto_selectable(&self) -> bool {
        matches!(self, RiskTier::Safe)
    }
}

impl fmt::Display for RiskTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RiskSummary {
    pub safe_count: usize,
    pub rebuild_count: usize,
    pub manual_count: usize,
    pub safe_bytes: u64,
    pub rebuild_bytes: u64,
    pub manual_bytes: u64,
}

impl RiskSummary {
    pub fn add(&mut self, tier: RiskTier, bytes: u64) {
        match tier {
            RiskTier::Safe => {
                self.safe_count += 1;
                self.safe_bytes += bytes;
            }
            RiskTier::Rebuild => {
                self.rebuild_count += 1;
                self.rebuild_bytes += bytes;
            }
            RiskTier::Manual => {
                self.manual_count += 1;
                self.manual_bytes += bytes;
            }
        }
    }
}
