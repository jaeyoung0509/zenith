use std::path::{Path, PathBuf};

use crate::models::{
    classify_structured_state, CleanFailureReason, CleanItemResult, CleanStatus, CleanStrategy,
    DeleteTarget, EntryKind, PathFacts, StructuredStateKind, StructuredStatePolicy,
};
use crate::models_inventory::ValidatedModelTarget;
use crate::safety::{Blacklist, SymlinkGuard, ToctouGuard};
use zenith_platform::PlatformEnvironment;

/// An authorized cleanup target that has passed execution-time safety revalidation.
///
/// This type has no public constructor outside the safety module. It cannot be
/// fabricated from an arbitrary frontend path string. A mutation primitive that
/// requires `&ValidatedTarget` guarantees that the unit still contains the
/// path, the lexical and canonical blacklist checks pass, symlink and reparse
/// boundaries hold, the captured identity and entry kind still match, the
/// target is not structured state, and any age constraint is still satisfied —
/// all immediately before execution.
#[derive(Debug, Clone)]
pub struct ValidatedTarget {
    item_id: String,
    name: String,
    path: PathBuf,
    strategy: CleanStrategy,
    expected_bytes: u64,
    exclusions: Vec<String>,
    /// The entry-level age policy, for a target whose strategy removes stale
    /// entries rather than everything. The primitive re-evaluates it for every
    /// file it touches, so this is what makes the removal a per-entry decision
    /// rather than a tree-level one.
    stale_policy: Option<crate::safety::StaleEntryPolicy>,
    min_age_days: Option<u32>,
    structured_state_policy: StructuredStatePolicy,
}

impl ValidatedTarget {
    fn from_planned(target: &DeleteTarget) -> Self {
        let stale_policy = match (target.strategy, target.min_age_days) {
            (CleanStrategy::DeleteStaleContents, Some(days)) => {
                Some(crate::safety::StaleEntryPolicy::from_days(days))
            }
            _ => None,
        };
        Self {
            item_id: target.item_id.clone(),
            name: target.name.clone(),
            path: target.path.clone(),
            strategy: target.strategy,
            expected_bytes: target.expected_bytes,
            exclusions: target.exclusions.clone(),
            stale_policy,
            min_age_days: target.min_age_days,
            structured_state_policy: target.structured_state_policy,
        }
    }

    pub fn item_id(&self) -> &str {
        &self.item_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn strategy(&self) -> CleanStrategy {
        self.strategy
    }

    pub fn expected_bytes(&self) -> u64 {
        self.expected_bytes
    }

    pub fn exclusions(&self) -> &[String] {
        &self.exclusions
    }

    pub fn stale_policy(&self) -> Option<crate::safety::StaleEntryPolicy> {
        self.stale_policy
    }

    pub fn min_age_days(&self) -> Option<u32> {
        self.min_age_days
    }

    pub fn structured_state_policy(&self) -> StructuredStatePolicy {
        self.structured_state_policy
    }
}

/// The outcome of revalidating a planned cleanup target immediately before mutation.
///
/// `Skipped` and `Failed` are different answers: a skip means the target is no
/// longer the object the plan authorized (or is gone), and the safe response
/// was to leave whatever is there alone; a failure means the object was still
/// the right one and Zenith could not remove it.
#[derive(Debug)]
pub enum RevalidationOutcome {
    Validated(ValidatedTarget),
    Skipped(CleanItemResult),
    Failed(CleanItemResult),
}

/// Builds the per-item result the execution guard reports for one target.
///
/// Every result carries the plan-time estimate and the run's own measurement
/// as separate fields, so a caller cannot read an expectation as a measurement.
fn outcome(
    target: &DeleteTarget,
    status: CleanStatus,
    reason: Option<CleanFailureReason>,
    message: Option<String>,
) -> CleanItemResult {
    CleanItemResult {
        item_id: target.item_id.clone(),
        name: target.name.clone(),
        path: target.path.to_string_lossy().to_string(),
        status,
        success: matches!(status, CleanStatus::Success | CleanStatus::Partial),
        estimated_bytes: target.expected_bytes,
        bytes_reclaimed: 0,
        moved_to_trash_bytes: 0,
        failure_reason: reason,
        error_message: message,
    }
}

fn skipped(
    target: &DeleteTarget,
    reason: CleanFailureReason,
    message: impl Into<String>,
) -> RevalidationOutcome {
    RevalidationOutcome::Skipped(outcome(
        target,
        CleanStatus::Skipped,
        Some(reason),
        Some(message.into()),
    ))
}

fn failed(
    target: &DeleteTarget,
    reason: CleanFailureReason,
    message: impl Into<String>,
) -> RevalidationOutcome {
    RevalidationOutcome::Failed(outcome(
        target,
        CleanStatus::Failed,
        Some(reason),
        Some(message.into()),
    ))
}

/// The entry kind the filesystem states right now.
fn entry_kind_of(metadata: &std::fs::Metadata) -> EntryKind {
    let file_type = metadata.file_type();
    if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

/// Whether the platform marked this entry executable.
#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// Whether `path` sits on a different device than its parent, which makes it a
/// mount boundary rather than a cache directory.
///
/// A cache namespace is created by the application that owns it, so a device
/// change between a path and its parent means something else was mounted there.
/// Deleting "through" it would remove another volume's contents.
#[cfg(unix)]
fn crosses_mount_boundary(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let (Some(own), Some(parent)) = (
        std::fs::metadata(path).ok(),
        path.parent()
            .and_then(|parent| std::fs::metadata(parent).ok()),
    ) else {
        // An unreadable parent is not evidence of a boundary; the presence and
        // blacklist checks already decided what to do about unreadable paths.
        return false;
    };
    own.dev() != parent.dev()
}

#[cfg(not(unix))]
fn crosses_mount_boundary(_path: &Path) -> bool {
    // Windows reports mount points and junctions as reparse points, which the
    // symlink boundary check refuses before this one can matter.
    false
}

/// Inspect the whole authorized unit before its first mutation. A directory
/// with an ordinary name can still contain a database or configuration file.
/// Symlinks are classified by name but never followed.
pub(crate) fn structured_descendant_with_policy(
    path: &Path,
    policy: StructuredStatePolicy,
) -> std::io::Result<Option<(PathBuf, StructuredStateKind)>> {
    let mut pending = vec![path.to_path_buf()];
    while let Some(current) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&current)?;
        let kind = entry_kind_of(&metadata);
        let name = current
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default();
        let facts = PathFacts::new(&name, kind)
            .executable(kind == EntryKind::File && is_executable(&metadata));
        if let Some(structured) = classify_structured_state(facts) {
            if !policy.permits(structured) {
                return Ok(Some((current, structured)));
            }
        }
        if kind == EntryKind::Directory && !SymlinkGuard::is_symlink_metadata(&current)? {
            if current != path && crosses_mount_boundary(&current) {
                return Err(std::io::Error::other(format!(
                    "{} crosses a mount boundary",
                    current.display()
                )));
            }
            for entry in std::fs::read_dir(&current)? {
                pending.push(entry?.path());
            }
        }
    }
    Ok(None)
}

pub struct SafetyValidator;

impl SafetyValidator {
    /// Revalidates a planned delete target against all runtime invariants:
    /// unit containment, presence, lexical and canonical blacklist, symlink and
    /// reparse boundaries, mount boundaries, TOCTOU identity, entry kind,
    /// structured-state protection, strategy compatibility, and intensive
    /// cleanup age constraints.
    pub fn revalidate(
        target: &DeleteTarget,
        environment: &PlatformEnvironment,
    ) -> RevalidationOutcome {
        let path = &target.path;

        // 0. The plan states which unit authorized this path. A target that
        //    cannot name it, or names a path other than the one it deletes, is
        //    not a target this guard can reason about.
        if !target.unit.is_declared() {
            return failed(
                target,
                CleanFailureReason::Unknown,
                format!(
                    "Target {} does not name the cleanup unit that authorized it; refusing to mutate",
                    path.display()
                ),
            );
        }
        let unit_path = Path::new(&target.unit.path);
        if unit_path != path || !path.starts_with(Path::new(&target.unit.root)) {
            return failed(
                target,
                CleanFailureReason::ChangedSinceScan,
                format!(
                    "Target {} is outside the cleanup unit that authorized it ({}); refusing to mutate",
                    path.display(),
                    target.unit.path
                ),
            );
        }

        // 1. Presence probe. A path that no longer exists is already absent.
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return skipped(
                    target,
                    CleanFailureReason::NotFound,
                    format!("{} was already absent before cleanup", path.display()),
                );
            }
            Err(error) => {
                let error_str = error.to_string();
                return failed(
                    target,
                    CleanFailureReason::PermissionDenied,
                    zenith_platform::environment::describe_access_refusal(
                        environment,
                        path,
                        &error_str,
                    ),
                );
            }
        };

        // Every present filesystem target must carry the identity captured by
        // the planner. Absence is handled above, while provider-backed
        // strategies never reach a filesystem mutation primitive.
        if matches!(
            target.strategy,
            CleanStrategy::DeleteContents
                | CleanStrategy::DeleteDirectory
                | CleanStrategy::DeleteStaleContents
        ) && target.identity.is_none()
        {
            return failed(
                target,
                CleanFailureReason::ChangedSinceScan,
                format!(
                    "Filesystem identity is missing for {}; refusing to mutate",
                    path.display()
                ),
            );
        }

        // 2. Blacklist check (lexical & canonical, fail closed on mutation)
        if let Err(e) = Blacklist::validate_with(path, environment) {
            return skipped(target, CleanFailureReason::Blacklisted, e.to_string());
        }
        if let Err(e) = SymlinkGuard::validate_canonical_blacklist_strict(path, environment) {
            if crate::safety::is_already_absent(&e) {
                return skipped(
                    target,
                    CleanFailureReason::NotFound,
                    format!("{} was already absent before cleanup", path.display()),
                );
            }
            return skipped(target, CleanFailureReason::Blacklisted, e.to_string());
        }

        // 3. Symlink, junction, mount point, or unknown reparse point: a
        //    deletion boundary, never a path to follow.
        match SymlinkGuard::is_symlink_strict(path) {
            Ok(false) => {}
            Ok(true) => {
                return skipped(
                    target,
                    CleanFailureReason::SafetyBoundary,
                    format!(
                        "{} is a link, junction, or mount point; cleanup does not delete through it",
                        path.display()
                    ),
                );
            }
            Err(e) if crate::safety::is_already_absent(&e) => {
                return skipped(
                    target,
                    CleanFailureReason::NotFound,
                    format!("{} was already absent before cleanup", path.display()),
                );
            }
            Err(e) => {
                return failed(target, CleanFailureReason::ChangedSinceScan, e.to_string());
            }
        }

        if crosses_mount_boundary(path) {
            return skipped(
                target,
                CleanFailureReason::SafetyBoundary,
                format!(
                    "{} sits on a different volume than its parent; cleanup does not cross a mount boundary",
                    path.display()
                ),
            );
        }

        // 4. TOCTOU identity verification
        if let Some(expected_identity) = &target.identity {
            let verify_result = if target.strategy == CleanStrategy::DeleteStaleContents {
                ToctouGuard::verify_entity(path, expected_identity)
            } else {
                ToctouGuard::verify(path, expected_identity)
            };
            if let Err(e) = verify_result {
                if crate::safety::is_already_absent(&e) {
                    return skipped(
                        target,
                        CleanFailureReason::NotFound,
                        format!("{} was already absent before cleanup", path.display()),
                    );
                }
                return skipped(target, CleanFailureReason::ChangedSinceScan, e.to_string());
            }
        }

        // 5. The entry kind the scan approved must still hold: a directory that
        //    became a file (or the reverse) is no longer the approved object.
        let current_kind = entry_kind_of(&metadata);
        if current_kind != target.target_kind {
            return skipped(
                target,
                CleanFailureReason::ChangedSinceScan,
                format!(
                    "{} was a {:?} when it was scanned and is a {:?} now; refusing to mutate",
                    path.display(),
                    target.target_kind,
                    current_kind
                ),
            );
        }

        // 6. Structured state is never generic cleanup's to remove, even when a
        //    discovery rule produced the target.
        if let Some((kind, _)) = crate::safety::validator::structured_state_at(path) {
            if !target.structured_state_policy.permits(kind) {
                return skipped(
                    target,
                    CleanFailureReason::StructuredStore,
                    format!(
                        "{} is {}; generic cleanup does not remove structured state",
                        path.display(),
                        kind.display_name()
                    ),
                );
            }
        }

        // A target with a harmless basename may contain structured state.
        // Finish the full walk before allowing any mutation of this unit.
        if current_kind == EntryKind::Directory
            && matches!(
                target.strategy,
                CleanStrategy::DeleteContents | CleanStrategy::DeleteDirectory
            )
        {
            match structured_descendant_with_policy(path, target.structured_state_policy) {
                Ok(Some((nested, kind))) => {
                    return skipped(
                        target,
                        CleanFailureReason::StructuredStore,
                        format!(
                            "{} contains {} ({}); generic cleanup leaves the entire unit untouched",
                            path.display(),
                            nested.display(),
                            kind.display_name()
                        ),
                    );
                }
                Err(error) => {
                    return skipped(
                        target,
                        CleanFailureReason::SafetyBoundary,
                        format!(
                            "Could not inspect the whole unit {} before cleanup: {error}",
                            path.display()
                        ),
                    );
                }
                Ok(None) => {}
            }
        }

        // 7. The strategy must still fit the object it would act on.
        if target.strategy == CleanStrategy::DeleteContents && current_kind != EntryKind::Directory
        {
            return skipped(
                target,
                CleanFailureReason::ChangedSinceScan,
                format!(
                    "{} is no longer a directory, so there are no contents to remove",
                    path.display()
                ),
            );
        }

        // 8. Stale temp directory TOCTOU: re-verify the freshness invariant
        //    against the same age rule the scan applied.
        if let Some(days) = target.min_age_days {
            if current_kind == EntryKind::Directory {
                // The execution-time age re-check is not part of a
                // cancellable scan: it must observe the whole tree before a
                // deletion is allowed, so it runs to completion.
                let stale_policy = (target.strategy == CleanStrategy::DeleteStaleContents)
                    .then(|| crate::safety::StaleEntryPolicy::from_days(days));
                let counters = crate::scanner::TraversalCounters::default();
                let context = crate::scanner::WalkContext::new(
                    environment,
                    &crate::models::NeverCancelled,
                    crate::scanner::ScanLimits::default(),
                    &counters,
                    &crate::scanner::NoRootProgress,
                );
                let stats = crate::scanner::DirectoryScanner::measure_tree_stats(
                    &context,
                    path,
                    &target.exclusions,
                    0,
                    stale_policy,
                );
                if !stats.complete {
                    if let Err(error) = std::fs::symlink_metadata(path) {
                        if error.kind() == std::io::ErrorKind::NotFound {
                            return skipped(
                                target,
                                CleanFailureReason::NotFound,
                                format!("{} was already absent before cleanup", path.display()),
                            );
                        }
                    }
                    return skipped(
                        target,
                        CleanFailureReason::ChangedSinceScan,
                        "Directory structure could not be fully verified; aborted to protect active files",
                    );
                }

                // A stale-entry target is allowed to be recent as a whole: what
                // it authorizes is the entries inside it that are not. The
                // measurement above counted them, and the primitive counts them
                // again for every file it touches.
                if stale_policy.is_some() {
                    if stats.stale_bytes == 0 {
                        return skipped(
                            target,
                            CleanFailureReason::NotFound,
                            format!(
                                "Nothing under {} has been inactive for {days} days",
                                path.display()
                            ),
                        );
                    }
                    return RevalidationOutcome::Validated(ValidatedTarget::from_planned(target));
                }
                let newest = stats.newest_mtime.and_then(|modified| {
                    modified
                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                        .ok()
                        .map(|elapsed| elapsed.as_secs())
                });
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                // One age rule for the scan and the execution boundary: the
                // same evaluation that produced the item's verdict.
                let age = crate::models::AgeObservation::evaluate(days, newest, now);
                if !age.satisfied {
                    return skipped(
                        target,
                        CleanFailureReason::ChangedSinceScan,
                        format!(
                            "Directory contains files modified within the last {days} days; aborted cleanup to protect active processes"
                        ),
                    );
                }
            }
        }

        RevalidationOutcome::Validated(ValidatedTarget::from_planned(target))
    }
}

/// Authority token required by `SafeTreeDeleter` to perform filesystem mutations.
///
/// Ensures raw paths cannot be deleted without first obtaining an authority token
/// through a domain safety gate (`ValidatedTarget` for general cleanup,
/// `ValidatedModelTarget` for model inventory cleanup).
pub struct FilesystemDeleteAuthority<'a> {
    inner: AuthorityKind<'a>,
}

enum AuthorityKind<'a> {
    Cleanup(&'a ValidatedTarget),
    /// A model-inventory deletion. The type is minted by
    /// `models_inventory::ValidatedModelTarget`, which resolves the allowed
    /// root from the model's own source, so this arm cannot carry a path that
    /// skipped that scope check.
    ModelInventory(&'a ValidatedModelTarget),
}

impl<'a> FilesystemDeleteAuthority<'a> {
    pub(crate) fn path(&self) -> &Path {
        match self.inner {
            AuthorityKind::Cleanup(target) => target.path(),
            AuthorityKind::ModelInventory(model) => model.path(),
        }
    }

    pub(crate) fn exclusions(&self) -> &[String] {
        match self.inner {
            AuthorityKind::Cleanup(target) => target.exclusions(),
            AuthorityKind::ModelInventory(_) => &[],
        }
    }

    /// The entry-level age policy this authority carries, when it carries one.
    ///
    /// A model-inventory deletion has no such policy: the reviewed scope that
    /// authorized it names the objects, not an age.
    pub(crate) fn stale_policy(&self) -> Option<crate::safety::StaleEntryPolicy> {
        match self.inner {
            AuthorityKind::Cleanup(target) => target.stale_policy(),
            AuthorityKind::ModelInventory(_) => None,
        }
    }

    pub(crate) fn structured_state_policy(&self) -> Option<StructuredStatePolicy> {
        match self.inner {
            AuthorityKind::Cleanup(target) => Some(target.structured_state_policy()),
            AuthorityKind::ModelInventory(_) => None,
        }
    }
}

impl<'a> From<&'a ValidatedTarget> for FilesystemDeleteAuthority<'a> {
    fn from(target: &'a ValidatedTarget) -> Self {
        Self {
            inner: AuthorityKind::Cleanup(target),
        }
    }
}

impl<'a> From<&'a ValidatedModelTarget> for FilesystemDeleteAuthority<'a> {
    fn from(target: &'a ValidatedModelTarget) -> Self {
        Self {
            inner: AuthorityKind::ModelInventory(target),
        }
    }
}

/// Classifies one entry for a cleanup target.
///
/// Every caller is generic cleanup: an owner-managed store that legitimately
/// holds names this classifier protects is not cleaned through a filesystem
/// primitive at all, so there is no per-signature exemption to consult here.
pub fn structured_state_at(path: &Path) -> Option<(StructuredStateKind, EntryKind)> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    let entry_kind = entry_kind_of(&metadata);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let facts = PathFacts::new(&name, entry_kind)
        .executable(entry_kind == EntryKind::File && is_executable(&metadata));
    classify_structured_state(facts).map(|kind| (kind, entry_kind))
}

/// The entry kind of a path, as the plan records it.
pub fn entry_kind_at(path: &Path) -> Option<EntryKind> {
    std::fs::symlink_metadata(path)
        .ok()
        .map(|metadata| entry_kind_of(&metadata))
}
