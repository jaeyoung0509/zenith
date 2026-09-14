//! The operating system's Trash / Recycle Bin, behind a port.

use std::path::Path;

/// Port for moving a path to the operating system's Trash / Recycle Bin.
///
/// This is the raw native primitive. It is deliberately *not* an authority
/// boundary: a trait any crate can implement cannot be a capability, and a
/// type-level seal across a crate edge is not expressible without moving the
/// reviewer into this crate. Authorization therefore stays where the review
/// evidence lives — `TrashExecutor` in the reviewed-storage context validates a
/// target and is the only production caller — and this trait documents that
/// obligation instead of pretending to enforce it.
pub trait TrashBackend: Send + Sync {
    fn move_to_trash(&self, path: &Path) -> Result<(), String>;
}

/// Native OS implementation using the `trash` crate.
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeTrashBackend;

impl TrashBackend for NativeTrashBackend {
    fn move_to_trash(&self, path: &Path) -> Result<(), String> {
        trash::delete(path).map_err(|error| format!("Could not move to Trash: {error}"))
    }
}

/// In-memory mock trash backend for tests, recording operations without mutating the filesystem.
///
/// A poisoned lock here means a test panicked while recording a move, and the
/// recorded state is disposable: recovering it keeps the failure the test
/// already reported instead of turning it into a lock panic.
#[derive(Debug, Default)]
pub struct MockTrashBackend {
    moved: std::sync::Mutex<Vec<std::path::PathBuf>>,
    failing: std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>,
}

impl MockTrashBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_failing_path(path: impl Into<std::path::PathBuf>) -> Self {
        let backend = Self::new();
        backend
            .failing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(path.into());
        backend
    }

    /// The paths the backend was asked to move, in order.
    pub fn moved(&self) -> Vec<std::path::PathBuf> {
        self.moved
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl TrashBackend for MockTrashBackend {
    fn move_to_trash(&self, path: &Path) -> Result<(), String> {
        let refused = self
            .failing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(path);
        if refused {
            return Err(format!("Mock Trash refused to move {:?}", path));
        }
        self.moved
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(path.to_path_buf());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn mock_trash_backend_records_and_can_fail() {
        let backend = MockTrashBackend::new();
        let target = PathBuf::from("/tmp/test-item");
        assert!(backend.move_to_trash(&target).is_ok());
        assert_eq!(backend.moved(), vec![target.clone()]);

        let failing = MockTrashBackend::with_failing_path(&target);
        assert!(failing.move_to_trash(&target).is_err());
    }
}
