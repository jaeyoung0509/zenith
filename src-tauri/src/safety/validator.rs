use std::path::{Path, PathBuf};

use crate::models::{
    CleanFailureReason, CleanItemResult, CleanStatus, CleanStrategy, DeleteTarget,
};
use crate::models_inventory::ValidatedModelTarget;
use crate::safety::{Blacklist, SymlinkGuard, ToctouGuard};
use zenith_platform::PlatformEnvironment;

/// An authorized cleanup target that has passed execution-time safety revalidation.
///
/// This type has no public constructor outside the safety module. It cannot be
/// fabricated from an arbitrary frontend path string. A mutation primitive that
/// requires `&ValidatedTarget` guarantees that lexical blacklist, canonical
/// blacklist, symlink integrity, and TOCTOU identity checks all succeeded immediately
/// before execution.
#[derive(Debug, Clone)]
pub struct ValidatedTarget {
    item_id: String,
    name: String,
    path: PathBuf,
    strategy: CleanStrategy,
    expected_bytes: u64,
    exclusions: Vec<String>,
}

impl ValidatedTarget {
    fn from_planned(target: &DeleteTarget) -> Self {
        Self {
            item_id: target.item_id.clone(),
            name: target.name.clone(),
            path: target.path.clone(),
            strategy: target.strategy,
            expected_bytes: target.expected_bytes,
            exclusions: target.exclusions.clone(),
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
}

/// The outcome of revalidating a planned cleanup target immediately before mutation.
#[derive(Debug)]
pub enum RevalidationOutcome {
    Validated(ValidatedTarget),
    AlreadyAbsent(CleanItemResult),
    Failed(CleanItemResult),
}

pub struct SafetyValidator;

impl SafetyValidator {
    fn already_absent(target: &DeleteTarget) -> CleanItemResult {
        CleanItemResult {
            item_id: target.item_id.clone(),
            name: target.name.clone(),
            path: target.path.to_string_lossy().to_string(),
            status: CleanStatus::Success,
            success: true,
            bytes_reclaimed: 0,
            failure_reason: None,
            error_message: None,
        }
    }

    /// Revalidates a planned delete target against all runtime invariants:
    /// presence, lexical & canonical blacklist, symlink safety, TOCTOU identity,
    /// and intensive cleanup age constraints.
    pub fn revalidate(
        target: &DeleteTarget,
        environment: &PlatformEnvironment,
    ) -> RevalidationOutcome {
        let path = &target.path;

        // 0. Presence probe. A path that no longer exists is already absent.
        match std::fs::symlink_metadata(path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return RevalidationOutcome::AlreadyAbsent(Self::already_absent(target));
            }
            Err(error) => {
                let error_str = error.to_string();
                return RevalidationOutcome::Failed(CleanItemResult {
                    item_id: target.item_id.clone(),
                    name: target.name.clone(),
                    path: path.to_string_lossy().to_string(),
                    status: CleanStatus::Failed,
                    success: false,
                    bytes_reclaimed: 0,
                    failure_reason: Some(CleanFailureReason::PermissionDenied),
                    error_message: Some(zenith_platform::environment::describe_access_refusal(
                        environment,
                        path,
                        &error_str,
                    )),
                });
            }
        }

        // Every present filesystem target must carry the identity captured by
        // the planner. Absence is handled above, while provider-backed
        // strategies never reach a filesystem mutation primitive.
        if matches!(
            target.strategy,
            CleanStrategy::DeleteContents | CleanStrategy::DeleteDirectory
        ) && target.identity.is_none()
        {
            return RevalidationOutcome::Failed(CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                status: CleanStatus::Failed,
                success: false,
                bytes_reclaimed: 0,
                failure_reason: Some(CleanFailureReason::ChangedSinceScan),
                error_message: Some(format!(
                    "Filesystem identity is missing for {}; refusing to mutate",
                    path.display()
                )),
            });
        }

        // 1. Blacklist check (lexical & canonical, fail closed on mutation)
        if let Err(e) = Blacklist::validate_with(path, environment) {
            return RevalidationOutcome::Failed(CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                status: CleanStatus::Failed,
                success: false,
                bytes_reclaimed: 0,
                failure_reason: Some(CleanFailureReason::Blacklisted),
                error_message: Some(e.to_string()),
            });
        }
        if let Err(e) = SymlinkGuard::validate_canonical_blacklist_strict(path, environment) {
            if crate::safety::is_already_absent(&e) {
                return RevalidationOutcome::AlreadyAbsent(Self::already_absent(target));
            }
            return RevalidationOutcome::Failed(CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                status: CleanStatus::Failed,
                success: false,
                bytes_reclaimed: 0,
                failure_reason: Some(CleanFailureReason::Blacklisted),
                error_message: Some(e.to_string()),
            });
        }

        // 1b. Symlink metadata must be readable; failure fails closed.
        if let Err(e) = SymlinkGuard::is_symlink_strict(path) {
            if crate::safety::is_already_absent(&e) {
                return RevalidationOutcome::AlreadyAbsent(Self::already_absent(target));
            }
            return RevalidationOutcome::Failed(CleanItemResult {
                item_id: target.item_id.clone(),
                name: target.name.clone(),
                path: path.to_string_lossy().to_string(),
                status: CleanStatus::Failed,
                success: false,
                bytes_reclaimed: 0,
                failure_reason: Some(CleanFailureReason::ChangedSinceScan),
                error_message: Some(e.to_string()),
            });
        }

        // 2. Check path existence.
        if !path.exists() && !SymlinkGuard::is_symlink(path) {
            return RevalidationOutcome::AlreadyAbsent(Self::already_absent(target));
        }

        // 3. TOCTOU identity verification
        if let Some(expected_identity) = &target.identity {
            if let Err(e) = ToctouGuard::verify(path, expected_identity) {
                if crate::safety::is_already_absent(&e) {
                    return RevalidationOutcome::AlreadyAbsent(Self::already_absent(target));
                }
                return RevalidationOutcome::Failed(CleanItemResult {
                    item_id: target.item_id.clone(),
                    name: target.name.clone(),
                    path: path.to_string_lossy().to_string(),
                    status: CleanStatus::Failed,
                    success: false,
                    bytes_reclaimed: 0,
                    failure_reason: Some(CleanFailureReason::ChangedSinceScan),
                    error_message: Some(e.to_string()),
                });
            }
        }

        // 3b. Stale Temp Directory TOCTOU: re-verify freshness invariant if target has min_age_days
        if let Some(days) = target.min_age_days {
            if path.is_dir() {
                // The execution-time age re-check is not part of a
                // cancellable scan: it must observe the whole tree before a
                // deletion is allowed, so it runs to completion.
                let stats = crate::scanner::DirectoryScanner::measure_tree_stats(
                    environment,
                    path,
                    &target.exclusions,
                    0,
                    32,
                    &crate::models::NeverCancelled,
                );
                if !stats.complete {
                    if let Err(error) = std::fs::symlink_metadata(path) {
                        if error.kind() == std::io::ErrorKind::NotFound {
                            return RevalidationOutcome::AlreadyAbsent(Self::already_absent(
                                target,
                            ));
                        }
                    }
                    return RevalidationOutcome::Failed(CleanItemResult {
                        item_id: target.item_id.clone(),
                        name: target.name.clone(),
                        path: path.to_string_lossy().to_string(),
                        status: CleanStatus::Failed,
                        success: false,
                        bytes_reclaimed: 0,
                        failure_reason: Some(CleanFailureReason::ChangedSinceScan),
                        error_message: Some(
                            "Directory structure could not be fully verified; aborted to protect active files"
                                .to_string(),
                        ),
                    });
                }
                let Some(newest) = stats.newest_mtime else {
                    return RevalidationOutcome::Failed(CleanItemResult {
                        item_id: target.item_id.clone(),
                        name: target.name.clone(),
                        path: path.to_string_lossy().to_string(),
                        status: CleanStatus::Failed,
                        success: false,
                        bytes_reclaimed: 0,
                        failure_reason: Some(CleanFailureReason::ChangedSinceScan),
                        error_message: Some(
                            "Directory modification timestamp unavailable; aborted to protect active files"
                                .to_string(),
                        ),
                    });
                };
                let minimum_age = std::time::Duration::from_secs(days as u64 * 86_400);
                if std::time::SystemTime::now()
                    .duration_since(newest)
                    .unwrap_or_default()
                    < minimum_age
                {
                    return RevalidationOutcome::Failed(CleanItemResult {
                        item_id: target.item_id.clone(),
                        name: target.name.clone(),
                        path: path.to_string_lossy().to_string(),
                        status: CleanStatus::Failed,
                        success: false,
                        bytes_reclaimed: 0,
                        failure_reason: Some(CleanFailureReason::ChangedSinceScan),
                        error_message: Some(format!(
                            "Directory contains files modified within the last {days} days; aborted cleanup to protect active processes"
                        )),
                    });
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
