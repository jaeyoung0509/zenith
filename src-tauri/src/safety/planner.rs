use crate::models::{
    CleanStrategy, DeletePlan, DeleteTarget, RiskSummary, RiskTier, ScanItem, ScanResult,
    ZenithError,
};
use crate::safety::{Blacklist, SymlinkGuard, ToctouGuard};
use crate::signatures::SignatureRegistry;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::SystemTime;
use uuid::Uuid;

pub struct SafetyPlanner;

impl SafetyPlanner {
    pub fn create_plan_from_scan(
        scan: &ScanResult,
        scan_id: &str,
        selected_item_ids: &[String],
        registry: &SignatureRegistry,
    ) -> Result<DeletePlan, ZenithError> {
        if scan.scan_id != scan_id {
            return Err(ZenithError::InvalidPlan(
                "The scan is no longer current".into(),
            ));
        }
        let requested: HashSet<&str> = selected_item_ids.iter().map(String::as_str).collect();
        if requested.is_empty() || requested.len() != selected_item_ids.len() {
            return Err(ZenithError::InvalidPlan(
                "Selection is empty or contains duplicate item IDs".into(),
            ));
        }

        let mut trusted_items = scan
            .categories
            .iter()
            .flat_map(|category| category.items.iter())
            .filter(|item| requested.contains(item.id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if trusted_items.len() != requested.len() {
            return Err(ZenithError::InvalidPlan(
                "Selected item was not present in the trusted scan".into(),
            ));
        }
        for item in &mut trusted_items {
            item.is_selected = true;
        }
        let mut plan = Self::create_plan(&trusted_items, registry)?;
        plan.scan_id = scan_id.to_string();
        Ok(plan)
    }

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

            if item.risk == RiskTier::Manual || signature.strategy == CleanStrategy::Manual {
                return Err(ZenithError::UnsupportedManualOperation(item.name.clone()));
            }

            // 2. Resolve target path
            let path = PathBuf::from(&item.path);

            // A scan item must remain inside the path scope declared by its signature.
            // Age-filtered temp signatures emit direct children; normal signatures
            // may only target the declared root itself.
            if !signature.paths.is_empty() {
                let resolved_roots = registry.resolve_paths(signature);
                let allowed = resolved_roots.iter().any(|root| {
                    path == *root
                        || (signature.min_age_days.is_some()
                            && path.parent() == Some(root.as_path()))
                });
                if !allowed {
                    return Err(ZenithError::SignatureMismatch(item.signature_id.clone()));
                }

                // 2b. Ancestor symlink escape protection: ensure no directory between anchor/root and path is a symlink
                for root in &resolved_roots {
                    if path.starts_with(root) {
                        SymlinkGuard::validate_no_symlink_ancestors(&path, root)?;
                    }
                }
            } else {
                SymlinkGuard::validate_anchored_path(&path)?;
            }

            // 3. Hard Blacklist check (lexical & canonical)
            Blacklist::validate(&path)?;
            SymlinkGuard::validate_canonical_blacklist(&path)?;

            // 4. Symlink Target check
            SymlinkGuard::validate_symlink_target(&path)?;

            // 5. Strategy resolution
            let strategy = signature.strategy;

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
                exclusions: signature.exclusions.clone(),
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
            scan_id: String::new(),
            targets,
            expected_reclaim_bytes,
            risk: risk_summary,
            created_at: now,
        })
    }
}
