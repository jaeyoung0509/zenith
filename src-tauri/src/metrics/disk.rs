use crate::models::{DiskMetrics, DiskVolume, ZenithError};
use sysinfo::Disks;

pub struct DiskMetricsCollector;

impl DiskMetricsCollector {
    pub fn get_volumes() -> Vec<DiskVolume> {
        let disks = Disks::new_with_refreshed_list();
        let mut volumes: Vec<_> = disks
            .iter()
            .filter(|disk| disk.total_space() > 0)
            .map(|disk| {
                let total = disk.total_space();
                let available = disk.available_space();
                let used = total.saturating_sub(available);
                let mount_point = disk.mount_point().to_string_lossy().into_owned();
                DiskVolume {
                    name: disk.name().to_string_lossy().into_owned(),
                    mount_point: mount_point.clone(),
                    file_system: disk.file_system().to_string_lossy().into_owned(),
                    disk_type: format!("{:?}", disk.kind()),
                    total_bytes: total,
                    used_bytes: used,
                    available_bytes: available,
                    percent_used: if total > 0 {
                        ((used as f64 / total as f64) * 1_000.0).round() / 10.0
                    } else {
                        0.0
                    },
                    is_removable: disk.is_removable(),
                    is_primary: mount_point == "/",
                }
            })
            .collect();

        volumes.sort_by(|left, right| {
            right
                .is_primary
                .cmp(&left.is_primary)
                .then_with(|| left.mount_point.cmp(&right.mount_point))
        });
        volumes
    }

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
