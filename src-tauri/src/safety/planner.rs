use crate::models::{
    CleanStrategy, CleanupMode, DeletePlan, DeleteTarget, RiskSummary, RiskTier, ScanItem,
    ScanResult, Signature, ZenithError,
};
use crate::safety::{entry_kind_at, structured_state_at, Blacklist, SymlinkGuard, ToctouGuard};
use crate::signatures::SignatureRegistry;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::SystemTime;
use uuid::Uuid;
use zenith_platform::PlatformEnvironment;

/// Whether an item's ownership follows from the signature that discovered it.
///
/// A catalog entry that names an owner states it, and the item must agree. An
/// entry that names nobody leaves the child to speak for itself: a namespace
/// enumerated under a broad root reports the inference from its own name, which
/// is the only statement available and is labelled as an inference.
fn ownership_is_derivable(item: &ScanItem, signature: &Signature) -> bool {
    let catalog = signature.ownership();
    if item.ownership == catalog {
        return true;
    }
    if catalog.is_known()
        || item.ownership.confidence != crate::models::OwnershipConfidence::Inferred
    {
        return false;
    }
    item.unit
        .path
        .rsplit(['/', '\\'])
        .next()
        .is_some_and(|name| name == item.ownership.owner)
}

pub struct SafetyPlanner;

impl SafetyPlanner {
    pub fn create_plan_from_scan(
        scan: &ScanResult,
        scan_id: &str,
        selected_item_ids: &[String],
        registry: &SignatureRegistry,
        environment: &PlatformEnvironment,
    ) -> Result<DeletePlan, ZenithError> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        scan.validate_for_cleanup(scan_id, now)?;
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
        let mut plan = Self::create_plan_for(&trusted_items, registry, environment)?;
        plan.scan_id = scan_id.to_string();
        Ok(plan)
    }

    /// Creates a verified and locked DeletePlan from a list of candidate ScanItems.
    ///
    /// Native convenience for one-line callers and tests; production paths go
    /// through [`Self::create_plan_from_scan`], which takes the environment the
    /// scan was produced with.
    pub fn create_plan(
        items: &[ScanItem],
        registry: &SignatureRegistry,
    ) -> Result<DeletePlan, ZenithError> {
        Self::create_plan_for(items, registry, &PlatformEnvironment::native())
    }

    fn create_plan_for(
        items: &[ScanItem],
        registry: &SignatureRegistry,
        environment: &PlatformEnvironment,
    ) -> Result<DeletePlan, ZenithError> {
        let mut targets = Vec::new();
        let mut expected_reclaim_bytes = 0u64;
        let mut risk_summary = RiskSummary::default();

        for item in items {
            // Only consider selected items
            if !item.is_selected {
                continue;
            }

            // Read-only adapter observations do not need a registry signature, but they must
            // always fail before any generic filesystem planning is attempted.
            if item.risk == RiskTier::Manual {
                return Err(ZenithError::UnsupportedManualOperation(item.name.clone()));
            }

            if !item.has_current_disposition() {
                return Err(ZenithError::InvalidPlan(format!(
                    "Item '{}' cleanup eligibility changed since the scan; scan again",
                    item.name
                )));
            }

            if !item.allows_cleanup() {
                return Err(ZenithError::InvalidPlan(format!(
                    "Item '{}' was not completely inspected or is inaccessible and cannot be cleaned",
                    item.name
                )));
            }

            // A plan authorizes a unit, not a path string: an item that cannot
            // name the unit that produced it, or names a different path than
            // the one it deletes, is not plannable.
            if !item.unit.is_declared() {
                return Err(ZenithError::InvalidPlan(format!(
                    "Item '{}' does not name the cleanup unit that authorized it; scan again",
                    item.name
                )));
            }
            if item.unit.path != item.path {
                return Err(ZenithError::InvalidPlan(format!(
                    "Item '{}' names a cleanup unit that does not match its path; scan again",
                    item.name
                )));
            }

            // 1. Verify signature exists in registry
            let signature = registry
                .get(&item.signature_id)
                .ok_or_else(|| ZenithError::SignatureMismatch(item.signature_id.clone()))?;

            if signature.strategy == CleanStrategy::Manual {
                return Err(ZenithError::UnsupportedManualOperation(item.name.clone()));
            }

            // The unit granularity the item claims must be the granularity the
            // signature declares, so a discovery rule cannot widen what a
            // signature authorizes.
            if item.unit.kind != signature.unit_kind() {
                return Err(ZenithError::InvalidPlan(format!(
                    "Item '{}' claims a cleanup unit the signature does not declare; scan again",
                    item.name
                )));
            }

            // The ownership the item reports must be derivable from the catalog
            // entry: either exactly what the entry states, or — when the entry
            // states nothing — the inference an enumerated child carries from its
            // own name. A message built from the plan then never describes a
            // location by a claim the catalog cannot support.
            if !ownership_is_derivable(item, signature) {
                return Err(ZenithError::InvalidPlan(format!(
                    "Item '{}' reports ownership the catalog does not state; scan again",
                    item.name
                )));
            }

            // 2. Resolve target path and strategy
            let path = PathBuf::from(&item.path);
            let strategy = signature.strategy;
            let mut identity = None;

            if strategy == CleanStrategy::DockerPrune {
                // DockerPrune uses pseudo paths (e.g. docker://images/dangling) and dedicated Docker CLI adapters.
                // It does not operate on arbitrary host filesystem paths.
            } else {
                // Filesystem strategies: DeleteContents, DeleteDirectory, ExternalCommand
                if !signature.paths.is_empty() {
                    // The roots that authorize this path, re-derived from the
                    // signature: a literal root, a selected root, or a parent
                    // of one for a signature that enumerates children.
                    let resolved_roots = registry.authorizing_roots(signature, &path, environment);
                    if resolved_roots.is_empty() {
                        return Err(ZenithError::SignatureMismatch(item.signature_id.clone()));
                    }

                    // 2b. Ancestor symlink escape protection: ensure no directory between anchor/root and path is a symlink
                    for root in &resolved_roots {
                        if path.starts_with(root) {
                            SymlinkGuard::validate_no_symlink_ancestors(&path, root, environment)?;
                        }
                    }
                } else {
                    SymlinkGuard::validate_anchored_path(&path, environment)?;
                }

                // 3. Hard Blacklist check (lexical & canonical)
                Blacklist::validate_with(&path, environment)?;
                SymlinkGuard::validate_canonical_blacklist(&path, environment)?;

                // 4. Symlink Target check
                SymlinkGuard::validate_symlink_target(&path, environment)?;

                // 5. Capture current file identity for TOCTOU protection
                if path.exists() || SymlinkGuard::is_symlink(&path) {
                    identity = ToctouGuard::capture(&path);
                }

                // 6. Structured state is not generic cleanup's to remove. The
                //    execution guard refuses it too; refusing here keeps a plan
                //    from offering a target that could never be cleaned.
                if let Some((kind, _)) = structured_state_at(&path) {
                    return Err(ZenithError::InvalidPlan(format!(
                        "`{}` is {} and can only be handled by a dedicated provider, not by generic cleanup",
                        item.name,
                        kind.display_name()
                    )));
                }
                if matches!(
                    strategy,
                    CleanStrategy::DeleteContents | CleanStrategy::DeleteDirectory
                ) && path.is_dir()
                {
                    match crate::safety::validator::structured_descendant(&path) {
                        Ok(Some((nested, kind))) => {
                            return Err(ZenithError::InvalidPlan(format!(
                                "`{}` contains {} ({}); generic cleanup cannot remove this unit",
                                item.name,
                                nested.display(),
                                kind.display_name()
                            )));
                        }
                        Err(error) => {
                            return Err(ZenithError::InvalidPlan(format!(
                                "Could not inspect all of `{}` before planning cleanup: {error}",
                                item.name
                            )));
                        }
                        Ok(None) => {}
                    }
                }

                // 7. The entry kind the scan observed must still hold, so a
                //    plan states the kind of object it intends to delete.
                if let Some(current_kind) = entry_kind_at(&path) {
                    if current_kind != item.entry_kind {
                        return Err(ZenithError::InvalidPlan(format!(
                            "`{}` changed kind since the scan; scan again before cleaning",
                            item.name
                        )));
                    }
                }
            }

            let bytes = item.cleanable_bytes();
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
                min_age_days: signature.min_age_days,
                unit: item.unit.clone(),
                target_kind: item.entry_kind,
                owner: item.ownership.clone(),
                process_guard: signature.process_guard(),
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
            mode: CleanupMode::PermanentDelete,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::SafetyPlanner;
    use crate::models::{Category, FileSize, ObservationQuality, RiskTier, ScanItem, ZenithError};
    use crate::signatures::SignatureRegistry;

    #[test]
    fn rejects_manual_adapter_observations_before_signature_resolution() {
        let size = FileSize::new(1024, Some(512));
        let item = ScanItem {
            id: "container.orbstack.storage".to_string(),
            signature_id: "adapter.orbstack.storage".to_string(),
            name: "OrbStack VM Storage".to_string(),
            category: Category::Container,
            risk: RiskTier::Manual,
            path: "/untrusted/data.img.raw".to_string(),
            size,
            file_count: 1,
            description: String::new(),
            cache_metadata: Default::default(),
            disposition: crate::models::derive_cleanup_disposition(
                crate::models::DispositionFacts::new(
                    RiskTier::Manual,
                    ObservationQuality::Fresh,
                    &Default::default(),
                    &size,
                    None,
                ),
            ),
            unit: crate::models::CleanupUnit::fixed_path("/untrusted/data.img.raw"),
            ownership: Default::default(),
            age: None,
            stale: None,
            structured_state: None,
            entry_kind: crate::models::EntryKind::File,
            gate: Default::default(),
            owner_running: false,
            is_selected: true,
            last_modified: None,
            exists: true,
            quality: ObservationQuality::Fresh,
            incomplete_reason: None,
            skipped_entry_count: 0,
        };

        let result = SafetyPlanner::create_plan(&[item], &SignatureRegistry::new());
        assert!(matches!(
            result,
            Err(ZenithError::UnsupportedManualOperation(name))
                if name == "OrbStack VM Storage"
        ));
    }

    /// A namespace enumerated under a broad root names nobody in the catalog, so
    /// its item reports the inference from its own name. That inference is what
    /// the catalog can support, and a plan may be built from it.
    #[test]
    fn an_inferred_owner_from_an_enumerated_child_is_plannable() {
        use crate::models::{CleanupOwnership, CleanupUnit, EntryKind, Signature};

        let mut registry = SignatureRegistry::new();
        registry.register(Signature {
            id: "test.broad-root".into(),
            name: "Broad root".into(),
            category: Category::System,
            risk: RiskTier::Safe,
            strategy: crate::models::CleanStrategy::DeleteDirectory,
            paths: vec!["/tmp/broad-root".into()],
            exclusions: vec![],
            description: String::new(),
            min_age_days: Some(7),
            include_prefixes: vec![],
            exclude_prefixes: vec![],
            intensive_only: false,
            platforms: vec![],
            discovery: Default::default(),
            unit: None,
            owner: String::new(),
            priority: 0,
            fail_if_running: Vec::new(),
            provider: String::new(),
            management_mode: Default::default(),
            artifact_kind: Default::default(),
            consequence: String::new(),
            reclaimable_is_lower_bound: false,
        });

        let mut item = ScanItem::mock(
            "test.broad-root.0.com.example.app",
            "test.broad-root",
            "com.example.app",
            Category::System,
            RiskTier::Safe,
            "/tmp/broad-root/com.example.app",
            FileSize::new(1024, Some(1024)),
            1,
        );
        item.unit =
            CleanupUnit::child_namespace("/tmp/broad-root", "/tmp/broad-root/com.example.app");
        item.ownership = CleanupOwnership::inferred("com.example.app");
        item.entry_kind = EntryKind::Directory;
        let mut item = item.with_derived_disposition();
        item.is_selected = true;

        // The inference is accepted ...
        let plan = super::ownership_is_derivable(&item, registry.get("test.broad-root").unwrap());
        assert!(plan, "an inference from the unit's own name is derivable");

        // ... and a claim the catalog cannot support is not.
        let mut forged = item.clone();
        forged.ownership = CleanupOwnership::inferred("somebody.else");
        assert!(!super::ownership_is_derivable(
            &forged,
            registry.get("test.broad-root").unwrap()
        ));
    }

    #[test]
    fn rejects_a_disposition_that_no_longer_matches_the_scan_facts() {
        let mut item = ScanItem::mock(
            "test.stale",
            "test.signature",
            "Stale item",
            Category::Developer,
            RiskTier::Safe,
            "/tmp/stale-item",
            FileSize::new(1024, Some(1024)),
            1,
        );
        item.quality = ObservationQuality::Unavailable;
        item.incomplete_reason = Some("Access was revoked".into());

        let result = SafetyPlanner::create_plan(&[item], &SignatureRegistry::new());
        assert!(matches!(
            result,
            Err(ZenithError::InvalidPlan(message))
                if message.contains("eligibility changed since the scan")
        ));
    }
}
