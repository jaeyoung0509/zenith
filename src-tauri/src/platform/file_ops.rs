//! Shared atomic file writes.
//!
//! Callers get the guarantee that a destination is never deleted before its
//! replacement is fully written: the new contents land in a unique temporary
//! file in the same directory, and only then is the destination atomically
//! replaced. A failed replacement leaves the previous destination untouched.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

/// Writes `bytes` to `destination` atomically.
pub fn atomic_write(destination: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = destination.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "atomic write destination has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "atomic write destination has no usable file name",
            )
        })?;
    let temporary = parent.join(format!(".{file_name}.tmp.{}", uuid::Uuid::new_v4()));

    let write_result = (|| -> std::io::Result<()> {
        let mut file = File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }

    let replace_result = atomic_replace(&temporary, destination);
    if replace_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    replace_result
}

/// Replaces `destination` with `temporary` in one step.
pub(crate) fn atomic_replace(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        windows_replace(temporary, destination)
    }
    #[cfg(not(windows))]
    {
        fs::rename(temporary, destination)
    }
}

/// On Windows a plain rename fails when the destination exists, so the native
/// replacement primitives are used instead of deleting the destination first.
#[cfg(windows)]
fn windows_replace(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    if !destination.exists() {
        return fs::rename(temporary, destination);
    }

    let temporary_wide = crate::platform::NativePlatformPaths::to_verbatim_wide(temporary);
    let destination_wide = crate::platform::NativePlatformPaths::to_verbatim_wide(destination);

    // ReplaceFileW preserves the destination's attributes and ACLs and never
    // removes it before the replacement is ready.
    let replaced = unsafe {
        ReplaceFileW(
            destination_wide.as_ptr(),
            temporary_wide.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced != 0 {
        return Ok(());
    }

    // Fallback for filesystems that do not implement ReplaceFile: MoveFileExW
    // still replaces in one call with no delete-before-replace window.
    let moved = unsafe {
        MoveFileExW(
            temporary_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{atomic_replace, atomic_write};
    use std::fs;

    fn temp_named_entries(dir: &std::path::Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .filter(|name| name.contains(".tmp."))
            .collect()
    }

    #[test]
    fn first_write_creates_the_destination_without_leftover_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("settings.json");

        atomic_write(&destination, b"{\"value\":1}").unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"{\"value\":1}");
        assert!(temp_named_entries(dir.path()).is_empty());
    }

    #[test]
    fn replacement_swaps_contents_without_leftover_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("audit.json");
        fs::write(&destination, b"old").unwrap();

        atomic_write(&destination, b"new").unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"new");
        assert!(temp_named_entries(dir.path()).is_empty());
    }

    #[test]
    fn failed_replacement_leaves_the_existing_destination_intact() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("settings.json");
        let missing_temporary = dir.path().join("missing.tmp");
        fs::write(&destination, b"previous settings").unwrap();

        let result = atomic_replace(&missing_temporary, &destination);

        assert!(result.is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"previous settings");
    }
}
