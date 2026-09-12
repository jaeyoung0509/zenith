use crate::models::{Category, FileSize, ObservationQuality, RiskTier, ScanItem};
use crate::platform::description::PlatformEnvironment;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

const ORBSTACK_STORAGE_PATH: &str =
    "Library/Group Containers/HUAQ24HBR6.dev.orbstack/data/data.img.raw";

pub struct OrbStackAdapter;

impl OrbStackAdapter {
    /// Reports OrbStack's stateful VM disk for visibility without making it cleanable.
    ///
    /// The reviewed path is a macOS group container, so this adapter reports
    /// nothing on any other platform rather than looking for the same literal
    /// folders under a different profile layout. The profile comes from the
    /// environment instead of the host.
    pub fn scan_items(environment: &PlatformEnvironment) -> Vec<ScanItem> {
        if !cfg!(target_os = "macos") {
            return Vec::new();
        }
        let Some(home) = environment.user_home() else {
            return Vec::new();
        };
        Self::scan_path(&Self::storage_path_for_home(&home))
            .into_iter()
            .collect()
    }

    fn storage_path_for_home(home: &Path) -> PathBuf {
        home.join(ORBSTACK_STORAGE_PATH)
    }

    fn scan_path(path: &Path) -> Option<ScanItem> {
        let metadata = fs::symlink_metadata(path).ok()?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return None;
        }

        let logical = metadata.len();
        #[cfg(unix)]
        let allocated = metadata.blocks().saturating_mul(512);
        #[cfg(not(unix))]
        let allocated = logical;

        if allocated == 0 {
            return None;
        }

        let last_modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs());

        Some(ScanItem {
            id: "container.orbstack.storage".to_string(),
            signature_id: "adapter.orbstack.storage".to_string(),
            name: "OrbStack VM Storage".to_string(),
            category: Category::Container,
            risk: RiskTier::Manual,
            path: path.to_string_lossy().to_string(),
            size: FileSize::new(logical, Some(allocated)),
            file_count: 1,
            description:
                "Active container and Linux VM data. Inspect or compact it in OrbStack; Zenith will not delete it."
                    .to_string(),
            cache_metadata: Default::default(),
            is_selected: false,
            last_modified,
            exists: true,
            quality: ObservationQuality::Fresh,
            incomplete_reason: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{OrbStackAdapter, ORBSTACK_STORAGE_PATH};
    #[cfg(unix)]
    use crate::models::{Category, RiskTier};
    use crate::platform::description::PlatformEnvironment;
    use crate::platform::path_algebra::PathFlavor;
    #[cfg(unix)]
    use std::fs::OpenOptions;
    #[cfg(unix)]
    use std::io::{Seek, SeekFrom, Write};

    fn profile_with_storage() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path().join("profile");
        let storage = home.join(ORBSTACK_STORAGE_PATH);
        std::fs::create_dir_all(storage.parent().unwrap()).unwrap();
        std::fs::write(&storage, b"data").unwrap();
        (fixture, home, storage)
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn storage_is_resolved_from_the_stated_profile() {
        let (_fixture, home, storage) = profile_with_storage();
        let environment = PlatformEnvironment::simulated(PathFlavor::Posix).with_home(&home);

        let items = OrbStackAdapter::scan_items(&environment);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, storage.to_string_lossy());

        // An unstated profile reports nothing instead of falling back to the
        // host profile.
        let unstated = PlatformEnvironment::simulated(PathFlavor::Posix);
        assert!(OrbStackAdapter::scan_items(&unstated).is_empty());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn other_platforms_report_no_orbstack_storage() {
        let (_fixture, home, _storage) = profile_with_storage();
        let environment = PlatformEnvironment::simulated(PathFlavor::Posix).with_home(&home);

        assert!(OrbStackAdapter::scan_items(&environment).is_empty());
    }

    #[test]
    fn resolves_only_the_reviewed_group_container_path() {
        let home = std::path::Path::new("/Users/tester");
        assert_eq!(
            OrbStackAdapter::storage_path_for_home(home),
            home.join(ORBSTACK_STORAGE_PATH)
        );
    }

    #[test]
    fn missing_and_empty_sparse_files_are_not_reported() {
        let fixture = tempfile::tempdir().unwrap();
        let missing = fixture.path().join("missing.img.raw");
        assert!(OrbStackAdapter::scan_path(&missing).is_none());

        let empty = fixture.path().join("empty.img.raw");
        std::fs::File::create(&empty).unwrap();
        assert!(OrbStackAdapter::scan_path(&empty).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn sparse_storage_reports_allocated_bytes_as_manual() {
        let fixture = tempfile::tempdir().unwrap();
        let disk = fixture.path().join("data.img.raw");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&disk)
            .unwrap();
        file.seek(SeekFrom::Start(16 * 1024 * 1024)).unwrap();
        file.write_all(&[1]).unwrap();
        file.sync_all().unwrap();

        let item = OrbStackAdapter::scan_path(&disk).expect("allocated sparse disk");
        let allocated = item.size.allocated.expect("allocated size");

        assert_eq!(item.category, Category::Container);
        assert_eq!(item.risk, RiskTier::Manual);
        assert!(!item.is_selected);
        assert!(allocated > 0);
        assert!(allocated < item.size.logical);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_storage_is_not_reported() {
        use std::os::unix::fs::symlink;

        let fixture = tempfile::tempdir().unwrap();
        let target = fixture.path().join("target.img.raw");
        std::fs::write(&target, b"data").unwrap();
        let linked = fixture.path().join("data.img.raw");
        symlink(&target, &linked).unwrap();

        assert!(OrbStackAdapter::scan_path(&linked).is_none());
    }
}
