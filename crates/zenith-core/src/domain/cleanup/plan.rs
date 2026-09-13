use super::CleanStrategy;
use crate::domain::identity::CleanupIdentity;
use crate::domain::{RiskSummary, RiskTier};
use std::path::PathBuf;
use uuid::Uuid;

/// One authorized deletion target.
///
/// `identity` is `None` only for pseudo-path strategies (`DockerPrune`, which
/// runs through a provider CLI and never touches a host path) and for a target
/// that was already gone when the plan was built. Every filesystem strategy
/// that can be revalidated carries a captured [`CleanupIdentity`] so the
/// executor can fail closed when the path changed after planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteTarget {
    pub item_id: String,
    pub signature_id: String,
    pub name: String,
    pub path: PathBuf,
    pub strategy: CleanStrategy,
    pub expected_bytes: u64,
    pub risk: RiskTier,
    pub identity: Option<CleanupIdentity>,
    pub exclusions: Vec<String>,
    pub min_age_days: Option<u32>,
}

/// Backend-private authorization state for one cleanup run.
///
/// This type is never serialized. The frontend submits a scan ID, selected
/// item IDs, and an opaque one-shot plan ID; it never supplies a path, a
/// strategy, or an identity. Extending this type with `Serialize` or
/// `specta::Type` would turn mutation authority into a replayable payload, so
/// the projections in [`crate::application::dto::cleanup`] exist instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletePlan {
    pub id: Uuid,
    pub scan_id: String,
    pub targets: Vec<DeleteTarget>,
    pub expected_reclaim_bytes: u64,
    pub risk: RiskSummary,
    pub created_at: u64,
}
