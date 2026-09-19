//! Counting a location once when more than one rule describes it.
//!
//! A catalog can describe the same storage twice: two signatures can name one
//! cache directory, and a broader rule can name a directory that contains a
//! unit a narrower rule already enumerated. Counting both would report a total
//! no user could reconcile with their disk, and a plan built from both would
//! claim to reclaim the same bytes twice.
//!
//! The resolution here is the accounting decision only: the broader unit keeps
//! the bytes — it is the object a plan would delete — and every unit inside it
//! becomes provenance on it. Nothing is discarded. The suppressed unit's own
//! verdict travels with the provenance, so the surviving unit is judged by the
//! strictest rule that described the location, and the interface can say which
//! rule imposed that verdict and how many bytes are accounted for this way.

use super::{
    CacheManagementMode, CategoryResult, CleanupEligibility, CleanupUnitIdentity, CleanupUnitKind,
    EligibilityGate, PathIdentity, ScanItem,
};
use crate::domain::RiskTier;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// One other catalog rule that named the same location.
///
/// The entry is provenance, not a second target: its bytes are already counted
/// by the unit that carries it, and its verdict is what the container's
/// disposition is re-derived with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CleanupOverlap {
    pub signature_id: String,
    pub name: String,
    pub unit_path: String,
    pub eligibility: CleanupEligibility,
    #[serde(default = "default_overlap_risk")]
    pub risk: RiskTier,
    #[serde(default)]
    pub management_mode: CacheManagementMode,
    #[serde(default)]
    pub consequence: String,
    #[serde(default)]
    pub unit_kind: CleanupUnitKind,
    /// The gate the other rule was discovered under.
    ///
    /// A rule whose scope is switched off is discovered for visibility, not for
    /// authority: it must not withhold, downgrade, or constrain the rule that
    /// the current settings do run.
    #[serde(default)]
    pub gate: EligibilityGate,
    /// This overlap prevents the broader item from authorizing generic cleanup.
    ///
    /// That can be an operation-authority disagreement, or unresolved coverage:
    /// when a partial container and a complete nested observation may share
    /// bytes, the broader item cannot independently claim those same bytes as
    /// reclaimable.
    #[serde(default)]
    pub authority_conflict: bool,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub observed_bytes: u64,
}

impl CleanupOverlap {
    /// The provenance of one item that did not count its own bytes.
    pub fn of(item: &ScanItem) -> Self {
        Self {
            signature_id: item.signature_id.clone(),
            name: item.name.clone(),
            unit_path: item.unit.path.clone(),
            eligibility: item.disposition.eligibility,
            risk: item.risk,
            management_mode: item.cache_metadata.management_mode,
            consequence: item.cache_metadata.consequence.clone(),
            unit_kind: item.unit.kind,
            gate: item.gate,
            authority_conflict: false,
            observed_bytes: item.observed_bytes(),
        }
    }
}

fn default_overlap_risk() -> RiskTier {
    RiskTier::Safe
}

/// A discovery whose bytes a unit found earlier already counted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlappedDiscovery {
    /// The identity of the unit that counted these bytes.
    pub retained: CleanupUnitIdentity,
    pub overlap: CleanupOverlap,
}

/// How much of a scan was reported as another unit's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OverlapReport {
    pub suppressed_count: u64,
    pub suppressed_bytes: u64,
    /// Containments the resolution kept as two rows. The bytes are counted in
    /// the totals, but they may already be part of the container's measurement,
    /// so the union is between `total_bytes - ambiguous_bytes` and
    /// `total_bytes`.
    pub ambiguous_count: u64,
    pub ambiguous_bytes: u64,
}

/// The relationship established by the filesystem for two scanned units.
/// `None` from the resolver callback means that textual identity is the only
/// available evidence; an explicit `Distinct` always wins over text folding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitRelationship {
    Distinct,
    /// The same location.
    Equivalent,
    /// The same location through two rules that disagree about what may be done
    /// with it. It is still one location, so its bytes are counted once, but
    /// neither rule's operation may authorize the other's: the surviving item
    /// states the conflict and is not cleanable.
    EquivalentConflict,
    /// One location inside another.
    Contained,
    /// A nested rule requires a different mutation authority, so the broader
    /// filesystem item is inventory only and must never become a fallback.
    AuthorityConflict,
}

/// Folds units that describe the same location into one accounting decision.
///
/// `overlapped` carries the discoveries that were dropped where they were
/// found, because the unit that already counted them had been seen. This pass
/// adds what only the whole result can see: a unit that *contains* a unit
/// reported by another rule.
///
/// The candidates are visited broadest-first with ties broken by identity, so
/// which unit keeps the bytes is a property of the units rather than of the
/// order the filesystem happened to answer in. Categories the resolution
/// touched state their totals again from the items they still retain.
///
/// Containment is folded only when the container is a complete observation. A
/// partial walk, a skipped entry, or an incomplete reason leaves the nested
/// observation as its own item because path containment is not proof that the
/// broader measurement included those bytes. A cleanup-authority conflict also
/// keeps both observations and blocks the broader item from generic cleanup.
pub fn resolve_unit_overlaps(
    categories: &mut [CategoryResult],
    overlapped: &[OverlappedDiscovery],
    identity: PathIdentity,
) -> OverlapReport {
    resolve_unit_overlaps_with(categories, overlapped, identity, |_, _| None)
}

pub fn resolve_unit_overlaps_with<F>(
    categories: &mut [CategoryResult],
    overlapped: &[OverlappedDiscovery],
    identity: PathIdentity,
    relationship: F,
) -> OverlapReport
where
    F: Fn(&ScanItem, &ScanItem) -> Option<UnitRelationship>,
{
    let mut report = OverlapReport::default();
    let mut touched: HashSet<usize> = HashSet::new();

    for discovery in overlapped {
        if let Some((category, item)) = locate(categories, &discovery.retained, identity) {
            let retained = &mut categories[category].items[item];
            retained.risk = retained.risk.max(discovery.overlap.risk);
            retained.cache_metadata.management_mode = stricter_management_mode(
                retained.cache_metadata.management_mode,
                discovery.overlap.management_mode,
            );
            if discovery.overlap.risk == retained.risk && !discovery.overlap.consequence.is_empty()
            {
                retained.cache_metadata.consequence = discovery.overlap.consequence.clone();
            }
            retained.overlaps.push(discovery.overlap.clone());
            touched.insert(category);
        }
    }

    let mut candidates: Vec<(usize, CleanupUnitIdentity, u8, u8, usize, usize)> = Vec::new();
    for (category_index, category) in categories.iter().enumerate() {
        for (item_index, item) in category.items.iter().enumerate() {
            if !item.unit.is_declared() {
                continue;
            }
            let key = item.unit_identity(identity);
            candidates.push((
                key.components().count(),
                key,
                // A rule the current settings do run outranks one whose scope is
                // switched off: a gated rule states what the catalog could find,
                // not what this scan may clean, so it cannot take the location's
                // surviving row away from the rule that is actually running.
                u8::from(!item.gate.is_open()),
                retention_priority(item),
                category_index,
                item_index,
            ));
        }
    }
    candidates.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.3.cmp(&right.3))
            .then_with(|| left.1.cmp(&right.1))
    });

    let mut retained: Vec<(CleanupUnitIdentity, usize, usize)> = Vec::new();
    let mut suppressed_keys: Vec<CleanupUnitIdentity> = Vec::new();
    // (suppressed category, suppressed item, container category, container item,
    //  whether its bytes are already stated by a unit folded above it,
    //  whether this is an exact duplicate rather than containment,
    //  whether the two rules disagree about the operation that may be run)
    let mut suppressed: Vec<(usize, usize, usize, usize, bool, bool, bool)> = Vec::new();
    let mut authority_conflicts: Vec<(usize, usize, CleanupOverlap)> = Vec::new();
    // (container category, nested bytes) for a containment the resolution kept
    // as two rows: the nested bytes may already be inside the container's
    // measurement, so they are stated instead of added as if disjoint.
    let mut ambiguities: Vec<(usize, u64)> = Vec::new();
    for (_, key, _, _, category_index, item_index) in candidates {
        // Candidates are visited broadest-first, so a unit inside a unit that
        // was folded already is part of that unit's bytes: it is folded too,
        // but contributes them once.
        let already_counted = suppressed_keys.iter().any(|outer| key.is_within(outer));
        let candidate = &categories[category_index].items[item_index];
        let mut conflict_target = None;
        let mut ambiguity_target = false;
        let mut coverage_conflict_targets: Vec<(usize, usize)> = Vec::new();
        let suppression_target = retained
            .iter()
            .find_map(|(outer, outer_category, outer_item)| {
                let container = &categories[*outer_category].items[*outer_item];
                let relation = relationship(candidate, container).unwrap_or_else(|| {
                    if key == *outer {
                        UnitRelationship::Equivalent
                    } else if key.is_within(outer) {
                        UnitRelationship::Contained
                    } else {
                        UnitRelationship::Distinct
                    }
                });
                match relation {
                    UnitRelationship::Equivalent => {
                        Some((*outer_category, *outer_item, true, false))
                    }
                    UnitRelationship::EquivalentConflict => {
                        Some((*outer_category, *outer_item, true, true))
                    }
                    UnitRelationship::Contained
                        if completely_observed(container)
                            && (container.gate.is_open() || !candidate.gate.is_open()) =>
                    {
                        if containment_authority_is_compatible(container, candidate) {
                            Some((*outer_category, *outer_item, false, false))
                        } else {
                            conflict_target = Some((*outer_category, *outer_item));
                            ambiguity_target = true;
                            None
                        }
                    }
                    UnitRelationship::AuthorityConflict if completely_observed(container) => {
                        conflict_target = Some((*outer_category, *outer_item));
                        ambiguity_target = true;
                        None
                    }
                    UnitRelationship::Contained | UnitRelationship::AuthorityConflict => {
                        // Containment the resolution cannot fold: a partial walk
                        // cannot prove which entries it measured, and a gated
                        // container must not absorb a rule the settings do run.
                        // The nested unit keeps its own row. An active broader
                        // item is blocked from cleanup because otherwise both
                        // rows could claim the same reclaimable bytes.
                        ambiguity_target = true;
                        if container.gate.is_open() {
                            coverage_conflict_targets.push((*outer_category, *outer_item));
                        }
                        None
                    }
                    UnitRelationship::Distinct => None,
                }
            });
        match suppression_target {
            Some((container_category, container_item, is_duplicate, is_conflict)) => {
                suppressed.push((
                    category_index,
                    item_index,
                    container_category,
                    container_item,
                    already_counted,
                    is_duplicate,
                    is_conflict,
                ));
                if !already_counted {
                    suppressed_keys.push(key);
                }
            }
            None => {
                if ambiguity_target {
                    ambiguities.push((
                        category_index,
                        categories[category_index].items[item_index].observed_bytes(),
                    ));
                }
                for (outer_category, outer_item) in coverage_conflict_targets {
                    let mut conflict =
                        CleanupOverlap::of(&categories[category_index].items[item_index]);
                    conflict.authority_conflict = true;
                    authority_conflicts.push((outer_category, outer_item, conflict));
                }
                if let Some((outer_category, outer_item)) = conflict_target {
                    let mut conflict =
                        CleanupOverlap::of(&categories[category_index].items[item_index]);
                    conflict.authority_conflict = true;
                    authority_conflicts.push((outer_category, outer_item, conflict));
                }
                retained.push((key, category_index, item_index));
            }
        }
    }

    for (category_index, bytes) in ambiguities {
        let category = &mut categories[category_index];
        category.ambiguous_overlap_count += 1;
        category.ambiguous_overlap_bytes += bytes;
        report.ambiguous_count += 1;
        report.ambiguous_bytes += bytes;
        touched.insert(category_index);
    }

    for (category_index, item_index, conflict) in authority_conflicts {
        categories[category_index].items[item_index]
            .overlaps
            .push(conflict);
        touched.insert(category_index);
    }

    let mut removed: HashMap<usize, Vec<usize>> = HashMap::new();
    for (
        category_index,
        item_index,
        container_category,
        container_item,
        already_counted,
        is_duplicate,
        is_conflict,
    ) in suppressed
    {
        let bytes = categories[category_index].items[item_index].observed_bytes();
        let suppressed_risk = categories[category_index].items[item_index].risk;
        let suppressed_management = categories[category_index].items[item_index]
            .cache_metadata
            .management_mode;
        let suppressed_consequence = categories[category_index].items[item_index]
            .cache_metadata
            .consequence
            .clone();
        let mut entry = CleanupOverlap::of(&categories[category_index].items[item_index]);
        entry.authority_conflict = is_conflict;
        // Anything already folded into the suppressed unit travels with it, so
        // provenance survives more than one level of nesting.
        let carried = std::mem::take(&mut categories[category_index].items[item_index].overlaps);

        let container = &mut categories[container_category].items[container_item];
        let constrains = entry.gate.is_open();
        container.overlaps.push(entry);
        container.overlaps.extend(carried);
        // A rule whose scope is switched off is recorded as provenance and
        // nothing more: it is not authority over the location in this scan.
        if constrains {
            container.risk = container.risk.max(suppressed_risk);
            container.cache_metadata.management_mode = stricter_management_mode(
                container.cache_metadata.management_mode,
                suppressed_management,
            );
            if suppressed_risk == container.risk && !suppressed_consequence.is_empty() {
                container.cache_metadata.consequence = suppressed_consequence;
            }
        }
        if is_duplicate {
            categories[container_category].suppressed_duplicate_count += 1;
            categories[container_category].suppressed_duplicate_bytes += bytes;
        } else {
            categories[container_category].suppressed_overlap_count += 1;
            // A unit inside a unit that was folded already states the same bytes:
            // the count says how many units were folded, and the byte total counts
            // each location once so it can be reconciled against `total_bytes`.
            if !already_counted {
                categories[container_category].suppressed_overlap_bytes += bytes;
                report.suppressed_bytes += bytes;
            }
            report.suppressed_count += 1;
        }
        touched.insert(container_category);
        touched.insert(category_index);
        removed.entry(category_index).or_default().push(item_index);
    }

    for (category_index, mut items) in removed {
        items.sort_unstable_by(|left, right| right.cmp(left));
        items.dedup();
        for item_index in items {
            categories[category_index].items.remove(item_index);
        }
    }

    for category_index in touched {
        let category = &mut categories[category_index];
        for item in &mut category.items {
            if item.overlaps.is_empty() {
                continue;
            }
            // The verdict is re-derived from the item's facts, which now
            // include the rules that described the same location: the strictest
            // one applies, and a selection the new verdict no longer supports
            // is dropped here rather than at plan time.
            item.rederive_disposition();
        }
        category.recompute_accounting();
    }

    report
}

/// Lower values win when two rules name exactly the same unit. A rule that
/// owns a specialized operation (or owns no generic operation at all) must be
/// retained ahead of a filesystem rule, otherwise an accounting decision can
/// silently replace provider authority with path authority.
fn retention_priority(item: &ScanItem) -> u8 {
    if item.risk == crate::domain::RiskTier::Manual {
        return 0;
    }
    match item.unit.kind {
        CleanupUnitKind::ProviderAction | CleanupUnitKind::ContainerResource => 1,
        CleanupUnitKind::FixedPath
        | CleanupUnitKind::ChildNamespace
        | CleanupUnitKind::NamedSubtree => 2,
    }
}

fn completely_observed(item: &ScanItem) -> bool {
    item.quality == crate::domain::ObservationQuality::Fresh
        && item.skipped_entry_count == 0
        && item.incomplete_reason.is_none()
}

fn containment_authority_is_compatible(container: &ScanItem, nested: &ScanItem) -> bool {
    container.unit.kind.is_filesystem()
        && nested.unit.kind.is_filesystem()
        && container.risk != RiskTier::Manual
        && nested.risk != RiskTier::Manual
}

fn stricter_management_mode(
    left: CacheManagementMode,
    right: CacheManagementMode,
) -> CacheManagementMode {
    let rank = |mode| match mode {
        CacheManagementMode::Zenith => 0,
        CacheManagementMode::ToolManaged => 1,
        CacheManagementMode::Advisory => 2,
    };
    if rank(right) > rank(left) {
        right
    } else {
        left
    }
}

/// The category and item a retained identity still names, if it survived.
fn locate(
    categories: &[CategoryResult],
    retained: &CleanupUnitIdentity,
    identity: PathIdentity,
) -> Option<(usize, usize)> {
    categories
        .iter()
        .enumerate()
        .find_map(|(category_index, category)| {
            category
                .items
                .iter()
                .position(|item| item.unit_identity(identity) == *retained)
                .map(|item_index| (category_index, item_index))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::scan::{CleanupEligibility, CleanupUnit, FileSize, ScanItem};
    use crate::domain::{Category, ObservationQuality, RiskTier};

    const SENSITIVE: PathIdentity = PathIdentity::CaseSensitive;

    fn item(signature: &str, path: &str, bytes: u64, risk: RiskTier) -> ScanItem {
        ScanItem::mock(
            format!("{signature}:{path}"),
            signature,
            signature,
            Category::System,
            risk,
            path,
            FileSize::new(bytes, Some(bytes)),
            1,
        )
    }

    /// The same accounting the accumulator performs, so a fixture states the
    /// totals a scan would have produced before the resolution ran.
    fn category(items: Vec<ScanItem>) -> CategoryResult {
        let mut result = CategoryResult {
            category: Category::System,
            display_name: Category::System.display_name().to_string(),
            items,
            total_bytes: 0,
            cleanable_bytes: 0,
            safe_bytes: 0,
            rebuild_bytes: 0,
            manual_bytes: 0,
            quality: ObservationQuality::Fresh,
            skipped_entry_count: 0,
            incomplete_item_count: 0,
            eligibility: crate::domain::scan::EligibilitySummary::default(),
            suppressed_duplicate_count: 0,
            suppressed_duplicate_bytes: 0,
            suppressed_overlap_count: 0,
            suppressed_overlap_bytes: 0,
            ambiguous_overlap_count: 0,
            ambiguous_overlap_bytes: 0,
        };
        result.recompute_accounting();
        result
    }

    #[test]
    fn a_contained_unit_is_counted_by_its_container() {
        let parent = item(
            "system.parent",
            "/Users/tester/Library/Cache",
            1_000,
            RiskTier::Safe,
        );
        let child = item(
            "system.child",
            "/Users/tester/Library/Cache/nested",
            400,
            RiskTier::Safe,
        );
        let mut categories = vec![category(vec![parent, child])];
        assert_eq!(categories[0].total_bytes, 1_400);

        let report = resolve_unit_overlaps(&mut categories, &[], SENSITIVE);

        assert_eq!(report.suppressed_count, 1);
        assert_eq!(report.suppressed_bytes, 400);
        assert_eq!(categories[0].items.len(), 1);
        assert_eq!(
            categories[0].total_bytes, 1_000,
            "the bytes are counted once"
        );
        assert_eq!(categories[0].suppressed_overlap_count, 1);
        assert_eq!(categories[0].suppressed_overlap_bytes, 400);
        let overlaps = &categories[0].items[0].overlaps;
        assert_eq!(overlaps.len(), 1);
        assert_eq!(overlaps[0].signature_id, "system.child");
        assert_eq!(overlaps[0].unit_path, "/Users/tester/Library/Cache/nested");
        assert_eq!(overlaps[0].observed_bytes, 400);
    }

    /// Which unit keeps the bytes cannot depend on which rule the scan happened
    /// to visit first.
    #[test]
    fn the_broader_unit_keeps_the_bytes_whichever_is_discovered_first() {
        let parent = item(
            "system.parent",
            "/Users/tester/Library/Cache",
            1_000,
            RiskTier::Safe,
        );
        let child = item(
            "system.child",
            "/Users/tester/Library/Cache/nested",
            400,
            RiskTier::Safe,
        );

        let mut forward = vec![category(vec![parent.clone(), child.clone()])];
        let mut backward = vec![category(vec![child, parent.clone()])];
        let forward_report = resolve_unit_overlaps(&mut forward, &[], SENSITIVE);
        let backward_report = resolve_unit_overlaps(&mut backward, &[], SENSITIVE);

        assert_eq!(forward_report, backward_report);
        assert_eq!(forward[0].total_bytes, backward[0].total_bytes);
        assert_eq!(forward[0].items.len(), 1);
        assert_eq!(backward[0].items.len(), 1);
        assert_eq!(forward[0].items[0].path, parent.path);
        assert_eq!(backward[0].items[0].path, parent.path);
        assert_eq!(forward[0].items[0].overlaps, backward[0].items[0].overlaps);
    }

    /// A second rule can never make a location more deletable than the first:
    /// the surviving unit states the stricter verdict, and a selection the new
    /// verdict no longer supports is dropped.
    #[test]
    fn the_strictest_rule_decides_the_surviving_unit() {
        let parent = item(
            "system.parent",
            "/Users/tester/Library/Cache",
            1_000,
            RiskTier::Safe,
        );
        assert_eq!(
            parent.disposition.eligibility,
            CleanupEligibility::AutoCleanable
        );
        assert!(parent.is_selected);

        let child = item(
            "system.child",
            "/Users/tester/Library/Cache/nested",
            400,
            RiskTier::Rebuild,
        );
        let mut categories = vec![category(vec![parent, child])];
        resolve_unit_overlaps(&mut categories, &[], SENSITIVE);

        let surviving = &categories[0].items[0];
        assert_eq!(
            surviving.disposition.eligibility,
            CleanupEligibility::Reviewable
        );
        assert!(
            !surviving.is_selected,
            "a unit that needs review is not pre-selected by another rule's verdict"
        );
        assert_eq!(surviving.cleanable_bytes(), 1_000);
        assert!(surviving
            .disposition
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("system.child")));
        assert_eq!(categories[0].cleanable_bytes, 1_000);
        assert_eq!(
            surviving.risk,
            RiskTier::Rebuild,
            "the retained item exposes the effective risk, not only the folded eligibility"
        );
        assert_eq!(categories[0].safe_bytes, 0);
        assert_eq!(categories[0].rebuild_bytes, 1_000);
    }

    #[test]
    fn a_provider_authority_survives_an_exact_filesystem_overlap_in_either_order() {
        let filesystem = item(
            "system.filesystem",
            "/Users/tester/Library/Cache",
            1_000,
            RiskTier::Safe,
        );
        let mut provider = item(
            "system.provider",
            "/Users/tester/Library/Cache",
            1_000,
            RiskTier::Rebuild,
        );
        provider.unit.kind = crate::domain::scan::CleanupUnitKind::ProviderAction;

        for items in [
            vec![filesystem.clone(), provider.clone()],
            vec![provider.clone(), filesystem.clone()],
        ] {
            let mut categories = vec![category(items)];
            resolve_unit_overlaps(&mut categories, &[], SENSITIVE);

            assert_eq!(categories[0].items.len(), 1);
            let surviving = &categories[0].items[0];
            assert_eq!(surviving.signature_id, "system.provider");
            assert_eq!(
                surviving.unit.kind,
                crate::domain::scan::CleanupUnitKind::ProviderAction
            );
            assert_eq!(surviving.overlaps[0].signature_id, "system.filesystem");
        }
    }

    #[test]
    fn an_incomplete_container_does_not_suppress_a_complete_child_observation() {
        let mut parent = item(
            "system.parent",
            "/Users/tester/Library/Cache",
            600,
            RiskTier::Safe,
        );
        parent.quality = ObservationQuality::Partial;
        parent.skipped_entry_count = 1;
        parent.incomplete_reason = Some("one subtree was not observed".into());
        parent.rederive_disposition();
        let child = item(
            "system.child",
            "/Users/tester/Library/Cache/nested",
            400,
            RiskTier::Safe,
        );
        let mut categories = vec![category(vec![parent, child])];

        let report = resolve_unit_overlaps(&mut categories, &[], SENSITIVE);

        assert_eq!(report.suppressed_count, 0, "nothing was folded");
        assert_eq!(
            report.ambiguous_count, 1,
            "the containment is stated, not folded"
        );
        assert_eq!(
            report.ambiguous_bytes, 400,
            "the nested bytes may already be inside the container's measurement"
        );
        assert_eq!(categories[0].items.len(), 2);
        assert_eq!(categories[0].total_bytes, 1_000);
        assert_eq!(categories[0].ambiguous_overlap_count, 1);
        assert_eq!(categories[0].ambiguous_overlap_bytes, 400);

        let parent = categories[0]
            .items
            .iter()
            .find(|item| item.path == "/Users/tester/Library/Cache")
            .expect("the incomplete container remains visible");
        assert_eq!(
            parent.disposition.eligibility,
            CleanupEligibility::Blocked,
            "an unresolved observed overlap cannot leave the broader target independently cleanable"
        );
        assert_eq!(
            categories[0].cleanable_bytes, 400,
            "the ambiguous parent and child cannot both contribute reclaimable bytes"
        );
    }

    #[test]
    fn a_gated_nested_rule_does_not_block_an_active_incomplete_container() {
        let mut parent = item(
            "system.parent",
            "/Users/tester/Library/Cache",
            600,
            RiskTier::Safe,
        );
        parent.quality = ObservationQuality::Partial;
        parent.skipped_entry_count = 1;
        parent.incomplete_reason = Some("one subtree was not observed".into());
        parent.rederive_disposition();

        let mut child = item(
            "system.child",
            "/Users/tester/Library/Cache/nested",
            400,
            RiskTier::Safe,
        );
        child.gate = crate::domain::scan::EligibilityGate::IntensiveDisabled;
        child.rederive_disposition();

        let mut categories = vec![category(vec![parent, child])];
        resolve_unit_overlaps(&mut categories, &[], SENSITIVE);

        let parent = categories[0]
            .items
            .iter()
            .find(|item| item.path == "/Users/tester/Library/Cache")
            .expect("the parent remains visible");
        assert_eq!(
            parent.disposition.eligibility,
            CleanupEligibility::Reviewable,
            "a gated nested observation is provenance/accounting only and cannot withhold the active rule"
        );
    }

    #[test]
    fn a_filesystem_container_does_not_absorb_a_nested_provider_authority() {
        let filesystem = item(
            "system.filesystem",
            "/Users/tester/Library/Cache",
            1_000,
            RiskTier::Safe,
        );
        let mut provider = item(
            "system.provider",
            "/Users/tester/Library/Cache/provider",
            400,
            RiskTier::Rebuild,
        );
        provider.unit.kind = CleanupUnitKind::ProviderAction;
        let mut categories = vec![category(vec![filesystem, provider])];

        let report = resolve_unit_overlaps(&mut categories, &[], SENSITIVE);

        assert_eq!(report.suppressed_count, 0);
        assert_eq!(
            report.ambiguous_bytes, 400,
            "the pair is stated as possibly shared bytes"
        );
        assert_eq!(categories[0].items.len(), 2);
        assert!(categories[0]
            .items
            .iter()
            .any(|item| item.unit.kind == CleanupUnitKind::ProviderAction));
    }

    /// A rule that refuses the location entirely carries to the container, so a
    /// broader rule cannot delete a store another rule protects.
    #[test]
    fn a_blocked_rule_withholds_the_container_from_cleanup() {
        let parent = item(
            "system.parent",
            "/Users/tester/Library/Cache",
            1_000,
            RiskTier::Safe,
        );
        let child = item(
            "system.child",
            "/Users/tester/Library/Cache/nested",
            400,
            RiskTier::Manual,
        );
        assert_eq!(
            child.disposition.eligibility,
            CleanupEligibility::Blocked,
            "a manual rule is not cleanable generically"
        );

        let mut categories = vec![category(vec![parent, child])];
        resolve_unit_overlaps(&mut categories, &[], SENSITIVE);

        let surviving = &categories[0].items[0];
        assert_eq!(
            surviving.disposition.eligibility,
            CleanupEligibility::Blocked
        );
        assert_eq!(surviving.cleanable_bytes(), 0);
        assert!(!surviving.is_selected);
        assert_eq!(categories[0].cleanable_bytes, 0);
        assert_eq!(categories[0].items.len(), 2);
        assert_eq!(categories[0].total_bytes, 1_400);
        assert!(surviving
            .overlaps
            .iter()
            .any(|overlap| overlap.authority_conflict));
    }

    /// A duplicate that was dropped where it was found still names the rule that
    /// did not count the bytes, and its verdict still applies.
    #[test]
    fn a_duplicate_discovery_becomes_provenance() {
        let retained = item(
            "system.first",
            "/Users/tester/Library/Cache",
            1_000,
            RiskTier::Safe,
        );
        let duplicate = item(
            "system.second",
            "/Users/tester/Library/Cache",
            1_000,
            RiskTier::Rebuild,
        );
        let mut categories = vec![category(vec![retained.clone()])];
        let overlapped = vec![OverlappedDiscovery {
            retained: retained.unit_identity(SENSITIVE),
            overlap: CleanupOverlap::of(&duplicate),
        }];

        let report = resolve_unit_overlaps(&mut categories, &overlapped, SENSITIVE);

        assert_eq!(
            report,
            OverlapReport::default(),
            "the discovery was already counted"
        );
        assert_eq!(categories[0].total_bytes, 1_000);
        let surviving = &categories[0].items[0];
        assert_eq!(surviving.overlaps.len(), 1);
        assert_eq!(surviving.overlaps[0].signature_id, "system.second");
        assert_eq!(surviving.risk, RiskTier::Rebuild);
        assert_eq!(
            surviving.disposition.eligibility,
            CleanupEligibility::Reviewable
        );
    }

    /// Provenance is not lost when a unit is itself contained by a broader one.
    #[test]
    fn provenance_survives_more_than_one_level() {
        let root = item("system.root", "/Users/tester/Cache", 2_000, RiskTier::Safe);
        let middle = item(
            "system.middle",
            "/Users/tester/Cache/middle",
            800,
            RiskTier::Safe,
        );
        let leaf = item(
            "system.leaf",
            "/Users/tester/Cache/middle/leaf",
            200,
            RiskTier::Rebuild,
        );

        let mut categories = vec![category(vec![leaf.clone(), middle, root])];
        let report = resolve_unit_overlaps(&mut categories, &[], SENSITIVE);

        assert_eq!(categories[0].items.len(), 1);
        assert_eq!(categories[0].total_bytes, 2_000);
        assert_eq!(
            report.suppressed_count, 2,
            "both rules that lost their own item are counted"
        );
        assert_eq!(
            report.suppressed_bytes, 800,
            "the bytes are stated once, by the outermost unit that was folded"
        );
        assert_eq!(categories[0].suppressed_overlap_bytes, 800);
        let surviving = &categories[0].items[0];
        let signatures: Vec<&str> = surviving
            .overlaps
            .iter()
            .map(|overlap| overlap.signature_id.as_str())
            .collect();
        assert!(signatures.contains(&"system.middle"));
        assert!(
            signatures.contains(&"system.leaf"),
            "the rule that protected the leaf still reaches the unit that kept the bytes: {signatures:?}"
        );
        assert_eq!(
            surviving.disposition.eligibility,
            CleanupEligibility::Reviewable
        );
    }

    /// A unit that names no path is not a unit: it neither suppresses another
    /// nor is suppressed by one.
    #[test]
    fn an_undeclared_unit_is_never_resolved() {
        let mut undeclared = item(
            "system.undeclared",
            "/Users/tester/Cache",
            100,
            RiskTier::Safe,
        );
        undeclared.unit = CleanupUnit::default();
        let declared = item(
            "system.declared",
            "/Users/tester/Cache",
            1_000,
            RiskTier::Safe,
        );

        let mut categories = vec![category(vec![undeclared, declared])];
        let report = resolve_unit_overlaps(&mut categories, &[], SENSITIVE);

        assert_eq!(report, OverlapReport::default());
        assert_eq!(categories[0].items.len(), 2);
        assert_eq!(categories[0].total_bytes, 1_100);
    }

    /// Case folding is the stated filesystem's property, not the platform's.
    #[test]
    fn case_variant_spellings_are_one_unit_on_a_folding_filesystem() {
        let stored = item(
            "system.stored",
            r"C:\Users\tester\Cache",
            1_000,
            RiskTier::Safe,
        );
        let alias = item(
            "system.alias",
            r"c:\users\tester\cache",
            1_000,
            RiskTier::Safe,
        );

        let mut folding = vec![category(vec![stored.clone(), alias.clone()])];
        let report = resolve_unit_overlaps(&mut folding, &[], PathIdentity::CaseInsensitive);
        assert_eq!(report, OverlapReport::default());
        assert_eq!(folding[0].items.len(), 1);
        assert_eq!(folding[0].total_bytes, 1_000);
        assert_eq!(folding[0].suppressed_duplicate_count, 1);

        let mut sensitive = vec![category(vec![stored, alias])];
        let report = resolve_unit_overlaps(&mut sensitive, &[], PathIdentity::CaseSensitive);
        assert_eq!(report, OverlapReport::default());
        assert_eq!(sensitive[0].items.len(), 2);
        assert_eq!(sensitive[0].total_bytes, 2_000);
    }

    #[test]
    fn stable_identity_can_keep_case_variants_distinct_on_a_folding_platform_default() {
        let upper = item("system.upper", r"C:\work\Cache", 1_000, RiskTier::Safe);
        let lower = item("system.lower", r"C:\work\cache", 2_000, RiskTier::Safe);
        let mut categories = vec![category(vec![upper, lower])];

        let report = resolve_unit_overlaps_with(
            &mut categories,
            &[],
            PathIdentity::CaseInsensitive,
            |_, _| Some(UnitRelationship::Distinct),
        );

        assert_eq!(report, OverlapReport::default());
        assert_eq!(categories[0].items.len(), 2);
        assert_eq!(categories[0].total_bytes, 3_000);
    }

    #[test]
    fn stable_ancestor_identity_can_find_containment_across_case_variants() {
        let parent = item(
            "system.parent",
            "/Volumes/Data/Cache",
            1_000,
            RiskTier::Safe,
        );
        let child = item(
            "system.child",
            "/volumes/data/cache/nested",
            400,
            RiskTier::Safe,
        );
        let mut categories = vec![category(vec![parent, child])];

        let report = resolve_unit_overlaps_with(
            &mut categories,
            &[],
            PathIdentity::CaseSensitive,
            |candidate, outer| {
                (candidate.signature_id == "system.child" && outer.signature_id == "system.parent")
                    .then_some(UnitRelationship::Contained)
            },
        );

        assert_eq!(report.suppressed_count, 1);
        assert_eq!(categories[0].items.len(), 1);
        assert_eq!(categories[0].total_bytes, 1_000);
    }
}
