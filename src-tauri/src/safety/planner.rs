use crate::models::{
    CleanStrategy, DeletePlan, DeleteTarget, RiskSummary, RiskTier, ScanItem, ZenithError,
};
use crate::safety::{Blacklist, SymlinkGuard, ToctouGuard};
use crate::signatures::SignatureRegistry;
use std::path::PathBuf;
use std::time::SystemTime;
use uuid::Uuid;

pub struct SafetyPlanner;

impl SafetyPlanner {
    /// Creates a verified and locked DeletePlan from a list of candidate ScanItems.
    pub fn create_plan(
        items: &[ScanItem],
        registry: &SignatureRegistry,
    ) -> Result<DeletePlan, ZenithError> {
        let mut targets = Vec::new();
        let mut expected_reclaim_bytes = 0u64;
        let mut risk_summary = RiskSummary::default();

        for item in items {
            // Only consider selected items
            if !item.is_selected {
                continue;
            }

            // 1. Verify signature exists in registry
            let signature = registry
                .get(&item.signature_id)
                .ok_or_else(|| ZenithError::SignatureMismatch(item.signature_id.clone()))?;

            // 2. Resolve target path
            let path = PathBuf::from(&item.path);

            // A scan item must remain inside the path scope declared by its signature.
            // Age-filtered temp signatures emit direct children; normal signatures
            // may only target the declared root itself.
            if !signature.paths.is_empty() {
                let allowed = registry.resolve_paths(signature).iter().any(|root| {
                    path == *root
                        || (signature.min_age_days.is_some()
                            && path.parent() == Some(root.as_path()))
                });
                if !allowed {
                    return Err(ZenithError::SignatureMismatch(item.signature_id.clone()));
                }
            }

            // 3. Hard Blacklist check
            Blacklist::validate(&path)?;

            // 4. Symlink Target check
            SymlinkGuard::validate_symlink_target(&path)?;

            // 5. Strategy resolution
            let strategy = match signature.strategy {
                CleanStrategy::Manual if item.risk == RiskTier::Manual => {
                    // Manual items can be planned if explicitly requested
                    CleanStrategy::Manual
                }
                other => other,
            };

            // 6. Capture current file identity for TOCTOU protection
            let identity = if path.exists() || SymlinkGuard::is_symlink(&path) {
                ToctouGuard::capture(&path)
            } else {
                None
            };

            let bytes = item.size.reclaimable();
            expected_reclaim_bytes += bytes;
            risk_summary.add(item.risk, bytes);

            targets.push(DeleteTarget {
                item_id: item.id.clone(),
                signature_id: item.signature_id.clone(),
                name: item.name.clone(),
                path,
                strategy,
                expected_bytes: bytes,
                risk: item.risk,
                identity,
            });
        }

        if targets.is_empty() {
            return Err(ZenithError::InvalidPlan(
                "No valid cleanable targets were selected".to_string(),
            ));
        }

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Ok(DeletePlan {
            id: Uuid::new_v4(),
            targets,
            expected_reclaim_bytes,
            risk: risk_summary,
            created_at: now,
        })
    }
}
