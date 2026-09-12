use crate::models::{CacheSizeSemantics, FileSize, ObservationQuality, ScanItem, Signature};
use crate::platform::PlatformEnvironment;
use crate::scanner::{PathMeasurement, SizeCalculator};
use crate::signatures::SignatureLoader;
use rayon::ThreadPool;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub struct DirectoryScanner;

impl DirectoryScanner {
    /// Scans all configured paths for a given signature and returns discovered ScanItems.
    pub fn scan_signature(
        signature: &Signature,
        environment: &PlatformEnvironment,
    ) -> Vec<ScanItem> {
        Self::scan_signature_with_pool(signature, None, environment)
    }

    pub(crate) fn scan_signature_with_pool(
        signature: &Signature,
        pool: Option<&ThreadPool>,
        environment: &PlatformEnvironment,
    ) -> Vec<ScanItem> {
        let mut items = Vec::new();

        // If signature has no explicit file paths (e.g. Docker commands), return early or handle in Docker adapter
        if signature.paths.is_empty() {
            return items;
        }

        for (idx, pattern) in signature.paths.iter().enumerate() {
            let path_buf = match SignatureLoader::expand_path(pattern, environment) {
                Some(p) => p,
                None => continue,
            };

            if let Some(min_age_days) = signature.min_age_days {
                items.extend(Self::scan_aged_children(
                    environment,
                    signature,
                    &path_buf,
                    idx,
                    min_age_days,
                ));
                continue;
            }

            // `Path::exists()` collapses every metadata error into `false`.
            // Keep permission and I/O failures observable instead of treating
            // a configured path as if it simply did not exist.
            let (exists, measurement) = match fs::symlink_metadata(&path_buf) {
                Ok(metadata) if metadata.file_type().is_symlink() => (
                    true,
                    PathMeasurement::unavailable(format!(
                        "Configured path {} is a symlink; cleanup is blocked",
                        path_buf.display()
                    )),
                ),
                Ok(_) => (
                    true,
                    SizeCalculator::measure_path_with_pool(
                        &path_buf,
                        &signature.exclusions,
                        pool,
                        environment,
                    ),
                ),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    (false, PathMeasurement::complete(FileSize::default(), 0))
                }
                Err(err) => (
                    true,
                    PathMeasurement::unavailable(format!(
                        "Could not inspect configured path {}: {}",
                        path_buf.display(),
                        err
                    )),
                ),
            };

            let size = measurement.size;
            let file_count = measurement.file_count;
            let (quality, incomplete_reason) = if !exists || measurement.complete {
                (ObservationQuality::Fresh, None)
            } else if size.reclaimable() == 0 {
                (
                    ObservationQuality::Unavailable,
                    measurement
                        .incomplete_reason
                        .or_else(|| Some("Inaccessible path; read failed".into())),
                )
            } else {
                (
                    ObservationQuality::Partial,
                    measurement
                        .incomplete_reason
                        .or_else(|| Some("Incomplete scan; some entries could not be read".into())),
                )
            };

            let mut cache_metadata = signature.cache_metadata();
            if quality == ObservationQuality::Partial {
                cache_metadata.size_semantics = CacheSizeSemantics::ConservativeLowerBound;
            } else if quality == ObservationQuality::Unavailable {
                cache_metadata.size_semantics = CacheSizeSemantics::Informational;
            }

            let last_modified = if exists {
                fs::metadata(&path_buf)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
            } else {
                None
            };

            let item_id = if signature.paths.len() > 1 {
                format!("{}.{}", signature.id, idx)
            } else {
                signature.id.clone()
            };

            let display_name = if signature.paths.len() > 1 {
                format!(
                    "{} ({})",
                    signature.name,
                    path_buf
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(pattern)
                )
            } else {
                signature.name.clone()
            };

            // Only auto-select if RiskTier is Safe, size > 0, and quality is Fresh
            let is_selected = signature.risk.is_auto_selectable()
                && size.reclaimable() > 0
                && quality == ObservationQuality::Fresh;

            items.push(ScanItem {
                id: item_id,
                signature_id: signature.id.clone(),
                name: display_name,
                category: signature.category,
                risk: signature.risk,
                path: path_buf.to_string_lossy().to_string(),
                size,
                file_count,
                description: signature.description.clone(),
                cache_metadata,
                is_selected,
                last_modified,
                exists,
                quality,
                incomplete_reason,
            });
        }

        items
    }

    fn scan_aged_children(
        environment: &PlatformEnvironment,
        signature: &Signature,
        root: &std::path::Path,
        path_index: usize,
        min_age_days: u32,
    ) -> Vec<ScanItem> {
        match fs::symlink_metadata(root) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return vec![Self::unavailable_aged_item(
                    signature,
                    root,
                    format!("{}.{}.unavailable", signature.id, path_index),
                    signature.name.clone(),
                    FileSize::default(),
                    0,
                    None,
                    format!(
                        "Configured path {} is a symlink; cleanup is blocked",
                        root.display()
                    ),
                )];
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return vec![],
            Err(err) => {
                return vec![Self::unavailable_aged_item(
                    signature,
                    root,
                    format!("{}.{}.unavailable", signature.id, path_index),
                    signature.name.clone(),
                    FileSize::default(),
                    0,
                    None,
                    format!("Could not inspect {}: {}", root.display(), err),
                )];
            }
        }
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return vec![],
            Err(err) => {
                return vec![Self::unavailable_aged_item(
                    signature,
                    root,
                    format!("{}.{}.unavailable", signature.id, path_index),
                    signature.name.clone(),
                    FileSize::default(),
                    0,
                    None,
                    format!("Could not inspect {}: {}", root.display(), err),
                )];
            }
        };
        let minimum_age = Duration::from_secs(u64::from(min_age_days) * 86_400);
        let now = SystemTime::now();
        let mut items = Vec::new();
        let mut entry_failure = None;

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    if entry_failure.is_none() {
                        entry_failure = Some(format!(
                            "Could not read an entry in {}: {}",
                            root.display(),
                            err
                        ));
                    }
                    continue;
                }
            };
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !signature.include_prefixes.is_empty()
                && !signature
                    .include_prefixes
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
            {
                continue;
            }
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    items.push(Self::unavailable_aged_item(
                        signature,
                        &path,
                        format!("{}.{}.{}", signature.id, path_index, name),
                        name,
                        FileSize::default(),
                        0,
                        None,
                        format!(
                            "Candidate {} is a symlink; cleanup is blocked",
                            path.display()
                        ),
                    ));
                    continue;
                }
                Ok(_) => {}
                Err(err) => {
                    items.push(Self::unavailable_aged_item(
                        signature,
                        &path,
                        format!("{}.{}.{}", signature.id, path_index, name),
                        name,
                        FileSize::default(),
                        0,
                        None,
                        format!("Could not inspect {}: {}", path.display(), err),
                    ));
                    continue;
                }
            }
            if signature
                .exclude_prefixes
                .iter()
                .any(|prefix| name.starts_with(prefix))
            {
                continue;
            }

            // Single-pass fail-closed tree measurement
            let stats = Self::measure_tree_stats(environment, &path, &signature.exclusions, 0, 32);
            // An incomplete tree cannot prove the candidate's newest timestamp,
            // so retain it for observability but block cleanup.
            if !stats.complete {
                let size = FileSize::new(stats.logical, Some(stats.allocated));
                let last_modified = stats
                    .newest_mtime
                    .and_then(|modified| modified.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_secs());
                items.push(Self::unavailable_aged_item(
                    signature,
                    &path,
                    format!("{}.{}.{}", signature.id, path_index, name),
                    name,
                    size,
                    stats.file_count,
                    last_modified,
                    stats.incomplete_reason.unwrap_or_else(|| {
                        format!("Could not completely inspect {}", path.display())
                    }),
                ));
                continue;
            }

            let Some(modified) = stats.newest_mtime else {
                continue;
            };
            if now.duration_since(modified).unwrap_or_default() < minimum_age {
                continue;
            }

            let size = FileSize::new(stats.logical, Some(stats.allocated));
            if size.reclaimable() == 0 {
                continue;
            }

            let description = format!(
                "{} (unchanged for at least {} days)",
                signature.description, min_age_days
            );

            items.push(ScanItem {
                id: format!("{}.{}.{}", signature.id, path_index, name),
                signature_id: signature.id.clone(),
                name,
                category: signature.category,
                risk: signature.risk,
                path: path.to_string_lossy().to_string(),
                size,
                file_count: stats.file_count,
                description,
                cache_metadata: signature.cache_metadata(),
                is_selected: signature.risk.is_auto_selectable(),
                last_modified: modified
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .ok()
                    .map(|duration| duration.as_secs()),
                exists: true,
                quality: ObservationQuality::Fresh,
                incomplete_reason: None,
            });
        }

        if let Some(reason) = entry_failure {
            items.push(Self::unavailable_aged_item(
                signature,
                root,
                format!("{}.{}.unavailable", signature.id, path_index),
                format!("{} (scan incomplete)", signature.name),
                FileSize::default(),
                0,
                None,
                reason,
            ));
        }

        items
    }

    #[allow(clippy::too_many_arguments)]
    fn unavailable_aged_item(
        signature: &Signature,
        path: &Path,
        id: String,
        name: String,
        size: FileSize,
        file_count: usize,
        last_modified: Option<u64>,
        reason: String,
    ) -> ScanItem {
        let mut cache_metadata = signature.cache_metadata();
        cache_metadata.size_semantics = if size.reclaimable() == 0 {
            CacheSizeSemantics::Informational
        } else {
            CacheSizeSemantics::ConservativeLowerBound
        };
        ScanItem {
            id,
            signature_id: signature.id.clone(),
            name,
            category: signature.category,
            risk: signature.risk,
            path: path.to_string_lossy().into_owned(),
            size,
            file_count,
            description: format!(
                "{} Age eligibility could not be verified, so cleanup is blocked.",
                signature.description
            ),
            cache_metadata,
            is_selected: false,
            last_modified,
            exists: true,
            quality: ObservationQuality::Unavailable,
            incomplete_reason: Some(reason),
        }
    }

    /// Measures directory statistics (size, count, newest mtime) in a single recursive pass.
    /// Marks complete = false if any error, symlink escape, or depth cutoff occurs.
    pub fn measure_tree_stats(
        environment: &PlatformEnvironment,
        path: &Path,
        exclusions: &[String],
        current_depth: usize,
        max_depth: usize,
    ) -> TreeStats {
        let mut stats = TreeStats {
            logical: 0,
            allocated: 0,
            file_count: 0,
            newest_mtime: None,
            complete: true,
            incomplete_reason: None,
        };

        if current_depth > max_depth {
            stats.complete = false;
            stats.incomplete_reason = Some(format!(
                "Directory depth limit of {} exceeded at {}",
                max_depth,
                path.display()
            ));
            return stats;
        }

        let meta = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(err) => {
                stats.complete = false;
                stats.incomplete_reason = Some(format!(
                    "Failed to read metadata for {}: {}",
                    path.display(),
                    err
                ));
                return stats;
            }
        };

        // Signed app bundles are protected by macOS App Management (TCC):
        // chmod and deletion inside them fail with EPERM even for the owning
        // user, so generic cleanup can never succeed. Treat any bundle in the
        // tree like an unreadable subtree and fail closed.
        if meta.is_dir()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        {
            stats.complete = false;
            stats.incomplete_reason = Some(format!(
                "Protected application bundle encountered in {}",
                path.display()
            ));
            return stats;
        }

        if let Ok(modified) = meta.modified() {
            stats.newest_mtime = Some(match stats.newest_mtime {
                Some(existing) => existing.max(modified),
                None => modified,
            });
        }

        if meta.file_type().is_symlink() || meta.is_file() {
            let len = meta.len();
            stats.logical = len;
            #[cfg(unix)]
            {
                stats.allocated = meta.blocks() * 512;
            }
            #[cfg(windows)]
            {
                stats.allocated = crate::scanner::get_allocated_size(path).unwrap_or(len);
            }
            #[cfg(not(any(unix, windows)))]
            {
                stats.allocated = len;
            }
            stats.file_count = 1;
            return stats;
        }

        if !meta.is_dir() {
            return stats;
        }

        let entries = match fs::read_dir(path) {
            Ok(e) => e,
            Err(err) => {
                stats.complete = false;
                stats.incomplete_reason = Some(format!(
                    "Failed to read directory {}: {}",
                    path.display(),
                    err
                ));
                return stats;
            }
        };

        for entry in entries {
            let ent = match entry {
                Ok(e) => e,
                Err(err) => {
                    stats.complete = false;
                    if stats.incomplete_reason.is_none() {
                        stats.incomplete_reason = Some(format!(
                            "Failed to read entry in {}: {}",
                            path.display(),
                            err
                        ));
                    }
                    continue;
                }
            };
            let child_path = ent.path();

            if crate::safety::Blacklist::is_blacklisted_with(&child_path, environment) {
                continue;
            }

            let child_str = child_path.to_string_lossy();
            if exclusions.iter().any(|ex| {
                child_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n == ex)
                    .unwrap_or(false)
                    || child_str.contains(ex)
            }) {
                continue;
            }

            let sub_stats = Self::measure_tree_stats(
                environment,
                &child_path,
                exclusions,
                current_depth + 1,
                max_depth,
            );
            if !sub_stats.complete {
                stats.complete = false;
                if stats.incomplete_reason.is_none() {
                    stats.incomplete_reason = sub_stats.incomplete_reason;
                }
            }
            stats.logical += sub_stats.logical;
            stats.allocated += sub_stats.allocated;
            stats.file_count += sub_stats.file_count;
            if let Some(sub_mtime) = sub_stats.newest_mtime {
                stats.newest_mtime = Some(match stats.newest_mtime {
                    Some(existing) => existing.max(sub_mtime),
                    None => sub_mtime,
                });
            }
        }

        stats
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeStats {
    pub logical: u64,
    pub allocated: u64,
    pub file_count: usize,
    pub newest_mtime: Option<SystemTime>,
    pub complete: bool,
    pub incomplete_reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::DirectoryScanner;
    use crate::models::{Category, CleanStrategy, ObservationQuality, RiskTier, Signature};
    use crate::platform::path_algebra::PathFlavor;
    use crate::platform::PlatformEnvironment;

    fn environment() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::current())
    }

    #[test]
    fn aged_child_scan_excludes_protected_prefixes_and_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let eligible = root.path().join("third.party.cache");
        let protected = root.path().join("com.apple.protected");
        std::fs::create_dir(&eligible).unwrap();
        std::fs::create_dir(&protected).unwrap();
        std::fs::write(eligible.join("data.bin"), vec![1u8; 4096]).unwrap();
        std::fs::write(protected.join("data.bin"), vec![1u8; 4096]).unwrap();

        // Only a POSIX host can create the symlink; the prefix exclusion below
        // is asserted on every platform.
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = root.path().join("linked-cache");
            symlink(&eligible, &link).unwrap();
        }

        let signature = Signature {
            id: "system.test.intensive".into(),
            name: "Test intensive caches".into(),
            category: Category::System,
            risk: RiskTier::Safe,
            strategy: CleanStrategy::DeleteDirectory,
            paths: vec![root.path().to_string_lossy().into_owned()],
            exclusions: vec![],
            description: String::new(),
            min_age_days: Some(0),
            include_prefixes: vec![],
            exclude_prefixes: vec!["com.apple.".into()],
            intensive_only: true,
            platforms: vec![],
            provider: String::new(),
            management_mode: Default::default(),
            artifact_kind: Default::default(),
            consequence: String::new(),
            reclaimable_is_lower_bound: false,
        };

        let items = DirectoryScanner::scan_signature(&signature, &environment());
        let eligible_item = items
            .iter()
            .find(|item| item.name == "third.party.cache")
            .expect("eligible cache remains visible");
        assert_eq!(eligible_item.quality, ObservationQuality::Fresh);
        assert!(!items.iter().any(|item| item.name == "com.apple.protected"));
        #[cfg(unix)]
        {
            let symlink = items
                .iter()
                .find(|item| item.name == "linked-cache")
                .expect("eligible symlink is retained as a blocked observation");
            assert_eq!(symlink.quality, ObservationQuality::Unavailable);
            assert!(!symlink.is_selected);
            assert!(symlink
                .incomplete_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("symlink")));
        }
    }

    #[test]
    fn aged_scan_fails_closed_for_trees_containing_app_bundles() {
        let root = tempfile::tempdir().unwrap();
        let eligible = root.path().join("plain.cache");
        let nested = root.path().join("bundled.cache");
        let bundle_root = nested.join("Tool.app/Contents/MacOS");
        let mixed_case = root.path().join("mixed-case-bundled.cache");
        let mixed_case_bundle_root = mixed_case.join("Tool.App/Contents/MacOS");
        let standalone = root.path().join("Standalone.app/Contents/MacOS");
        std::fs::create_dir_all(&bundle_root).unwrap();
        std::fs::create_dir_all(&mixed_case_bundle_root).unwrap();
        std::fs::create_dir_all(&standalone).unwrap();
        std::fs::create_dir(&eligible).unwrap();
        std::fs::write(eligible.join("data.bin"), vec![1u8; 4096]).unwrap();
        std::fs::write(bundle_root.join("tool"), vec![1u8; 4096]).unwrap();
        std::fs::write(mixed_case_bundle_root.join("tool"), vec![1u8; 4096]).unwrap();
        std::fs::write(standalone.join("tool"), vec![1u8; 4096]).unwrap();

        let signature = Signature {
            id: "system.test.bundles".into(),
            name: "Test bundle guard".into(),
            category: Category::System,
            risk: RiskTier::Safe,
            strategy: CleanStrategy::DeleteDirectory,
            paths: vec![root.path().to_string_lossy().into_owned()],
            exclusions: vec![],
            description: String::new(),
            min_age_days: Some(0),
            include_prefixes: vec![],
            exclude_prefixes: vec![],
            intensive_only: true,
            platforms: vec![],
            provider: String::new(),
            management_mode: Default::default(),
            artifact_kind: Default::default(),
            consequence: String::new(),
            reclaimable_is_lower_bound: false,
        };

        let items = DirectoryScanner::scan_signature(&signature, &environment());
        let plain = items
            .iter()
            .find(|item| item.name == "plain.cache")
            .expect("complete cache remains visible");
        assert_eq!(plain.quality, ObservationQuality::Fresh);
        for name in [
            "bundled.cache",
            "mixed-case-bundled.cache",
            "Standalone.app",
        ] {
            let blocked = items
                .iter()
                .find(|item| item.name == name)
                .unwrap_or_else(|| panic!("{name} should be retained as an incomplete item"));
            assert_eq!(blocked.quality, ObservationQuality::Unavailable);
            assert!(!blocked.is_selected);
            assert!(blocked.incomplete_reason.is_some());
        }

        // The guard must also fail closed at delete-time TOCTOU re-verification.
        let stats = DirectoryScanner::measure_tree_stats(&environment(), &nested, &[], 0, 32);
        assert!(!stats.complete);
        let mixed_case_stats =
            DirectoryScanner::measure_tree_stats(&environment(), &mixed_case, &[], 0, 32);
        assert!(!mixed_case_stats.complete);
    }
}
