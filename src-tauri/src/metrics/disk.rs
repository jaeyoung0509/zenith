use crate::models::{DiskMetrics, ZenithError};
use sysinfo::Disks;

pub struct DiskMetricsCollector;

impl DiskMetricsCollector {
    /// Queries the primary root disk usage.
    pub fn get_primary_disk() -> Result<DiskMetrics, ZenithError> {
        let disks = Disks::new_with_refreshed_list();

        // Prefer root "/" disk
        for disk in &disks {
            if disk.mount_point() == std::path::Path::new("/") {
                let total = disk.total_space();
                let available = disk.available_space();
                let used = total.saturating_sub(available);
                let percent = if total > 0 {
                    (used as f64 / total as f64) * 100.0
                } else {
                    0.0
                };

                return Ok(DiskMetrics {
                    mount_point: "/".to_string(),
                    total_bytes: total,
                    used_bytes: used,
                    free_bytes: available,
                    available_bytes: available,
                    percent_used: (percent * 10.0).round() / 10.0,
                });
            }
        }

        // Fallback to first disk
        if let Some(disk) = disks.first() {
            let total = disk.total_space();
            let available = disk.available_space();
            let used = total.saturating_sub(available);
            let percent = if total > 0 {
                (used as f64 / total as f64) * 100.0
            } else {
                0.0
            };

            return Ok(DiskMetrics {
                mount_point: disk.mount_point().to_string_lossy().to_string(),
                total_bytes: total,
                used_bytes: used,
                free_bytes: available,
                available_bytes: available,
                percent_used: (percent * 10.0).round() / 10.0,
            });
        }

        Err(ZenithError::Io("No disk volume found".to_string()))
    }
}
