use crate::models::{DiskMetrics, DiskVolume, ZenithError};
use crate::platform::description::PlatformEnvironment;
use sysinfo::Disks;

/// The primary mount point of the described platform.
///
/// Windows does not guarantee that the system drive is `C:` or the first disk
/// the OS enumerates, so the drive is derived from the environment's stated
/// system roots (ProgramData, then Program Files) instead of being assumed.
fn primary_mount(environment: &PlatformEnvironment) -> Option<String> {
    if environment.flavor().is_windows() {
        let system_root = environment
            .program_data()
            .or_else(|| environment.program_files())?;
        let text = system_root.to_string_lossy();
        let drive = text.split(['\\', '/']).next().unwrap_or_default();
        windows_drive_root(drive)
    } else {
        Some("/".to_string())
    }
}

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

/// One reported volume and the file identity the platform gave it.
///
/// `DiskVolume` is the frontend contract and carries no identity, so the
/// identity is reported alongside it. `None` is a fact — a network share, a
/// FUSE mount, or a virtual filesystem exposes no stable identifier — and is
/// never synthesized from `mount_point`, because the identifier exists to
/// detect that a mount was replaced.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeReport {
    pub volume: DiskVolume,
    pub identity: Option<String>,
}

pub struct DiskMetricsCollector;

impl DiskMetricsCollector {
    /// Volumes as the environment states them, or the OS list when the
    /// environment states none. Native runs keep the OS path exactly.
    pub fn get_volumes(environment: &PlatformEnvironment) -> Vec<DiskVolume> {
        Self::get_volume_reports(environment)
            .into_iter()
            .map(|report| report.volume)
            .collect()
    }

    /// Volumes with their file identity, for callers that need to detect that a
    /// mount was replaced.
    pub fn get_volume_reports(environment: &PlatformEnvironment) -> Vec<VolumeReport> {
        let primary = primary_mount(environment);
        // The OS list is read once; it supplies sizes for stated volumes whose
        // mount point the platform can measure, and is the whole list when the
        // environment states none.
        let disks = Disks::new_with_refreshed_list();
        let mut reports: Vec<VolumeReport> = match environment.volumes() {
            Some(stated) => stated
                .iter()
                .map(|volume| {
                    let mount_point = volume.mount_point.to_string_lossy().into_owned();
                    let measured = os_measurement(&disks, &mount_point);
                    VolumeReport {
                        volume: DiskVolume {
                            name: volume.name.clone(),
                            mount_point: mount_point.clone(),
                            file_system: volume.filesystem.clone().unwrap_or_default(),
                            disk_type: String::new(),
                            total_bytes: measured.map_or(0, |(total, _, _)| total),
                            used_bytes: measured.map_or(0, |(_, used, _)| used),
                            available_bytes: measured.map_or(0, |(_, _, available)| available),
                            percent_used: measured
                                .map_or(0.0, |(total, used, _)| percent(total, used)),
                            is_removable: false,
                            is_primary: primary
                                .as_ref()
                                .is_some_and(|root| same_mount(&mount_point, root)),
                        },
                        identity: volume.id.clone(),
                    }
                })
                .collect(),
            None => disks
                .iter()
                .filter(|disk| disk.total_space() > 0)
                .map(|disk| {
                    let total = disk.total_space();
                    let available = disk.available_space();
                    let used = total.saturating_sub(available);
                    let mount_point = disk.mount_point().to_string_lossy().into_owned();
                    VolumeReport {
                        volume: DiskVolume {
                            name: disk.name().to_string_lossy().into_owned(),
                            mount_point: mount_point.clone(),
                            file_system: disk.file_system().to_string_lossy().into_owned(),
                            disk_type: format!("{:?}", disk.kind()),
                            total_bytes: total,
                            used_bytes: used,
                            available_bytes: available,
                            percent_used: percent(total, used),
                            is_removable: disk.is_removable(),
                            is_primary: primary
                                .as_ref()
                                .is_some_and(|root| same_mount(&mount_point, root)),
                        },
                        identity: None,
                    }
                })
                .collect(),
        };

        reports.sort_by(|left, right| {
            right
                .volume
                .is_primary
                .cmp(&left.volume.is_primary)
                .then_with(|| left.volume.mount_point.cmp(&right.volume.mount_point))
        });
        reports
    }

    /// Queries the primary root disk usage.
    pub fn get_primary_disk(environment: &PlatformEnvironment) -> Result<DiskMetrics, ZenithError> {
        let volume = Self::get_volume_reports(environment)
            .into_iter()
            .map(|report| report.volume)
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

/// Space facts the OS exposes for a stated mount point, when it knows one.
fn os_measurement(disks: &Disks, mount_point: &str) -> Option<(u64, u64, u64)> {
    disks
        .iter()
        .find(|disk| same_mount(&disk.mount_point().to_string_lossy(), mount_point))
        .map(|disk| {
            let total = disk.total_space();
            let available = disk.available_space();
            (total, total.saturating_sub(available), available)
        })
}

fn percent(total: u64, used: u64) -> f64 {
    if total > 0 {
        ((used as f64 / total as f64) * 1_000.0).round() / 10.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::description::VolumeIdentity;
    use crate::platform::path_algebra::PathFlavor;
    use crate::platform::paths::SimulatedPaths;
    use std::path::PathBuf;
    use std::sync::Arc;

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

    #[test]
    fn a_non_system_drive_is_the_primary_volume_when_the_environment_states_it() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Windows)
            .with_roots(Arc::new(
                SimulatedPaths::new()
                    .with_flavor(PathFlavor::Windows)
                    .with_program_data(r"D:\ProgramData"),
            ))
            .with_volumes(vec![
                VolumeIdentity {
                    name: "cd".to_string(),
                    mount_point: PathBuf::from("C:\\"),
                    filesystem: Some("NTFS".to_string()),
                    id: Some("vol-c".to_string()),
                },
                VolumeIdentity {
                    name: "system".to_string(),
                    mount_point: PathBuf::from("D:\\"),
                    filesystem: Some("NTFS".to_string()),
                    id: Some("vol-d".to_string()),
                },
            ]);

        let reports = DiskMetricsCollector::get_volume_reports(&environment);

        assert_eq!(primary_mount(&environment), Some("D:\\".to_string()));
        assert_eq!(reports.len(), 2);
        // The primary volume is the one the stated system drive names, not the
        // first enumerated volume and not `C:`.
        assert!(reports[0].volume.is_primary);
        assert_eq!(reports[0].volume.mount_point, r"D:\");
        assert!(!reports[1].volume.is_primary);
        assert_eq!(
            DiskMetricsCollector::get_primary_disk(&environment)
                .expect("stated primary volume")
                .mount_point,
            r"D:\"
        );
    }

    #[test]
    fn a_volume_without_a_file_identity_is_reported_as_having_none() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Posix).with_volumes(vec![
            VolumeIdentity {
                name: "network".to_string(),
                mount_point: PathBuf::from("/Volumes/work"),
                filesystem: None,
                id: None,
            },
            VolumeIdentity {
                name: "data".to_string(),
                mount_point: PathBuf::from("/Volumes/data"),
                filesystem: Some("apfs".to_string()),
                id: Some("4a1b-9f".to_string()),
            },
        ]);

        let reports = DiskMetricsCollector::get_volume_reports(&environment);
        assert_eq!(reports.len(), 2);
        let network = &reports[1];
        assert_eq!(network.volume.mount_point, "/Volumes/work");
        assert_eq!(network.volume.name, "network");
        assert_eq!(network.volume.file_system, "");
        assert!(
            network.identity.is_none(),
            "an unpublished identity must stay absent"
        );
        assert_ne!(network.identity.as_deref(), Some("/Volumes/work"));
        assert_eq!(reports[0].identity.as_deref(), Some("4a1b-9f"));
    }

    #[test]
    fn a_stated_volume_set_replaces_the_os_volume_list() {
        let stated =
            PlatformEnvironment::simulated(PathFlavor::Posix).with_volumes(vec![VolumeIdentity {
                name: "only".to_string(),
                mount_point: PathBuf::from("/definitely-not-mounted"),
                filesystem: None,
                id: None,
            }]);
        let reports = DiskMetricsCollector::get_volume_reports(&stated);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].volume.mount_point, "/definitely-not-mounted");

        // An unstated description falls back to the OS list, which cannot
        // contain a mount point the test invented.
        let unstated = PlatformEnvironment::simulated(PathFlavor::Posix);
        assert!(unstated.volumes().is_none());
        assert!(DiskMetricsCollector::get_volume_reports(&unstated)
            .iter()
            .all(|report| report.volume.mount_point != "/definitely-not-mounted"));
    }
}
