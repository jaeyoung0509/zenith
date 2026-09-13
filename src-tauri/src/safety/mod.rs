pub mod blacklist;
pub mod planner;
pub mod symlink;
pub mod toctou;
pub mod tree_deleter;

pub use blacklist::Blacklist;
pub use planner::SafetyPlanner;
pub use symlink::SymlinkGuard;
pub use toctou::ToctouGuard;
pub use tree_deleter::{SafeTreeDeleter, TreeDeleteReport};

use crate::models::ZenithError;

/// Whether an error means the path is already gone.
///
/// This is the single place that decides absence, so a permission refusal, a
/// symlink escape, or a changed identity can never be read as "nothing to do".
/// Only [`ZenithError::Missing`] qualifies; every other variant keeps failing
/// closed.
pub fn is_already_absent(error: &ZenithError) -> bool {
    matches!(error, ZenithError::Missing(_))
}

#[cfg(test)]
mod tests {
    use super::is_already_absent;
    use crate::models::ZenithError;

    /// Only genuine absence counts as already-absent: a changed identity, a
    /// permission refusal, and a symlink escape must all keep failing closed.
    #[test]
    fn is_already_absent_only_matches_missing() {
        assert!(is_already_absent(&ZenithError::Missing(
            "/already/gone".into()
        )));
        assert!(!is_already_absent(&ZenithError::ChangedSinceScan(
            "identity mismatch".into()
        )));
        assert!(!is_already_absent(&ZenithError::PermissionDenied(
            "/protected".into()
        )));
        assert!(!is_already_absent(&ZenithError::SymlinkEscape(
            "/escape".into()
        )));
    }

    #[test]
    fn missing_display_names_the_path_without_calling_it_a_change() {
        assert_eq!(
            ZenithError::Missing("/already/gone".into()).to_string(),
            "Path no longer exists: /already/gone"
        );
    }
}
