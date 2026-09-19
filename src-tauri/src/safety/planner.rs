use crate::models::{
    CleanStrategy, CleanupMode, CleanupUnitIdentity, DeletePlan, DeleteTarget, PathIdentity,
    RiskSummary, RiskTier, ScanItem, ScanResult, Signature, UnitRelationship, ZenithError,
};
use crate::safety::{entry_kind_at, structured_state_at, Blacklist, SymlinkGuard, ToctouGuard};
use crate::scanner::relationship::unit_relationship;
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

        // A plan authorizes each location once. The scan already reports an
        // overlapping unit as the broader unit's provenance, and a caller that
        // hands the planner both — an older scan, or a selection assembled by
        // hand — must not produce a plan whose expected reclaim counts the same
        // bytes twice. The broader unit wins, exactly as it does in the scan's
        // accounting, and the plan says which unit was folded into which.
        //
        // Whether two units are one location is the same question the scan
        // answered, and it is answered by the same function: stable filesystem
        // identity first, path text as the conservative fallback. Deriving it
        // from the OS family instead would let a plan disagree with the scan
        // about a case-sensitive directory on a folding platform.
        let mut candidates: Vec<(usize, CleanupUnitIdentity, usize)> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.is_selected && item.unit.is_declared())
            .map(|(index, item)| {
                let key = item.unit_identity(PathIdentity::CaseSensitive);
                (key.components().count(), key, index)
            })
            .collect();
        candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        let mut authorized: Vec<usize> = Vec::new();
        let mut overlapped: HashSet<usize> = HashSet::new();
        for (_, _key, index) in candidates {
            let contained = authorized.iter().any(|outer| {
                let outer_item = &items[*outer];
                match unit_relationship(&items[index], outer_item) {
                    UnitRelationship::Equivalent
                    | UnitRelationship::EquivalentConflict
                    | UnitRelationship::Contained
                    | UnitRelationship::AuthorityConflict => true,
                    UnitRelationship::Distinct => false,
                }
            });
            if contained {
                overlapped.insert(index);
                crate::diagnostics::log_error(
                    "cleanup",
                    &format!(
                        "Planned unit {} is inside a unit this plan already authorizes; its bytes are counted once",
                        items[index].path
                    ),
                );
            } else {
                authorized.push(index);
            }
        }

        for (index, item) in items.iter().enumerate() {
            // Only consider selected items
            if !item.is_selected {
                continue;
            }

            // Read-only adapter observations do not need a registry signature,
            // but they must always fail before any generic filesystem planning
            // is attempted. The one exception is a provider action: it owns no
            // host path either, and the signature below must still declare the
            // provider operation before it is allowed through.
            if item.risk == RiskTier::Manual && !item.lifecycle_provider_action {
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

            // A manual-tier item that claims a provider unit is executable only
            // when the catalog entry declares the provider action: a discovery
            // rule cannot carry the tier past the refusal above on its own.
            let provider_action = signature.strategy == CleanStrategy::LifecycleProvider;
            if item.lifecycle_provider_action != provider_action {
                return Err(ZenithError::InvalidPlan(format!(
                    "Item '{}' disagrees with the catalog about whether a lifecycle provider owns its cleanup; scan again",
                    item.name
                )));
            }
            if item.risk == RiskTier::Manual && !provider_action {
                return Err(ZenithError::UnsupportedManualOperation(item.name.clone()));
            }

            // A provider action is dispatched by the id the catalog named, so a
            // plan that cannot name one describes a target nothing can carry
            // out. It is refused here rather than discovered at execution time.
            let provider_id = if provider_action {
                match signature
                    .provider_id
                    .as_deref()
                    .filter(|provider_id| !provider_id.trim().is_empty())
                {
                    Some(provider_id) => Some(provider_id.to_string()),
                    None => {
                        return Err(ZenithError::InvalidPlan(format!(
                            "Item '{}' is a lifecycle provider action whose signature names no provider; scan again",
                            item.name
                        )))
                    }
                }
            } else {
                None
            };

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

            // A provider action is not a filesystem operation at all: it owns
            // no host path, so the pseudo location carries no deletion
            // authority, and the checks below — which exist to authorize a
            // path — do not apply to it. The provider re-derives its own state
            // at execution time instead.
            if matches!(
                strategy,
                CleanStrategy::DockerPrune | CleanStrategy::LifecycleProvider
            ) {
                // DockerPrune uses pseudo paths (e.g. docker://images/dangling)
                // and dedicated Docker CLI adapters; a lifecycle provider uses
                // the stable location its own implementation declares. Neither
                // operates on a host filesystem path.
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

            // A unit inside a unit this plan already authorizes is that unit's
            // bytes: the plan authorizes the location once. The item is skipped
            // only here, after every refusal above has run, so an unplannable
            // item still fails the plan instead of disappearing from it.
            if overlapped.contains(&index) {
                continue;
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
                provider_id: provider_id.clone(),
                requires_confirmation: item.requires_confirmation,
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
            overlaps: Vec::new(),
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

    /// A provider-backed unit is plannable by explicit selection, and the plan
    /// names the one operation that carries it out. The manual tier still
    /// refuses everything generic: a unit that claims a provider without a
    /// catalog entry declaring one is refused, and so is a lifecycle signature
    /// that names no provider at all.
    #[test]
    fn a_provider_backed_unit_plans_through_the_provider_the_catalog_named() {
        use crate::cleaner::LifecycleProviderRegistry;
        use crate::models::{CleanStrategy, Signature};
        use zenith_platform::path_algebra::PathFlavor;
        use zenith_platform::PlatformEnvironment;

        fn provider_signature(provider_id: Option<&str>) -> Signature {
            Signature {
                id: "test.stated.store".to_string(),
                name: "Stated Store".to_string(),
                category: Category::System,
                risk: RiskTier::Manual,
                strategy: CleanStrategy::LifecycleProvider,
                paths: Vec::new(),
                exclusions: vec![],
                description: String::new(),
                min_age_days: None,
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
                provider_id: provider_id.map(str::to_string),
                management_mode: Default::default(),
                artifact_kind: Default::default(),
                consequence: String::new(),
                reclaimable_is_lower_bound: false,
            }
        }

        let environment = PlatformEnvironment::simulated(PathFlavor::current());
        let providers = LifecycleProviderRegistry::new(vec![
            crate::cleaner::providers::test_support::StatedProvider::holding(2_048, 2).shared(),
        ]);
        let mut registry = SignatureRegistry::new();
        registry.register(provider_signature(Some("test.stated")));

        let items = providers.scan_items(&registry, Category::System, false, &[], &environment);
        let mut item = items
            .into_iter()
            .next()
            .expect("the provider offers a candidate");
        assert!(
            !item.is_selected,
            "the scan offers a provider action without pre-selecting it"
        );
        item.is_selected = true;

        let plan = SafetyPlanner::create_plan(&[item.clone()], &registry)
            .expect("an explicitly selected provider action is plannable");
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].strategy, CleanStrategy::LifecycleProvider);
        assert_eq!(plan.targets[0].provider_id.as_deref(), Some("test.stated"));
        assert_eq!(
            plan.targets[0].unit.kind,
            crate::models::CleanupUnitKind::ProviderAction
        );
        assert_eq!(plan.expected_reclaim_bytes, 2_048);

        // The catalog entry is the authority: the same item under a signature
        // that does not declare the provider operation is refused.
        let mut filesystem_registry = SignatureRegistry::new();
        let mut filesystem_signature = provider_signature(Some("test.stated"));
        filesystem_signature.strategy = CleanStrategy::DeleteContents;
        filesystem_signature.risk = RiskTier::Manual;
        filesystem_registry.register(filesystem_signature);
        let refused = SafetyPlanner::create_plan(&[item.clone()], &filesystem_registry);
        assert!(
            matches!(&refused, Err(ZenithError::UnsupportedManualOperation(name)) if name == "Stated Store"),
            "a manual unit is refused unless the catalog declares the provider action: {refused:?}"
        );

        // A catalog entry that names no provider describes an action nothing
        // can carry out, so the plan refuses it rather than leaving a target
        // the executor would have to guess about.
        let mut unnamed_registry = SignatureRegistry::new();
        unnamed_registry.register(provider_signature(None));
        let refused = SafetyPlanner::create_plan(&[item], &unnamed_registry);
        assert!(
            matches!(&refused, Err(ZenithError::InvalidPlan(message)) if message.contains("names no provider")),
            "a lifecycle signature without a provider id cannot be planned: {refused:?}"
        );
    }

    /// A plan authorizes each location once: a unit inside another selected
    /// unit is that unit's bytes, and the plan's expected reclaim states it.
    #[test]
    fn a_plan_counts_an_overlapping_unit_once() {
        use crate::models::{CleanStrategy, CleanupUnit, EntryKind, Signature};

        let fixture = tempfile::tempdir().expect("fixture");
        let parent = fixture.path().join("Cache");
        let child = parent.join("nested");
        std::fs::create_dir_all(&child).expect("fixture");
        std::fs::write(parent.join("data.bin"), vec![1u8; 8_192]).expect("fixture");
        std::fs::write(child.join("state.bin"), vec![2u8; 8_192]).expect("fixture");

        let mut registry = SignatureRegistry::new();
        for (id, name, path, strategy) in [
            (
                "test.parent",
                "Parent cache",
                parent.clone(),
                CleanStrategy::DeleteDirectory,
            ),
            (
                "test.child",
                "Nested cache",
                child.clone(),
                CleanStrategy::DeleteContents,
            ),
        ] {
            registry.register(Signature {
                id: id.into(),
                name: name.into(),
                category: Category::System,
                risk: RiskTier::Safe,
                strategy,
                paths: vec![path.to_string_lossy().into_owned()],
                exclusions: vec![],
                description: String::new(),
                min_age_days: None,
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
                provider_id: None,
                management_mode: Default::default(),
                artifact_kind: Default::default(),
                consequence: String::new(),
                reclaimable_is_lower_bound: false,
            });
        }

        let selected =
            |id: &str, signature: &str, name: &str, path: &std::path::Path, bytes: u64| {
                let mut item = ScanItem::mock(
                    id,
                    signature,
                    name,
                    Category::System,
                    RiskTier::Safe,
                    path.to_string_lossy(),
                    FileSize::new(bytes, Some(bytes)),
                    1,
                );
                item.unit = CleanupUnit::fixed_path(path.to_string_lossy());
                item.entry_kind = EntryKind::Directory;
                item.rederive_disposition();
                item.is_selected = true;
                item
            };
        let parent_item = selected(
            "parent-item",
            "test.parent",
            "Parent cache",
            &parent,
            16_384,
        );
        let child_item = selected("child-item", "test.child", "Nested cache", &child, 8_192);

        let plan = SafetyPlanner::create_plan(&[child_item, parent_item], &registry)
            .expect("the plan is built");

        assert_eq!(
            plan.targets.len(),
            1,
            "a location inside an authorized unit is not authorized a second time"
        );
        assert_eq!(plan.targets[0].path, parent);
        assert_eq!(
            plan.expected_reclaim_bytes, 16_384,
            "the expectation counts the broader unit's bytes once"
        );
    }

    /// A unit inside another selected unit is skipped only after every refusal
    /// has run: an unplannable item fails the plan instead of quietly leaving
    /// it, even when the broader unit would have covered its bytes.
    #[test]
    fn an_unplannable_contained_item_still_fails_the_plan() {
        use crate::models::{CleanStrategy, CleanupUnit, EntryKind, Signature};

        let fixture = tempfile::tempdir().expect("fixture");
        let parent = fixture.path().join("Cache");
        let child = parent.join("nested");
        std::fs::create_dir_all(&child).expect("fixture");
        std::fs::write(parent.join("data.bin"), vec![1u8; 8_192]).expect("fixture");
        std::fs::write(child.join("state.bin"), vec![2u8; 8_192]).expect("fixture");

        let mut registry = SignatureRegistry::new();
        for (id, name, path) in [
            ("test.parent", "Parent cache", parent.clone()),
            ("test.child", "Nested cache", child.clone()),
        ] {
            registry.register(Signature {
                id: id.into(),
                name: name.into(),
                category: Category::System,
                risk: RiskTier::Safe,
                strategy: CleanStrategy::DeleteDirectory,
                paths: vec![path.to_string_lossy().into_owned()],
                exclusions: vec![],
                description: String::new(),
                min_age_days: None,
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
                provider_id: None,
                management_mode: Default::default(),
                artifact_kind: Default::default(),
                consequence: String::new(),
                reclaimable_is_lower_bound: false,
            });
        }

        let mut parent_item = ScanItem::mock(
            "parent-item",
            "test.parent",
            "Parent cache",
            Category::System,
            RiskTier::Safe,
            parent.to_string_lossy(),
            FileSize::new(16_384, Some(16_384)),
            1,
        );
        parent_item.unit = CleanupUnit::fixed_path(parent.to_string_lossy());
        parent_item.entry_kind = EntryKind::Directory;
        parent_item.rederive_disposition();
        parent_item.is_selected = true;

        // A manual-risk item inside the parent: the scan's own accounting would
        // have folded its verdict into the parent, but a hand-built selection
        // can still present the pair, and the refusal must win.
        let mut child_item = ScanItem::mock(
            "child-item",
            "test.child",
            "Nested cache",
            Category::System,
            RiskTier::Manual,
            child.to_string_lossy(),
            FileSize::new(8_192, Some(8_192)),
            1,
        );
        child_item.unit = CleanupUnit::fixed_path(child.to_string_lossy());
        child_item.entry_kind = EntryKind::Directory;
        child_item.rederive_disposition();
        child_item.is_selected = true;

        let result = SafetyPlanner::create_plan(&[child_item, parent_item], &registry);

        match &result {
            Err(ZenithError::UnsupportedManualOperation(name)) => {
                assert_eq!(name, "Nested cache");
            }
            other => panic!("the contained unit's refusal is reported, not skipped: {other:?}"),
        }
    }

    /// A plan folds only what the filesystem identified as one unit. Two
    /// distinct directories that differ by name are two targets, and the
    /// containment rule that does fold is the one the scan used, so a plan and
    /// the scan cannot disagree about a case-sensitive volume (see
    /// `scanner::relationship` for the identity-first rule itself).
    #[test]
    fn a_plan_keeps_distinct_units_distinct() {
        use crate::models::{CleanStrategy, CleanupUnit, EntryKind, Signature};

        let fixture = tempfile::tempdir().expect("fixture");
        let first = fixture.path().join("Cache");
        let second = fixture.path().join("Other");
        std::fs::create_dir_all(&first).expect("fixture");
        std::fs::create_dir_all(&second).expect("fixture");
        std::fs::write(first.join("data.bin"), vec![1u8; 8_192]).expect("fixture");
        std::fs::write(second.join("data.bin"), vec![2u8; 8_192]).expect("fixture");

        let mut registry = SignatureRegistry::new();
        for (id, name, path) in [
            ("test.first", "First cache", first.clone()),
            ("test.second", "Second cache", second.clone()),
        ] {
            registry.register(Signature {
                id: id.into(),
                name: name.into(),
                category: Category::System,
                risk: RiskTier::Safe,
                strategy: CleanStrategy::DeleteContents,
                paths: vec![path.to_string_lossy().into_owned()],
                exclusions: vec![],
                description: String::new(),
                min_age_days: None,
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
                provider_id: None,
                management_mode: Default::default(),
                artifact_kind: Default::default(),
                consequence: String::new(),
                reclaimable_is_lower_bound: false,
            });
        }

        let selected = |id: &str, signature: &str, name: &str, path: &std::path::Path| {
            let mut item = ScanItem::mock(
                id,
                signature,
                name,
                Category::System,
                RiskTier::Safe,
                path.to_string_lossy(),
                FileSize::new(8_192, Some(8_192)),
                1,
            );
            item.unit = CleanupUnit::fixed_path(path.to_string_lossy());
            item.entry_kind = EntryKind::Directory;
            item.rederive_disposition();
            item.is_selected = true;
            item
        };
        let first_item = selected("first-item", "test.first", "First cache", &first);
        let second_item = selected("second-item", "test.second", "Second cache", &second);

        let plan = SafetyPlanner::create_plan(&[first_item, second_item], &registry)
            .expect("two separate units are plannable");

        assert_eq!(
            plan.targets.len(),
            2,
            "separate locations are separate targets"
        );
        assert_eq!(plan.expected_reclaim_bytes, 16_384);
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
            provider_id: None,
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
