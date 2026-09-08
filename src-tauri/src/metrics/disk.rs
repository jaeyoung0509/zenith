use crate::models::{DiskMetrics, DiskVolume, ZenithError};
use sysinfo::Disks;

fn primary_mount() -> Option<String> {
    #[cfg(windows)]
    {
        std::env::var("SystemDrive")
            .ok()
            .and_then(|drive| windows_drive_root(&drive))
    }
    #[cfg(not(windows))]
    {
        Some("/".to_string())
    }
}

#[cfg(any(windows, test))]
fn windows_drive_root(drive: &str) -> Option<String> {
    let drive = drive.trim_end_matches(['\\', '/']);
    let bytes = drive.as_bytes();
    (bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        .then(|| format!("{}:\\", (bytes[0] as char).to_ascii_uppercase()))
}

fn same_mount(left: &str, right: &str) -> bool {
    left.trim_end_matches(['\\', '/'])
        .eq_ignore_ascii_case(right.trim_end_matches(['\\', '/']))
}

pub struct DiskMetricsCollector;

impl DiskMetricsCollector {
    pub fn get_volumes() -> Vec<DiskVolume> {
        let disks = Disks::new_with_refreshed_list();
        let primary = primary_mount();
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
                    is_primary: primary
                        .as_ref()
                        .is_some_and(|root| same_mount(&mount_point, root)),
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
        let volume = Self::get_volumes()
            .into_iter()
            .find(|volume| volume.is_primary)
            .ok_or_else(|| {
                ZenithError::Io("Could not resolve the system disk volume".to_string())
            })?;
        Ok(DiskMetrics {
            mount_point: volume.mount_point,
            total_bytes: volume.total_bytes,
            used_bytes: volume.used_bytes,
            free_bytes: volume.available_bytes,
            available_bytes: volume.available_bytes,
            percent_used: volume.percent_used,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_drive_is_not_assumed_to_be_c_or_the_first_disk() {
        let root = windows_drive_root("d:").unwrap();
        let mounts = ["C:\\", "E:\\", "D:\\"];
        assert_eq!(
            mounts.iter().position(|mount| same_mount(mount, &root)),
            Some(2)
        );
        assert_eq!(windows_drive_root("D:/"), Some("D:\\".into()));
        assert!(windows_drive_root("D:\\Users\\다른 계정").is_none());
        assert!(windows_drive_root("").is_none());
    }
}
