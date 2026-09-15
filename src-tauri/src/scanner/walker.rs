use crate::models::{
    classify_structured_state, derive_cleanup_disposition, AgeObservation, CacheSizeSemantics,
    CancellationProbe, CleanupEligibility, CleanupOwnership, CleanupUnit, DispositionFacts,
    EligibilityGate, EntryKind, FileSize, ObservationQuality, PathFacts, ScanItem, Signature,
    StructuredStateKind,
};
use crate::scanner::{PathMeasurement, SizeCalculator};
use crate::signatures::SignatureLoader;
use rayon::ThreadPool;
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use zenith_platform::PlatformEnvironment;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub struct DirectoryScanner;

/// What the filesystem states about a candidate, in the terms the shared rule
/// uses: the entry kind, whether it is executable, and whether the name alone
/// already marks it as structured state.
struct CandidateFacts {
    entry_kind: EntryKind,
    structured_state: Option<StructuredStateKind>,
}

impl CandidateFacts {
    fn read(path: &std::path::Path, metadata: &fs::Metadata) -> Self {
        let entry_kind = if metadata.is_dir() {
            EntryKind::Directory
        } else if metadata.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let structured_state = classify_structured_state(
            PathFacts::new(&name, entry_kind).executable(is_executable(metadata)),
        );
        Self {
            entry_kind,
            structured_state,
        }
    }
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    false
}

/// Whether a cache namespace name is excluded by the signature's prefix list.
///
/// Exclusion is case-insensitive: a cache namespace's on-disk casing is not
/// stable (APFS is case-insensitive by default, so `familycircled` and
/// `FamilyCircle` resolve to the same directory), and an exclusion that matches
/// more is the fail-safe direction. `include_prefixes` stays case-sensitive,
/// because widening an inclusion widens the cleanup surface.
fn is_excluded_namespace(name: &str, exclude_prefixes: &[String]) -> bool {
    let lowered = name.to_lowercase();
    exclude_prefixes
        .iter()
        .any(|prefix| lowered.starts_with(&prefix.to_lowercase()))
}

impl DirectoryScanner {
    /// Scans all configured paths for a given signature and returns discovered ScanItems.
    pub fn scan_signature(
        signature: &Signature,
        environment: &PlatformEnvironment,
        cancellation: &dyn CancellationProbe,
    ) -> Vec<ScanItem> {
        Self::scan_signature_with_pool(
            signature,
            None,
            environment,
            cancellation,
            EligibilityGate::Open,
        )
    }

    pub(crate) fn scan_signature_with_pool(
        signature: &Signature,
        pool: Option<&ThreadPool>,
        environment: &PlatformEnvironment,
        cancellation: &dyn CancellationProbe,
        gate: EligibilityGate,
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
                    cancellation,
                    gate,
                ));
                continue;
            }

            // `Path::exists()` collapses every metadata error into `false`.
            // Keep permission and I/O failures observable instead of treating
            // a configured path as if it simply did not exist.
            let (exists, measurement, facts) = match fs::symlink_metadata(&path_buf) {
                Ok(metadata) if metadata.file_type().is_symlink() => (
                    true,
                    PathMeasurement::unavailable(format!(
                        "Configured path {} is a symlink; cleanup is blocked",
                        path_buf.display()
                    )),
                    None,
                ),
                Ok(metadata) => {
                    let facts = CandidateFacts::read(&path_buf, &metadata);
                    (
                        true,
                        SizeCalculator::measure_path_with_pool(
                            &path_buf,
                            &signature.exclusions,
                            pool,
                            environment,
                            cancellation,
                        ),
                        Some(facts),
                    )
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => (
                    false,
                    PathMeasurement::complete(FileSize::default(), 0),
                    None,
                ),
                Err(err) => (
                    true,
                    PathMeasurement::unavailable(format!(
                        "Could not inspect configured path {}: {}",
                        path_buf.display(),
                        err
                    )),
                    None,
                ),
            };

            let size = measurement.size;
            let file_count = measurement.file_count;
            let skipped_entry_count = measurement.skipped_entries;
            let (quality, incomplete_reason) = if !exists || measurement.complete {
                (ObservationQuality::Fresh, None)
            } else if size.observed_bytes() == 0 {
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

            let path_text = path_buf.to_string_lossy().to_string();
            let disposition = derive_cleanup_disposition(
                DispositionFacts::new(
                    signature.risk,
                    quality,
                    &cache_metadata,
                    &size,
                    incomplete_reason.as_deref(),
                )
                .with_gate(gate)
                .with_structured_state(facts.as_ref().and_then(|facts| facts.structured_state)),
            );

            items.push(ScanItem {
                id: item_id,
                signature_id: signature.id.clone(),
                name: display_name,
                category: signature.category,
                risk: signature.risk,
                path: path_text.clone(),
                size,
                file_count,
                description: signature.description.clone(),
                cache_metadata,
                disposition,
                unit: CleanupUnit::new(signature.unit_kind(), path_text.clone(), path_text),
                ownership: signature.ownership(),
                age: None,
                structured_state: facts.as_ref().and_then(|facts| facts.structured_state),
                entry_kind: facts
                    .as_ref()
                    .map(|facts| facts.entry_kind)
                    .unwrap_or(EntryKind::Directory),
                gate,
                is_selected: false,
                last_modified,
                exists,
                quality,
                incomplete_reason,
                skipped_entry_count,
            });
        }

        // A selection is derived from the facts above, so it is applied after
        // every field is set: only an auto-cleanable unit with reclaimable
        // bytes may be pre-selected.
        for item in &mut items {
            let disposition = item.derive_disposition();
            item.disposition = disposition;
            item.is_selected = item.is_pre_selectable();
        }

        items
    }

    /// Enumerates the children of an aged root and reports each as its own unit.
    ///
    /// A child is inventoried whatever its age: the age policy decides
    /// eligibility, not visibility. That is the difference between "this cache
    /// is 4 GB and it is being written right now" and a namespace that silently
    /// vanishes from the totals.
    #[allow(clippy::too_many_arguments)]
    fn scan_aged_children(
        environment: &PlatformEnvironment,
        signature: &Signature,
        root: &std::path::Path,
        path_index: usize,
        min_age_days: u32,
        cancellation: &dyn CancellationProbe,
        gate: EligibilityGate,
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
                    1,
                    format!(
                        "Configured path {} is a symlink; cleanup is blocked",
                        root.display()
                    ),
                    gate,
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
                    1,
                    format!("Could not inspect {}: {}", root.display(), err),
                    gate,
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
                    1,
                    format!("Could not inspect {}: {}", root.display(), err),
                    gate,
                )];
            }
        };
        let now = SystemTime::now();
        let now_secs = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let root_text = root.to_string_lossy().to_string();
        let unit_kind = signature.unit_kind();
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
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    items.push(Self::unavailable_aged_item(
                        signature,
                        &path,
                        format!("{}.{}.{}", signature.id, path_index, name),
                        name,
                        FileSize::default(),
                        0,
                        None,
                        1,
                        format!(
                            "Candidate {} is a symlink; cleanup is blocked",
                            path.display()
                        ),
                        gate,
                    ));
                    continue;
                }
                Ok(metadata) => metadata,
                Err(err) => {
                    items.push(Self::unavailable_aged_item(
                        signature,
                        &path,
                        format!("{}.{}.{}", signature.id, path_index, name),
                        name,
                        FileSize::default(),
                        0,
                        None,
                        1,
                        format!("Could not inspect {}: {}", path.display(), err),
                        gate,
                    ));
                    continue;
                }
            };
            if is_excluded_namespace(&name, &signature.exclude_prefixes) {
                continue;
            }

            let facts = CandidateFacts::read(&path, &metadata);
            let path_text = path.to_string_lossy().to_string();
            let unit = CleanupUnit::new(unit_kind, root_text.clone(), path_text.clone());
            let ownership = if signature.ownership().is_known() {
                signature.ownership()
            } else {
                // A namespace enumerated under a broad root has no catalog
                // owner: its own name is the only statement about who it
                // belongs to, and it is reported as an inference.
                CleanupOwnership::inferred(name.clone())
            };

            // Single-pass fail-closed tree measurement
            let stats = Self::measure_tree_stats(
                environment,
                &path,
                &signature.exclusions,
                0,
                32,
                cancellation,
            );
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
                    stats.skipped_entries,
                    stats.incomplete_reason.unwrap_or_else(|| {
                        format!("Could not completely inspect {}", path.display())
                    }),
                    gate,
                ));
                continue;
            }

            let size = FileSize::new(stats.logical, Some(stats.allocated));
            let newest_modified = stats
                .newest_mtime
                .and_then(|modified| modified.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs());
            let age = AgeObservation::evaluate(min_age_days, newest_modified, now_secs);

            // An empty fresh observation carries nothing a user could act on.
            if size.observed_bytes() == 0 && age.satisfied {
                continue;
            }

            let description = if age.satisfied {
                format!(
                    "{} (unchanged for at least {} days)",
                    signature.description, min_age_days
                )
            } else {
                format!(
                    "{} (used within the last {} days; kept until it is stale)",
                    signature.description, min_age_days
                )
            };

            let cache_metadata = signature.cache_metadata();
            let disposition = derive_cleanup_disposition(
                DispositionFacts::new(
                    signature.risk,
                    ObservationQuality::Fresh,
                    &cache_metadata,
                    &size,
                    None,
                )
                .with_gate(gate)
                .with_age(Some(&age))
                .with_structured_state(facts.structured_state),
            );
            let is_selected = disposition.eligibility == CleanupEligibility::AutoCleanable
                && disposition.cleanable_bytes.unwrap_or(0) > 0;

            items.push(ScanItem {
                id: format!("{}.{}.{}", signature.id, path_index, name),
                signature_id: signature.id.clone(),
                name,
                category: signature.category,
                risk: signature.risk,
                path: path_text,
                size,
                file_count: stats.file_count,
                description,
                cache_metadata,
                disposition,
                unit,
                ownership,
                age: Some(age),
                structured_state: facts.structured_state,
                entry_kind: facts.entry_kind,
                gate,
                is_selected,
                last_modified: newest_modified,
                exists: true,
                quality: ObservationQuality::Fresh,
                incomplete_reason: None,
                skipped_entry_count: stats.skipped_entries,
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
                1,
                reason,
                gate,
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
        skipped_entry_count: u64,
        reason: String,
        gate: EligibilityGate,
    ) -> ScanItem {
        let mut cache_metadata = signature.cache_metadata();
        // An uninspectable candidate's size is informational: it can never be
        // cleaned, so it must not render as a reclaimable lower bound.
        cache_metadata.size_semantics = CacheSizeSemantics::Informational;
        let disposition = derive_cleanup_disposition(
            DispositionFacts::new(
                signature.risk,
                ObservationQuality::Unavailable,
                &cache_metadata,
                &size,
                Some(&reason),
            )
            .with_gate(gate),
        );
        let path_text = path.to_string_lossy().to_string();
        ScanItem {
            id,
            signature_id: signature.id.clone(),
            name,
            category: signature.category,
            risk: signature.risk,
            path: path_text.clone(),
            size,
            file_count,
            description: format!(
                "{} Age eligibility could not be verified, so cleanup is blocked.",
                signature.description
            ),
            cache_metadata,
            disposition,
            unit: CleanupUnit::new(signature.unit_kind(), path_text.clone(), path_text),
            ownership: signature.ownership(),
            age: None,
            structured_state: None,
            entry_kind: EntryKind::Directory,
            gate,
            is_selected: false,
            last_modified,
            exists: true,
            quality: ObservationQuality::Unavailable,
            incomplete_reason: Some(reason),
            skipped_entry_count,
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
        cancellation: &dyn CancellationProbe,
    ) -> TreeStats {
        let mut stats = TreeStats {
            logical: 0,
            allocated: 0,
            file_count: 0,
            newest_mtime: None,
            complete: true,
            incomplete_reason: None,
            skipped_entries: 0,
        };

        if current_depth > max_depth {
            stats.complete = false;
            stats.skipped_entries += 1;
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
                stats.skipped_entries += 1;
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
            stats.skipped_entries += 1;
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
                stats.skipped_entries += 1;
                stats.incomplete_reason = Some(format!(
                    "Failed to read directory {}: {}",
                    path.display(),
                    err
                ));
                return stats;
            }
        };

        for entry in entries {
            if cancellation.is_cancelled() {
                stats.complete = false;
                stats.skipped_entries += 1;
                if stats.incomplete_reason.is_none() {
                    stats.incomplete_reason =
                        Some(format!("Scan cancelled while measuring {}", path.display()));
                }
                break;
            }
            let ent = match entry {
                Ok(e) => e,
                Err(err) => {
                    stats.complete = false;
                    stats.skipped_entries += 1;
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
                stats.skipped_entries += 1;
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
                stats.skipped_entries += 1;
                continue;
            }

            let sub_stats = Self::measure_tree_stats(
                environment,
                &child_path,
                exclusions,
                current_depth + 1,
                max_depth,
                cancellation,
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
            stats.skipped_entries += sub_stats.skipped_entries;
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
    /// Entries the walk did not account for: excluded, blacklisted, protected,
    /// unreadable, or beyond the depth limit.
    pub skipped_entries: u64,
}

#[cfg(test)]
mod tests {
    use crate::models::NeverCancelled;

    use super::DirectoryScanner;
    use crate::models::{
        CacheSizeSemantics, Category, CleanStrategy, ObservationQuality, RiskTier, Signature,
    };
    use zenith_platform::path_algebra::PathFlavor;
    use zenith_platform::PlatformEnvironment;

    fn environment() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::current())
    }

    /// A shared helper for the aged-child fixtures: an empty root, plus the
    /// signature shape every aged test needs.
    fn child_signature(root: &std::path::Path, min_age_days: u32) -> Signature {
        Signature {
            id: "system.test.aged".into(),
            name: "Test aged caches".into(),
            category: Category::System,
            risk: RiskTier::Safe,
            strategy: CleanStrategy::DeleteDirectory,
            paths: vec![root.to_string_lossy().into_owned()],
            exclusions: vec![],
            description: "Aged child fixture".into(),
            min_age_days: Some(min_age_days),
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
            management_mode: Default::default(),
            artifact_kind: Default::default(),
            consequence: String::new(),
            reclaimable_is_lower_bound: false,
        }
    }

    /// Backdates an entry so the age policy sees it as inactive.
    fn age_entry(path: &std::path::Path, days: u64) {
        let when = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(days * 86_400))
            .expect("the fixture clock has a past");
        // A POSIX directory can only be opened for reading, and a read-only
        // handle can still set its timestamp; Windows needs write access to
        // change one, and only the file fixtures are backdated there.
        #[cfg(unix)]
        let entry = std::fs::File::open(path).expect("open fixture entry");
        #[cfg(not(unix))]
        let entry = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open fixture entry");
        entry.set_modified(when).expect("backdate fixture entry");
    }

    /// One recently written child must not hide its stale sibling, and it must
    /// stay visible itself: the age policy decides eligibility, not discovery.
    #[test]
    fn a_recent_candidate_does_not_hide_a_stale_sibling() {
        let root = tempfile::tempdir().unwrap();
        let stale = root.path().join("stale.cache");
        let recent = root.path().join("recent.cache");
        std::fs::write(&stale, vec![1u8; 4096]).unwrap();
        std::fs::write(&recent, vec![2u8; 8192]).unwrap();
        age_entry(&stale, 30);

        let signature = child_signature(root.path(), 7);
        let items = DirectoryScanner::scan_signature(&signature, &environment(), &NeverCancelled);

        let stale_item = items
            .iter()
            .find(|item| item.name == "stale.cache")
            .expect("the stale child is inventoried");
        assert_eq!(
            stale_item.disposition.eligibility,
            crate::models::CleanupEligibility::AutoCleanable
        );
        assert!(stale_item.is_selected);
        assert!(stale_item.cleanable_bytes() > 0);

        let recent_item = items
            .iter()
            .find(|item| item.name == "recent.cache")
            .expect("the recent child is inventoried rather than dropped");
        assert_eq!(
            recent_item.disposition.eligibility,
            crate::models::CleanupEligibility::Recent
        );
        assert_eq!(recent_item.cleanable_bytes(), 0);
        assert!(!recent_item.is_selected);
        assert!(
            recent_item.observed_bytes() > 0,
            "an ineligible child still reports the bytes it holds"
        );
        assert!(recent_item
            .disposition
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains('7')));
        let age = recent_item
            .age
            .expect("the age verdict travels with the unit");
        assert_eq!(age.min_age_days, 7);
        assert!(!age.satisfied);
    }

    /// The same rule for a cache namespace, whose own directory timestamp is
    /// part of the freshness evidence. Only a POSIX host can backdate a
    /// directory, so the case is asserted where it can be built.
    #[cfg(unix)]
    #[test]
    fn an_aged_namespace_directory_is_cleanable_while_a_recent_one_is_kept() {
        let root = tempfile::tempdir().unwrap();
        let stale = root.path().join("stale.namespace");
        let recent = root.path().join("recent.namespace");
        std::fs::create_dir(&stale).unwrap();
        std::fs::create_dir(&recent).unwrap();
        let stale_file = stale.join("data.bin");
        let recent_file = recent.join("index.bin");
        std::fs::write(&stale_file, vec![1u8; 4096]).unwrap();
        std::fs::write(&recent_file, vec![2u8; 8192]).unwrap();
        // Both the file and the namespace directory must be old, because the
        // measurement takes the newest timestamp in the whole tree.
        age_entry(&stale_file, 30);
        age_entry(&stale, 30);

        let signature = child_signature(root.path(), 7);
        let items = DirectoryScanner::scan_signature(&signature, &environment(), &NeverCancelled);

        let stale_item = items
            .iter()
            .find(|item| item.name == "stale.namespace")
            .expect("the stale namespace is inventoried");
        assert_eq!(
            stale_item.disposition.eligibility,
            crate::models::CleanupEligibility::AutoCleanable
        );
        assert!(stale_item.is_selected);

        let recent_item = items
            .iter()
            .find(|item| item.name == "recent.namespace")
            .expect("the recent namespace is inventoried");
        assert_eq!(
            recent_item.disposition.eligibility,
            crate::models::CleanupEligibility::Recent
        );
        assert!(recent_item.observed_bytes() > 0);
    }

    /// A signature that is discovered outside its scope reports what it found
    /// with a gate, and the gate is the only reason it is not cleanable.
    #[test]
    fn an_always_discovered_signature_reports_a_policy_gate() {
        let root = tempfile::tempdir().unwrap();
        let namespace = root.path().join("third.party");
        std::fs::write(&namespace, vec![7u8; 4096]).unwrap();
        age_entry(&namespace, 30);

        let mut signature = child_signature(root.path(), 7);
        signature.id = "system.test.intensive".into();
        signature.intensive_only = true;
        signature.discovery = crate::models::DiscoveryScope::Always;

        let gated = DirectoryScanner::scan_signature_with_pool(
            &signature,
            None,
            &environment(),
            &NeverCancelled,
            crate::models::EligibilityGate::IntensiveCleanupDisabled,
        );
        let item = gated
            .iter()
            .find(|item| item.name == "third.party")
            .expect("the gated signature is still discovered");
        assert_eq!(
            item.disposition.eligibility,
            crate::models::CleanupEligibility::PolicyGated
        );
        assert_eq!(item.cleanable_bytes(), 0);
        assert!(!item.is_selected);
        assert!(item.observed_bytes() >= 4096);

        // The same facts with the opt-in on produce an eligible unit: the gate
        // is the only difference.
        let open = DirectoryScanner::scan_signature(&signature, &environment(), &NeverCancelled);
        let item = open
            .iter()
            .find(|item| item.name == "third.party")
            .expect("the unit is discovered in either mode");
        assert_eq!(
            item.disposition.eligibility,
            crate::models::CleanupEligibility::AutoCleanable
        );
        assert!(item.is_selected);
    }

    /// An age rule may not manufacture a target out of structured state, and the
    /// candidate stays visible with the reason instead of vanishing.
    #[test]
    fn a_child_holding_structured_state_is_blocked_not_cleanable() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("session.sqlite");
        std::fs::write(&database, vec![3u8; 4096]).unwrap();
        age_entry(&database, 30);

        // An executable bit with an unremarkable name is structured state too,
        // and only a POSIX host can set that bit.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let helper = root.path().join("helper");
            std::fs::write(&helper, b"#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
            age_entry(&helper, 30);
        }

        let signature = child_signature(root.path(), 7);
        let items = DirectoryScanner::scan_signature(&signature, &environment(), &NeverCancelled);

        let item = items
            .iter()
            .find(|item| item.name == "session.sqlite")
            .expect("the structured candidate is retained as a blocked observation");
        assert_eq!(
            item.disposition.eligibility,
            crate::models::CleanupEligibility::Blocked
        );
        assert_eq!(item.cleanable_bytes(), 0);
        assert!(item.structured_state.is_some());
        assert!(item
            .disposition
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("structured state")));

        #[cfg(unix)]
        {
            let item = items
                .iter()
                .find(|item| item.name == "helper")
                .expect("an executable is retained as a blocked observation");
            assert_eq!(
                item.disposition.eligibility,
                crate::models::CleanupEligibility::Blocked
            );
            assert_eq!(
                item.structured_state,
                Some(crate::models::StructuredStateKind::Executable)
            );
        }
    }

    /// Each enumerated child is its own unit, with the root it was found under.
    #[test]
    fn enumerated_children_carry_their_unit_and_owner() {
        let root = tempfile::tempdir().unwrap();
        let namespace = root.path().join("com.example.client");
        std::fs::create_dir(&namespace).unwrap();
        std::fs::write(namespace.join("blob.bin"), vec![1u8; 1024]).unwrap();
        age_entry(&namespace.join("blob.bin"), 30);

        let signature = child_signature(root.path(), 7);
        let items = DirectoryScanner::scan_signature(&signature, &environment(), &NeverCancelled);
        let item = items
            .iter()
            .find(|item| item.name == "com.example.client")
            .expect("child namespace");

        assert_eq!(
            item.unit.kind,
            crate::models::CleanupUnitKind::ChildNamespace
        );
        assert!(item.unit.is_declared());
        // The unit names the path it authorizes and the root it was enumerated
        // under, so execution can re-assert containment without the signature.
        assert_eq!(
            std::path::Path::new(&item.unit.path),
            std::path::Path::new(&item.unit.root).join("com.example.client")
        );
        assert!(
            std::path::Path::new(&item.unit.root).ends_with(
                root.path()
                    .file_name()
                    .expect("the fixture root has a name")
            ),
            "the unit root is the configured root: {}",
            item.unit.root
        );
        assert_eq!(item.entry_kind, crate::models::EntryKind::Directory);
        // No catalog owner for a namespace under a broad root: the name is the
        // only statement about who it belongs to, and it is reported as an
        // inference rather than as a fact.
        assert_eq!(
            item.ownership.confidence,
            crate::models::OwnershipConfidence::Inferred
        );
        assert_eq!(item.ownership.owner, "com.example.client");
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
            discovery: Default::default(),
            unit: None,
            owner: String::new(),
            priority: 0,
            fail_if_running: Vec::new(),
            provider: String::new(),
            management_mode: Default::default(),
            artifact_kind: Default::default(),
            consequence: String::new(),
            reclaimable_is_lower_bound: false,
        };

        let items = DirectoryScanner::scan_signature(&signature, &environment(), &NeverCancelled);
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

    /// The aged walker classifies its boundaries with the stated flavor, so the
    /// tree statistics report what a Windows machine would refuse.
    #[test]
    fn windows_flavor_tree_stats_report_the_stated_boundaries() {
        let root = tempfile::tempdir().unwrap();
        let candidate = root.path().join("candidate.cache");
        std::fs::create_dir_all(candidate.join(".git")).unwrap();
        std::fs::write(candidate.join(".git/objects"), vec![1u8; 1_024]).unwrap();
        // A reserved device name is a Windows boundary, but Windows itself
        // cannot create every one of them, so the fixture states it only where
        // the host can hold it and the expectation follows.
        let _ = std::fs::create_dir_all(candidate.join("nul"));
        // Windows reports success for a reserved name without creating it, so
        // the fixture is what the filesystem actually holds.
        let reserved_created = std::fs::symlink_metadata(candidate.join("nul")).is_ok();
        if reserved_created {
            std::fs::write(candidate.join("nul/payload"), vec![2u8; 2_048]).unwrap();
        }
        std::fs::write(candidate.join("data.bin"), vec![3u8; 4_096]).unwrap();

        let windows = PlatformEnvironment::simulated(PathFlavor::Windows);
        let stats =
            DirectoryScanner::measure_tree_stats(&windows, &candidate, &[], 0, 32, &NeverCancelled);

        assert_eq!(stats.file_count, 1);
        assert_eq!(stats.logical, 4_096);
        assert!(
            stats.complete,
            "a protected name is a deliberate boundary, not a read failure"
        );
        assert_eq!(
            stats.skipped_entries,
            1 + u64::from(reserved_created),
            "`.git` and, where the host holds one, the reserved device name are not measured"
        );

        let posix = PlatformEnvironment::simulated(PathFlavor::Posix);
        let stats =
            DirectoryScanner::measure_tree_stats(&posix, &candidate, &[], 0, 32, &NeverCancelled);
        assert_eq!(stats.skipped_entries, 1, "only `.git` is a POSIX boundary");
        assert_eq!(
            stats.logical,
            4_096 + if reserved_created { 2_048 } else { 0 }
        );
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
        std::fs::write(nested.join("data.bin"), vec![5u8; 2048]).unwrap();
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
            discovery: Default::default(),
            unit: None,
            owner: String::new(),
            priority: 0,
            fail_if_running: Vec::new(),
            provider: String::new(),
            management_mode: Default::default(),
            artifact_kind: Default::default(),
            consequence: String::new(),
            reclaimable_is_lower_bound: false,
        };

        let items = DirectoryScanner::scan_signature(&signature, &environment(), &NeverCancelled);
        let plain = items
            .iter()
            .find(|item| item.name == "plain.cache")
            .expect("complete cache remains visible");
        assert_eq!(plain.quality, ObservationQuality::Fresh);
        assert!(
            plain.is_selected,
            "a complete aged safe cache with reclaimable bytes is auto-selected"
        );
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
            assert_eq!(
                blocked.cache_metadata.size_semantics,
                CacheSizeSemantics::Informational,
                "an uninspectable item's size is never a reclaimable lower bound"
            );
            assert_eq!(blocked.cleanable_bytes(), 0);
            assert!(blocked.incomplete_reason.is_some());
        }
        let measured_blocked = items
            .iter()
            .find(|item| item.name == "bundled.cache")
            .expect("the nested bundle candidate is retained");
        assert!(
            measured_blocked.size.reclaimable() > 0,
            "what the walk could measure is still reported"
        );

        // The guard must also fail closed at delete-time TOCTOU re-verification.
        let stats = DirectoryScanner::measure_tree_stats(
            &environment(),
            &nested,
            &[],
            0,
            32,
            &NeverCancelled,
        );
        assert!(!stats.complete);
        let mixed_case_stats = DirectoryScanner::measure_tree_stats(
            &environment(),
            &mixed_case,
            &[],
            0,
            32,
            &NeverCancelled,
        );
        assert!(!mixed_case_stats.complete);
    }

    /// An excluded namespace is dropped regardless of the case it happens to
    /// use on disk (APFS is case-insensitive), so the shipped signature's
    /// `FamilyCircle` entry also removes the `familycircled` namespace the scan
    /// actually observed, and the tool-managed `ms-playwright` namespace never
    /// reaches the candidate list.
    #[test]
    fn aged_child_scan_drops_excluded_namespaces_case_insensitively() {
        let root = tempfile::tempdir().unwrap();
        let playwright = root.path().join("ms-playwright");
        let apple_lowercase = root.path().join("familycircled");
        let apple_declared_case = root.path().join("FamilyCircle");
        let third_party = root.path().join("third.party.cache");
        for dir in [
            &playwright,
            &apple_lowercase,
            &apple_declared_case,
            &third_party,
        ] {
            std::fs::create_dir(dir).unwrap();
            std::fs::write(dir.join("data.bin"), vec![1u8; 4096]).unwrap();
        }

        let signature = Signature {
            id: "system.test.namespaces".into(),
            name: "Test cache namespaces".into(),
            category: Category::System,
            risk: RiskTier::Safe,
            strategy: CleanStrategy::DeleteDirectory,
            paths: vec![root.path().to_string_lossy().into_owned()],
            exclusions: vec![],
            description: String::new(),
            min_age_days: Some(0),
            include_prefixes: vec![],
            exclude_prefixes: vec!["ms-playwright".into(), "FamilyCircle".into()],
            intensive_only: true,
            platforms: vec![],
            discovery: Default::default(),
            unit: None,
            owner: String::new(),
            priority: 0,
            fail_if_running: Vec::new(),
            provider: String::new(),
            management_mode: Default::default(),
            artifact_kind: Default::default(),
            consequence: String::new(),
            reclaimable_is_lower_bound: false,
        };

        let items = DirectoryScanner::scan_signature(&signature, &environment(), &NeverCancelled);
        let names: Vec<&str> = items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["third.party.cache"],
            "only the ordinary third-party cache is offered"
        );
        assert_eq!(items[0].quality, ObservationQuality::Fresh);
        assert!(items[0].is_selected);
    }
}
