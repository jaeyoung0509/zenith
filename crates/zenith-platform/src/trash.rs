use std::path::Path;

/// Port for moving files and directories to the operating system's Trash / Recycle Bin.
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
#[derive(Debug, Default)]
pub struct MockTrashBackend {
    pub moved_paths: std::sync::Mutex<Vec<std::path::PathBuf>>,
    pub fail_paths: std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>,
}

impl MockTrashBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_failing_path(path: impl Into<std::path::PathBuf>) -> Self {
        let backend = Self::new();
        backend.fail_paths.lock().unwrap().insert(path.into());
        backend
    }

    pub fn moved(&self) -> Vec<std::path::PathBuf> {
        self.moved_paths.lock().unwrap().clone()
    }
}

impl TrashBackend for MockTrashBackend {
    fn move_to_trash(&self, path: &Path) -> Result<(), String> {
        if self.fail_paths.lock().unwrap().contains(path) {
            return Err(format!("Mock Trash refused to move {:?}", path));
        }
        self.moved_paths.lock().unwrap().push(path.to_path_buf());
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
