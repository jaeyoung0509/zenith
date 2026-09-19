use crate::cache_providers::CacheProviderRegistry;
use crate::docker::DockerAdapter;
use crate::execution_budget::shared_scan_pool;
use crate::models::{
    resolve_unit_overlaps, Category, CategoryResult, CleanupOverlap, CleanupUnit,
    CleanupUnitIdentity, EligibilitySummary, FileIdentity, ObservationQuality, OverlappedDiscovery,
    PathIdentity, RiskTier, ScanEvent, ScanItem, ScanResult,
};
use crate::orbstack::OrbStackAdapter;
use crate::scanner::DirectoryScanner;
use crate::signatures::SignatureRegistry;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use uuid::Uuid;
use zenith_platform::PlatformEnvironment;

pub struct ScanEngine;

/// Resolve the actual directory entry for a spelling that reached `entity`.
/// On a case-folding volume, `pip` can open an entry stored as `Pip`; on a
/// case-sensitive volume both spellings may be separate hardlinks to one inode.
fn actual_entry_name(path: &Path, entity: FileIdentity) -> Option<OsString> {
    let requested = path.file_name()?;
    let mut folded_match = None;
    for entry in std::fs::read_dir(path.parent()?).ok()?.flatten() {
        let name = entry.file_name();
        if name != requested
            && !name
                .to_string_lossy()
                .eq_ignore_ascii_case(&requested.to_string_lossy())
        {
            continue;
        }
        let matches_entity = crate::safety::ToctouGuard::capture(&entry.path())
            .is_some_and(|identity| identity.entity() == entity);
        if !matches_entity {
            continue;
        }
        if name == requested {
            return Some(name);
        }
        if folded_match.is_some() {
            return None;
        }
        folded_match = Some(name);
    }
    folded_match
}

fn same_directory_entry(first: &Path, second: &Path, entity: FileIdentity) -> bool {
    let parents_match = first
        .parent()
        .and_then(crate::safety::ToctouGuard::capture)
        .zip(
            second
                .parent()
                .and_then(crate::safety::ToctouGuard::capture),
        )
        .is_some_and(|(left, right)| left.entity().same_entity(right.entity()));
    parents_match
        && actual_entry_name(first, entity)
            .zip(actual_entry_name(second, entity))
            .is_some_and(|(left, right)| left == right)
}

fn aggregate_quality(
    qualities: impl IntoIterator<Item = ObservationQuality>,
) -> ObservationQuality {
    let mut saw_any = false;
    let mut all_fresh = true;
    let mut all_unavailable = true;
    for quality in qualities {
        saw_any = true;
        all_fresh &= quality == ObservationQuality::Fresh;
        all_unavailable &= quality == ObservationQuality::Unavailable;
    }
    if !saw_any || all_fresh {
        ObservationQuality::Fresh
    } else if all_unavailable {
        ObservationQuality::Unavailable
    } else {
        ObservationQuality::Partial
    }
}

/// The retained items and byte populations of one category.
///
/// `total_bytes` is the observed footprint, including blocked/advisory rows;
/// `cleanable_bytes` and the Safe/Rebuild buckets include only eligible bytes.
struct CategoryAccumulator {
    items: Vec<ScanItem>,
    total_bytes: u64,
    cleanable_bytes: u64,
    safe_bytes: u64,
    rebuild_bytes: u64,
    manual_bytes: u64,
    eligibility: EligibilitySummary,
    suppressed_duplicate_count: u64,
    suppressed_duplicate_bytes: u64,
}

impl Default for CategoryAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl CategoryAccumulator {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            total_bytes: 0,
            cleanable_bytes: 0,
            safe_bytes: 0,
            rebuild_bytes: 0,
            manual_bytes: 0,
            eligibility: EligibilitySummary::default(),
            suppressed_duplicate_count: 0,
            suppressed_duplicate_bytes: 0,
        }
    }

    /// Accounts one scanned item, or drops it when it is not worth retaining.
    ///
    /// Returns the retained item so the caller can stream it to the frontend.
    ///
    /// Two discovery rules can name the same location, and a scan that counted
    /// it twice would report a total no user could reconcile with their disk.
    /// The first unit wins — signatures are visited most-specific-first — and
    /// the duplicate's bytes are recorded as suppressed rather than dropped.
    /// The rule that lost is carried as provenance on the unit that counted the
    /// bytes: its verdict still decides the surviving unit's eligibility, and
    /// the interface can name the rule it came from.
    fn push(
        &mut self,
        item: ScanItem,
        identity: PathIdentity,
        seen_units: &mut HashSet<CleanupUnitIdentity>,
        seen_entities: &mut HashMap<FileIdentity, Vec<PathBuf>>,
        overlapped: &mut Vec<OverlappedDiscovery>,
    ) -> Option<&ScanItem> {
        let item = item.with_derived_disposition();
        let bytes = item.cleanable_bytes();
        let observed = item.observed_bytes();
        // An item that is absent, or a complete observation of an empty path,
        // carries nothing a user could act on.
        let is_empty_fresh = item.quality == ObservationQuality::Fresh && observed == 0;
        if !item.exists || is_empty_fresh {
            return None;
        }

        // Text is a fallback for paths without a stable OS identity, and the
        // stated filesystem decides whether that text folds case. Path syntax
        // alone says nothing about the mounted volume's case behavior.
        let unit = item.unit_identity(identity);
        let entity = item
            .unit
            .kind
            .is_filesystem()
            .then(|| crate::safety::ToctouGuard::capture(Path::new(&item.unit.path)))
            .flatten()
            .map(|identity| identity.entity())
            .filter(|entity| !entity.is_unknown());
        let duplicate_of = if seen_units.contains(&unit) {
            Some(unit.clone())
        } else {
            // The same object under another spelling: a hardlinked alias shares
            // the inode, and a case-folding volume answers to either spelling of
            // a name.
            entity.and_then(|entity| {
                seen_entities
                    .get(&entity)?
                    .iter()
                    .find(|seen| same_directory_entry(seen, Path::new(&item.unit.path), entity))
                    .map(|path| {
                        CleanupUnit::fixed_path(path.to_string_lossy().into_owned())
                            .identity(identity)
                    })
            })
        };
        if let Some(retained) = duplicate_of {
            self.suppressed_duplicate_count += 1;
            self.suppressed_duplicate_bytes += observed;
            overlapped.push(OverlappedDiscovery {
                retained,
                overlap: CleanupOverlap::of(&item),
            });
            crate::diagnostics::log_error(
                "scanner",
                &format!(
                    "Suppressed duplicate discovery of {} (already counted by another signature)",
                    item.path
                ),
            );
            return None;
        }
        seen_units.insert(unit);
        if let Some(entity) = entity {
            seen_entities
                .entry(entity)
                .or_default()
                .push(PathBuf::from(&item.unit.path));
        }

        self.total_bytes += observed;
        self.cleanable_bytes += bytes;
        self.eligibility.add(&item);
        if item.disposition.eligibility.is_cleanable() {
            match item.risk {
                RiskTier::Safe => self.safe_bytes += bytes,
                RiskTier::Rebuild => self.rebuild_bytes += bytes,
                RiskTier::Manual => {}
            }
        } else if item.risk == RiskTier::Manual {
            self.manual_bytes += observed;
        }

        if !item.allows_cleanup() {
            // An item whose observation cannot support a cleanup is reported
            // once, through its own `quality` and through the scan's durable
            // `incomplete_reasons`. It never becomes a second, destructive
            // error surface in the middle of a scan that otherwise succeeded.
            crate::diagnostics::log_error(
                "scanner",
                &format!(
                    "Could not fully inspect {}: {}",
                    item.name,
                    item.disposition
                        .reason
                        .as_deref()
                        .or(item.incomplete_reason.as_deref())
                        .unwrap_or("Inaccessible")
                ),
            );
        }

        self.items.push(item);
        self.items.last()
    }

    fn finalize(
        mut self,
        category: Category,
        forced_quality: Option<ObservationQuality>,
    ) -> CategoryResult {
        self.items.sort_by(|left, right| {
            right
                .size
                .observed_bytes()
                .cmp(&left.size.observed_bytes())
                .then_with(|| left.name.cmp(&right.name))
        });

        let quality = forced_quality
            .unwrap_or_else(|| aggregate_quality(self.items.iter().map(|item| item.quality)));
        let skipped_entry_count = self.items.iter().map(|item| item.skipped_entry_count).sum();
        let incomplete_item_count = self
            .items
            .iter()
            .filter(|item| item.quality != ObservationQuality::Fresh)
            .count() as u64;

        CategoryResult {
            category,
            display_name: category.display_name().to_string(),
            items: self.items,
            total_bytes: self.total_bytes,
            cleanable_bytes: self.cleanable_bytes,
            safe_bytes: self.safe_bytes,
            rebuild_bytes: self.rebuild_bytes,
            manual_bytes: self.manual_bytes,
            quality,
            skipped_entry_count,
            incomplete_item_count,
            eligibility: self.eligibility,
            suppressed_duplicate_count: self.suppressed_duplicate_count,
            suppressed_duplicate_bytes: self.suppressed_duplicate_bytes,
            // Filled by `resolve_unit_overlaps`, which is the only place that
            // can see the whole result: a unit is overlapped by one in another
            // category as readily as by one beside it.
            suppressed_overlap_count: 0,
            suppressed_overlap_bytes: 0,
        }
    }
}

impl ScanEngine {
    /// Executes a full or filtered scan across all categories, emitting streaming events.
    ///
    /// The environment is threaded explicitly: signature paths, size
    /// exclusions, and external provider caches all resolve through it, so a
    /// scan cannot silently fall back to the running process's own profile.
    pub fn scan<F>(
        registry: &SignatureRegistry,
        categories_filter: Option<&[Category]>,
        excluded_signatures: &[String],
        intensive_cleanup: bool,
        environment: &PlatformEnvironment,
        cancellation: &dyn crate::models::CancellationProbe,
        mut on_event: F,
    ) -> ScanResult
    where
        F: FnMut(ScanEvent),
    {
        let scan_id = Uuid::new_v4().to_string();
        let started_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        on_event(ScanEvent::Started {
            scan_id: scan_id.clone(),
        });

        let target_categories = categories_filter.unwrap_or(&[
            Category::Ai,
            Category::Developer,
            Category::Container,
            Category::System,
        ]);

        let mut category_results = Vec::new();
        let mut total_bytes = 0u64;
        let mut cleanable_bytes = 0u64;
        let mut safe_bytes = 0u64;
        let mut rebuild_bytes = 0u64;
        let mut manual_bytes = 0u64;
        let mut skipped_entry_count = 0u64;
        let mut incomplete_item_count = 0u64;
        let mut was_cancelled = false;
        let mut eligibility = EligibilitySummary::default();
        let mut suppressed_duplicate_count = 0u64;
        let mut suppressed_duplicate_bytes = 0u64;
        // The scan states whether the filesystem it looked at folds case: a
        // Windows volume folds by default and a POSIX one does not. Volume
        // settings can differ from the platform default, so a path that exists
        // is still decided by its stable filesystem identity; the stated fact
        // is what text-only comparison uses.
        let identity = if environment.flavor().is_windows() {
            PathIdentity::CaseInsensitive
        } else {
            PathIdentity::CaseSensitive
        };
        // Discoveries that were dropped where they were found because the unit
        // that counted them already existed. The pass after the scan folds them
        // into that unit as provenance.
        let mut overlapped: Vec<OverlappedDiscovery> = Vec::new();
        // One identity set for the whole scan: a unit two signatures both found
        // is counted once, whichever category it was found under.
        let mut seen_units: HashSet<CleanupUnitIdentity> = HashSet::new();
        // One process-table pass for the whole scan: the scan asks which
        // application bundles are running, and every signature sees the same
        // answer.
        let running_apps = crate::applications::RunningApplications::probe();
        let mut seen_entities: HashMap<FileIdentity, Vec<PathBuf>> = HashMap::new();
        // Reuse the explicitly bounded shared scan pool (see
        // `execution_budget`); never expand pools per request.
        let directory_pool = shared_scan_pool();

        for &category in target_categories {
            if cancellation.is_cancelled() {
                was_cancelled = true;
                break;
            }

            on_event(ScanEvent::CategoryStarted { category });

            let mut accumulator = CategoryAccumulator::new();

            // 1. Scan filesystem signatures for this category
            // Discovery and eligibility are decided separately: the registry
            // returns what this scope looks at, and the gate below states what
            // the scope permits for each unit that is found.
            let signatures = registry.by_category_for_mode(category, intensive_cleanup);
            for sig in signatures {
                if cancellation.is_cancelled() {
                    was_cancelled = true;
                    break;
                }
                if excluded_signatures.iter().any(|id| id == &sig.id) {
                    continue;
                }
                let gate = sig.eligibility_gate(intensive_cleanup);
                let items = DirectoryScanner::scan_signature_with_pool(
                    sig,
                    directory_pool,
                    environment,
                    cancellation,
                    gate,
                    &running_apps,
                );
                for item in items {
                    if let Some(retained) = accumulator.push(
                        item,
                        identity,
                        &mut seen_units,
                        &mut seen_entities,
                        &mut overlapped,
                    ) {
                        on_event(ScanEvent::ItemFound {
                            item: retained.clone(),
                        });
                    }
                }
            }

            // 2. Typed container adapters can report cleanable or observation-only storage.
            if !was_cancelled && category == Category::Developer {
                for item in CacheProviderRegistry::scan_items(registry, environment) {
                    if cancellation.is_cancelled() {
                        was_cancelled = true;
                        break;
                    }
                    if let Some(retained) = accumulator.push(
                        item,
                        identity,
                        &mut seen_units,
                        &mut seen_entities,
                        &mut overlapped,
                    ) {
                        on_event(ScanEvent::ItemFound {
                            item: retained.clone(),
                        });
                    }
                }
            }

            // 3. Typed container adapters can report cleanable or observation-only storage.
            if !was_cancelled && category == Category::Container {
                let adapter_items = DockerAdapter::scan_items(environment)
                    .into_iter()
                    .chain(OrbStackAdapter::scan_items(environment));
                for item in adapter_items {
                    if cancellation.is_cancelled() {
                        was_cancelled = true;
                        break;
                    }
                    if let Some(retained) = accumulator.push(
                        item,
                        identity,
                        &mut seen_units,
                        &mut seen_entities,
                        &mut overlapped,
                    ) {
                        on_event(ScanEvent::ItemFound {
                            item: retained.clone(),
                        });
                    }
                }
            }

            let category_result = accumulator.finalize(
                category,
                was_cancelled.then_some(ObservationQuality::Partial),
            );
            category_results.push(category_result);

            if was_cancelled {
                break;
            }
        }

        // Counting a location once is a decision about the whole result: a unit
        // inside a broader one is that unit's provenance, not a second total,
        // and the broader unit can be in another category. The categories are
        // restated before any number derived from them is reported.
        let overlap = resolve_unit_overlaps(&mut category_results, &overlapped, identity);
        for category_result in &category_results {
            total_bytes += category_result.total_bytes;
            cleanable_bytes += category_result.cleanable_bytes;
            safe_bytes += category_result.safe_bytes;
            rebuild_bytes += category_result.rebuild_bytes;
            manual_bytes += category_result.manual_bytes;
            skipped_entry_count += category_result.skipped_entry_count;
            incomplete_item_count += category_result.incomplete_item_count;
            eligibility.merge(&category_result.eligibility);
            suppressed_duplicate_count += category_result.suppressed_duplicate_count;
            suppressed_duplicate_bytes += category_result.suppressed_duplicate_bytes;
        }
        let suppressed_overlap_count = overlap.suppressed_count;
        let suppressed_overlap_bytes = overlap.suppressed_bytes;

        // The per-category events carry the totals the resolution settled: a
        // category that folded an overlapping unit away reports what it now
        // accounts for, not what it held mid-scan.
        for (index, category_result) in category_results.iter().enumerate() {
            if was_cancelled && index + 1 == category_results.len() {
                break;
            }
            on_event(ScanEvent::CategoryFinished {
                category: category_result.category,
                bytes: category_result.total_bytes,
                item_count: category_result.items.len(),
            });
        }

        let finished_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut incomplete_reasons = Vec::new();
        for cat in &category_results {
            for item in &cat.items {
                if let Some(reason) = &item.incomplete_reason {
                    if !incomplete_reasons.contains(reason) {
                        incomplete_reasons.push(reason.clone());
                    }
                }
            }
        }
        if was_cancelled {
            incomplete_reasons.push("Scan was cancelled before completion".to_string());
        }
        let scan_quality = if was_cancelled {
            ObservationQuality::Partial
        } else {
            aggregate_quality(category_results.iter().map(|cat| cat.quality))
        };

        let result = ScanResult {
            scan_id,
            valid_for_seconds: ScanResult::VALID_FOR_SECONDS,
            started_at,
            finished_at,
            categories: category_results,
            total_bytes,
            cleanable_bytes,
            safe_bytes,
            rebuild_bytes,
            manual_bytes,
            quality: scan_quality,
            incomplete_reasons,
            skipped_entry_count,
            incomplete_item_count,
            eligibility,
            suppressed_duplicate_count,
            suppressed_duplicate_bytes,
            suppressed_overlap_count,
            suppressed_overlap_bytes,
        };

        on_event(ScanEvent::Finished {
            result: result.clone(),
        });

        result
    }
}

#[cfg(test)]
mod tests {
    use super::{aggregate_quality, same_directory_entry, CategoryAccumulator, ScanEngine};
    use crate::models::{
        Category, CleanStrategy, FileSize, ObservationQuality, PathIdentity, RiskTier, ScanEvent,
        ScanItem, Signature,
    };
    use crate::signatures::SignatureRegistry;
    use std::collections::HashSet;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use zenith_platform::path_algebra::PathFlavor;
    use zenith_platform::PlatformEnvironment;

    /// A scan environment with no tools and no stated profile, so a scan in a
    /// test only reports the fixture signatures it was given.
    fn scan_environment() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::current())
            .with_missing_tool("docker")
            .with_missing_tool("npm")
            .with_missing_tool("pnpm")
            .with_missing_tool("uv")
    }

    fn signature(
        id: &str,
        name: &str,
        category: Category,
        path: &Path,
        exclusions: Vec<String>,
        min_age_days: Option<u32>,
    ) -> Signature {
        Signature {
            id: id.to_string(),
            name: name.to_string(),
            category,
            risk: RiskTier::Safe,
            strategy: CleanStrategy::DeleteContents,
            paths: vec![path.to_string_lossy().into_owned()],
            exclusions,
            description: "test-only signature".into(),
            min_age_days,
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
        }
    }

    /// Three entries a measurement must not account for: the named exclusion,
    /// the protected `.git` directory, and a signed app bundle.
    fn cache_fixture(root: &Path) {
        std::fs::create_dir_all(root.join("keep")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("Tool.app/Contents")).unwrap();
        std::fs::write(root.join("data.bin"), vec![1u8; 4_096]).unwrap();
        std::fs::write(root.join("keep/payload.bin"), vec![2u8; 2_048]).unwrap();
        std::fs::write(root.join(".git/objects"), vec![3u8; 1_024]).unwrap();
        std::fs::write(root.join("Tool.app/Contents/payload"), vec![4u8; 512]).unwrap();
    }

    /// Two signatures that name the same location are one unit: the bytes are
    /// counted once, and the suppressed duplicate is reported rather than
    /// dropped silently.
    #[test]
    fn overlapping_signatures_count_one_unit_and_report_the_suppression() {
        let fixture = tempfile::tempdir().unwrap();
        let shared = fixture.path().join("shared-cache");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("data.bin"), vec![1u8; 8_192]).unwrap();

        let mut registry = SignatureRegistry::new();
        registry.register(signature(
            "test.overlap.specific",
            "Specific cache",
            Category::Developer,
            &shared,
            vec![],
            None,
        ));
        registry.register(signature(
            "test.overlap.broad",
            "Broad cache",
            Category::Developer,
            &shared,
            vec![],
            None,
        ));

        let result = ScanEngine::scan(
            &registry,
            None,
            &[],
            false,
            &scan_environment(),
            &crate::models::NeverCancelled,
            |_| {},
        );

        let developer = result
            .categories
            .iter()
            .find(|category| category.category == Category::Developer)
            .expect("the developer category is scanned");
        assert_eq!(
            developer.items.len(),
            1,
            "one location is one unit however many signatures name it"
        );
        assert_eq!(developer.total_bytes, 8_192);
        assert_eq!(developer.suppressed_duplicate_count, 1);
        assert_eq!(developer.suppressed_duplicate_bytes, 8_192);
        assert_eq!(result.suppressed_duplicate_count, 1);
        assert_eq!(result.suppressed_duplicate_bytes, 8_192);
        let provenance: Vec<&str> = developer.items[0]
            .overlaps
            .iter()
            .map(|overlap| overlap.signature_id.as_str())
            .collect();
        let suppressed = if developer.items[0].signature_id == "test.overlap.specific" {
            "test.overlap.broad"
        } else {
            "test.overlap.specific"
        };
        assert_eq!(
            provenance,
            vec![suppressed],
            "the rule that did not count the bytes is still named on the unit that did"
        );
    }

    /// A unit inside a broader one is that unit's bytes: the scan counts them
    /// once, keeps the narrower rule as provenance, and states how much was
    /// folded away.
    #[test]
    fn a_child_unit_is_accounted_by_the_broader_unit_that_contains_it() {
        let fixture = tempfile::tempdir().unwrap();
        let parent = fixture.path().join("Cache");
        let child = parent.join("Service Worker");
        std::fs::create_dir_all(&child).unwrap();
        // Both files are whole allocation blocks, so the expected totals state
        // the same thing on every filesystem a scan runs against.
        std::fs::write(parent.join("data.bin"), vec![1u8; 8_192]).unwrap();
        std::fs::write(child.join("worker.bin"), vec![2u8; 8_192]).unwrap();

        let mut registry = SignatureRegistry::new();
        registry.register(signature(
            "test.broad",
            "Broad cache",
            Category::Developer,
            &parent,
            vec![],
            None,
        ));
        registry.register(signature(
            "test.narrow",
            "Service worker cache",
            Category::Developer,
            &child,
            vec![],
            None,
        ));

        let result = ScanEngine::scan(
            &registry,
            None,
            &[],
            false,
            &scan_environment(),
            &crate::models::NeverCancelled,
            |_| {},
        );

        let developer = result
            .categories
            .iter()
            .find(|category| category.category == Category::Developer)
            .expect("the developer category is scanned");
        assert_eq!(developer.items.len(), 1, "one location is one unit");
        assert_eq!(developer.items[0].path, parent.to_string_lossy());
        assert_eq!(
            developer.total_bytes, 16_384,
            "the broader unit reports the bytes its subtree holds, once"
        );
        assert_eq!(developer.suppressed_overlap_count, 1);
        assert_eq!(developer.suppressed_overlap_bytes, 8_192);
        assert_eq!(result.suppressed_overlap_count, 1);
        assert_eq!(result.suppressed_overlap_bytes, 8_192);
        assert_eq!(
            result.total_bytes, 16_384,
            "no total counts the contained unit a second time"
        );
        let provenance: Vec<&str> = developer.items[0]
            .overlaps
            .iter()
            .map(|overlap| overlap.signature_id.as_str())
            .collect();
        assert_eq!(provenance, vec!["test.narrow"]);
    }

    /// The strictest rule that described a location decides whether it may be
    /// cleaned, even when the broader rule found it first.
    #[test]
    fn a_stricter_child_rule_withholds_the_overlapping_unit() {
        let fixture = tempfile::tempdir().unwrap();
        let parent = fixture.path().join("Cache");
        let child = parent.join("local-state");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(parent.join("data.bin"), vec![1u8; 8_192]).unwrap();
        std::fs::write(child.join("state.bin"), vec![2u8; 8_192]).unwrap();

        let mut registry = SignatureRegistry::new();
        registry.register(signature(
            "test.broad",
            "Broad cache",
            Category::Developer,
            &parent,
            vec![],
            None,
        ));
        let mut manual = signature(
            "test.guarded",
            "Application state",
            Category::Developer,
            &child,
            vec![],
            None,
        );
        manual.risk = RiskTier::Manual;
        registry.register(manual);

        let result = ScanEngine::scan(
            &registry,
            None,
            &[],
            false,
            &scan_environment(),
            &crate::models::NeverCancelled,
            |_| {},
        );

        let developer = result
            .categories
            .iter()
            .find(|category| category.category == Category::Developer)
            .expect("the developer category is scanned");
        assert_eq!(developer.items.len(), 1);
        let surviving = &developer.items[0];
        assert_eq!(
            surviving.disposition.eligibility,
            crate::models::CleanupEligibility::Blocked,
            "the stricter rule keeps the bytes out of the automatic total"
        );
        assert!(!surviving.is_selected);
        assert_eq!(developer.cleanable_bytes, 0);
        assert_eq!(
            developer.total_bytes, 16_384,
            "the bytes are still discovered and reported"
        );
    }

    /// Text identity follows the stated filesystem: a case variant is one unit
    /// where the filesystem folds case, and two where it does not.
    #[test]
    fn case_variant_units_follow_the_stated_filesystem() {
        let upper = ScanItem::mock(
            "case-upper",
            "test.case",
            "Cache",
            Category::Developer,
            RiskTier::Safe,
            r"C:\Users\tester\Cache",
            FileSize::new(1_024, Some(1_024)),
            1,
        );
        let lower = ScanItem::mock(
            "case-lower",
            "test.case",
            "Cache",
            Category::Developer,
            RiskTier::Safe,
            r"c:\users\tester\cache",
            FileSize::new(1_024, Some(1_024)),
            1,
        );

        for (identity, expected_items, expected_suppressed) in [
            (PathIdentity::CaseInsensitive, 1, 1),
            (PathIdentity::CaseSensitive, 2, 0),
        ] {
            let mut accumulator = CategoryAccumulator::new();
            let mut paths = HashSet::new();
            let mut entities = std::collections::HashMap::new();
            let mut overlapped = Vec::new();
            for item in [upper.clone(), lower.clone()] {
                accumulator.push(item, identity, &mut paths, &mut entities, &mut overlapped);
            }
            assert_eq!(
                accumulator.items.len(),
                expected_items,
                "identity {identity:?}"
            );
            assert_eq!(accumulator.suppressed_duplicate_count, expected_suppressed);
        }
    }

    /// Two scans of one filesystem state state the same totals in the same
    /// order: nothing about the accounting may depend on map iteration or on
    /// the order a directory listing happened to answer in.
    #[test]
    fn repeated_scans_state_the_same_totals_in_the_same_order() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("Cache");
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("data.bin"), vec![1u8; 8_192]).unwrap();
        std::fs::write(nested.join("state.bin"), vec![2u8; 8_192]).unwrap();

        let mut registry = SignatureRegistry::new();
        registry.register(signature(
            "test.parent",
            "Parent cache",
            Category::Developer,
            &root,
            vec![],
            None,
        ));
        registry.register(signature(
            "test.child",
            "Nested cache",
            Category::Developer,
            &nested,
            vec![],
            None,
        ));

        let scan = || {
            ScanEngine::scan(
                &registry,
                None,
                &[],
                false,
                &scan_environment(),
                &crate::models::NeverCancelled,
                |_| {},
            )
        };
        let first = scan();
        let second = scan();

        assert_eq!(first.total_bytes, second.total_bytes);
        assert_eq!(first.cleanable_bytes, second.cleanable_bytes);
        assert_eq!(
            first.suppressed_overlap_count,
            second.suppressed_overlap_count
        );
        assert_eq!(
            first.suppressed_overlap_bytes,
            second.suppressed_overlap_bytes
        );
        assert_eq!(first.eligibility, second.eligibility);
        let describe = |result: &crate::models::ScanResult| -> Vec<(String, u64, u64)> {
            result
                .categories
                .iter()
                .flat_map(|category| {
                    category.items.iter().map(|item| {
                        (
                            item.id.clone(),
                            item.observed_bytes(),
                            item.cleanable_bytes(),
                        )
                    })
                })
                .collect()
        };
        assert_eq!(describe(&first), describe(&second));
    }

    #[test]
    fn distinct_hardlinks_remain_distinct_cleanup_units() {
        let fixture = tempfile::tempdir().unwrap();
        let first = fixture.path().join("Pip");
        let alias = fixture.path().join("pip-alias");
        std::fs::write(&first, b"shared bytes").unwrap();
        std::fs::hard_link(&first, &alias).unwrap();

        let mut accumulator = CategoryAccumulator::new();
        let mut paths = HashSet::new();
        let mut entities = std::collections::HashMap::new();
        for (index, path) in [&first, &alias].iter().enumerate() {
            let mut item = ScanItem::mock(
                format!("duplicate-{index}"),
                "test.duplicate",
                "Cache",
                Category::Developer,
                RiskTier::Safe,
                path.to_string_lossy().into_owned(),
                FileSize::new(12, Some(12)),
                1,
            );
            item.entry_kind = crate::models::EntryKind::File;
            accumulator.push(
                item,
                PathIdentity::CaseSensitive,
                &mut paths,
                &mut entities,
                &mut Vec::new(),
            );
        }
        assert_eq!(accumulator.items.len(), 2);
        assert_eq!(accumulator.suppressed_duplicate_count, 0);
    }

    #[test]
    fn directory_entry_identity_follows_the_actual_volume() {
        let fixture = tempfile::tempdir().unwrap();
        let stored = fixture.path().join("Pip");
        let alternate = fixture.path().join("pip");
        std::fs::create_dir(&stored).unwrap();
        let entity = crate::safety::ToctouGuard::capture(&stored)
            .expect("directory identity")
            .entity();

        if alternate.exists() {
            assert!(same_directory_entry(&stored, &alternate, entity));
        } else {
            std::fs::create_dir(&alternate).unwrap();
            assert!(!same_directory_entry(&stored, &alternate, entity));
        }

        let mut accumulator = CategoryAccumulator::new();
        let mut paths = HashSet::new();
        let mut entities = std::collections::HashMap::new();
        for (index, path) in [&stored, &alternate].iter().enumerate() {
            let item = ScanItem::mock(
                format!("case-{index}"),
                "test.case",
                "Cache",
                Category::Developer,
                RiskTier::Safe,
                path.to_string_lossy().into_owned(),
                FileSize::new(12, Some(12)),
                1,
            );
            accumulator.push(
                item,
                PathIdentity::CaseSensitive,
                &mut paths,
                &mut entities,
                &mut Vec::new(),
            );
        }
        let expected_units = if entity.same_entity(
            crate::safety::ToctouGuard::capture(&alternate)
                .expect("alternate identity")
                .entity(),
        ) {
            1
        } else {
            2
        };
        assert_eq!(accumulator.items.len(), expected_units);
    }

    /// The breakdown explains the total: every observed byte lands in exactly
    /// one eligibility bucket, and only the cleanable states carry cleanable
    /// bytes.
    #[test]
    fn the_scan_reports_an_eligibility_breakdown_that_sums_to_the_total() {
        let fixture = tempfile::tempdir().unwrap();
        let cleanable = fixture.path().join("cleanable-cache");
        let blocked = fixture.path().join("blocked-cache");
        std::fs::create_dir_all(&cleanable).unwrap();
        std::fs::create_dir_all(&blocked).unwrap();
        std::fs::write(cleanable.join("data.bin"), vec![1u8; 4_096]).unwrap();
        std::fs::write(blocked.join("session.sqlite"), vec![2u8; 2_048]).unwrap();

        let mut registry = SignatureRegistry::new();
        registry.register(signature(
            "test.breakdown.cleanable",
            "Cleanable cache",
            Category::Developer,
            &cleanable,
            vec![],
            None,
        ));
        registry.register(signature(
            "test.breakdown.blocked",
            "Structured state",
            Category::Developer,
            &blocked.join("session.sqlite"),
            vec![],
            None,
        ));

        let result = ScanEngine::scan(
            &registry,
            None,
            &[],
            false,
            &scan_environment(),
            &crate::models::NeverCancelled,
            |_| {},
        );

        let developer = result
            .categories
            .iter()
            .find(|category| category.category == Category::Developer)
            .expect("the developer category is scanned");

        let bucket_total: u64 = developer
            .eligibility
            .buckets
            .iter()
            .map(|bucket| bucket.observed_bytes)
            .sum();
        assert_eq!(bucket_total, developer.total_bytes);

        let auto = crate::models::CleanupEligibility::AutoCleanable;
        let blocked_state = crate::models::CleanupEligibility::Blocked;
        // The buckets carry exactly what the items reported: an allocated size
        // is a filesystem fact, so the assertion is against the items rather
        // than against a literal the block size could change.
        let cleanable_item = developer
            .items
            .iter()
            .find(|item| item.disposition.eligibility == auto)
            .expect("the cleanable cache is auto-cleanable");
        let blocked_item = developer
            .items
            .iter()
            .find(|item| item.disposition.eligibility == blocked_state)
            .expect("the structured file is blocked");
        assert_eq!(
            developer.eligibility_observed_bytes(auto),
            cleanable_item.observed_bytes()
        );
        assert_eq!(
            developer.eligibility_observed_bytes(blocked_state),
            blocked_item.observed_bytes()
        );
        assert_eq!(
            blocked_item.structured_state,
            Some(crate::models::StructuredStateKind::Database)
        );
        assert_eq!(cleanable_item.structured_state, None);
        assert_eq!(
            developer.eligibility.cleanable_bytes(blocked_state),
            0,
            "a blocked state never reports cleanable bytes"
        );
        assert_eq!(developer.eligibility_items(auto), 1);
        assert_eq!(developer.eligibility_items(blocked_state), 1);
        assert_eq!(
            result.eligibility_observed_bytes(auto),
            cleanable_item.observed_bytes()
        );
        assert_eq!(
            result.eligibility_observed_bytes(blocked_state),
            blocked_item.observed_bytes()
        );
    }

    /// The scan reports how much of each tree it could not measure, so a total
    /// that skipped entries is never presented as a complete one.
    #[test]
    fn scan_counts_skipped_entries_and_incomplete_items() {
        let fixture = tempfile::tempdir().unwrap();
        let plain_root = fixture.path().join("plain-cache");
        let aged_root = fixture.path().join("aged-cache");
        cache_fixture(&plain_root);
        cache_fixture(&aged_root.join("candidate.cache"));

        let mut registry = SignatureRegistry::new();
        registry.register(signature(
            "test.counts.plain",
            "Plain cache",
            Category::Developer,
            &plain_root,
            vec!["keep".to_string()],
            None,
        ));
        registry.register(signature(
            "test.counts.aged",
            "Aged cache",
            Category::System,
            &aged_root,
            vec!["keep".to_string()],
            Some(0),
        ));

        let result = ScanEngine::scan(
            &registry,
            None,
            &[],
            false,
            &scan_environment(),
            &crate::models::NeverCancelled,
            |_| {},
        );

        let developer = result
            .categories
            .iter()
            .find(|category| category.category == Category::Developer)
            .expect("the developer category is scanned");
        let plain = developer
            .items
            .iter()
            .find(|item| item.signature_id == "test.counts.plain")
            .expect("the plain cache is retained");
        assert_eq!(plain.quality, ObservationQuality::Partial);
        assert_eq!(
            plain.skipped_entry_count, 3,
            "the exclusion, `.git`, and the app bundle were not measured"
        );
        assert!(plain.size.reclaimable() > 0);
        assert!(
            !plain.is_selected,
            "a partial observation is never opted in"
        );

        let system = result
            .categories
            .iter()
            .find(|category| category.category == Category::System)
            .expect("the system category is scanned");
        let aged = system
            .items
            .iter()
            .find(|item| item.signature_id == "test.counts.aged")
            .expect("the incomplete aged candidate is retained for observability");
        assert_eq!(aged.quality, ObservationQuality::Unavailable);
        assert_eq!(aged.skipped_entry_count, 3);

        assert_eq!(result.skipped_entry_count, 6);
        assert_eq!(
            result.incomplete_item_count, 2,
            "both partial and unavailable observations are incomplete, regardless \
             of whether the partial item remains reviewable for cleanup"
        );
    }

    /// The per-category counters are what the category cards and the freshness
    /// notice render, so they have to explain the scan-level counters.
    #[test]
    fn category_counts_sum_to_the_scan_counts() {
        let fixture = tempfile::tempdir().unwrap();
        let developer_root = fixture.path().join("dev-cache");
        let system_root = fixture.path().join("system-cache");
        for root in [&developer_root, &system_root] {
            std::fs::create_dir(root).unwrap();
            std::fs::write(root.join("data.bin"), vec![1u8; 4_096]).unwrap();
            std::fs::create_dir(root.join(".git")).unwrap();
            std::fs::write(root.join(".git/objects"), vec![2u8; 1_024]).unwrap();
        }

        let mut registry = SignatureRegistry::new();
        registry.register(signature(
            "test.sum.developer",
            "Developer cache",
            Category::Developer,
            &developer_root,
            vec![],
            None,
        ));
        registry.register(signature(
            "test.sum.system",
            "System cache",
            Category::System,
            &system_root,
            vec![],
            None,
        ));

        let result = ScanEngine::scan(
            &registry,
            None,
            &[],
            false,
            &scan_environment(),
            &crate::models::NeverCancelled,
            |_| {},
        );

        let category_skipped: u64 = result
            .categories
            .iter()
            .map(|category| category.skipped_entry_count)
            .sum();
        let category_incomplete: u64 = result
            .categories
            .iter()
            .map(|category| category.incomplete_item_count)
            .sum();
        assert_eq!(result.skipped_entry_count, category_skipped);
        assert_eq!(result.incomplete_item_count, category_incomplete);
        assert_eq!(
            result.skipped_entry_count, 2,
            "each tree protects its own `.git` directory"
        );
        assert_eq!(
            result.incomplete_item_count, 0,
            "a deliberate exclusion does not make the observation incomplete"
        );
    }

    #[test]
    fn observation_quality_aggregation_distinguishes_total_and_partial_failure() {
        assert_eq!(aggregate_quality([]), ObservationQuality::Fresh);
        assert_eq!(
            aggregate_quality([ObservationQuality::Fresh, ObservationQuality::Fresh]),
            ObservationQuality::Fresh
        );
        assert_eq!(
            aggregate_quality([
                ObservationQuality::Unavailable,
                ObservationQuality::Unavailable,
            ]),
            ObservationQuality::Unavailable
        );
        assert_eq!(
            aggregate_quality([ObservationQuality::Fresh, ObservationQuality::Unavailable]),
            ObservationQuality::Partial
        );
    }

    /// Observed and cleanable totals remain distinct, so an uninspectable item
    /// stays visible without becoming actionable or a second error surface.
    #[test]
    fn category_totals_separate_observed_and_cleanable_bytes_and_emit_no_error_event() {
        let fixture = tempfile::tempdir().unwrap();
        let plain_root = fixture.path().join("plain-cache");
        let aged_root = fixture.path().join("aged-cache");
        std::fs::create_dir(&plain_root).unwrap();
        std::fs::write(plain_root.join("data.bin"), vec![1u8; 4_096]).unwrap();
        cache_fixture(&aged_root.join("candidate.cache"));

        let mut registry = SignatureRegistry::new();
        registry.register(signature(
            "test.consistency.plain",
            "Plain cache",
            Category::System,
            &plain_root,
            vec![],
            None,
        ));
        registry.register(signature(
            "test.consistency.aged",
            "Aged cache",
            Category::System,
            &aged_root,
            vec![],
            Some(0),
        ));

        let mut events = Vec::new();
        let result = ScanEngine::scan(
            &registry,
            Some(&[Category::System]),
            &[],
            false,
            &scan_environment(),
            &crate::models::NeverCancelled,
            |event| events.push(event),
        );

        let system = result
            .categories
            .iter()
            .find(|category| category.category == Category::System)
            .expect("the system category is scanned");
        let cleanable = system
            .items
            .iter()
            .find(|item| item.signature_id == "test.consistency.plain")
            .expect("the cleanable cache is retained");
        let blocked = system
            .items
            .iter()
            .find(|item| item.signature_id == "test.consistency.aged")
            .expect("the uninspectable aged candidate is retained for observability");

        assert!(
            blocked.size.observed_bytes() > 0,
            "the blocked item still reports what it could measure"
        );
        assert!(!blocked.allows_cleanup());
        assert_eq!(blocked.cleanable_bytes(), 0);
        assert_eq!(
            blocked.disposition.eligibility,
            crate::models::CleanupEligibility::Blocked
        );

        assert_eq!(
            system.total_bytes,
            cleanable.size.observed_bytes() + blocked.size.observed_bytes(),
            "detected bytes include retained blocked observations"
        );
        assert_eq!(system.cleanable_bytes, cleanable.size.observed_bytes());
        assert_eq!(system.safe_bytes, cleanable.size.observed_bytes());
        assert_eq!(system.rebuild_bytes, 0);
        assert_eq!(system.manual_bytes, 0);
        assert_eq!(system.incomplete_item_count, 1);

        let kinds: Vec<&str> = events
            .iter()
            .map(|event| match event {
                ScanEvent::Started { .. } => "started",
                ScanEvent::CategoryStarted { .. } => "category_started",
                ScanEvent::ItemFound { .. } => "item_found",
                ScanEvent::CategoryFinished { .. } => "category_finished",
                ScanEvent::Finished { .. } => "finished",
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "started",
                "category_started",
                "item_found",
                "item_found",
                "category_finished",
                "finished",
            ],
            "an item-level measurement gap is never a destructive scan event"
        );
    }

    #[test]
    fn scan_engine_honors_cancellation_probe() {
        struct AlwaysCancelled;
        impl crate::models::CancellationProbe for AlwaysCancelled {
            fn is_cancelled(&self) -> bool {
                true
            }
        }

        let fixture = tempfile::tempdir().unwrap();
        let dev_root = fixture.path().join("dev-cache");
        std::fs::create_dir_all(&dev_root).unwrap();
        std::fs::write(dev_root.join("data.bin"), vec![1u8; 100]).unwrap();

        let mut registry = SignatureRegistry::new();
        registry.register(signature(
            "test.cancel",
            "Cancelled cache",
            Category::Developer,
            &dev_root,
            vec![],
            None,
        ));

        let result = ScanEngine::scan(
            &registry,
            None,
            &[],
            false,
            &scan_environment(),
            &AlwaysCancelled,
            |_| {},
        );

        assert_eq!(result.quality, ObservationQuality::Partial);
        assert!(result
            .incomplete_reasons
            .iter()
            .any(|reason| reason.contains("cancelled")));
    }

    #[test]
    fn cancellation_between_signatures_preserves_global_accounting() {
        // Cancellation is observed through the progress stream rather than by
        // counting probe calls: the traversal consults the probe at every
        // directory boundary, so a call count would describe the walk instead
        // of the behaviour this test is about. The probe reports cancellation
        // as soon as the first item has been observed, which stops the run
        // between signatures exactly as a user cancel does.
        struct CancelAfterFirstItem {
            cancelled: AtomicBool,
        }

        impl crate::models::CancellationProbe for CancelAfterFirstItem {
            fn is_cancelled(&self) -> bool {
                self.cancelled.load(Ordering::SeqCst)
            }
        }

        let fixture = tempfile::tempdir().unwrap();
        let first_root = fixture.path().join("first-cache");
        let second_root = fixture.path().join("second-cache");
        std::fs::create_dir_all(&first_root).unwrap();
        std::fs::create_dir_all(&second_root).unwrap();
        std::fs::write(first_root.join("data.bin"), vec![1u8; 100]).unwrap();
        std::fs::write(second_root.join("data.bin"), vec![2u8; 200]).unwrap();

        let mut registry = SignatureRegistry::new();
        registry.register(signature(
            "test.cancel.first",
            "First cache",
            Category::Developer,
            &first_root,
            vec![],
            None,
        ));
        registry.register(signature(
            "test.cancel.second",
            "Second cache",
            Category::Developer,
            &second_root,
            vec![],
            None,
        ));

        let cancellation = CancelAfterFirstItem {
            cancelled: AtomicBool::new(false),
        };
        let result = ScanEngine::scan(
            &registry,
            Some(&[Category::Developer]),
            &[],
            false,
            &scan_environment(),
            &cancellation,
            |event| {
                if matches!(event, ScanEvent::ItemFound { .. }) {
                    cancellation.cancelled.store(true, Ordering::SeqCst);
                }
            },
        );

        assert_eq!(result.quality, ObservationQuality::Partial);
        assert_eq!(result.categories.len(), 1);
        assert!(result.categories[0].total_bytes > 0);
        assert_eq!(
            result.total_bytes,
            result
                .categories
                .iter()
                .map(|category| category.total_bytes)
                .sum::<u64>()
        );
        assert_eq!(
            result.cleanable_bytes,
            result
                .categories
                .iter()
                .map(|category| category.cleanable_bytes)
                .sum::<u64>()
        );
        assert_eq!(
            result.skipped_entry_count,
            result
                .categories
                .iter()
                .map(|category| category.skipped_entry_count)
                .sum::<u64>()
        );
        assert_eq!(
            result.incomplete_item_count,
            result
                .categories
                .iter()
                .map(|category| category.incomplete_item_count)
                .sum::<u64>()
        );
    }
}
