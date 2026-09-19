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

use super::{CategoryResult, CleanupEligibility, CleanupUnitIdentity, PathIdentity, ScanItem};
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
            observed_bytes: item.observed_bytes(),
        }
    }
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
/// The fold assumes a container's observation covers its own tree. A walk that
/// deliberately skipped entries — a blacklisted child, a signature exclusion, a
/// bundle, a depth cutoff — reports that on the item (`skipped_entry_count`,
/// `quality`), and a subtree it did not measure is a subtree whose bytes this
/// pass removes from the totals while the container's deletion also leaves them
/// in place. The bytes are not lost from the answer: the folded unit keeps its
/// own measurement in the container's provenance, so the location and its size
/// stay readable.
pub fn resolve_unit_overlaps(
    categories: &mut [CategoryResult],
    overlapped: &[OverlappedDiscovery],
    identity: PathIdentity,
) -> OverlapReport {
    let mut report = OverlapReport::default();
    let mut touched: HashSet<usize> = HashSet::new();

    for discovery in overlapped {
        if let Some((category, item)) = locate(categories, &discovery.retained, identity) {
            categories[category].items[item]
                .overlaps
                .push(discovery.overlap.clone());
            touched.insert(category);
        }
    }

    let mut candidates: Vec<(usize, CleanupUnitIdentity, usize, usize)> = Vec::new();
    for (category_index, category) in categories.iter().enumerate() {
        for (item_index, item) in category.items.iter().enumerate() {
            if !item.unit.is_declared() {
                continue;
            }
            let key = item.unit_identity(identity);
            candidates.push((key.components().count(), key, category_index, item_index));
        }
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

    let mut retained: Vec<(CleanupUnitIdentity, usize, usize)> = Vec::new();
    let mut suppressed_keys: Vec<CleanupUnitIdentity> = Vec::new();
    // (suppressed category, suppressed item, container category, container item,
    //  whether its bytes are already stated by a unit folded above it)
    let mut suppressed: Vec<(usize, usize, usize, usize, bool)> = Vec::new();
    for (_, key, category_index, item_index) in candidates {
        // Candidates are visited broadest-first, so a unit inside a unit that
        // was folded already is part of that unit's bytes: it is folded too,
        // but contributes them once.
        let already_counted = suppressed_keys.iter().any(|outer| key.is_within(outer));
        // An identical key was resolved where the unit was discovered. What
        // this pass adds is containment: a unit inside a broader one.
        match retained.iter().find(|(outer, _, _)| key.is_within(outer)) {
            Some((_, container_category, container_item)) => {
                suppressed.push((
                    category_index,
                    item_index,
                    *container_category,
                    *container_item,
                    already_counted,
                ));
                if !already_counted {
                    suppressed_keys.push(key);
                }
            }
            None => retained.push((key, category_index, item_index)),
        }
    }

    let mut removed: HashMap<usize, Vec<usize>> = HashMap::new();
    for (category_index, item_index, container_category, container_item, already_counted) in
        suppressed
    {
        let bytes = categories[category_index].items[item_index].observed_bytes();
        let entry = CleanupOverlap::of(&categories[category_index].items[item_index]);
        // Anything already folded into the suppressed unit travels with it, so
        // provenance survives more than one level of nesting.
        let carried = std::mem::take(&mut categories[category_index].items[item_index].overlaps);

        let container = &mut categories[container_category].items[container_item];
        container.overlaps.push(entry);
        container.overlaps.extend(carried);
        categories[container_category].suppressed_overlap_count += 1;
        // A unit inside a unit that was folded already states the same bytes:
        // the count says how many units were folded, and the byte total counts
        // each location once so it can be reconciled against `total_bytes`.
        if !already_counted {
            categories[container_category].suppressed_overlap_bytes += bytes;
            report.suppressed_bytes += bytes;
        }
        report.suppressed_count += 1;
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
        assert_eq!(categories[0].total_bytes, 1_000);
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
            RiskTier::Manual,
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
            CleanupEligibility::Blocked
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
        assert_eq!(report.suppressed_count, 1);
        assert_eq!(folding[0].items.len(), 1);
        assert_eq!(folding[0].total_bytes, 1_000);

        let mut sensitive = vec![category(vec![stored, alias])];
        let report = resolve_unit_overlaps(&mut sensitive, &[], PathIdentity::CaseSensitive);
        assert_eq!(report, OverlapReport::default());
        assert_eq!(sensitive[0].items.len(), 2);
        assert_eq!(sensitive[0].total_bytes, 2_000);
    }
}
