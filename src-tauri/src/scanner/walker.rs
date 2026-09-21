use super::observation::{
    NoRootProgress, ScanLimits, SignatureScan, TraversalCounters, WalkContext,
};
use crate::models::{
    classify_structured_state, derive_cleanup_disposition, AgeObservation, CacheSizeSemantics,
    CancellationProbe, CleanupEligibility, CleanupOwnership, CleanupUnit, DispositionFacts,
    EligibilityGate, EntryKind, FileSize, ObservationQuality, PathFacts, ScanItem, Signature,
    StaleEntryObservation, StructuredStateKind,
};
use crate::safety::SymlinkGuard;
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

/// One concrete root a catalog pattern authorizes.
///
/// `key` is the part of the path the pattern did not spell out — the matched
/// application, package, or profile — so two matches of one pattern never share
/// an item identity. A literal pattern produces one root with no key, and its
/// identity is exactly what it has always been.
struct SignatureRoot {
    path: std::path::PathBuf,
    key: Option<String>,
}

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
        // A walk nobody is watching: default bounds, counters nobody reads, no
        // progress listener. It behaves exactly like a watched walk.
        let counters = TraversalCounters::default();
        let context = WalkContext::new(
            environment,
            cancellation,
            ScanLimits::default(),
            &counters,
            &NoRootProgress,
        );
        Self::scan_signature_with_context(
            signature,
            None,
            &context,
            EligibilityGate::Open,
            &crate::applications::RunningApplications::default(),
        )
        .items
    }

    pub fn scan_signature_with_context(
        signature: &Signature,
        pool: Option<&ThreadPool>,
        context: &WalkContext<'_>,
        gate: EligibilityGate,
        running_apps: &crate::applications::RunningApplications,
    ) -> SignatureScan {
        let mut items = Vec::new();
        let mut scanned_roots = Vec::new();
        let mut selector_incomplete = false;

        // If signature has no explicit file paths (e.g. Docker commands), return early or handle in Docker adapter
        if signature.paths.is_empty() {
            return SignatureScan::new(items);
        }

        let unit_is_root = !signature.unit_kind().is_enumerated_child();

        for (idx, pattern) in signature.paths.iter().enumerate() {
            if context.cancellation.is_cancelled() {
                break;
            }
            let Some(path_buf) = SignatureLoader::expand_path(pattern, context.environment) else {
                continue;
            };

            // The pattern's traversal is consumed page by page: a namespace
            // larger than one page is walked across pages rather than cut off
            // at the first one, so every parent is evaluated and the tail of a
            // large namespace is reachable. Cancellation stops between pages,
            // and what was covered before it did stays a lower bound the scan
            // reports as one.
            let (roots, failures, incomplete) =
                Self::signature_roots(&path_buf, context.environment, context.cancellation);
            selector_incomplete |= incomplete;

            for (fail_idx, failure) in failures.iter().enumerate() {
                let reason = format!(
                    "Could not inspect {}: {}",
                    failure.path.display(),
                    zenith_platform::environment::describe_access_refusal(
                        context.environment,
                        &failure.path,
                        &failure.error,
                    )
                );
                items.push(Self::unavailable_selector_item(
                    signature,
                    &failure.path,
                    idx,
                    fail_idx,
                    failures.len(),
                    reason,
                    gate,
                ));
            }

            // One pattern can name many roots (`Application Support/*/GPUCache`),
            // and each root is scanned on its own so nothing about one match
            // decides another. A pattern with no selector yields exactly one
            // root and keeps its historical identity.
            for root in roots {
                if context.cancellation.is_cancelled() {
                    break;
                }
                let root_path = root.path.clone();
                // The root is reported before it is read: a scan spends its
                // time inside one root, so this is what lets the interface say
                // where it is rather than only that it is running.
                context.progress.root_started(signature, &root_path);
                if context.cancellation.is_cancelled() {
                    break;
                }
                scanned_roots.push(root_path.clone());
                if let Some(min_age_days) = signature.min_age_days {
                    if !unit_is_root {
                        items.extend(Self::scan_aged_children(
                            context,
                            signature,
                            &root_path,
                            idx,
                            min_age_days,
                            gate,
                            root.key.as_deref(),
                            running_apps,
                        ));
                        continue;
                    }
                    items.push(Self::scan_aged_unit(
                        context,
                        signature,
                        &root_path,
                        idx,
                        min_age_days,
                        gate,
                        root.key.as_deref(),
                    ));
                    continue;
                }

                if let Some(item) = Self::scan_fixed_unit(
                    signature,
                    &root_path,
                    idx,
                    pool,
                    context,
                    gate,
                    root.key.as_deref(),
                ) {
                    items.push(item);
                }
            }
        }

        // A selection is derived from the facts above, so it is applied after
        // every field is set: only an auto-cleanable unit with reclaimable
        // bytes may be pre-selected.
        for item in &mut items {
            let disposition = item.derive_disposition();
            item.disposition = disposition;
            item.is_selected = item.is_pre_selectable();
        }

        SignatureScan {
            items,
            roots: scanned_roots,
            selector_incomplete,
        }
    }

    /// The concrete roots one pattern authorizes, alongside any branches that
    /// failed to enumerate due to permission or I/O errors.
    ///
    /// A literal pattern is its own root; a selector pattern is expanded once,
    /// deterministically, and produces one entry per match with the key that
    /// distinguishes them.
    fn signature_roots(
        expanded: &std::path::Path,
        environment: &PlatformEnvironment,
        cancellation: &dyn crate::models::CancellationProbe,
    ) -> (
        Vec<SignatureRoot>,
        Vec<zenith_platform::selector::SelectionFailure>,
        bool,
    ) {
        let text = expanded.to_string_lossy().into_owned();
        if !zenith_platform::selector::PathSelector::is_pattern(&text) {
            return (
                vec![SignatureRoot {
                    path: expanded.to_path_buf(),
                    key: None,
                }],
                Vec::new(),
                false,
            );
        }

        let Ok(selector) =
            zenith_platform::selector::PathSelector::parse(&text, environment.flavor())
        else {
            // The catalog refuses a malformed selector at load time; a pattern
            // that reaches here is refused rather than treated as a literal
            // directory name.
            crate::diagnostics::log_error(
                "scanner",
                &format!("Refused a malformed catalog pattern: {text}"),
            );
            return (Vec::new(), Vec::new(), false);
        };

        let mut matches = Vec::new();
        let mut failures = Vec::new();
        let mut cursor = None;
        let mut incomplete = false;
        loop {
            let page = match selector.expand_page(environment.flavor(), cursor.as_ref()) {
                Ok(page) => page,
                Err(lost) => {
                    // A position that no longer names the same enumeration is
                    // not resumed into: the scan reports what it could not
                    // cover instead of claiming roots it never inspected.
                    crate::diagnostics::log_error(
                        "scanner",
                        &format!("Could not continue the traversal of `{text}`: {lost}"),
                    );
                    incomplete = true;
                    break;
                }
            };
            matches.extend(page.matches);
            failures.extend(page.failures);
            match page.continuation {
                Some(next) => {
                    cursor = Some(next);
                    if cancellation.is_cancelled() {
                        // Stopping between pages is the only way a traversal
                        // ends early, and the scan says so rather than
                        // presenting a partial enumeration as the whole one.
                        incomplete = true;
                        break;
                    }
                }
                None => break,
            }
        }
        let roots = matches
            .into_iter()
            .map(|path| {
                let key = selector
                    .match_key(&path.to_string_lossy(), environment.flavor())
                    .unwrap_or_default();
                SignatureRoot {
                    path,
                    key: Some(key),
                }
            })
            .collect();
        (roots, failures, incomplete)
    }

    /// Scans one root when the root itself is the cleanup unit.
    ///
    /// The root's whole tree is the unit, so its identity, its entry kind, and
    /// its structured-state classification describe exactly what a plan would
    /// delete.
    #[allow(clippy::too_many_arguments)]
    fn scan_fixed_unit(
        signature: &Signature,
        path_buf: &std::path::Path,
        idx: usize,
        pool: Option<&ThreadPool>,
        context: &WalkContext<'_>,
        gate: EligibilityGate,
        root_key: Option<&str>,
    ) -> Option<ScanItem> {
        let environment = context.environment;
        // A symlink, junction, or mount point is a boundary, not a target: the
        // guard classifies the reparse tag, so a cloud placeholder stays an
        // ordinary entry while an indirection is refused. `Path::exists()`
        // collapses every metadata error into `false`, so permission and I/O
        // failures stay observable instead of reading as an absent path.
        let is_link = SymlinkGuard::is_symlink(path_buf);
        let (exists, measurement, facts) = match fs::symlink_metadata(path_buf) {
            Ok(_) if is_link => (
                true,
                PathMeasurement::unavailable(format!(
                    "Configured path {} is a link, junction, or mount point; cleanup is blocked",
                    path_buf.display()
                )),
                None,
            ),
            Ok(metadata) => {
                let facts = CandidateFacts::read(path_buf, &metadata);
                (
                    true,
                    SizeCalculator::measure_path_with_pool(
                        path_buf,
                        &signature.exclusions,
                        pool,
                        context.environment,
                        context.cancellation,
                        context.limits,
                        context.counters,
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
                    zenith_platform::environment::describe_access_refusal(
                        environment,
                        path_buf,
                        &err.to_string(),
                    )
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
            fs::metadata(path_buf)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
        } else {
            None
        };

        let item_id = match (signature.paths.len() > 1, root_key) {
            (multi, Some(key)) => {
                let index = if multi {
                    format!(".{idx}")
                } else {
                    String::new()
                };
                format!("{}{}.{}", signature.id, index, key)
            }
            (true, None) => format!("{}.{}", signature.id, idx),
            (false, None) => signature.id.clone(),
        };

        let display_name = match root_key {
            // A selected root is one of several: the row names which one, so
            // two matches of the same pattern never look like one another.
            Some(key) => format!("{} ({})", signature.name, key),
            None if signature.paths.len() > 1 => format!(
                "{} ({})",
                signature.name,
                path_buf
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
            ),
            None => signature.name.clone(),
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
            .with_stale_entries(None)
            .with_structured_state(facts.as_ref().and_then(|facts| facts.structured_state)),
        );

        Some(ScanItem {
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
            stale: None,
            structured_state: facts.as_ref().and_then(|facts| facts.structured_state),
            entry_kind: facts
                .as_ref()
                .map(|facts| facts.entry_kind)
                .unwrap_or(EntryKind::Directory),
            gate,
            owner_running: false,
            lifecycle_provider_action: false,
            requires_confirmation: false,
            overlaps: Vec::new(),
            is_selected: false,
            last_modified,
            exists,
            quality,
            incomplete_reason,
            skipped_entry_count,
        })
    }

    /// Scans one root as a unit whose whole tree carries the age policy.
    ///
    /// A named cache subtree is deleted whole or not at all, so its freshness
    /// is the freshness of everything inside it. This is the shape the generic
    /// aged-child rule cannot cover: the root is the unit, not its children.
    #[allow(clippy::too_many_arguments)]
    fn scan_aged_unit(
        context: &WalkContext<'_>,
        signature: &Signature,
        path_buf: &std::path::Path,
        idx: usize,
        min_age_days: u32,
        gate: EligibilityGate,
        root_key: Option<&str>,
    ) -> ScanItem {
        let environment = context.environment;
        let is_link = SymlinkGuard::is_symlink(path_buf);
        let stats = fs::symlink_metadata(path_buf);
        let stats = match stats {
            Ok(_) if is_link => {
                return Self::unavailable_aged_item(
                    signature,
                    path_buf,
                    Self::item_id_for(signature, idx, root_key, None),
                    Self::display_name_for(signature, path_buf, root_key),
                    FileSize::default(),
                    0,
                    None,
                    1,
                    format!(
                        "Configured path {} is a link, junction, or mount point; cleanup is blocked",
                        path_buf.display()
                    ),
                    gate,
                    false,
                );
            }
            Ok(_) => Self::measure_tree_stats(context, path_buf, &signature.exclusions, 0, None),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Self::unavailable_aged_item(
                    signature,
                    path_buf,
                    Self::item_id_for(signature, idx, root_key, None),
                    Self::display_name_for(signature, path_buf, root_key),
                    FileSize::default(),
                    0,
                    None,
                    0,
                    format!("Configured path {} does not exist", path_buf.display()),
                    gate,
                    false,
                );
            }
            Err(err) => {
                return Self::unavailable_aged_item(
                    signature,
                    path_buf,
                    Self::item_id_for(signature, idx, root_key, None),
                    Self::display_name_for(signature, path_buf, root_key),
                    FileSize::default(),
                    0,
                    None,
                    1,
                    format!(
                        "Could not inspect {}: {}",
                        path_buf.display(),
                        zenith_platform::environment::describe_access_refusal(
                            environment,
                            path_buf,
                            &err.to_string(),
                        )
                    ),
                    gate,
                    false,
                );
            }
        };

        let size = FileSize::new(stats.logical, Some(stats.allocated));
        let newest_modified = stats
            .newest_mtime
            .and_then(|modified| modified.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs());
        let now_secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let age = AgeObservation::evaluate(min_age_days, newest_modified, now_secs);

        if !stats.complete {
            let reason = stats
                .incomplete_reason
                .clone()
                .unwrap_or_else(|| format!("Could not completely inspect {}", path_buf.display()));
            return Self::unavailable_aged_item(
                signature,
                path_buf,
                Self::item_id_for(signature, idx, root_key, None),
                Self::display_name_for(signature, path_buf, root_key),
                size,
                stats.file_count,
                newest_modified,
                stats.skipped_entries,
                reason,
                gate,
                false,
            );
        }

        let facts = match fs::symlink_metadata(path_buf) {
            Ok(metadata) => CandidateFacts::read(path_buf, &metadata),
            Err(_) => CandidateFacts {
                entry_kind: EntryKind::Directory,
                structured_state: None,
            },
        };

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

        let path_text = path_buf.to_string_lossy().to_string();
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

        ScanItem {
            id: Self::item_id_for(signature, idx, root_key, None),
            signature_id: signature.id.clone(),
            name: Self::display_name_for(signature, path_buf, root_key),
            category: signature.category,
            risk: signature.risk,
            path: path_text.clone(),
            size,
            file_count: stats.file_count,
            description,
            cache_metadata,
            disposition,
            unit: CleanupUnit::new(signature.unit_kind(), path_text.clone(), path_text),
            ownership: signature.ownership(),
            age: Some(age),
            stale: None,
            structured_state: facts.structured_state,
            entry_kind: facts.entry_kind,
            gate,
            owner_running: false,
            lifecycle_provider_action: false,
            requires_confirmation: false,
            overlaps: Vec::new(),
            is_selected: false,
            last_modified: newest_modified,
            exists: true,
            quality: ObservationQuality::Fresh,
            incomplete_reason: None,
            skipped_entry_count: stats.skipped_entries,
        }
    }

    /// The item identity for one root: signature, pattern, selected key, child.
    fn item_id_for(
        signature: &Signature,
        idx: usize,
        root_key: Option<&str>,
        child: Option<&str>,
    ) -> String {
        let mut id = signature.id.clone();
        if signature.paths.len() > 1 || root_key.is_some() {
            id.push_str(&format!(".{idx}"));
        }
        if let Some(key) = root_key {
            id.push('.');
            id.push_str(key);
        }
        if let Some(child) = child {
            id.push('.');
            id.push_str(child);
        }
        id
    }

    /// The display name for one root.
    fn display_name_for(
        signature: &Signature,
        path_buf: &std::path::Path,
        root_key: Option<&str>,
    ) -> String {
        match root_key {
            Some(key) => format!("{} ({})", signature.name, key),
            None if signature.paths.len() > 1 => format!(
                "{} ({})",
                signature.name,
                path_buf
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
            ),
            None => signature.name.clone(),
        }
    }

    /// Enumerates the children of an aged root and reports each as its own unit.
    ///
    /// A child is inventoried whatever its age: the age policy decides
    /// eligibility, not visibility. That is the difference between "this cache
    /// is 4 GB and it is being written right now" and a namespace that silently
    /// vanishes from the totals.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn scan_aged_children(
        context: &WalkContext<'_>,
        signature: &Signature,
        root: &std::path::Path,
        path_index: usize,
        min_age_days: u32,
        gate: EligibilityGate,
        root_key: Option<&str>,
        running_apps: &crate::applications::RunningApplications,
    ) -> Vec<ScanItem> {
        let environment = context.environment;
        if context.cancellation.is_cancelled() {
            return Vec::new();
        }
        // This function reads the namespace root directly. Count that root here
        // instead of in the generic signature loop so every filesystem entry is
        // represented exactly once in traversal metrics.
        context.counters.visit_entry();
        // A signature that removes stale entries ages each entry of a namespace;
        // every other aged signature removes a child whole and ages that child's
        // whole tree. The strategy is the difference, and it decides which
        // verdict the scan reports.
        let stale_policy = (signature.strategy
            == crate::models::CleanStrategy::DeleteStaleContents)
            .then(|| crate::safety::StaleEntryPolicy::from_days(min_age_days));
        let root_is_link = SymlinkGuard::is_symlink(root);
        match fs::symlink_metadata(root) {
            Ok(_) if root_is_link => {
                return vec![Self::unavailable_aged_item(
                    signature,
                    root,
                    format!(
                        "{}.unavailable",
                        Self::item_id_for(signature, path_index, root_key, None)
                    ),
                    Self::display_name_for(signature, root, root_key),
                    FileSize::default(),
                    0,
                    None,
                    1,
                    format!(
                        "Configured path {} is a link, junction, or mount point; cleanup is blocked",
                        root.display()
                    ),
                    gate,
                    false,
                )];
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return vec![],
            Err(err) => {
                return vec![Self::unavailable_aged_item(
                    signature,
                    root,
                    format!(
                        "{}.unavailable",
                        Self::item_id_for(signature, path_index, root_key, None)
                    ),
                    Self::display_name_for(signature, root, root_key),
                    FileSize::default(),
                    0,
                    None,
                    1,
                    format!(
                        "Could not inspect {}: {}",
                        root.display(),
                        zenith_platform::environment::describe_access_refusal(
                            environment,
                            root,
                            &err.to_string(),
                        )
                    ),
                    gate,
                    false,
                )];
            }
        }
        let entries = match fs::read_dir(root) {
            Ok(entries) => {
                context.counters.directory_read();
                entries
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return vec![],
            Err(err) => {
                return vec![Self::unavailable_aged_item(
                    signature,
                    root,
                    format!(
                        "{}.unavailable",
                        Self::item_id_for(signature, path_index, root_key, None)
                    ),
                    Self::display_name_for(signature, root, root_key),
                    FileSize::default(),
                    0,
                    None,
                    1,
                    format!(
                        "Could not inspect {}: {}",
                        root.display(),
                        zenith_platform::environment::describe_access_refusal(
                            environment,
                            root,
                            &err.to_string(),
                        )
                    ),
                    gate,
                    false,
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
            if context.cancellation.is_cancelled() {
                break;
            }
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
            let child_is_link = SymlinkGuard::is_symlink(&path);
            let metadata = match fs::symlink_metadata(&path) {
                Ok(_) if child_is_link => {
                    items.push(Self::unavailable_aged_item(
                        signature,
                        &path,
                        Self::item_id_for(signature, path_index, root_key, Some(&name)),
                        name,
                        FileSize::default(),
                        0,
                        None,
                        1,
                        format!(
                            "Candidate {} is a link, junction, or mount point; cleanup is blocked",
                            path.display()
                        ),
                        gate,
                        false,
                    ));
                    continue;
                }
                Ok(metadata) => metadata,
                Err(err) => {
                    items.push(Self::unavailable_aged_item(
                        signature,
                        &path,
                        Self::item_id_for(signature, path_index, root_key, Some(&name)),
                        name,
                        FileSize::default(),
                        0,
                        None,
                        1,
                        format!(
                            "Could not inspect {}: {}",
                            path.display(),
                            zenith_platform::environment::describe_access_refusal(
                                environment,
                                &path,
                                &err.to_string(),
                            )
                        ),
                        gate,
                        false,
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
            let stats =
                Self::measure_tree_stats(context, &path, &signature.exclusions, 0, stale_policy);
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
                    Self::item_id_for(signature, path_index, root_key, Some(&name)),
                    name,
                    size,
                    stats.file_count,
                    last_modified,
                    stats.skipped_entries,
                    stats.incomplete_reason.unwrap_or_else(|| {
                        format!("Could not completely inspect {}", path.display())
                    }),
                    gate,
                    false,
                ));
                continue;
            }

            let size = FileSize::new(stats.logical, Some(stats.allocated));
            let newest_modified = stats
                .newest_mtime
                .and_then(|modified| modified.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs());
            let age = AgeObservation::evaluate(min_age_days, newest_modified, now_secs);
            let stale = stale_policy.map(|_| {
                StaleEntryObservation::new(min_age_days, stats.stale_bytes, stats.stale_file_count)
            });

            // An empty fresh observation carries nothing a user could act on.
            if size.observed_bytes() == 0 && age.satisfied {
                continue;
            }

            let description = match stale {
                Some(stale) if stale.nothing_is_stale() => format!(
                    "{} (nothing inside has been inactive for {} days)",
                    signature.description, min_age_days
                ),
                Some(stale) => format!(
                    "{} ({} of it has been inactive for at least {} days)",
                    signature.description,
                    format_bytes(stale.stale_bytes),
                    min_age_days
                ),
                None if age.satisfied => format!(
                    "{} (unchanged for at least {} days)",
                    signature.description, min_age_days
                ),
                None => format!(
                    "{} (used within the last {} days; kept until it is stale)",
                    signature.description, min_age_days
                ),
            };

            // A namespace named after a running application's bundle
            // identifier belongs to that application, and the unit is reported
            // as selectable but never automatic.
            let owner_running = running_apps.owner_of(&name).is_some();

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
                .with_running_owner(owner_running)
                .with_age(stale.is_none().then_some(&age))
                .with_stale_entries(stale.as_ref())
                .with_structured_state(facts.structured_state),
            );
            let is_selected = disposition.eligibility == CleanupEligibility::AutoCleanable
                && disposition.cleanable_bytes.unwrap_or(0) > 0;

            let display_name = match root_key {
                Some(key) => format!("{name} ({key})"),
                None => name.clone(),
            };
            items.push(ScanItem {
                id: Self::item_id_for(signature, path_index, root_key, Some(&name)),
                signature_id: signature.id.clone(),
                name: display_name,
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
                age: stale.is_none().then_some(age),
                stale,
                structured_state: facts.structured_state,
                entry_kind: facts.entry_kind,
                gate,
                owner_running,
                lifecycle_provider_action: false,
                requires_confirmation: false,
                overlaps: Vec::new(),
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
                format!(
                    "{}.unavailable",
                    Self::item_id_for(signature, path_index, root_key, None)
                ),
                format!(
                    "{} (scan incomplete)",
                    Self::display_name_for(signature, root, root_key)
                ),
                FileSize::default(),
                0,
                None,
                1,
                reason,
                gate,
                false,
            ));
        }

        items
    }

    fn unavailable_selector_item(
        signature: &Signature,
        path: &Path,
        idx: usize,
        fail_idx: usize,
        fail_count: usize,
        reason: String,
        gate: EligibilityGate,
    ) -> ScanItem {
        let failure_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let display_name = if failure_name.is_empty() || failure_name == signature.name {
            signature.name.clone()
        } else {
            format!("{} ({failure_name})", signature.name)
        };
        let item_id = if fail_count > 1 {
            format!("{}.{idx}.unavailable.{fail_idx}", signature.id)
        } else if signature.paths.len() > 1 {
            format!("{}.{idx}.unavailable", signature.id)
        } else {
            format!("{}.unavailable", signature.id)
        };
        let description = if signature.min_age_days.is_some() {
            format!(
                "{} Age eligibility could not be verified, so cleanup is blocked.",
                signature.description
            )
        } else {
            signature.description.clone()
        };
        let mut cache_metadata = signature.cache_metadata();
        cache_metadata.size_semantics = CacheSizeSemantics::Informational;
        let disposition = derive_cleanup_disposition(
            DispositionFacts::new(
                signature.risk,
                ObservationQuality::Unavailable,
                &cache_metadata,
                &FileSize::default(),
                Some(&reason),
            )
            .with_gate(gate),
        );
        let path_text = path.to_string_lossy().to_string();
        ScanItem {
            id: item_id,
            signature_id: signature.id.clone(),
            name: display_name,
            category: signature.category,
            risk: signature.risk,
            path: path_text.clone(),
            size: FileSize::default(),
            file_count: 0,
            description,
            cache_metadata,
            disposition,
            unit: CleanupUnit::new(signature.unit_kind(), path_text.clone(), path_text),
            ownership: signature.ownership(),
            age: None,
            stale: None,
            structured_state: None,
            entry_kind: EntryKind::Directory,
            gate,
            owner_running: false,
            lifecycle_provider_action: false,
            requires_confirmation: false,
            overlaps: Vec::new(),
            is_selected: false,
            last_modified: None,
            exists: true,
            quality: ObservationQuality::Unavailable,
            incomplete_reason: Some(reason),
            skipped_entry_count: 1,
        }
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
        owner_running: bool,
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
            stale: None,
            structured_state: None,
            entry_kind: EntryKind::Directory,
            gate,
            owner_running,
            lifecycle_provider_action: false,
            requires_confirmation: false,
            overlaps: Vec::new(),
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
        context: &WalkContext<'_>,
        path: &Path,
        exclusions: &[String],
        current_depth: usize,
        stale_policy: Option<crate::safety::StaleEntryPolicy>,
    ) -> TreeStats {
        let environment = context.environment;
        let cancellation = context.cancellation;
        let max_depth = context.limits.max_depth;
        let mut stats = TreeStats {
            stale_bytes: 0,
            stale_file_count: 0,
            logical: 0,
            allocated: 0,
            file_count: 0,
            newest_mtime: None,
            complete: true,
            incomplete_reason: None,
            skipped_entries: 0,
        };

        if cancellation.is_cancelled() {
            stats.complete = false;
            stats.skipped_entries = 1;
            stats.incomplete_reason =
                Some(format!("Scan cancelled while measuring {}", path.display()));
            return stats;
        }

        context.counters.visit_entry();
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

        // The walk refuses an indirection and accounts for the link itself;
        // `SymlinkGuard` is the same classifier size.rs uses, so a junction is
        // a boundary in both walks rather than only in one.
        if SymlinkGuard::is_symlink(path) || meta.is_file() {
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

            // The entry's own verdict, from the entry's own facts. The
            // execution guard evaluates the same predicate for the same file,
            // so the estimate and the deletion agree by construction.
            if let Some(policy) = stale_policy {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let now = SystemTime::now();
                if policy.allows(
                    &name,
                    crate::safety::stale::entry_kind(&meta),
                    crate::safety::stale::is_executable(&meta),
                    meta.modified().ok(),
                    now,
                ) {
                    stats.stale_bytes += stats.allocated;
                    stats.stale_file_count = 1;
                }
            }
            return stats;
        }

        if !meta.is_dir() {
            return stats;
        }

        context.counters.directory_read();
        let entries = match fs::read_dir(path) {
            Ok(e) => e,
            Err(err) => {
                stats.complete = false;
                stats.skipped_entries += 1;
                stats.incomplete_reason = Some(format!(
                    "Failed to read directory {}: {}",
                    path.display(),
                    zenith_platform::environment::describe_access_refusal(
                        environment,
                        path,
                        &err.to_string(),
                    )
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
                            zenith_platform::environment::describe_access_refusal(
                                environment,
                                path,
                                &err.to_string(),
                            )
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
                context,
                &child_path,
                exclusions,
                current_depth + 1,
                stale_policy,
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
            stats.stale_bytes += sub_stats.stale_bytes;
            stats.stale_file_count += sub_stats.stale_file_count;
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
    /// Bytes in files whose *own* age satisfies the stale-entry policy, when the
    /// caller supplied one. Zero when it did not.
    pub stale_bytes: u64,
    /// How many files that subset holds.
    pub stale_file_count: u64,
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

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * KIB;
    const GIB: f64 = 1024.0 * MIB;
    const TIB: f64 = 1024.0 * GIB;

    let b = bytes as f64;
    let (val, unit) = if b >= TIB {
        (b / TIB, "TB")
    } else if b >= GIB {
        (b / GIB, "GB")
    } else if b >= MIB {
        (b / MIB, "MB")
    } else if b >= KIB {
        (b / KIB, "KB")
    } else {
        return format!("{bytes} B");
    };

    let formatted = format!("{val:.1}");
    let trimmed = formatted.strip_suffix(".0").unwrap_or(&formatted);
    format!("{trimmed} {unit}")
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
        PlatformEnvironment::simulated(PathFlavor::current()).with_home(
            if PathFlavor::current().is_windows() {
                r"Z:\ZenithFixtureHome"
            } else {
                "/zenith-fixture-home"
            },
        )
    }

    /// A shared helper for the aged-child fixtures: an empty root, plus the
    /// signature shape every aged test needs.
    /// A walk context for a test that states its own environment: default
    /// bounds, counters nobody reads, no progress listener, and no
    /// cancellation — the same shape production builds, minus the reporting.
    fn test_context<'a>(
        environment: &'a PlatformEnvironment,
    ) -> super::super::observation::WalkContext<'a> {
        static COUNTERS: super::super::observation::TraversalCounters =
            super::super::observation::TraversalCounters::new();
        super::super::observation::WalkContext::new(
            environment,
            &crate::models::NeverCancelled,
            super::super::observation::ScanLimits::default(),
            &COUNTERS,
            &super::super::observation::NoRootProgress,
        )
    }

    /// A wildcard namespace larger than one selector page is inventoried
    /// completely: every parent is evaluated, including the ones a single page
    /// cannot hold, and the components after the wildcard are evaluated against
    /// each of them.
    ///
    /// This is the defect the paged traversal replaced: a bound on work in
    /// flight used to decide which portion of a deterministic namespace could
    /// ever be inventoried, so the same tail was unreachable on every rescan.
    #[test]
    fn a_namespace_larger_than_one_page_is_inventoried_completely() {
        let fixture = tempfile::tempdir().expect("fixture");
        let containers = fixture.path().join("Containers");
        let parents = zenith_platform::selector::SELECTOR_PAGE_LIMIT + 12;
        for index in 0..parents {
            let cache = containers
                .join(format!("com.example.app{index:04}"))
                .join("Data/Library/Caches");
            std::fs::create_dir_all(&cache).expect("fixture");
            std::fs::write(cache.join("payload"), b"cache").expect("fixture");
        }

        let environment = environment().with_home(fixture.path());
        let registry = crate::signatures::SignatureRegistry::load_embedded_with(&environment)
            .expect("the shipped catalog loads");
        let mut signature = registry
            .get("system.intensive.containers_caches")
            .expect("the shipped container-cache signature exists")
            .clone();
        assert_eq!(
            signature.paths,
            vec!["~/Library/Containers/*/Data/Library/Caches"],
            "the regression follows the actual selector that exposed the blind spot"
        );
        // Keep the shipped risk, strategy, unit and age policy. Only relocate
        // its trusted root into a temporary fixture: tests never inspect the
        // developer's real Library/Containers tree.
        signature.paths = vec![format!(
            "{}/*/Data/Library/Caches",
            containers.to_string_lossy()
        )];
        signature.platforms.clear();

        let items = DirectoryScanner::scan_signature(&signature, &environment, &NeverCancelled);

        assert_eq!(
            items.len(),
            parents,
            "every container's cache root is inventoried, not only the first page's"
        );
        let mut ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), items.len(), "each root is one item");
        assert!(
            items
                .iter()
                .any(|item| item.path.contains(&format!("app{:04}", parents - 1))),
            "the last container is reached"
        );
    }

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
            provider_id: None,
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
        // handle can still set its timestamp. Windows refuses `GENERIC_WRITE`
        // for a directory, so a directory handle asks for
        // `FILE_WRITE_ATTRIBUTES` — which is what setting a timestamp needs —
        // and `FILE_FLAG_BACKUP_SEMANTICS`, which is what opens a directory.
        #[cfg(unix)]
        let entry = std::fs::File::open(path).expect("open fixture entry");
        #[cfg(windows)]
        let entry = {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            std::fs::OpenOptions::new()
                .access_mode(FILE_WRITE_ATTRIBUTES)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(path)
                .expect("open fixture entry")
        };
        #[cfg(not(any(unix, windows)))]
        let entry = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open fixture entry");
        entry.set_modified(when).expect("backdate fixture entry");
    }

    #[test]
    fn aged_tree_measurement_honors_cancellation_before_reading_a_file() {
        struct AlwaysCancelled;

        impl crate::models::CancellationProbe for AlwaysCancelled {
            fn is_cancelled(&self) -> bool {
                true
            }
        }

        let fixture = tempfile::tempdir().unwrap();
        let file = fixture.path().join("payload.bin");
        std::fs::write(&file, vec![1u8; 4_096]).unwrap();
        let environment = environment();
        let counters = super::super::observation::TraversalCounters::default();
        let context = super::super::observation::WalkContext::new(
            &environment,
            &AlwaysCancelled,
            super::super::observation::ScanLimits::default(),
            &counters,
            &super::super::observation::NoRootProgress,
        );

        let stats = DirectoryScanner::measure_tree_stats(&context, &file, &[], 0, None);

        assert!(!stats.complete);
        assert_eq!(stats.logical, 0);
        assert_eq!(stats.file_count, 0);
        assert!(stats
            .incomplete_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("cancelled")));
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

    /// One pattern can name many roots, and each match is its own unit: two
    /// applications with a same-named cache never share an identity, and one
    /// match's age never decides another's.
    #[test]
    fn a_selector_pattern_scans_every_match_as_its_own_unit() {
        let root = tempfile::tempdir().unwrap();
        let aged = root.path().join("com.example.aged").join("GPUCache");
        let fresh = root.path().join("com.example.fresh").join("GPUCache");
        std::fs::create_dir_all(&aged).unwrap();
        std::fs::create_dir_all(&fresh).unwrap();
        let aged_blob = aged.join("data_0");
        let fresh_blob = fresh.join("data_0");
        std::fs::write(&aged_blob, vec![1u8; 4096]).unwrap();
        std::fs::write(&fresh_blob, vec![2u8; 8192]).unwrap();
        // The namespace's own timestamp is part of the tree's freshness.
        age_entry(&aged_blob, 30);
        age_entry(&aged, 30);

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            // A linked application directory is not a root: expansion never
            // follows a link into another tree.
            let linked = root.path().join("com.example.linked");
            std::fs::create_dir_all(&linked).unwrap();
            symlink(&aged, linked.join("GPUCache")).unwrap();
        }

        let mut signature = child_signature(root.path(), 7);
        signature.paths = vec![format!("{}/*/GPUCache", root.path().to_string_lossy())];
        signature.unit = Some(crate::models::CleanupUnitKind::NamedSubtree);

        let items = DirectoryScanner::scan_signature(&signature, &environment(), &NeverCancelled);
        let names: Vec<&str> = items.iter().map(|item| item.name.as_str()).collect();
        let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();

        // The linked match comes from the POSIX-only symlink fixture above, so
        // each platform states its own exact expectation.
        #[cfg(unix)]
        let (expected_names, expected_ids) = (
            vec![
                "Test aged caches (com.example.aged)",
                "Test aged caches (com.example.fresh)",
                "Test aged caches (com.example.linked)",
            ],
            vec![
                "system.test.aged.0.com.example.aged",
                "system.test.aged.0.com.example.fresh",
                "system.test.aged.0.com.example.linked",
            ],
        );
        #[cfg(not(unix))]
        let (expected_names, expected_ids) = (
            vec![
                "Test aged caches (com.example.aged)",
                "Test aged caches (com.example.fresh)",
            ],
            vec![
                "system.test.aged.0.com.example.aged",
                "system.test.aged.0.com.example.fresh",
            ],
        );

        assert_eq!(
            names, expected_names,
            "each match is reported once, named by the component the pattern left open"
        );
        assert_eq!(
            ids, expected_ids,
            "two matches of one pattern never share an identity"
        );

        let aged_item = &items[0];
        assert_eq!(
            aged_item.disposition.eligibility,
            crate::models::CleanupEligibility::AutoCleanable
        );
        assert!(aged_item.is_selected);
        assert_eq!(
            aged_item.unit.kind,
            crate::models::CleanupUnitKind::NamedSubtree
        );
        assert_eq!(aged_item.unit.path, aged.to_string_lossy());
        assert_eq!(aged_item.entry_kind, crate::models::EntryKind::Directory);

        let fresh_item = &items[1];
        assert_eq!(
            fresh_item.disposition.eligibility,
            crate::models::CleanupEligibility::Recent
        );
        assert!(!fresh_item.is_selected);
        assert!(fresh_item.observed_bytes() >= 8192);

        #[cfg(unix)]
        {
            // A match that resolves to a link is reported as a blocked
            // observation: the scan says why it is not cleanable instead of
            // hiding the location.
            let linked_item = &items[2];
            assert_eq!(
                linked_item.disposition.eligibility,
                crate::models::CleanupEligibility::Blocked
            );
            assert!(linked_item
                .disposition
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("cleanup is blocked")));
            assert!(!linked_item.is_selected);
        }
    }

    /// A selector that matches nothing is an empty scan, not an error and not a
    /// literal directory name.
    #[test]
    fn a_selector_pattern_that_matches_nothing_reports_nothing() {
        let root = tempfile::tempdir().unwrap();
        let mut signature = child_signature(root.path(), 7);
        signature.paths = vec![format!("{}/*/GPUCache", root.path().to_string_lossy())];
        signature.unit = Some(crate::models::CleanupUnitKind::NamedSubtree);

        let items = DirectoryScanner::scan_signature(&signature, &environment(), &NeverCancelled);
        assert!(items.is_empty());
    }

    /// The pattern decides the granularity: a signature that enumerates
    /// children ages each child of every match separately.
    #[test]
    fn a_selector_pattern_can_enumerate_the_children_of_each_match() {
        let root = tempfile::tempdir().unwrap();
        for package in ["app.one", "app.two"] {
            let namespace = root.path().join(package);
            let temp = namespace.join("TempState");
            std::fs::create_dir_all(&temp).unwrap();
            std::fs::write(temp.join("scratch.bin"), vec![3u8; 2048]).unwrap();
            age_entry(&temp.join("scratch.bin"), 30);
            age_entry(&temp, 30);
            std::fs::create_dir_all(namespace.join("LocalState")).unwrap();
            std::fs::write(namespace.join("LocalState/state.bin"), vec![4u8; 1024]).unwrap();
        }

        let mut signature = child_signature(root.path(), 7);
        signature.paths = vec![format!("{}/*", root.path().to_string_lossy())];
        signature.include_prefixes = vec!["TempState".into()];

        let items = DirectoryScanner::scan_signature(&signature, &environment(), &NeverCancelled);
        let names: Vec<&str> = items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["TempState (app.one)", "TempState (app.two)"],
            "only the selected child of each match is a candidate"
        );
        for item in &items {
            assert_eq!(
                item.unit.kind,
                crate::models::CleanupUnitKind::ChildNamespace
            );
            assert_eq!(
                item.disposition.eligibility,
                crate::models::CleanupEligibility::AutoCleanable
            );
        }
        assert_ne!(items[0].id, items[1].id);
        assert!(!items.iter().any(|item| item.name.starts_with("LocalState")));
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

        let gated = DirectoryScanner::scan_signature_with_context(
            &signature,
            None,
            &test_context(&environment()),
            crate::models::EligibilityGate::IntensiveCleanupDisabled,
            &crate::applications::RunningApplications::default(),
        );
        let item = gated
            .items
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
            provider_id: None,
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
                .is_some_and(|reason| reason.contains("cleanup is blocked")));
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

        let windows = PlatformEnvironment::simulated(PathFlavor::Windows).with_home(
            if PathFlavor::Windows.is_windows() {
                r"Z:\ZenithFixtureHome"
            } else {
                "/zenith-fixture-home"
            },
        );
        let stats =
            DirectoryScanner::measure_tree_stats(&test_context(&windows), &candidate, &[], 0, None);

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

        let posix = PlatformEnvironment::simulated(PathFlavor::Posix).with_home(
            if PathFlavor::Posix.is_windows() {
                r"Z:\ZenithFixtureHome"
            } else {
                "/zenith-fixture-home"
            },
        );
        let stats =
            DirectoryScanner::measure_tree_stats(&test_context(&posix), &candidate, &[], 0, None);
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
            provider_id: None,
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
            &test_context(&environment()),
            &nested,
            &[],
            0,
            None,
        );
        assert!(!stats.complete);
        let mixed_case_stats = DirectoryScanner::measure_tree_stats(
            &test_context(&environment()),
            &mixed_case,
            &[],
            0,
            None,
        );
        assert!(!mixed_case_stats.complete);
    }

    /// Creates a directory junction at `link` pointing at `target`.
    ///
    /// A junction is the Windows reparse point a test can create without
    /// elevation (`mklink /J`), which is why it is the boundary this suite
    /// exercises: it is representative of the mount points and junctions a
    /// user's profile actually contains.
    #[cfg(windows)]
    fn create_junction(link: &std::path::Path, target: &std::path::Path) -> bool {
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output();
        matches!(output, Ok(output) if output.status.success())
    }

    /// A junction inside a candidate tree is a boundary: the walk accounts for
    /// the link and never counts, or descends into, the tree it points at.
    ///
    /// This is the Windows half of the traversal contract — on POSIX the same
    /// rule is covered by the symlink fixtures — and it is the reason the
    /// walker asks `SymlinkGuard` rather than `file_type().is_symlink()`: a
    /// junction is a directory to `std` and an indirection to Windows.
    #[cfg(windows)]
    #[test]
    fn a_junction_inside_a_candidate_is_never_traversed() {
        let fixture = tempfile::tempdir().unwrap();
        let outside = fixture.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("payload.bin"), vec![7u8; 64 * 1024]).unwrap();

        let candidate = fixture.path().join("candidate");
        std::fs::create_dir(&candidate).unwrap();
        std::fs::write(candidate.join("own.bin"), vec![3u8; 4_096]).unwrap();
        let link = candidate.join("linked-outside");
        assert!(
            create_junction(&link, &outside),
            "the test needs a junction to state the boundary it asserts"
        );

        let stats = DirectoryScanner::measure_tree_stats(
            &test_context(&environment()),
            &candidate,
            &[],
            0,
            None,
        );

        assert!(
            stats.complete,
            "a junction is a deliberate boundary, not a read failure: {:?}",
            stats.incomplete_reason
        );
        assert_eq!(
            stats.logical, 4_096,
            "the tree the junction points at is never counted as this unit's bytes"
        );

        // The scan reports the same candidate: its own bytes, never the
        // outside tree's.
        let signature = child_signature(fixture.path(), 7);
        let items = DirectoryScanner::scan_signature(&signature, &environment(), &NeverCancelled);
        let item = items
            .iter()
            .find(|item| item.path == candidate.to_string_lossy())
            .expect("the candidate is discovered");
        assert!(
            item.size.reclaimable() < 64 * 1024,
            "an outside tree behind a junction must not inflate the candidate: {item:?}"
        );
    }

    /// A configured root that is itself an indirection is refused, not walked.
    #[cfg(windows)]
    #[test]
    fn a_junction_root_is_refused_rather_than_walked() {
        let fixture = tempfile::tempdir().unwrap();
        let target = fixture.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("payload.bin"), vec![5u8; 8_192]).unwrap();
        let link = fixture.path().join("linked-root");
        assert!(
            create_junction(&link, &target),
            "the test needs a junction to state the refusal it asserts"
        );

        let mut signature = child_signature(fixture.path(), 0);
        signature.strategy = crate::models::CleanStrategy::DeleteDirectory;
        signature.min_age_days = None;
        signature.paths = vec![link.to_string_lossy().into_owned()];

        let scanned = DirectoryScanner::scan_signature_with_context(
            &signature,
            None,
            &test_context(&environment()),
            crate::models::EligibilityGate::Open,
            &crate::applications::RunningApplications::default(),
        );
        let item = scanned
            .items
            .iter()
            .find(|item| item.path == link.to_string_lossy())
            .expect("the refused root is still reported");
        assert!(
            !item.is_selected && item.cleanable_bytes() == 0,
            "a junction root is never a cleanup target: {item:?}"
        );
        assert!(
            item.incomplete_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("junction") || reason.contains("mount point")),
            "the refusal states what the root is: {item:?}"
        );
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
            provider_id: None,
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

    #[test]
    fn format_bytes_produces_readable_units() {
        assert_eq!(super::format_bytes(0), "0 B");
        assert_eq!(super::format_bytes(512), "512 B");
        assert_eq!(super::format_bytes(1024), "1 KB");
        assert_eq!(super::format_bytes(1536), "1.5 KB");
        assert_eq!(super::format_bytes(1048576), "1 MB");
        assert_eq!(super::format_bytes(524288000), "500 MB");
        assert_eq!(super::format_bytes(1073741824), "1 GB");
    }

    #[test]
    fn stale_entry_description_formats_bytes_correctly() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("com.example.client");
        std::fs::create_dir_all(&app).unwrap();
        let old_file = app.join("old.bin");
        std::fs::write(&old_file, vec![1u8; 1024]).unwrap();
        age_entry(&old_file, 30);
        age_entry(&app, 30);

        let signature = Signature {
            id: "system.test.stale_desc".into(),
            name: "Test stale desc".into(),
            category: Category::System,
            risk: RiskTier::Safe,
            strategy: CleanStrategy::DeleteStaleContents,
            paths: vec![root.path().to_string_lossy().into_owned()],
            exclusions: vec![],
            description: "App cache".into(),
            min_age_days: Some(7),
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
            provider_id: None,
            management_mode: Default::default(),
            artifact_kind: Default::default(),
            consequence: String::new(),
            reclaimable_is_lower_bound: false,
        };

        let items = DirectoryScanner::scan_signature(&signature, &environment(), &NeverCancelled);
        assert_eq!(items.len(), 1);
        assert!(
            items[0]
                .description
                .contains("of it has been inactive for at least 7 days")
                && !items[0].description.contains("7 of it has been inactive"),
            "description was: {}",
            items[0].description
        );
    }

    #[cfg(unix)]
    #[test]
    fn inaccessible_selector_pattern_produces_unavailable_item() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let containers = root.path().join("Containers");
        let app = containers.join("com.example.app");
        let caches = app.join("Data").join("Caches");
        std::fs::create_dir_all(&caches).unwrap();

        std::fs::set_permissions(&containers, std::fs::Permissions::from_mode(0o000)).unwrap();

        let signature = Signature {
            id: "system.test.containers".into(),
            name: "Sandboxed Application Cache".into(),
            category: Category::System,
            risk: RiskTier::Rebuild,
            strategy: CleanStrategy::DeleteStaleContents,
            paths: vec![format!(
                "{}/Containers/*/Data/Caches",
                root.path().to_string_lossy()
            )],
            exclusions: vec![],
            description: "Sandboxed caches".into(),
            min_age_days: Some(14),
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
            provider_id: None,
            management_mode: Default::default(),
            artifact_kind: Default::default(),
            consequence: String::new(),
            reclaimable_is_lower_bound: false,
        };

        let items = DirectoryScanner::scan_signature(&signature, &environment(), &NeverCancelled);
        std::fs::set_permissions(&containers, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(
            items.len(),
            1,
            "the inaccessible branch is retained as an unavailable item"
        );
        assert_eq!(items[0].quality, ObservationQuality::Unavailable);
        assert_eq!(items[0].cleanable_bytes(), 0);
        assert!(!items[0].is_selected);
        let reason = items[0]
            .incomplete_reason
            .as_deref()
            .expect("incomplete reason");
        assert!(reason.contains("Could not inspect"));
    }
}
