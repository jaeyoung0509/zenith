use crate::cache_providers::CacheProviderRegistry;
use crate::docker::DockerAdapter;
use crate::execution_budget::shared_scan_pool;
use crate::models::{
    Category, CategoryResult, ObservationQuality, RiskTier, ScanEvent, ScanItem, ScanResult,
};
use crate::orbstack::OrbStackAdapter;
use crate::platform::PlatformEnvironment;
use crate::scanner::DirectoryScanner;
use crate::signatures::SignatureRegistry;
use std::time::SystemTime;
use uuid::Uuid;

pub struct ScanEngine;

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
#[derive(Default)]
struct CategoryAccumulator {
    items: Vec<ScanItem>,
    total_bytes: u64,
    cleanable_bytes: u64,
    safe_bytes: u64,
    rebuild_bytes: u64,
    manual_bytes: u64,
}

impl CategoryAccumulator {
    /// Accounts one scanned item, or drops it when it is not worth retaining.
    ///
    /// Returns the retained item so the caller can stream it to the frontend.
    fn push(&mut self, mut item: ScanItem) -> Option<&ScanItem> {
        item.disposition = item.derive_disposition();
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
        // Reuse the explicitly bounded shared scan pool (see
        // `execution_budget`); never expand pools per request.
        let directory_pool = shared_scan_pool();

        for &category in target_categories {
            if cancellation.is_cancelled() {
                was_cancelled = true;
                break;
            }

            on_event(ScanEvent::CategoryStarted { category });

            let mut accumulator = CategoryAccumulator::default();

            // 1. Scan filesystem signatures for this category
            let signatures = registry.by_category_for_mode(category, intensive_cleanup);
            for sig in signatures {
                if cancellation.is_cancelled() {
                    was_cancelled = true;
                    break;
                }
                if excluded_signatures.iter().any(|id| id == &sig.id) {
                    continue;
                }
                let items =
                    DirectoryScanner::scan_signature_with_pool(sig, directory_pool, environment);
                for item in items {
                    if let Some(retained) = accumulator.push(item) {
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
                    if let Some(retained) = accumulator.push(item) {
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
                    if let Some(retained) = accumulator.push(item) {
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
            total_bytes += category_result.total_bytes;
            cleanable_bytes += category_result.cleanable_bytes;
            safe_bytes += category_result.safe_bytes;
            rebuild_bytes += category_result.rebuild_bytes;
            manual_bytes += category_result.manual_bytes;
            skipped_entry_count += category_result.skipped_entry_count;
            incomplete_item_count += category_result.incomplete_item_count;

            if !was_cancelled {
                on_event(ScanEvent::CategoryFinished {
                    category,
                    bytes: category_result.total_bytes,
                    item_count: category_result.items.len(),
                });
            }
            category_results.push(category_result);

            if was_cancelled {
                break;
            }
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
        };

        on_event(ScanEvent::Finished {
            result: result.clone(),
        });

        result
    }
}

#[cfg(test)]
mod tests {
    use super::{aggregate_quality, ScanEngine};
    use crate::models::{
        Category, CleanStrategy, ObservationQuality, RiskTier, ScanEvent, Signature,
    };
    use crate::platform::path_algebra::PathFlavor;
    use crate::platform::PlatformEnvironment;
    use crate::signatures::SignatureRegistry;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
        struct CancelAfter {
            calls: AtomicUsize,
            after: usize,
        }

        impl crate::models::CancellationProbe for CancelAfter {
            fn is_cancelled(&self) -> bool {
                self.calls.fetch_add(1, Ordering::SeqCst) >= self.after
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

        // Probe calls occur at the category boundary and before each
        // signature. Allow the first signature, then cancel before the second.
        let cancellation = CancelAfter {
            calls: AtomicUsize::new(0),
            after: 2,
        };
        let result = ScanEngine::scan(
            &registry,
            Some(&[Category::Developer]),
            &[],
            false,
            &scan_environment(),
            &cancellation,
            |_| {},
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
