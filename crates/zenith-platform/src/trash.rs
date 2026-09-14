//! The operating system's Trash / Recycle Bin, behind a port.

use std::path::Path;

/// A path the reviewed-storage layer has authorized for the OS Trash.
///
/// The port takes this rather than a `&Path` so a reviewed move cannot be
/// requested by handing a string to the backend: the caller has to produce a
/// value whose type records that scope, identity, and evidence checks already
/// passed. The reviewed-storage context (`src-tauri/src/trash_manager`) owns
/// the only implementation, and mints it immediately before the move.
pub trait ReviewedTrashEntry {
    /// The absolute path to move.
    fn path(&self) -> &Path;
}

/// Port for moving files and directories to the operating system's Trash / Recycle Bin.
pub trait TrashBackend: Send + Sync {
    fn move_to_trash(&self, entry: &dyn ReviewedTrashEntry) -> Result<(), String>;
}

/// Native OS implementation using the `trash` crate.
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeTrashBackend;

impl TrashBackend for NativeTrashBackend {
    fn move_to_trash(&self, entry: &dyn ReviewedTrashEntry) -> Result<(), String> {
        trash::delete(entry.path()).map_err(|error| format!("Could not move to Trash: {error}"))
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
    fn move_to_trash(&self, entry: &dyn ReviewedTrashEntry) -> Result<(), String> {
        let path = entry.path();
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

    struct Entry(PathBuf);

    impl ReviewedTrashEntry for Entry {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    #[test]
    fn mock_trash_backend_records_and_can_fail() {
        let backend = MockTrashBackend::new();
        let target = PathBuf::from("/tmp/test-item");
        assert!(backend.move_to_trash(&Entry(target.clone())).is_ok());
        assert_eq!(backend.moved(), vec![target.clone()]);

        let failing = MockTrashBackend::with_failing_path(&target);
        assert!(failing.move_to_trash(&Entry(target)).is_err());
    }
}
