use crate::docker::DockerAdapter;
use crate::models::{Category, CategoryResult, RiskTier, ScanEvent, ScanItem, ScanResult};
use crate::scanner::DirectoryScanner;
use crate::signatures::SignatureRegistry;
use rayon::{ThreadPool, ThreadPoolBuilder};
use std::sync::OnceLock;
use std::time::SystemTime;
use uuid::Uuid;

pub struct ScanEngine;

fn bounded_worker_count(available: usize, performance_cores: Option<usize>) -> usize {
    performance_cores
        .filter(|count| *count > 0)
        .unwrap_or(available)
        .min(available)
        .clamp(1, 4)
}

#[cfg(target_os = "macos")]
fn performance_core_count() -> Option<usize> {
    let mut count: libc::c_uint = 0;
    let mut size = std::mem::size_of_val(&count);
    // SAFETY: both output pointers reference initialized, correctly sized
    // storage and the sysctl name is statically NUL-terminated.
    let status = unsafe {
        libc::sysctlbyname(
            c"hw.perflevel0.physicalcpu".as_ptr(),
            (&mut count as *mut libc::c_uint).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (status == 0 && count > 0).then_some(count as usize)
}

#[cfg(not(target_os = "macos"))]
fn performance_core_count() -> Option<usize> {
    None
}

fn directory_scan_pool() -> Option<&'static ThreadPool> {
    static POOL: OnceLock<Option<ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        let available = std::thread::available_parallelism()
            .map(|available| available.get())
            .unwrap_or(1);
        let workers = bounded_worker_count(available, performance_core_count());
        if workers < 2 {
            return None;
        }
        ThreadPoolBuilder::new()
            .num_threads(workers)
            .thread_name(|index| format!("zenith-scan-{index}"))
            .build()
            .ok()
    })
    .as_ref()
}

impl ScanEngine {
    /// Executes a full or filtered scan across all categories, emitting streaming events.
    pub fn scan<F>(
        registry: &SignatureRegistry,
        categories_filter: Option<&[Category]>,
        excluded_signatures: &[String],
        intensive_cleanup: bool,
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
        let mut safe_bytes = 0u64;
        let mut rebuild_bytes = 0u64;
        let mut manual_bytes = 0u64;
        let directory_pool = directory_scan_pool();

        for &category in target_categories {
            on_event(ScanEvent::CategoryStarted { category });

            let mut category_items: Vec<ScanItem> = Vec::new();
            let mut category_total_bytes = 0u64;
            let mut cat_safe = 0u64;
            let mut cat_rebuild = 0u64;
            let mut cat_manual = 0u64;

            // 1. Scan filesystem signatures for this category
            let signatures = registry.by_category_for_mode(category, intensive_cleanup);
            for sig in signatures {
                if excluded_signatures.iter().any(|id| id == &sig.id) {
                    continue;
                }
                let items = DirectoryScanner::scan_signature_with_pool(sig, directory_pool);
                for item in items {
                    let bytes = item.size.reclaimable();
                    if !item.exists || bytes == 0 {
                        continue;
                    }
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

            category_items.sort_by(|left, right| {
                right
                    .size
                    .reclaimable()
                    .cmp(&left.size.reclaimable())
                    .then_with(|| left.name.cmp(&right.name))
            });

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

#[cfg(test)]
mod tests {
    use super::bounded_worker_count;

    #[test]
    fn scan_workers_follow_performance_core_cap() {
        assert_eq!(bounded_worker_count(8, Some(4)), 4);
        assert_eq!(bounded_worker_count(6, Some(3)), 3);
        assert_eq!(bounded_worker_count(4, Some(2)), 2);
        assert_eq!(bounded_worker_count(2, None), 2);
        assert_eq!(bounded_worker_count(1, None), 1);
        assert_eq!(bounded_worker_count(2, Some(8)), 2);
    }
}
