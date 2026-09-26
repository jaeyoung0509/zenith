use crate::cleaner::OwnerProviderRegistry;
use crate::models::{
    CleanFailureReason, CleanStrategy, CleanupUnitIdentity, DeletePlan, DeleteTarget,
    OwnerProviderSelection, PathIdentity, PlanItemRefusal, RiskSummary, RiskTier, ScanItem,
    ScanResult, Signature, UnitRelationship, ZenithError,
};
use crate::safety::{entry_kind_at, Blacklist, SymlinkGuard, ToctouGuard};
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

/// The item-scoped refusal for a location this build only reports.
///
/// A manual entry is not broken and not blocked: it is a location whose owner
/// — or whose absence of a reviewed operation — means Zenith inventories it and
/// removes nothing. Stating that per item is what keeps a correct refusal from
/// reading as a failed selection.
fn manual_refusal(item: &ScanItem) -> PlanItemRefusal {
    PlanItemRefusal {
        item_id: item.id.clone(),
        item_name: item.name.clone(),
        reason: CleanFailureReason::OwnerManaged,
        message: format!(
            "`{}` is reported for information only; this build has no reviewed operation that removes it",
            item.name
        ),
    }
}

/// An item-scoped refusal for a generic filesystem unit whose contents require
/// an owner-specific cleaner.
///
/// This is a normal policy answer, not a malformed plan. Keeping it in the
/// preview lets the rest of a reviewed selection proceed and avoids turning a
/// recognizable cache-layout change into a page-wide internal error.
fn safety_refusal(
    item: &ScanItem,
    reason: CleanFailureReason,
    message: impl Into<String>,
) -> PlanItemRefusal {
    PlanItemRefusal {
        item_id: item.id.clone(),
        item_name: item.name.clone(),
        reason,
        message: message.into(),
    }
}

/// The item-scoped refusal one provider refusal projects to.
fn plan_refusal(refusal: &crate::models::OwnerUnitRefusal) -> PlanItemRefusal {
    PlanItemRefusal {
        item_id: refusal.item_id.clone(),
        item_name: refusal.item_name.clone(),
        reason: refusal.reason,
        message: refusal.detail.clone(),
    }
}

pub struct SafetyPlanner;

impl SafetyPlanner {
    pub fn create_plan_from_scan(
        scan: &ScanResult,
        scan_id: &str,
        selected_item_ids: &[String],
        registry: &SignatureRegistry,
        environment: &PlatformEnvironment,
        owner_providers: &OwnerProviderRegistry,
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
        let mut plan =
            Self::create_plan_for(&trusted_items, registry, environment, owner_providers)?;
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
        owner_providers: &OwnerProviderRegistry,
    ) -> Result<DeletePlan, ZenithError> {
        Self::create_plan_for(
            items,
            registry,
            &PlatformEnvironment::native(),
            owner_providers,
        )
    }

    /// Creates a plan with the same environment that produced the scan.
    ///
    /// Most callers use [`Self::create_plan_from_scan`], which threads this
    /// environment through the scan-store workflow. This explicit variant is
    /// useful for adapters and deterministic tests that run against a
    /// simulated home directory; it also prevents owner-managed cache rules
    /// from silently consulting the process host instead of the scan host.
    pub fn create_plan_with_environment(
        items: &[ScanItem],
        registry: &SignatureRegistry,
        environment: &PlatformEnvironment,
        owner_providers: &OwnerProviderRegistry,
    ) -> Result<DeletePlan, ZenithError> {
        Self::create_plan_for(items, registry, environment, owner_providers)
    }

    fn create_plan_for(
        items: &[ScanItem],
        registry: &SignatureRegistry,
        environment: &PlatformEnvironment,
        owner_providers: &OwnerProviderRegistry,
    ) -> Result<DeletePlan, ZenithError> {
        let mut targets = Vec::new();
        let mut refusals: Vec<PlanItemRefusal> = Vec::new();
        let mut owner_authorizations = Vec::new();
        let mut owner_selections: std::collections::BTreeMap<String, Vec<OwnerProviderSelection>> =
            std::collections::BTreeMap::new();
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
                refusals.push(manual_refusal(item));
                continue;
            }

            if !item.has_current_disposition() {
                return Err(ZenithError::ChangedSinceScan(format!(
                    "Item '{}' cleanup eligibility changed since the scan; scan again",
                    item.name
                )));
            }

            if !item.allows_cleanup() {
                return Err(ZenithError::ChangedSinceScan(format!(
                    "Item '{}' was not completely inspected or is inaccessible and cannot be cleaned",
                    item.name
                )));
            }

            // A plan authorizes a unit, not a path string: an item that cannot
            // name the unit that produced it, or names a different path than
            // the one it deletes, is not plannable.
            if !item.unit.is_declared() {
                return Err(ZenithError::ChangedSinceScan(format!(
                    "Item '{}' does not name the cleanup unit that authorized it; scan again",
                    item.name
                )));
            }
            if item.unit.path != item.path {
                return Err(ZenithError::ChangedSinceScan(format!(
                    "Item '{}' names a cleanup unit that does not match its path; scan again",
                    item.name
                )));
            }

            // 1. Verify signature exists in registry
            let signature = registry
                .get(&item.signature_id)
                .ok_or_else(|| ZenithError::SignatureMismatch(item.signature_id.clone()))?;

            if signature.strategy == CleanStrategy::Manual {
                refusals.push(manual_refusal(item));
                continue;
            }

            // A manual-tier item that claims a provider unit is executable only
            // when the catalog entry declares a reviewed provider operation: a
            // discovery rule cannot carry the tier past the refusal above on
            // its own.
            let provider_owned = matches!(
                signature.strategy,
                CleanStrategy::LifecycleProvider | CleanStrategy::OwnerProvider
            );
            if item.lifecycle_provider_action != provider_owned {
                return Err(ZenithError::ChangedSinceScan(format!(
                    "Item '{}' disagrees with the catalog about whether a reviewed provider owns its cleanup; scan again",
                    item.name
                )));
            }
            if item.risk == RiskTier::Manual && !provider_owned {
                refusals.push(manual_refusal(item));
                continue;
            }

            // A provider action is dispatched by the id the catalog named, so a
            // plan that cannot name one describes a target nothing can carry
            // out. It is refused here rather than discovered at execution time.
            let provider_id = if signature.strategy == CleanStrategy::LifecycleProvider {
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
                return Err(ZenithError::ChangedSinceScan(format!(
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
                return Err(ZenithError::ChangedSinceScan(format!(
                    "Item '{}' reports ownership the catalog does not state; scan again",
                    item.name
                )));
            }

            // 2. Resolve target path and strategy
            let path = PathBuf::from(&item.path);
            let strategy = signature.strategy;
            let mut identity = None;
            let structured_state_policy = signature.structured_state_policy();

            // An owner-scoped store is not a filesystem target at all. The
            // provider enumerates and removes its own units, so the checks
            // below — which exist to authorize a path — do not describe it;
            // the provider re-derives every fact it mutates on. The selection
            // is collected and handed over once, after the loop, so one store
            // is read once instead of once per unit.
            if strategy == CleanStrategy::OwnerProvider {
                let Some(owner_provider_id) = signature
                    .provider_id
                    .as_deref()
                    .filter(|provider_id| !provider_id.trim().is_empty())
                else {
                    return Err(ZenithError::InvalidPlan(format!(
                        "Item '{}' belongs to an owner-scoped store whose signature names no provider; scan again",
                        item.name
                    )));
                };
                if owner_providers.get(owner_provider_id).is_none() {
                    // An entry no build implements is a target nothing can
                    // carry out, and it is refused for this item rather than
                    // failing the rest of the selection.
                    refusals.push(PlanItemRefusal {
                        item_id: item.id.clone(),
                        item_name: item.name.clone(),
                        reason: CleanFailureReason::ProviderUnavailable,
                        message: format!(
                            "No provider in this build implements `{owner_provider_id}`, so this store cannot be cleaned here"
                        ),
                    });
                    continue;
                }
                owner_selections
                    .entry(signature.id.clone())
                    .or_default()
                    .push(OwnerProviderSelection {
                        item_id: item.id.clone(),
                        name: item.name.clone(),
                        path: path.clone(),
                        expected_bytes: item.cleanable_bytes(),
                    });
                continue;
            }

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
                if let Some((kind, _)) = crate::safety::validator::structured_state_at(&path) {
                    if !structured_state_policy.permits(kind) {
                        refusals.push(safety_refusal(
                            item,
                            CleanFailureReason::StructuredStore,
                            format!(
                                "This unit is a {}; only a dedicated owner cleaner may remove it.",
                                kind.display_name()
                            ),
                        ));
                        continue;
                    }
                }
                if matches!(
                    strategy,
                    CleanStrategy::DeleteContents | CleanStrategy::DeleteDirectory
                ) && path.is_dir()
                {
                    match crate::safety::validator::structured_descendant_with_policy(
                        &path,
                        structured_state_policy,
                    ) {
                        Ok(Some((_nested, kind))) => {
                            refusals.push(safety_refusal(
                                item,
                                CleanFailureReason::StructuredStore,
                                format!(
                                    "This unit contains a {}; only a dedicated owner cleaner may remove the unit.",
                                    kind.display_name()
                                ),
                            ));
                            continue;
                        }
                        Err(error) => {
                            refusals.push(safety_refusal(
                                item,
                                CleanFailureReason::SafetyBoundary,
                                format!(
                                    "The unit could not be completely inspected before cleanup: {error}"
                                ),
                            ));
                            continue;
                        }
                        Ok(None) => {}
                    }
                }

                if structured_state_policy
                    == crate::models::StructuredStatePolicy::VerifiedRegenerableCache
                {
                    match crate::cleaner::running_executables_if_known(&signature.process_guard()) {
                        Some(running) if running.is_empty() => {}
                        Some(running) => {
                            refusals.push(safety_refusal(
                                item,
                                CleanFailureReason::SafetyBoundary,
                                format!(
                                    "Close the cache owner before review: {}",
                                    running.join(", ")
                                ),
                            ));
                            continue;
                        }
                        None => {
                            refusals.push(safety_refusal(
                                item,
                                CleanFailureReason::SafetyBoundary,
                                "The cache owner process state could not be verified",
                            ));
                            continue;
                        }
                    }
                }

                // 7. The entry kind the scan observed must still hold, so a
                //    plan states the kind of object it intends to delete.
                if let Some(current_kind) = entry_kind_at(&path) {
                    if current_kind != item.entry_kind {
                        return Err(ZenithError::ChangedSinceScan(format!(
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
                structured_state_policy,
                provider_id: provider_id.clone(),
                requires_confirmation: item.requires_confirmation,
            });
        }

        // One store is read once, and the provider decides which of the
        // selected units it can verify now. What it refuses is stated per item
        // so the refusal never has to be read as a failed operation.
        for (signature_id, selections) in owner_selections {
            match owner_providers.prepare(registry, &signature_id, &selections, environment) {
                Ok(authorization) => {
                    let bytes = authorization.expected_bytes();
                    expected_reclaim_bytes = expected_reclaim_bytes.saturating_add(bytes);
                    risk_summary.add(authorization.risk, bytes);
                    refusals.extend(authorization.refusals.iter().map(plan_refusal));
                    owner_authorizations.push(authorization);
                }
                Err(refusal) => {
                    let reason = crate::cleaner::owner_providers::refusal_reason(refusal.status);
                    if refusal.refusals.is_empty() {
                        refusals.extend(selections.iter().map(|selection| PlanItemRefusal {
                            item_id: selection.item_id.clone(),
                            item_name: selection.name.clone(),
                            reason,
                            message: refusal.detail.clone(),
                        }));
                    } else {
                        refusals.extend(refusal.refusals.iter().map(plan_refusal));
                    }
                }
            }
        }

        if targets.is_empty() && owner_authorizations.is_empty() {
            if !refusals.is_empty() {
                // The refusals are the answer, not a failure of the scan: they
                // name the items a current policy would not authorize. The
                // caller states them per item and keeps the inventory.
                return Err(ZenithError::RefusedSelection(refusals));
            }
            return Err(ZenithError::InvalidPlan(
                "No valid cleanable targets were selected".to_string(),
            ));
        }

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mode = DeletePlan::mode_for(&targets, &owner_authorizations);

        Ok(DeletePlan {
            id: Uuid::new_v4(),
            scan_id: String::new(),
            targets,
            refusals,
            owner_authorizations,
            expected_reclaim_bytes,
            risk: risk_summary,
            created_at: now,
            mode,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::SafetyPlanner;
    use crate::cleaner::OwnerProviderRegistry;
    use crate::models::{Category, FileSize, ObservationQuality, RiskTier, ScanItem, ZenithError};
    use crate::signatures::SignatureRegistry;

    /// A registry with no owner-scoped provider, which is what a generic
    /// cleanup assertion means: nothing in this build owns the store.
    fn no_owner_providers() -> OwnerProviderRegistry {
        OwnerProviderRegistry::new(Vec::new())
    }

    #[test]
    fn a_recent_cursor_renderer_cache_with_database_and_index_completes_review_and_cleanup() {
        use crate::models::{CleanupEligibility, StructuredStatePolicy};
        use crate::safety::{RevalidationOutcome, SafeTreeDeleter, SafetyValidator};
        use crate::scanner::DirectoryScanner;
        use zenith_platform::path_algebra::PathFlavor;
        use zenith_platform::PlatformEnvironment;

        let fixture = tempfile::tempdir().expect("disposable fixture");
        let home = fixture.path().join("home");
        let cache = home.join("Library/Application Support/Cursor/Cache");
        std::fs::create_dir_all(&cache).expect("cache fixture");
        std::fs::write(cache.join("index.db"), vec![b'd'; 8_192]).expect("database fixture");
        std::fs::write(cache.join("index.db-wal"), vec![b'w'; 4_096]).expect("WAL fixture");
        std::fs::write(cache.join("index.json"), vec![b'j'; 2_048]).expect("index fixture");

        let environment = PlatformEnvironment::simulated(PathFlavor::current()).with_home(&home);
        let registry = SignatureRegistry::load_embedded_with(&environment)
            .expect("catalog validates for the fixture environment");
        let signature = registry
            .get("ai.cursor.renderer_cache")
            .expect("verified Cursor cache signature");
        let mut items = DirectoryScanner::scan_signature(
            signature,
            &environment,
            &crate::models::NeverCancelled,
        );
        assert_eq!(
            items.len(),
            1,
            "only the existing named cache unit is found"
        );
        let item = &mut items[0];
        assert_eq!(
            item.disposition.eligibility,
            CleanupEligibility::AutoCleanable
        );
        assert!(item.age.is_none(), "a fresh cache needs no retention delay");
        assert!(item.cleanable_bytes() > 0);
        assert!(
            item.is_selected,
            "verified regenerable caches are selected by default"
        );

        let plan = SafetyPlanner::create_plan_with_environment(
            &items,
            &registry,
            &environment,
            &no_owner_providers(),
        )
        .expect("explicitly reviewed cache unit is plannable");
        assert_eq!(
            plan.targets[0].structured_state_policy,
            StructuredStatePolicy::VerifiedRegenerableCache
        );

        let validated = match SafetyValidator::revalidate(&plan.targets[0], &environment) {
            RevalidationOutcome::Validated(target) => target,
            RevalidationOutcome::Skipped(result) | RevalidationOutcome::Failed(result) => {
                panic!("unchanged disposable fixture must remain authorized: {result:?}")
            }
        };
        let report = SafeTreeDeleter::delete_path_validated(&validated, &environment);
        assert!(
            report.errors.is_empty(),
            "cleanup errors: {:?}",
            report.errors
        );
        assert!(
            !cache.exists(),
            "the reviewed cache unit is removed as a whole"
        );
    }

    #[test]
    fn brave_code_cache_review_removes_only_the_named_unit() {
        use crate::models::{CleanupEligibility, StructuredStatePolicy};
        use crate::safety::{RevalidationOutcome, SafeTreeDeleter, SafetyValidator};
        use crate::scanner::DirectoryScanner;
        use zenith_platform::path_algebra::PathFlavor;
        use zenith_platform::PlatformEnvironment;

        let fixture = tempfile::tempdir().expect("disposable fixture");
        let home = fixture.path().join("home");
        let profile = home.join("Library/Caches/BraveSoftware/Brave-Browser/Default");
        let cache = profile.join("Code Cache");
        std::fs::create_dir_all(&cache).expect("cache fixture");
        std::fs::write(cache.join("index.db"), vec![b'c'; 4096]).expect("cache database");
        let sibling = profile.join("Cookies");
        std::fs::write(&sibling, b"keep browser state").expect("protected sibling");
        let http_cache = profile.join("Cache");
        std::fs::create_dir_all(&http_cache).expect("HTTP cache fixture");
        std::fs::write(http_cache.join("entry"), b"observed only").unwrap();

        let environment = PlatformEnvironment::simulated(PathFlavor::current()).with_home(&home);
        let registry = SignatureRegistry::load_embedded_with(&environment).expect("catalog");
        let signature = registry
            .get("system.brave.code_cache")
            .expect("reviewed code cache signature");
        let observed = DirectoryScanner::scan_signature(
            registry
                .get("system.brave.http_cache")
                .expect("HTTP cache inventory"),
            &environment,
            &crate::models::NeverCancelled,
        );
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].risk, RiskTier::Rebuild);
        assert!(observed[0].cleanable_bytes() > 0);
        assert!(observed[0].is_selected);
        let mut items = DirectoryScanner::scan_signature(
            signature,
            &environment,
            &crate::models::NeverCancelled,
        );
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].disposition.eligibility,
            CleanupEligibility::AutoCleanable
        );
        assert_eq!(items[0].age.as_ref().map(|age| age.min_age_days), Some(0));
        assert!(items[0].is_selected);
        items[0].is_selected = true;

        let plan = SafetyPlanner::create_plan_with_environment(
            &items,
            &registry,
            &environment,
            &no_owner_providers(),
        )
        .expect("explicitly reviewed cache unit is plannable");
        assert_eq!(
            plan.targets[0].structured_state_policy,
            StructuredStatePolicy::VerifiedRegenerableCache
        );
        let validated = match SafetyValidator::revalidate(&plan.targets[0], &environment) {
            RevalidationOutcome::Validated(target) => target,
            RevalidationOutcome::Skipped(result) | RevalidationOutcome::Failed(result) => {
                panic!("unchanged disposable fixture must remain authorized: {result:?}")
            }
        };
        let report = SafeTreeDeleter::delete_path_validated(&validated, &environment);
        assert!(
            report.errors.is_empty(),
            "cleanup errors: {:?}",
            report.errors
        );
        assert!(!cache.exists());
        assert_eq!(std::fs::read(&sibling).unwrap(), b"keep browser state");
        assert_eq!(
            std::fs::read(http_cache.join("entry")).unwrap(),
            b"observed only"
        );
    }

    #[test]
    fn brave_code_cache_with_credentials_refuses_the_whole_unit() {
        use crate::scanner::DirectoryScanner;
        use zenith_platform::path_algebra::PathFlavor;
        use zenith_platform::PlatformEnvironment;

        let fixture = tempfile::tempdir().expect("disposable fixture");
        let home = fixture.path().join("home");
        let cache = home.join("Library/Caches/BraveSoftware/Brave-Browser/Default/Code Cache");
        std::fs::create_dir_all(&cache).expect("cache fixture");
        let generated = cache.join("index.db");
        let protected = cache.join("auth.json");
        std::fs::write(&generated, b"generated cache index").unwrap();
        std::fs::write(&protected, b"protected state").unwrap();

        let environment = PlatformEnvironment::simulated(PathFlavor::current()).with_home(&home);
        let registry = SignatureRegistry::load_embedded_with(&environment).expect("catalog");
        let signature = registry
            .get("system.brave.code_cache")
            .expect("Brave cache");
        let mut items = DirectoryScanner::scan_signature(
            signature,
            &environment,
            &crate::models::NeverCancelled,
        );
        assert_eq!(items.len(), 1);
        items[0].is_selected = true;
        let plan = SafetyPlanner::create_plan_with_environment(
            &items,
            &registry,
            &environment,
            &no_owner_providers(),
        );
        assert!(matches!(plan, Err(ZenithError::RefusedSelection(_))));
        assert_eq!(std::fs::read(&generated).unwrap(), b"generated cache index");
        assert_eq!(std::fs::read(&protected).unwrap(), b"protected state");
    }

    #[test]
    fn settings_and_credentials_invalidate_a_cursor_cache_unit_before_mutation() {
        use crate::models::{CleanFailureReason, StructuredStatePolicy};
        use crate::scanner::DirectoryScanner;
        use zenith_platform::path_algebra::PathFlavor;
        use zenith_platform::PlatformEnvironment;

        for protected_name in ["settings.json", "auth.json"] {
            let fixture = tempfile::tempdir().expect("disposable fixture");
            let home = fixture.path().join("home");
            let cache = home.join("Library/Application Support/Cursor/Cache");
            std::fs::create_dir_all(&cache).expect("cache fixture");
            let database = cache.join("index.db");
            let protected = cache.join(protected_name);
            std::fs::write(&database, b"regenerable cache database").expect("database fixture");
            std::fs::write(&protected, b"keep this protected state").expect("protected fixture");

            let environment =
                PlatformEnvironment::simulated(PathFlavor::current()).with_home(&home);
            let registry = SignatureRegistry::load_embedded_with(&environment)
                .expect("catalog validates for the fixture environment");
            let signature = registry
                .get("ai.cursor.renderer_cache")
                .expect("verified Cursor cache signature");
            let mut items = DirectoryScanner::scan_signature(
                signature,
                &environment,
                &crate::models::NeverCancelled,
            );
            assert_eq!(items.len(), 1);
            items[0].is_selected = true;

            let result = SafetyPlanner::create_plan_with_environment(
                &items,
                &registry,
                &environment,
                &no_owner_providers(),
            );
            match result {
                Err(ZenithError::RefusedSelection(refusals)) => assert_eq!(
                    refusals[0].reason,
                    CleanFailureReason::StructuredStore,
                    "the verified-cache exception still excludes {protected_name}"
                ),
                other => panic!("protected content refuses the whole unit: {other:?}"),
            }
            assert!(
                database.exists(),
                "the cache stays intact with {protected_name}"
            );
            assert!(protected.exists(), "protected entry is never removed");
            assert_eq!(
                registry
                    .get("ai.cursor.renderer_cache")
                    .unwrap()
                    .structured_state_policy(),
                StructuredStatePolicy::VerifiedRegenerableCache
            );
        }
    }

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
            provider_restriction: None,
            entry_kind: crate::models::EntryKind::File,
            gate: Default::default(),
            owner_running: false,
            lifecycle_provider_action: false,
            requires_confirmation: false,
            overlaps: Vec::new(),
            is_selected: true,
            last_modified: None,
            exists: true,
            quality: ObservationQuality::Fresh,
            incomplete_reason: None,
            skipped_entry_count: 0,
        };

        let result =
            SafetyPlanner::create_plan(&[item], &SignatureRegistry::new(), &no_owner_providers());
        match result {
            Err(ZenithError::RefusedSelection(refusals)) => {
                assert_eq!(refusals.len(), 1);
                assert_eq!(refusals[0].item_name, "OrbStack VM Storage");
                assert_eq!(
                    refusals[0].reason,
                    crate::models::CleanFailureReason::OwnerManaged
                );
            }
            other => panic!("a manual observation is refused per item: {other:?}"),
        }
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
                family: Default::default(),
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
                artifact_kind: Default::default(),
                consequence: String::new(),
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

        let plan = SafetyPlanner::create_plan(&[item.clone()], &registry, &no_owner_providers())
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
        let refused = SafetyPlanner::create_plan(
            &[item.clone()],
            &filesystem_registry,
            &no_owner_providers(),
        );
        assert!(
            matches!(&refused, Err(ZenithError::ChangedSinceScan(message)) if message.contains("disagrees with the catalog")),
            "a manual unit cannot claim provider authority the catalog does not declare: {refused:?}"
        );

        // A catalog entry that names no provider describes an action nothing
        // can carry out, so the plan refuses it rather than leaving a target
        // the executor would have to guess about.
        let mut unnamed_registry = SignatureRegistry::new();
        unnamed_registry.register(provider_signature(None));
        let refused = SafetyPlanner::create_plan(&[item], &unnamed_registry, &no_owner_providers());
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
                family: Default::default(),
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
                artifact_kind: Default::default(),
                consequence: String::new(),
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

        let plan = SafetyPlanner::create_plan(
            &[child_item, parent_item],
            &registry,
            &no_owner_providers(),
        )
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
                family: Default::default(),
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
                artifact_kind: Default::default(),
                consequence: String::new(),
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

        let result = SafetyPlanner::create_plan(
            &[child_item, parent_item],
            &registry,
            &no_owner_providers(),
        );

        match &result {
            Ok(plan) => {
                assert_eq!(
                    plan.refusals.len(),
                    1,
                    "the contained unit's refusal is reported, not skipped"
                );
                assert_eq!(plan.refusals[0].item_name, "Nested cache");
                assert_eq!(
                    plan.refusals[0].reason,
                    crate::models::CleanFailureReason::OwnerManaged
                );
                assert_eq!(
                    plan.targets.len(),
                    1,
                    "the parent the user also selected is still authorized"
                );
                assert_eq!(plan.targets[0].item_id, "parent-item");
            }
            other => panic!("the plan reports the refusal and keeps the rest: {other:?}"),
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
                family: Default::default(),
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
                artifact_kind: Default::default(),
                consequence: String::new(),
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

        let plan = SafetyPlanner::create_plan(
            &[first_item, second_item],
            &registry,
            &no_owner_providers(),
        )
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
            family: Default::default(),
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
            artifact_kind: Default::default(),
            consequence: String::new(),
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

        let result =
            SafetyPlanner::create_plan(&[item], &SignatureRegistry::new(), &no_owner_providers());
        assert!(matches!(
            result,
            Err(ZenithError::ChangedSinceScan(message))
                if message.contains("eligibility changed since the scan")
        ));
    }
}
