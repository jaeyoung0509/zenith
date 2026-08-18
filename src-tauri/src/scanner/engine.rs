use crate::docker::DockerAdapter;
use crate::models::{Category, CategoryResult, RiskTier, ScanEvent, ScanItem, ScanResult};
use crate::scanner::DirectoryScanner;
use crate::signatures::SignatureRegistry;
use std::time::SystemTime;
use uuid::Uuid;

pub struct ScanEngine;

impl ScanEngine {
    /// Executes a full or filtered scan across all categories, emitting streaming events.
    pub fn scan<F>(
        registry: &SignatureRegistry,
        categories_filter: Option<&[Category]>,
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
            Category::Model,
            Category::System,
        ]);

        let mut category_results = Vec::new();
        let mut total_bytes = 0u64;
        let mut safe_bytes = 0u64;
        let mut rebuild_bytes = 0u64;
        let mut manual_bytes = 0u64;

        for &category in target_categories {
            on_event(ScanEvent::CategoryStarted { category });

            let mut category_items: Vec<ScanItem> = Vec::new();
            let mut category_total_bytes = 0u64;
            let mut cat_safe = 0u64;
            let mut cat_rebuild = 0u64;
            let mut cat_manual = 0u64;

            // 1. Scan filesystem signatures for this category
            let signatures = registry.by_category(category);
            for sig in signatures {
                let items = DirectoryScanner::scan_signature(sig);
                for item in items {
                    let bytes = item.size.reclaimable();
                    category_total_bytes += bytes;

                    match item.risk {
                        RiskTier::Safe => cat_safe += bytes,
                        RiskTier::Rebuild => cat_rebuild += bytes,
                        RiskTier::Manual => cat_manual += bytes,
                    }

                    on_event(ScanEvent::ItemFound { item: item.clone() });
                    category_items.push(item);
                }
            }

            // 2. Special handling for Docker if category is Container
            if category == Category::Container {
                let docker_items = DockerAdapter::scan_items();
                for item in docker_items {
                    let bytes = item.size.reclaimable();
                    category_total_bytes += bytes;

                    match item.risk {
                        RiskTier::Safe => cat_safe += bytes,
                        RiskTier::Rebuild => cat_rebuild += bytes,
                        RiskTier::Manual => cat_manual += bytes,
                    }

                    on_event(ScanEvent::ItemFound { item: item.clone() });
                    category_items.push(item);
                }
            }

            total_bytes += category_total_bytes;
            safe_bytes += cat_safe;
            rebuild_bytes += cat_rebuild;
            manual_bytes += cat_manual;

            let cat_item_count = category_items.len();
            category_results.push(CategoryResult {
                category,
                display_name: category.display_name().to_string(),
                items: category_items,
                total_bytes: category_total_bytes,
                safe_bytes: cat_safe,
                rebuild_bytes: cat_rebuild,
                manual_bytes: cat_manual,
            });

            on_event(ScanEvent::CategoryFinished {
                category,
                bytes: category_total_bytes,
                item_count: cat_item_count,
            });
        }

        let finished_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let result = ScanResult {
            scan_id,
            started_at,
            finished_at,
            categories: category_results,
            total_bytes,
            safe_bytes,
            rebuild_bytes,
            manual_bytes,
        };

        on_event(ScanEvent::Finished {
            result: result.clone(),
        });

        result
    }
}
