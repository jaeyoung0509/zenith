use std::path::{Path, PathBuf};

use crate::models::{
    CleanFailureReason, CleanItemResult, CleanStatus, CleanStrategy, DeleteTarget, ZenithError,
};
use crate::platform::PlatformEnvironment;
use crate::safety::{Blacklist, SymlinkGuard, ToctouGuard};

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
                    error_message: Some(crate::platform::environment::describe_access_refusal(
                        environment,
                        path,
                        &error_str,
                    )),
                });
            }
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
                let stats = crate::scanner::DirectoryScanner::measure_tree_stats(
                    environment,
                    path,
                    &target.exclusions,
                    0,
                    32,
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

/// An authorized local AI model deletion target that has passed model inventory scope
/// and symlink validations.
///
/// Can only be constructed through `ValidatedModelTarget::validate()`.
#[derive(Debug, Clone)]
pub struct ValidatedModelTarget {
    path: PathBuf,
}

impl ValidatedModelTarget {
    /// Validates that `path` is directly scoped under `allowed_root`, does not point to
    /// `allowed_root` itself, is not blacklisted, and contains no intermediate symlink ancestors.
    pub fn validate(
        path: &Path,
        allowed_root: &Path,
        environment: &PlatformEnvironment,
    ) -> Result<Self, ZenithError> {
        if path == allowed_root || !path.starts_with(allowed_root) {
            return Err(ZenithError::PathNotAllowed(
                path.to_string_lossy().to_string(),
            ));
        }

        Blacklist::validate_with(path, environment)?;
        SymlinkGuard::validate_canonical_blacklist_strict(path, environment)?;
        SymlinkGuard::validate_no_symlink_ancestors(path, allowed_root, environment)?;

        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Authority token required by `SafeTreeDeleter` to perform filesystem mutations.
///
/// Ensures raw paths cannot be deleted without first obtaining an authority token
/// through a domain safety gate (`ValidatedTarget` for general cleanup,
/// `ValidatedModelTarget` for model inventory cleanup).
pub enum FilesystemDeleteAuthority<'a> {
    Cleanup(&'a ValidatedTarget),
    ModelInventory(&'a ValidatedModelTarget),
    #[doc(hidden)]
    TestDirect {
        path: &'a Path,
        exclusions: &'a [String],
    },
}

impl<'a> FilesystemDeleteAuthority<'a> {
    #[doc(hidden)]
    pub fn test_direct(path: &'a Path, exclusions: &'a [String]) -> Self {
        Self::TestDirect { path, exclusions }
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::Cleanup(target) => target.path(),
            Self::ModelInventory(model) => model.path(),
            Self::TestDirect { path, .. } => path,
        }
    }

    pub fn exclusions(&self) -> &[String] {
        match self {
            Self::Cleanup(target) => target.exclusions(),
            Self::ModelInventory(_) => &[],
            Self::TestDirect { exclusions, .. } => exclusions,
        }
    }
}

impl<'a> From<&'a ValidatedTarget> for FilesystemDeleteAuthority<'a> {
    fn from(target: &'a ValidatedTarget) -> Self {
        Self::Cleanup(target)
    }
}

impl<'a> From<&'a ValidatedModelTarget> for FilesystemDeleteAuthority<'a> {
    fn from(target: &'a ValidatedModelTarget) -> Self {
        Self::ModelInventory(target)
    }
}
