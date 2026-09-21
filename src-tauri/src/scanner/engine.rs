use super::observation::{RootProgressSink, ScanLimits, TraversalCounters, WalkContext};
use crate::cache_providers::CacheProviderRegistry;
use crate::cleaner::LifecycleProviderRegistry;
use crate::docker::DockerAdapter;
use crate::execution_budget::shared_scan_pool;
use crate::models::{
    resolve_unit_overlaps_with, Category, CategoryResult, EligibilitySummary, ObservationQuality,
    PathIdentity, RiskTier, ScanEvent, ScanGap, ScanGapKind, ScanItem, ScanResult,
    UnitRelationship,
};
use crate::orbstack::OrbStackAdapter;
use crate::scanner::DirectoryScanner;
use crate::signatures::SignatureRegistry;
use std::cell::RefCell;
use std::path::Path;
use std::time::SystemTime;
use uuid::Uuid;
use zenith_core::domain::ScanMetrics;
use zenith_platform::PlatformEnvironment;

use crate::scanner::relationship::unit_relationship;

pub struct ScanEngine;

/// The relationship two units have, together with the cleanup policy the scan
/// can act on.
///
/// The structural answer comes from [`unit_relationship`], which the planner
/// uses as well, so a plan and the scan it came from cannot disagree about
/// whether two units are one location.
fn scan_unit_relationship(
    candidate: &ScanItem,
    container: &ScanItem,
    registry: &SignatureRegistry,
) -> Option<UnitRelationship> {
    match unit_relationship(candidate, container) {
        UnitRelationship::Equivalent => Some(
            if exact_authorities_can_fold(candidate, container, registry) {
                UnitRelationship::Equivalent
            } else {
                // One location through two rules that disagree about what may be
                // done with it: the bytes are counted once and neither rule's
                // operation decides for the other.
                UnitRelationship::EquivalentConflict
            },
        ),
        UnitRelationship::Contained => {
            if containment_policies_are_compatible(candidate, container, registry) {
                Some(UnitRelationship::Contained)
            } else {
                Some(UnitRelationship::AuthorityConflict)
            }
        }
        other => Some(other),
    }
}

fn exact_authorities_can_fold(
    left: &ScanItem,
    right: &ScanItem,
    registry: &SignatureRegistry,
) -> bool {
    if left.signature_id == right.signature_id {
        return true;
    }
    if left.risk == RiskTier::Manual || right.risk == RiskTier::Manual {
        return true;
    }
    let left_specialized = !left.unit.kind.is_filesystem();
    let right_specialized = !right.unit.kind.is_filesystem();
    if left_specialized != right_specialized {
        return true;
    }
    if left_specialized {
        return false;
    }
    containment_policies_are_compatible(left, right, registry)
}

fn containment_policies_are_compatible(
    nested: &ScanItem,
    container: &ScanItem,
    registry: &SignatureRegistry,
) -> bool {
    // A rule whose scope is switched off states nothing about what may be done
    // here: it is discovered for visibility, so it cannot constrain a rule the
    // current settings do run.
    if !nested.gate.is_open() || !container.gate.is_open() {
        return true;
    }
    if !nested.unit.kind.is_filesystem()
        || !container.unit.kind.is_filesystem()
        || nested.risk == RiskTier::Manual
        || container.risk == RiskTier::Manual
    {
        return false;
    }
    if nested.signature_id == container.signature_id {
        return true;
    }
    let (Some(nested_signature), Some(container_signature)) = (
        registry.get(&nested.signature_id),
        registry.get(&container.signature_id),
    ) else {
        return false;
    };
    nested_signature.strategy == container_signature.strategy
        && nested_signature.management_mode == container_signature.management_mode
        && nested_signature.min_age_days == container_signature.min_age_days
        && same_string_set(
            &nested_signature.exclusions,
            &container_signature.exclusions,
        )
        && same_string_set(
            &nested_signature.fail_if_running,
            &container_signature.fail_if_running,
        )
        && nested_signature.ownership() == container_signature.ownership()
}

fn same_string_set(left: &[String], right: &[String]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut left = left.iter().map(String::as_str).collect::<Vec<_>>();
    let mut right = right.iter().map(String::as_str).collect::<Vec<_>>();
    left.sort_unstable();
    right.sort_unstable();
    left == right
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

fn add_scan_gap(gaps: &mut Vec<ScanGap>, kind: ScanGapKind, count: u64) {
    if count == 0 {
        return;
    }
    if let Some(existing) = gaps.iter_mut().find(|gap| gap.kind == kind) {
        existing.count = existing.count.saturating_add(count);
    } else {
        gaps.push(ScanGap { kind, count });
    }
}

/// Maps an incomplete retained observation to a stable remediation category.
///
/// The item still carries the original backend reason for diagnostics, but the
/// UI never has to parse that prose. Stable cancellation, depth, and access
/// refusal markers are classified first; only an access refusal on a protected
/// macOS path is attributed to Full Disk Access.
fn scan_gap_kind(environment: &PlatformEnvironment, item: &ScanItem) -> Option<ScanGapKind> {
    if item.quality == ObservationQuality::Fresh && item.incomplete_reason.is_none() {
        return None;
    }
    let reason = item.incomplete_reason.as_deref().unwrap_or_default();
    if reason.to_ascii_lowercase().contains("cancel") {
        return Some(ScanGapKind::Cancelled);
    }
    let lower = reason.to_ascii_lowercase();
    if lower.contains("depth limit") {
        return Some(ScanGapKind::DepthLimit);
    }
    let is_access_refusal = lower.contains("permission denied")
        || lower.contains("operation not permitted")
        || lower.contains("access denied");
    if is_access_refusal
        && zenith_platform::environment::refusal_may_be_full_disk_access(
            environment,
            Path::new(&item.path),
        )
    {
        return Some(ScanGapKind::FullDiskAccess);
    }
    if is_access_refusal {
        return Some(ScanGapKind::PermissionDenied);
    }
    Some(ScanGapKind::IoError)
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
    /// Duplicate and containment decisions require the complete scan: stable
    /// filesystem identity, authority compatibility, and observation coverage
    /// can all cross category boundaries. This accumulator therefore retains
    /// every actionable discovery; the resolver makes the deterministic fold
    /// after every category has finished.
    fn push(&mut self, item: ScanItem) -> Option<&ScanItem> {
        let item = item.with_derived_disposition();
        let bytes = item.cleanable_bytes();
        let observed = item.observed_bytes();
        // An item that is absent, or a complete observation of an empty path,
        // carries nothing a user could act on.
        let is_empty_fresh = item.quality == ObservationQuality::Fresh && observed == 0;
        if !item.exists || is_empty_fresh {
            return None;
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

        if item.quality != ObservationQuality::Fresh || item.incomplete_reason.is_some() {
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
            ambiguous_overlap_count: 0,
            ambiguous_overlap_bytes: 0,
        }
    }
}

impl ScanEngine {
    /// Executes a full or filtered scan across all categories, emitting streaming events.
    ///
    /// The environment is threaded explicitly: signature paths, size
    /// exclusions, and external provider caches all resolve through it, so a
    /// scan cannot silently fall back to the running process's own profile.
    /// Lifecycle providers are threaded the same way: which providers exist is
    /// decided by the caller that owns them, not by the scan.
    /// The request's own facts are passed explicitly rather than bundled: each
    /// one is a decision the caller owns, and a struct would only move the same
    /// list one call site away.
    #[allow(clippy::too_many_arguments)]
    pub fn scan<F>(
        registry: &SignatureRegistry,
        lifecycle_providers: &LifecycleProviderRegistry,
        owner_providers: &crate::cleaner::OwnerProviderRegistry,
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
        let started = std::time::Instant::now();
        // The loop and the walker's root reports write through one channel, so
        // the emitted stream keeps the order the scan produced it in.
        let events = ScanEvents {
            emit: RefCell::new(&mut on_event),
        };

        events.send(ScanEvent::Started {
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
        // Traversals that stopped before covering every root they name. A
        // cancellation is the only reason one stops today, and the scan states
        // both facts: the typed gap says the scan was stopped, and this says
        // which coverage it therefore does not have.
        let mut scanned_incomplete_selectors = 0u64;
        let mut eligibility = EligibilitySummary::default();
        let mut suppressed_duplicate_count = 0u64;
        let mut suppressed_duplicate_bytes = 0u64;
        let mut gaps = Vec::new();
        // One process-table pass for the whole scan: the scan asks which
        // application bundles are running, and every signature sees the same
        // answer.
        let running_apps = crate::applications::RunningApplications::probe();
        // Reuse the explicitly bounded shared scan pool (see
        // `execution_budget`); never expand pools per request.
        let directory_pool = shared_scan_pool();
        // The scan's own bounds and counters. The limits are stated once, and
        // the counters are what turn them into a measurement: the peak
        // outstanding directory-task count is checked against the bound by the
        // tests and reported to the interface.
        let limits = ScanLimits::default();
        let counters = TraversalCounters::default();

        for &category in target_categories {
            if cancellation.is_cancelled() {
                was_cancelled = true;
                break;
            }

            events.send(ScanEvent::CategoryStarted { category });

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
                let context =
                    WalkContext::new(environment, cancellation, limits, &counters, &events);
                let scanned = DirectoryScanner::scan_signature_with_context(
                    sig,
                    directory_pool,
                    &context,
                    gate,
                    &running_apps,
                );
                if scanned.selector_incomplete {
                    scanned_incomplete_selectors = scanned_incomplete_selectors.saturating_add(1);
                }
                for item in scanned.items {
                    if let Some(retained) = accumulator.push(item) {
                        events.send(ScanEvent::ItemFound {
                            item: retained.clone(),
                        });
                    }
                }
                if cancellation.is_cancelled() {
                    was_cancelled = true;
                    break;
                }
            }

            // 2. Typed container adapters can report cleanable or observation-only storage.
            if !was_cancelled && category == Category::Developer {
                for item in CacheProviderRegistry::scan_items(registry, environment) {
                    if cancellation.is_cancelled() {
                        was_cancelled = true;
                        break;
                    }
                    if let Some(retained) = accumulator.push(item) {
                        events.send(ScanEvent::ItemFound {
                            item: retained.clone(),
                        });
                    }
                }
            }

            // 3. Lifecycle providers own stores no path-shaped rule may touch.
            //    Their candidates are discovered here — in the category their
            //    catalog entry declares — and are never auto-selected.
            if !was_cancelled {
                for item in lifecycle_providers.scan_items(
                    registry,
                    category,
                    intensive_cleanup,
                    excluded_signatures,
                    environment,
                ) {
                    if cancellation.is_cancelled() {
                        was_cancelled = true;
                        break;
                    }
                    if let Some(retained) = accumulator.push(item) {
                        events.send(ScanEvent::ItemFound {
                            item: retained.clone(),
                        });
                    }
                }
            }

            // 4. Owner-scoped providers enumerate the units of a store whose
            //    semantics only its owner knows. Their candidates are
            //    discovered here — in the category their catalog entry declares
            //    — and every one of them is offered for explicit selection.
            if !was_cancelled {
                for item in owner_providers.scan_items(
                    registry,
                    category,
                    intensive_cleanup,
                    excluded_signatures,
                    environment,
                ) {
                    if cancellation.is_cancelled() {
                        was_cancelled = true;
                        break;
                    }
                    if let Some(retained) = accumulator.push(item) {
                        events.send(ScanEvent::ItemFound {
                            item: retained.clone(),
                        });
                    }
                }
            }

            // 5. Typed container adapters can report cleanable or observation-only storage.
            if !was_cancelled && category == Category::Container {
                let adapter_items = DockerAdapter::scan_items(environment)
                    .into_iter()
                    .chain(OrbStackAdapter::scan_items(environment));
                for item in adapter_items {
                    if cancellation.is_cancelled() {
                        was_cancelled = true;
                        break;
                    }
                    if let Some(retained) = accumulator.push(item) {
                        events.send(ScanEvent::ItemFound {
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
        let overlap = resolve_unit_overlaps_with(
            &mut category_results,
            &[],
            PathIdentity::CaseSensitive,
            |candidate, container| scan_unit_relationship(candidate, container, registry),
        );
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
        let ambiguous_overlap_count = overlap.ambiguous_count;
        let ambiguous_overlap_bytes = overlap.ambiguous_bytes;

        // The per-category events carry the totals the resolution settled: a
        // category that folded an overlapping unit away reports what it now
        // accounts for, not what it held mid-scan.
        for (index, category_result) in category_results.iter().enumerate() {
            if was_cancelled && index + 1 == category_results.len() {
                break;
            }
            events.send(ScanEvent::CategoryFinished {
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
                if let Some(kind) = scan_gap_kind(environment, item) {
                    add_scan_gap(&mut gaps, kind, 1);
                }
            }
        }
        if was_cancelled {
            incomplete_reasons.push("Scan was cancelled before completion".to_string());
            add_scan_gap(&mut gaps, ScanGapKind::Cancelled, 1);
        }
        let has_selector_truncation = scanned_incomplete_selectors > 0;
        if has_selector_truncation {
            incomplete_reasons.push(
                "A pattern's traversal stopped before it covered every root it names".to_string(),
            );
        }
        // Item-derived gaps already contribute their own observation quality,
        // including the all-unavailable case. Only scan-level incompleteness
        // that is not represented by a retained item must force Partial.
        let scan_quality = if was_cancelled || has_selector_truncation {
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
            gaps,
            skipped_entry_count,
            incomplete_item_count,
            eligibility,
            suppressed_duplicate_count,
            suppressed_duplicate_bytes,
            suppressed_overlap_count,
            suppressed_overlap_bytes,
            ambiguous_overlap_count,
            ambiguous_overlap_bytes,
            cancelled: was_cancelled,
            metrics: ScanMetrics {
                duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                visited_entries: counters.visited_entries(),
                directories_read: counters.directories_read(),
                peak_outstanding_directory_tasks: counters.peak_outstanding_directory_tasks(),
            },
        };

        events.send(ScanEvent::Finished {
            result: result.clone(),
        });

        result
    }

    /// Rebuilds one authoritative snapshot from completed scan slices.
    ///
    /// Slices are never added as byte deltas. Their retained rows are combined
    /// first and the normal cross-category overlap and aggregate rules are run
    /// again, so a later category can still suppress or conflict with an
    /// earlier observation.
    pub fn merge_slices(
        registry: &SignatureRegistry,
        environment: &PlatformEnvironment,
        slices: &[ScanResult],
    ) -> Option<ScanResult> {
        let latest = slices.last()?.clone();
        let mut categories = slices
            .iter()
            .flat_map(|slice| slice.categories.clone())
            .collect::<Vec<_>>();
        let overlap = resolve_unit_overlaps_with(
            &mut categories,
            &[],
            PathIdentity::CaseSensitive,
            |candidate, container| scan_unit_relationship(candidate, container, registry),
        );

        let mut total_bytes = 0u64;
        let mut cleanable_bytes = 0u64;
        let mut safe_bytes = 0u64;
        let mut rebuild_bytes = 0u64;
        let mut manual_bytes = 0u64;
        let mut skipped_entry_count = 0u64;
        let mut incomplete_item_count = 0u64;
        let mut eligibility = EligibilitySummary::default();
        let mut suppressed_duplicate_count = 0u64;
        let mut suppressed_duplicate_bytes = 0u64;
        let mut incomplete_reasons = Vec::new();
        let mut gaps = Vec::new();
        for category in &categories {
            total_bytes = total_bytes.saturating_add(category.total_bytes);
            cleanable_bytes = cleanable_bytes.saturating_add(category.cleanable_bytes);
            safe_bytes = safe_bytes.saturating_add(category.safe_bytes);
            rebuild_bytes = rebuild_bytes.saturating_add(category.rebuild_bytes);
            manual_bytes = manual_bytes.saturating_add(category.manual_bytes);
            skipped_entry_count = skipped_entry_count.saturating_add(category.skipped_entry_count);
            incomplete_item_count =
                incomplete_item_count.saturating_add(category.incomplete_item_count);
            eligibility.merge(&category.eligibility);
            suppressed_duplicate_count =
                suppressed_duplicate_count.saturating_add(category.suppressed_duplicate_count);
            suppressed_duplicate_bytes =
                suppressed_duplicate_bytes.saturating_add(category.suppressed_duplicate_bytes);
            for item in &category.items {
                if let Some(reason) = &item.incomplete_reason {
                    if !incomplete_reasons.contains(reason) {
                        incomplete_reasons.push(reason.clone());
                    }
                }
                if let Some(kind) = scan_gap_kind(environment, item) {
                    add_scan_gap(&mut gaps, kind, 1);
                }
            }
        }
        let cancelled = slices.iter().any(|slice| slice.cancelled);
        if cancelled {
            incomplete_reasons.push("Scan was cancelled before completion".to_string());
            add_scan_gap(&mut gaps, ScanGapKind::Cancelled, 1);
        }
        let quality = if cancelled {
            ObservationQuality::Partial
        } else {
            aggregate_quality(categories.iter().map(|category| category.quality))
        };
        let metrics = ScanMetrics {
            duration_ms: slices
                .iter()
                .map(|slice| slice.metrics.duration_ms)
                .fold(0u64, u64::saturating_add),
            visited_entries: slices
                .iter()
                .map(|slice| slice.metrics.visited_entries)
                .fold(0u64, u64::saturating_add),
            directories_read: slices
                .iter()
                .map(|slice| slice.metrics.directories_read)
                .fold(0u64, u64::saturating_add),
            peak_outstanding_directory_tasks: slices
                .iter()
                .map(|slice| slice.metrics.peak_outstanding_directory_tasks)
                .max()
                .unwrap_or_default(),
        };

        Some(ScanResult {
            scan_id: latest.scan_id,
            valid_for_seconds: latest.valid_for_seconds,
            started_at: slices
                .iter()
                .map(|slice| slice.started_at)
                .min()
                .unwrap_or(latest.started_at),
            finished_at: latest.finished_at,
            categories,
            total_bytes,
            cleanable_bytes,
            safe_bytes,
            rebuild_bytes,
            manual_bytes,
            quality,
            incomplete_reasons,
            gaps,
            skipped_entry_count,
            incomplete_item_count,
            eligibility,
            suppressed_duplicate_count,
            suppressed_duplicate_bytes,
            suppressed_overlap_count: overlap.suppressed_count,
            suppressed_overlap_bytes: overlap.suppressed_bytes,
            ambiguous_overlap_count: overlap.ambiguous_count,
            ambiguous_overlap_bytes: overlap.ambiguous_bytes,
            cancelled,
            metrics,
        })
    }
}

/// The scan's single event channel.
///
/// The loop and the walker's root reports write through one value, so the
/// emitted stream keeps the order the scan produced it in; the interior
/// mutability is what lets the walker hold the channel while the loop still
/// emits items.
struct ScanEvents<'a, F: FnMut(ScanEvent)> {
    emit: std::cell::RefCell<&'a mut F>,
}

impl<F: FnMut(ScanEvent)> ScanEvents<'_, F> {
    fn send(&self, event: ScanEvent) {
        (self.emit.borrow_mut())(event);
    }
}

impl<F: FnMut(ScanEvent)> RootProgressSink for ScanEvents<'_, F> {
    fn root_started(&self, signature: &crate::models::Signature, root: &std::path::Path) {
        self.send(ScanEvent::RootStarted {
            category: signature.category,
            signature_id: signature.id.clone(),
            name: signature.name.clone(),
            root: root.to_string_lossy().into_owned(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{add_scan_gap, aggregate_quality, scan_gap_kind, CategoryAccumulator, ScanEngine};
    use crate::cleaner::LifecycleProviderRegistry;
    use crate::models::{
        Category, CleanStrategy, FileSize, ObservationQuality, PathIdentity, RiskTier, ScanEvent,
        ScanGapKind, ScanItem, Signature,
    };
    use crate::scanner::relationship::{same_directory_entry, unit_relationship};
    use crate::signatures::SignatureRegistry;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use zenith_platform::path_algebra::PathFlavor;
    use zenith_platform::PlatformEnvironment;

    /// A scan environment with no tools and no stated profile, so a scan in a
    /// test only reports the fixture signatures it was given.
    fn scan_environment() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::current())
            .with_home(if PathFlavor::current().is_windows() {
                r"Z:\ZenithFixtureHome"
            } else {
                "/zenith-fixture-home"
            })
            .with_missing_tool("docker")
            .with_missing_tool("npm")
            .with_missing_tool("pnpm")
            .with_missing_tool("uv")
    }

    #[test]
    fn scan_gap_classification_is_typed_and_full_disk_access_is_context_aware() {
        // Keep the fixture path in the simulated macOS path algebra all the
        // way through. `std::path::Path::join` follows the CI host, so on
        // Windows it would inject backslashes into an otherwise POSIX path and
        // make the Full Disk Access containment check correctly reject it.
        let home = "/Users/fixture";
        let environment = PlatformEnvironment::simulated(PathFlavor::Posix)
            .with_roots(std::sync::Arc::new(
                zenith_platform::paths::SimulatedPaths::new()
                    .with_flavor(PathFlavor::Posix)
                    .with_home(home),
            ))
            .with_platform(crate::models::PlatformKind::Macos);
        let mut protected = ScanItem::mock(
            "gap.protected",
            "gap.protected",
            "Protected",
            Category::System,
            RiskTier::Safe,
            zenith_platform::path_algebra::join(
                home,
                "Library/Containers/com.example/Data/Library/Caches",
                PathFlavor::Posix,
            ),
            FileSize::default(),
            0,
        );
        protected.quality = ObservationQuality::Unavailable;
        protected.incomplete_reason = Some("Operation not permitted".to_string());
        assert_eq!(
            scan_gap_kind(&environment, &protected),
            Some(ScanGapKind::FullDiskAccess)
        );

        protected.incomplete_reason = Some("Directory depth limit exceeded".to_string());
        assert_eq!(
            scan_gap_kind(&environment, &protected),
            Some(ScanGapKind::DepthLimit)
        );

        protected.path = zenith_platform::path_algebra::join(home, "ordinary", PathFlavor::Posix);
        protected.incomplete_reason = Some("I/O failure".to_string());
        assert_eq!(
            scan_gap_kind(&environment, &protected),
            Some(ScanGapKind::IoError)
        );
        protected.incomplete_reason = Some("Permission denied".to_string());
        assert_eq!(
            scan_gap_kind(&environment, &protected),
            Some(ScanGapKind::PermissionDenied)
        );
    }

    #[test]
    fn scan_gap_counts_merge_without_requiring_frontend_prose_parsing() {
        let mut gaps = Vec::new();
        add_scan_gap(&mut gaps, ScanGapKind::DepthLimit, 1);
        add_scan_gap(&mut gaps, ScanGapKind::DepthLimit, 2);
        add_scan_gap(&mut gaps, ScanGapKind::PermissionDenied, 1);
        assert_eq!(
            gaps,
            vec![
                crate::models::ScanGap {
                    kind: ScanGapKind::DepthLimit,
                    count: 3,
                },
                crate::models::ScanGap {
                    kind: ScanGapKind::PermissionDenied,
                    count: 1,
                },
            ]
        );
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
            provider_id: None,
            management_mode: Default::default(),
            artifact_kind: Default::default(),
            consequence: String::new(),
            reclaimable_is_lower_bound: false,
        }
    }

    /// A lifecycle provider's candidate reaches the scan through the category
    /// its catalog entry declares: discovered with the provider's own
    /// measurement, offered for explicit selection, and never pre-selected —
    /// while nothing about the underlying store becomes generically deletable.
    #[test]
    fn a_lifecycle_provider_candidate_is_discovered_and_never_pre_selected() {
        use crate::cleaner::providers::test_support::StatedProvider;
        use crate::models::{CleanupUnitKind, PlatformKind};

        let mut registry = SignatureRegistry::new();
        let mut entry = signature(
            "test.stated.store",
            "Stated Store",
            Category::System,
            Path::new("/unused"),
            vec![],
            None,
        );
        entry.risk = RiskTier::Manual;
        entry.strategy = CleanStrategy::LifecycleProvider;
        entry.provider_id = Some("test.stated".to_string());
        entry.platforms = vec![PlatformKind::current()];
        entry.paths = Vec::new();
        registry.register(entry);

        let providers =
            LifecycleProviderRegistry::new(vec![StatedProvider::holding(6_000, 3).shared()]);
        let result = ScanEngine::scan(
            &registry,
            &providers,
            &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
            Some(&[Category::System]),
            &[],
            false,
            &scan_environment(),
            &crate::models::NeverCancelled,
            |_| {},
        );

        let items = &result.categories[0].items;
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.id, "test.stated.store");
        assert_eq!(item.unit.kind, CleanupUnitKind::ProviderAction);
        assert_eq!(item.size.observed_bytes(), 6_000);
        assert_eq!(item.file_count, 3);
        assert!(item.disposition.is_cleanable());
        assert!(
            !item.is_selected,
            "a scan never pre-selects a provider action"
        );
        assert_eq!(result.total_bytes, 6_000);
        assert_eq!(
            result.cleanable_bytes, 6_000,
            "the bytes are cleanable, through the provider that reported them"
        );
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
            &LifecycleProviderRegistry::new(Vec::new()),
            &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
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

    #[test]
    fn exact_provider_and_filesystem_duplicates_retain_provider_authority_in_either_order() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("provider-cache");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("data.bin"), vec![1u8; 4_096]).unwrap();

        let filesystem = ScanItem::mock(
            "filesystem",
            "test.filesystem",
            "Filesystem cache",
            Category::Developer,
            RiskTier::Safe,
            path.to_string_lossy(),
            FileSize::new(4_096, Some(4_096)),
            1,
        );
        let mut provider = ScanItem::mock(
            "provider",
            "test.provider",
            "Provider cache",
            Category::Developer,
            RiskTier::Rebuild,
            path.to_string_lossy(),
            FileSize::new(4_096, Some(4_096)),
            1,
        );
        provider.unit.kind = crate::models::CleanupUnitKind::ProviderAction;

        for items in [
            vec![filesystem.clone(), provider.clone()],
            vec![provider.clone(), filesystem.clone()],
        ] {
            let mut accumulator = CategoryAccumulator::new();
            for item in items {
                accumulator.push(item);
            }
            let mut categories = vec![accumulator.finalize(Category::Developer, None)];
            crate::models::resolve_unit_overlaps(&mut categories, &[], PathIdentity::CaseSensitive);
            assert_eq!(categories[0].items.len(), 1);
            assert_eq!(categories[0].items[0].signature_id, "test.provider");
        }
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
            &LifecycleProviderRegistry::new(Vec::new()),
            &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
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
            &LifecycleProviderRegistry::new(Vec::new()),
            &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
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
        assert_eq!(developer.items.len(), 2);
        let surviving = developer
            .items
            .iter()
            .find(|item| item.signature_id == "test.broad")
            .expect("the broader observation remains visible but blocked");
        assert_eq!(
            surviving.disposition.eligibility,
            crate::models::CleanupEligibility::Blocked,
            "the stricter rule keeps the bytes out of the automatic total"
        );
        assert!(!surviving.is_selected);
        assert_eq!(developer.cleanable_bytes, 0);
        assert_eq!(
            developer.total_bytes, 24_576,
            "an unresolved authority conflict preserves both observations instead of claiming coverage"
        );
    }

    #[test]
    fn differing_process_guards_prevent_containment_from_merging_authority() {
        let fixture = tempfile::tempdir().unwrap();
        let parent = fixture.path().join("Cache");
        let child = parent.join("guarded");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(parent.join("data.bin"), vec![1u8; 8_192]).unwrap();
        std::fs::write(child.join("guarded.bin"), vec![2u8; 8_192]).unwrap();

        let mut registry = SignatureRegistry::new();
        registry.register(signature(
            "test.guard.parent",
            "Parent cache",
            Category::Developer,
            &parent,
            vec![],
            None,
        ));
        let mut guarded = signature(
            "test.guard.child",
            "Guarded cache",
            Category::Developer,
            &child,
            vec![],
            None,
        );
        guarded.fail_if_running = vec!["guarded-tool".into()];
        registry.register(guarded);

        let result = ScanEngine::scan(
            &registry,
            &LifecycleProviderRegistry::new(Vec::new()),
            &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
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

        assert_eq!(developer.items.len(), 2);
        let parent = developer
            .items
            .iter()
            .find(|item| item.signature_id == "test.guard.parent")
            .expect("the broader observation remains visible");
        assert_eq!(
            parent.disposition.eligibility,
            crate::models::CleanupEligibility::Blocked
        );
        assert!(parent
            .overlaps
            .iter()
            .any(|overlap| overlap.authority_conflict));
    }

    /// When no filesystem object exists, text is a conservative case-sensitive
    /// fallback rather than an OS-wide guess about the volume.
    #[test]
    fn unresolved_case_variant_units_remain_distinct() {
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

        let mut accumulator = CategoryAccumulator::new();
        for item in [upper, lower] {
            accumulator.push(item);
        }
        let mut categories = vec![accumulator.finalize(Category::Developer, None)];
        crate::models::resolve_unit_overlaps_with(
            &mut categories,
            &[],
            PathIdentity::CaseSensitive,
            |candidate, container| Some(unit_relationship(candidate, container)),
        );
        assert_eq!(categories[0].items.len(), 2);
        assert_eq!(categories[0].suppressed_duplicate_count, 0);
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
                &LifecycleProviderRegistry::new(Vec::new()),
                &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
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
            accumulator.push(item);
        }
        let mut categories = vec![accumulator.finalize(Category::Developer, None)];
        crate::models::resolve_unit_overlaps_with(
            &mut categories,
            &[],
            PathIdentity::CaseSensitive,
            |candidate, container| Some(unit_relationship(candidate, container)),
        );
        assert_eq!(categories[0].items.len(), 2);
        assert_eq!(categories[0].suppressed_duplicate_count, 0);
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
            accumulator.push(item);
        }
        let mut categories = vec![accumulator.finalize(Category::Developer, None)];
        crate::models::resolve_unit_overlaps_with(
            &mut categories,
            &[],
            PathIdentity::CaseSensitive,
            |candidate, container| Some(unit_relationship(candidate, container)),
        );
        let expected_units = if entity.same_entity(
            crate::safety::ToctouGuard::capture(&alternate)
                .expect("alternate identity")
                .entity(),
        ) {
            1
        } else {
            2
        };
        assert_eq!(categories[0].items.len(), expected_units);
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
            &LifecycleProviderRegistry::new(Vec::new()),
            &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
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
            &LifecycleProviderRegistry::new(Vec::new()),
            &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
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
            &LifecycleProviderRegistry::new(Vec::new()),
            &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
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
            &LifecycleProviderRegistry::new(Vec::new()),
            &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
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
                ScanEvent::RootStarted { .. } => "root_started",
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
                // Each root is named before it is read, which is what lets the
                // interface show where a long scan is.
                "root_started",
                "item_found",
                "root_started",
                "item_found",
                "category_finished",
                "finished",
            ],
            "an item-level measurement gap is never a destructive scan event"
        );
    }

    /// A subtree the walk cannot read is reported rather than silently
    /// absent: what it could not measure stays visible as a skipped entry with
    /// a reason, and the scan does not claim a complete observation.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_subtree_is_reported_rather_than_silently_missing() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("partial-cache");
        std::fs::create_dir_all(root.join("open")).unwrap();
        std::fs::write(root.join("open/data.bin"), vec![1u8; 4_096]).unwrap();
        let closed = root.join("closed");
        std::fs::create_dir_all(closed.join("inner")).unwrap();
        std::fs::write(closed.join("inner/hidden.bin"), vec![2u8; 4_096]).unwrap();
        std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o000)).unwrap();

        let mut registry = SignatureRegistry::new();
        registry.register(signature(
            "test.partial",
            "Partly readable cache",
            Category::Developer,
            &root,
            vec![],
            None,
        ));

        let result = ScanEngine::scan(
            &registry,
            &LifecycleProviderRegistry::new(Vec::new()),
            &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
            Some(&[Category::Developer]),
            &[],
            false,
            &scan_environment(),
            &crate::models::NeverCancelled,
            |_| {},
        );

        // Restore before the fixture is dropped so the temporary tree can be
        // removed with it.
        let _ = std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o755));

        assert!(
            result.skipped_entry_count >= 1,
            "the unreadable directory is counted, not dropped: {result:?}"
        );
        assert_eq!(
            result.quality,
            ObservationQuality::Partial,
            "a scan that could not read part of a tree says so"
        );
        assert!(
            !result.gaps.is_empty(),
            "a partial observation carries a typed gap instead of only prose"
        );
        assert!(
            !result.cancelled,
            "an unreadable subtree is not a cancellation"
        );
        let item = result.categories[0]
            .items
            .iter()
            .find(|item| item.signature_id == "test.partial")
            .expect("the partially readable root is still reported");
        assert!(
            item.incomplete_reason.is_some(),
            "the item states why it is incomplete: {item:?}"
        );
    }

    #[test]
    fn cancellation_inside_the_final_signature_stops_before_the_next_root_and_sets_the_flag() {
        struct Probe {
            cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
        }

        impl crate::models::CancellationProbe for Probe {
            fn is_cancelled(&self) -> bool {
                self.cancelled.load(std::sync::atomic::Ordering::SeqCst)
            }
        }

        let fixture = tempfile::tempdir().unwrap();
        let first_root = fixture.path().join("first-root");
        let second_root = fixture.path().join("second-root");
        std::fs::create_dir_all(&first_root).unwrap();
        std::fs::create_dir_all(&second_root).unwrap();
        std::fs::write(first_root.join("data.bin"), vec![1u8; 128]).unwrap();
        std::fs::write(second_root.join("data.bin"), vec![2u8; 128]).unwrap();

        let mut sig = signature(
            "test.cancel.final-signature",
            "Final signature",
            Category::Developer,
            &first_root,
            vec![],
            None,
        );
        sig.paths = vec![
            first_root.to_string_lossy().into_owned(),
            second_root.to_string_lossy().into_owned(),
        ];

        let mut registry = SignatureRegistry::new();
        registry.register(sig);

        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probe = Probe {
            cancelled: cancelled.clone(),
        };
        let mut roots = Vec::new();

        let result = ScanEngine::scan(
            &registry,
            &LifecycleProviderRegistry::new(Vec::new()),
            &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
            Some(&[Category::Developer]),
            &[],
            false,
            &scan_environment(),
            &probe,
            |event| {
                if let ScanEvent::RootStarted { root, .. } = event {
                    roots.push(root);
                    cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            },
        );

        assert!(
            result.cancelled,
            "a stop inside the final signature must survive into the final result"
        );
        assert_eq!(
            roots.len(),
            1,
            "once cancellation is observed, the signature must not start another root"
        );
    }

    #[test]
    fn traversal_metrics_count_each_root_once() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("one-root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("one-file.bin"), vec![1u8; 128]).unwrap();

        let mut registry = SignatureRegistry::new();
        registry.register(signature(
            "test.metrics.single-root",
            "Single root",
            Category::Developer,
            &root,
            vec![],
            None,
        ));

        let result = ScanEngine::scan(
            &registry,
            &LifecycleProviderRegistry::new(Vec::new()),
            &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
            Some(&[Category::Developer]),
            &[],
            false,
            &scan_environment(),
            &crate::models::NeverCancelled,
            |_| {},
        );

        assert_eq!(
            result.metrics.visited_entries, 2,
            "one directory root plus one file is two visited filesystem entries"
        );
        assert_eq!(result.metrics.directories_read, 1);
    }

    #[test]
    fn aged_traversal_metrics_count_each_entry_once() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("aged-root");
        let child = root.join("child");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(child.join("one-file.bin"), vec![1u8; 128]).unwrap();

        let mut registry = SignatureRegistry::new();
        registry.register(signature(
            "test.metrics.aged-root",
            "Aged root",
            Category::Developer,
            &root,
            vec![],
            Some(0),
        ));

        let result = ScanEngine::scan(
            &registry,
            &LifecycleProviderRegistry::new(Vec::new()),
            &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
            Some(&[Category::Developer]),
            &[],
            false,
            &scan_environment(),
            &crate::models::NeverCancelled,
            |_| {},
        );

        assert_eq!(
            result.metrics.visited_entries, 3,
            "one root, one child directory, and one file are three visited entries"
        );
        assert_eq!(result.metrics.directories_read, 2);
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
            &LifecycleProviderRegistry::new(Vec::new()),
            &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
            None,
            &[],
            false,
            &scan_environment(),
            &AlwaysCancelled,
            |_| {},
        );

        assert_eq!(result.quality, ObservationQuality::Partial);
        assert!(
            result.cancelled,
            "a cancelled scan says so as a fact, not only in prose"
        );
        assert!(result
            .gaps
            .iter()
            .any(|gap| gap.kind == crate::models::ScanGapKind::Cancelled));
        assert!(result
            .incomplete_reasons
            .iter()
            .any(|reason| reason.contains("cancelled")));
    }

    /// A wide tree scans to the same result twice, and the run states what its
    /// traversal did: entries visited, directories read, and the peak number of
    /// directory tasks it kept outstanding, which never exceeds the stated
    /// bound.
    #[test]
    fn a_wide_tree_scans_deterministically_within_its_stated_bound() {
        const DIRECTORIES: usize = 32;
        const FILES_PER_DIRECTORY: usize = 2;

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("wide-cache");
        for directory in 0..DIRECTORIES {
            let path = root.join(format!("namespace-{directory:03}"));
            std::fs::create_dir_all(&path).unwrap();
            for file in 0..FILES_PER_DIRECTORY {
                std::fs::write(path.join(format!("entry-{file}.bin")), vec![4u8; 2_048]).unwrap();
            }
        }

        let mut registry = SignatureRegistry::new();
        registry.register(signature(
            "test.wide",
            "Wide cache",
            Category::Developer,
            &root,
            vec![],
            None,
        ));

        let providers = LifecycleProviderRegistry::new(Vec::new());
        let scan = || {
            ScanEngine::scan(
                &registry,
                &providers,
                &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
                Some(&[Category::Developer]),
                &[],
                false,
                &scan_environment(),
                &crate::models::NeverCancelled,
                |_| {},
            )
        };
        let first = scan();
        let second = scan();

        let summary = |result: &crate::models::ScanResult| {
            result.categories[0]
                .items
                .iter()
                .map(|item| (item.id.clone(), item.observed_bytes()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            summary(&first),
            summary(&second),
            "two scans of one tree state the same items in the same order"
        );
        assert_eq!(first.total_bytes, second.total_bytes);
        // The counts describe the tree, so they repeat; the duration and the
        // peak task count describe one run and are compared to the bound
        // instead of to each other.
        assert_eq!(
            first.metrics.visited_entries,
            second.metrics.visited_entries
        );
        assert_eq!(
            first.metrics.directories_read,
            second.metrics.directories_read
        );
        assert!(
            !first.cancelled && !second.cancelled,
            "nothing cancelled these scans, and the result says so"
        );

        let limits = crate::scanner::ScanLimits::default();
        assert!(first.metrics.visited_entries > 0);
        assert!(first.metrics.directories_read > 0);
        assert!(
            first.metrics.peak_outstanding_directory_tasks
                <= limits.max_concurrent_directory_reads as u64,
            "the traversal stays inside its stated bound: {:?}",
            first.metrics
        );
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
            &LifecycleProviderRegistry::new(Vec::new()),
            &crate::cleaner::OwnerProviderRegistry::new(Vec::new()),
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
