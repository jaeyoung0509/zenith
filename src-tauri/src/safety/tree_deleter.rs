use crate::platform::PlatformEnvironment;
#[cfg(windows)]
use crate::safety::ToctouGuard;
use crate::safety::{Blacklist, SymlinkGuard};
use crate::signatures::SignatureLoader;
#[cfg(unix)]
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeDeleteReport {
    pub reclaimed_bytes: u64,
    pub deleted_files: usize,
    pub skipped_files: usize,
    pub errors: Vec<String>,
    /// Raw OS error codes recorded alongside `errors`, in push order, so
    /// failure classification can use the code instead of localized text.
    pub os_error_codes: Vec<i32>,
}

impl TreeDeleteReport {
    pub fn is_success(&self) -> bool {
        self.errors.is_empty()
    }
}

pub struct SafeTreeDeleter;

/// Records the directory identity and mode before cleanup temporarily adds the
/// owner permissions needed to remove a read-only tree.
#[derive(Debug, Default)]
struct PermissionSnapshot {
    #[cfg(unix)]
    original_mode: Option<u32>,
    #[cfg(unix)]
    directory: Option<fs::File>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    directory: Option<WindowsDeleteHandle>,
}

#[cfg(windows)]
#[derive(Debug)]
struct WindowsDeleteHandle {
    handle: windows_sys::Win32::Foundation::HANDLE,
    device: u64,
    inode: u64,
    is_dir: bool,
    is_reparse_point: bool,
    attributes: u32,
    was_readonly: bool,
}

#[cfg(windows)]
impl Drop for WindowsDeleteHandle {
    fn drop(&mut self) {
        if !self.handle.is_null()
            && self.handle != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE
        {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(windows)]
fn retry_on_sharing_violation<T, F>(mut f: F) -> io::Result<T>
where
    F: FnMut() -> io::Result<T>,
{
    const MAX_RETRIES: usize = 4;
    const INITIAL_BACKOFF_MS: u64 = 10;

    let mut attempt = 0;
    loop {
        match f() {
            Ok(val) => return Ok(val),
            Err(err) => {
                let is_sharing = err.raw_os_error()
                    == Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32);
                if is_sharing && attempt < MAX_RETRIES {
                    let delay =
                        std::time::Duration::from_millis(INITIAL_BACKOFF_MS * (1 << attempt));
                    std::thread::sleep(delay);
                    attempt += 1;
                    continue;
                }
                return Err(err);
            }
        }
    }
}

#[cfg(windows)]
fn format_io_error(path: &Path, err: &io::Error) -> String {
    if err.raw_os_error() == Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32) {
        return format!(
            "{}: Sharing violation (file in use by another process): {}",
            path.display(),
            err
        );
    }
    if err.raw_os_error() == Some(windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED as i32) {
        return format!(
            "{}: Access denied (permission denied): {}",
            path.display(),
            err
        );
    }
    format!("{}: {}", path.display(), err)
}

/// Records a Windows deletion failure with its raw OS error code so callers
/// can classify it without parsing localized message text.
#[cfg(windows)]
fn push_windows_io_error(report: &mut TreeDeleteReport, path: &Path, error: &io::Error) {
    if let Some(code) = error.raw_os_error() {
        report.os_error_codes.push(code);
    }
    report.errors.push(format_io_error(path, error));
}

#[cfg(windows)]
impl WindowsDeleteHandle {
    fn open(path: &Path) -> io::Result<Self> {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, DELETE,
            FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES, OPEN_EXISTING,
        };

        let wide = crate::platform::NativePlatformPaths::to_verbatim_wide(path);

        let handle = retry_on_sharing_violation(|| {
            let h = unsafe {
                CreateFileW(
                    wide.as_ptr(),
                    DELETE | FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                    std::ptr::null_mut(),
                )
            };
            if h == INVALID_HANDLE_VALUE || h.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(h)
            }
        })?;

        let mut result = Self {
            handle,
            device: 0,
            inode: 0,
            is_dir: false,
            is_reparse_point: false,
            attributes: 0,
            was_readonly: false,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(result.handle, &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let (dev, ino) = crate::safety::toctou::windows_identity_from_handle(result.handle)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "filesystem returned an unverifiable zero file identity",
                )
            })?;
        result.device = dev;
        result.inode = ino;
        result.is_dir = info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
        result.is_reparse_point = info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        result.attributes = info.dwFileAttributes;
        result.was_readonly = info.dwFileAttributes & FILE_ATTRIBUTE_READONLY != 0;
        Ok(result)
    }

    fn clear_readonly(&mut self) -> io::Result<()> {
        use windows_sys::Win32::Storage::FileSystem::{
            FileBasicInfo, SetFileInformationByHandle, FILE_ATTRIBUTE_NORMAL,
            FILE_ATTRIBUTE_READONLY, FILE_BASIC_INFO,
        };

        if self.attributes & FILE_ATTRIBUTE_READONLY == 0 {
            return Ok(());
        }

        let new_attrs = self.attributes & !FILE_ATTRIBUTE_READONLY;
        let mut basic_info: FILE_BASIC_INFO = unsafe { std::mem::zeroed() };
        basic_info.FileAttributes = if new_attrs == 0 {
            FILE_ATTRIBUTE_NORMAL
        } else {
            new_attrs
        };

        retry_on_sharing_violation(|| {
            let ok = unsafe {
                SetFileInformationByHandle(
                    self.handle,
                    FileBasicInfo,
                    &basic_info as *const FILE_BASIC_INFO as *const std::ffi::c_void,
                    std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                )
            };
            if ok == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        })?;

        self.attributes = new_attrs;
        Ok(())
    }

    fn restore_readonly(&self) -> io::Result<()> {
        use windows_sys::Win32::Storage::FileSystem::{
            FileBasicInfo, SetFileInformationByHandle, FILE_ATTRIBUTE_READONLY, FILE_BASIC_INFO,
        };

        let mut basic_info: FILE_BASIC_INFO = unsafe { std::mem::zeroed() };
        basic_info.FileAttributes = self.attributes | FILE_ATTRIBUTE_READONLY;

        retry_on_sharing_violation(|| {
            let ok = unsafe {
                SetFileInformationByHandle(
                    self.handle,
                    FileBasicInfo,
                    &basic_info as *const FILE_BASIC_INFO as *const std::ffi::c_void,
                    std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                )
            };
            if ok == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        })
    }

    fn delete(mut self) -> io::Result<()> {
        self.clear_readonly()?;

        let result = self.mark_for_deletion();

        if result.is_err() && self.was_readonly {
            if let Err(restore_error) = self.restore_readonly() {
                eprintln!(
                    "Failed to restore readonly attribute after deletion failure: {}",
                    restore_error
                );
            }
        }

        result
    }

    fn mark_for_deletion(&self) -> io::Result<()> {
        use windows_sys::Win32::Storage::FileSystem::{
            FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
        };

        let disposition = FILE_DISPOSITION_INFO { DeleteFile: 1 };
        retry_on_sharing_violation(|| {
            let ok = unsafe {
                SetFileInformationByHandle(
                    self.handle,
                    FileDispositionInfo,
                    &disposition as *const FILE_DISPOSITION_INFO as *const std::ffi::c_void,
                    std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            };
            if ok == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        })
    }
}

impl SafeTreeDeleter {
    pub fn delete_contents(
        root: &Path,
        exclusions: &[String],
        environment: &PlatformEnvironment,
    ) -> TreeDeleteReport {
        let mut report = TreeDeleteReport::default();
        let root_metadata = match fs::symlink_metadata(root) {
            Ok(metadata) => metadata,
            Err(error) => {
                // Fail closed: metadata failure aborts instead of being
                // treated as "nothing to delete".
                if !root.exists() {
                    return report;
                }
                report.errors.push(format!("{}: {}", root.display(), error));
                return report;
            }
        };
        let root_is_link = match SymlinkGuard::is_symlink_strict(root) {
            Ok(is_link) => is_link,
            Err(error) => {
                report.errors.push(error.to_string());
                return report;
            }
        };
        if root_is_link || !root_metadata.is_dir() {
            Self::delete_entry(root, root, exclusions, environment, &mut report);
            return report;
        }

        if let Err(e) = Blacklist::validate_with(root, environment) {
            report.errors.push(e.to_string());
            return report;
        }
        if let Err(e) = SymlinkGuard::validate_canonical_blacklist_strict(root, environment) {
            report.errors.push(e.to_string());
            return report;
        }

        let permissions = match Self::prepare_directory(root, &root_metadata) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                report.errors.push(error);
                return report;
            }
        };
        if let Err(error) = Self::verify_directory_identity(root, &permissions) {
            report.errors.push(error);
            Self::restore_directory_permissions(root, permissions, &mut report);
            return report;
        }

        #[cfg(unix)]
        {
            if let Some(ref dir_file) = permissions.directory {
                Self::delete_dir_contents_via_fd(
                    root,
                    dir_file,
                    root,
                    exclusions,
                    environment,
                    &mut report,
                );
                Self::restore_directory_permissions(root, permissions, &mut report);
                return report;
            }
        }

        let entries = match fs::read_dir(root) {
            Ok(e) => e,
            Err(e) => {
                report.errors.push(e.to_string());
                Self::restore_directory_permissions(root, permissions, &mut report);
                return report;
            }
        };

        for entry in entries {
            match entry {
                Ok(ent) => {
                    Self::delete_entry(&ent.path(), root, exclusions, environment, &mut report)
                }
                Err(e) => report.errors.push(e.to_string()),
            }
        }
        Self::restore_directory_permissions(root, permissions, &mut report);
        report
    }

    pub fn delete_path(
        root: &Path,
        exclusions: &[String],
        environment: &PlatformEnvironment,
    ) -> TreeDeleteReport {
        let mut report = TreeDeleteReport::default();
        match fs::symlink_metadata(root) {
            Ok(_) => {}
            Err(error) => {
                if !root.exists() && !SymlinkGuard::is_symlink(root) {
                    return report;
                }
                report.errors.push(format!("{}: {}", root.display(), error));
                return report;
            }
        }
        if let Err(e) = Blacklist::validate_with(root, environment) {
            report.errors.push(e.to_string());
            return report;
        }
        if let Err(e) = SymlinkGuard::validate_canonical_blacklist_strict(root, environment) {
            report.errors.push(e.to_string());
            return report;
        }
        Self::delete_entry(root, root, exclusions, environment, &mut report);
        report
    }

    /// Deletes directory children using the already-verified parent directory
    /// descriptor. The final unlink for every child goes through `unlinkat`
    /// on that descriptor, so replacing or redirecting any parent component
    /// after validation cannot redirect deletion outside the planned scope.
    #[cfg(unix)]
    fn delete_dir_contents_via_fd(
        dir_path: &Path,
        dir_file: &fs::File,
        verified_root: &Path,
        exclusions: &[String],
        environment: &PlatformEnvironment,
        report: &mut TreeDeleteReport,
    ) {
        let entries = match fs::read_dir(dir_path) {
            Ok(e) => e.collect::<Vec<_>>(),
            Err(e) => {
                report.errors.push(e.to_string());
                return;
            }
        };

        for entry in entries {
            let ent = match entry {
                Ok(ent) => ent,
                Err(e) => {
                    report.errors.push(e.to_string());
                    continue;
                }
            };
            let child_path = ent.path();
            let Some(file_name) = child_path.file_name() else {
                report.errors.push(format!(
                    "Could not determine file name for {}",
                    child_path.display()
                ));
                continue;
            };

            if Self::is_excluded(&child_path, exclusions, environment)
                || Blacklist::is_blacklisted_with(&child_path, environment)
            {
                report.skipped_files += 1;
                continue;
            }

            let metadata = match fs::symlink_metadata(&child_path) {
                Ok(m) => m,
                Err(e) => {
                    report
                        .errors
                        .push(format!("{}: {}", child_path.display(), e));
                    continue;
                }
            };

            if let Err(error) = Self::validate_verified_scope(&child_path, verified_root) {
                report.errors.push(error);
                continue;
            }
            if let Err(error) =
                SymlinkGuard::validate_canonical_blacklist_strict(&child_path, environment)
            {
                report
                    .errors
                    .push(format!("{}: {}", child_path.display(), error));
                continue;
            }
            if let Err(error) = Self::validate_entry_owner(&child_path, &metadata) {
                report.errors.push(error);
                continue;
            }

            if metadata.file_type().is_symlink() || metadata.is_file() {
                if let Err(error) = Self::verify_entry_identity(&child_path, &metadata) {
                    report.errors.push(error);
                    continue;
                }
                let bytes = allocated_bytes(&metadata);
                match Self::unlink_via_parent(dir_file, file_name, false) {
                    Ok(()) => {
                        report.reclaimed_bytes += bytes;
                        report.deleted_files += 1;
                    }
                    Err(e) => {
                        report
                            .errors
                            .push(format!("{}: {}", child_path.display(), e));
                    }
                }
                continue;
            }

            if !metadata.is_dir() {
                continue;
            }

            // Recurse with the child's own verified descriptor.
            let child_permissions = match Self::prepare_directory(&child_path, &metadata) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    report.errors.push(error);
                    continue;
                }
            };
            if let Err(error) = Self::verify_directory_identity(&child_path, &child_permissions) {
                report.errors.push(error);
                Self::restore_directory_permissions(&child_path, child_permissions, report);
                continue;
            }
            if let Some(ref child_file) = child_permissions.directory {
                Self::delete_dir_contents_via_fd(
                    &child_path,
                    child_file,
                    verified_root,
                    exclusions,
                    environment,
                    report,
                );
            }
            if let Err(error) = Self::verify_directory_identity(&child_path, &child_permissions) {
                report.errors.push(error);
                Self::restore_directory_permissions(&child_path, child_permissions, report);
                continue;
            }
            // Remove the now-empty child directory relative to the verified
            // parent descriptor. The child's own permissions are restored
            // implicitly by removing it; only restore on failure.
            match Self::unlink_via_parent(dir_file, file_name, true) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => {
                    Self::restore_directory_permissions(&child_path, child_permissions, report);
                }
                Err(error) => {
                    report
                        .errors
                        .push(format!("{}: {}", child_path.display(), error));
                    Self::restore_directory_permissions(&child_path, child_permissions, report);
                }
            }
        }
    }

    /// Unlinks a child name relative to a verified parent directory
    /// descriptor. Never re-resolves the full path for the final mutation.
    #[cfg(unix)]
    fn unlink_via_parent(parent: &fs::File, name: &OsStr, is_dir: bool) -> io::Result<()> {
        use std::os::unix::ffi::OsStrExt;

        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes.contains(&b'/') || bytes == b"." || bytes == b".." {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to unlink unsafe child name",
            ));
        }
        let mut buf = Vec::with_capacity(bytes.len() + 1);
        buf.extend_from_slice(bytes);
        buf.push(0);
        let flags = if is_dir { libc::AT_REMOVEDIR } else { 0 };
        // SAFETY: `buf` is a NUL-terminated non-empty basename without `/`,
        // and `parent` is a verified directory fd opened with O_NOFOLLOW.
        let res = unsafe {
            libc::unlinkat(
                parent.as_raw_fd(),
                buf.as_ptr() as *const libc::c_char,
                flags,
            )
        };
        if res == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn delete_entry(
        path: &Path,
        verified_root: &Path,
        exclusions: &[String],
        environment: &PlatformEnvironment,
        report: &mut TreeDeleteReport,
    ) {
        if Self::is_excluded(path, exclusions, environment)
            || Blacklist::is_blacklisted_with(path, environment)
        {
            report.skipped_files += 1;
            return;
        }

        let metadata = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                report.errors.push(format!("{}: {}", path.display(), e));
                return;
            }
        };

        if let Err(error) = Self::validate_verified_scope(path, verified_root) {
            report.errors.push(error);
            return;
        }

        // Re-check the canonical location at every recursive entry before any
        // permission change or deletion. Symlink entries are still removed as
        // links, never traversed. Canonicalization failure fails closed.
        if let Err(error) = SymlinkGuard::validate_canonical_blacklist_strict(path, environment) {
            report.errors.push(format!("{}: {}", path.display(), error));
            return;
        }

        let is_link = match SymlinkGuard::is_symlink_strict(path) {
            Ok(is_link) => is_link,
            Err(error) => {
                report.errors.push(format!("{}: {}", path.display(), error));
                return;
            }
        };

        if let Err(error) = Self::validate_entry_owner(path, &metadata) {
            report.errors.push(error);
            return;
        }

        if is_link || metadata.is_file() {
            if let Err(error) = Self::verify_entry_identity(path, &metadata) {
                report.errors.push(error);
                return;
            }
            let bytes = allocated_bytes(&metadata);
            #[cfg(unix)]
            {
                // Prefer descriptor-relative unlink via the verified parent.
                // Falls back to path removal only when the parent cannot be
                // opened safely, in which case the error fails closed.
                match Self::remove_file_via_verified_parent(path) {
                    Ok(()) => {
                        report.reclaimed_bytes += bytes;
                        report.deleted_files += 1;
                    }
                    Err(e) => {
                        report.errors.push(format!("{}: {}", path.display(), e));
                    }
                }
                return;
            }
            #[cfg(windows)]
            {
                match Self::delete_windows_entry(path, &metadata) {
                    Ok(()) => {
                        report.reclaimed_bytes += bytes;
                        report.deleted_files += 1;
                    }
                    Err(e) => {
                        push_windows_io_error(report, path, &e);
                    }
                }
                return;
            }
            #[cfg(not(any(unix, windows)))]
            {
                match fs::remove_file(path) {
                    Ok(()) => {
                        report.reclaimed_bytes += bytes;
                        report.deleted_files += 1;
                    }
                    Err(e) => report.errors.push(format!("{}: {}", path.display(), e)),
                }
                return;
            }
        }

        if !metadata.is_dir() {
            return;
        }

        let permissions = match Self::prepare_directory(path, &metadata) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                report.errors.push(error);
                return;
            }
        };
        if let Err(error) = Self::verify_directory_identity(path, &permissions) {
            report.errors.push(error);
            Self::restore_directory_permissions(path, permissions, report);
            return;
        }

        #[cfg(unix)]
        if let Some(ref _dir_file) = permissions.directory {
            Self::delete_dir_contents_via_fd(
                path,
                _dir_file,
                verified_root,
                exclusions,
                environment,
                report,
            );
            if let Err(error) = Self::verify_directory_identity(path, &permissions) {
                report.errors.push(error);
                Self::restore_directory_permissions(path, permissions, report);
                return;
            }
            // The top-level directory itself is removed relative to its own
            // verified parent so a replaced parent cannot redirect it.
            // `delete_contents` never reaches here for its root (it returns
            // after `delete_dir_contents_via_fd`); `delete_path` removes it.
            let remove_result = Self::remove_dir_via_verified_parent(path);
            match remove_result {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => {
                    Self::restore_directory_permissions(path, permissions, report);
                }
                Err(error) => {
                    report.errors.push(format!("{}: {}", path.display(), error));
                    Self::restore_directory_permissions(path, permissions, report);
                }
            }
            return;
        }

        let entries = match fs::read_dir(path) {
            Ok(e) => e,
            Err(e) => {
                report.errors.push(format!("{}: {}", path.display(), e));
                Self::restore_directory_permissions(path, permissions, report);
                return;
            }
        };

        for entry in entries {
            match entry {
                Ok(ent) => {
                    Self::delete_entry(&ent.path(), verified_root, exclusions, environment, report)
                }
                Err(e) => report.errors.push(format!("{}: {}", path.display(), e)),
            }
        }

        if let Err(error) = Self::verify_directory_identity(path, &permissions) {
            report.errors.push(error);
            Self::restore_directory_permissions(path, permissions, report);
            return;
        }

        #[cfg(windows)]
        {
            let mut permissions = permissions;
            let remove_result = permissions
                .directory
                .take()
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "verified Windows directory handle is unavailable",
                    )
                })
                .and_then(WindowsDeleteHandle::delete);
            match remove_result {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => {
                    Self::restore_directory_permissions(path, permissions, report);
                }
                Err(error) => {
                    push_windows_io_error(report, path, &error);
                    Self::restore_directory_permissions(path, permissions, report);
                }
            }
        }

        #[cfg(not(windows))]
        match fs::remove_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => {
                Self::restore_directory_permissions(path, permissions, report);
            }
            Err(error) => {
                report.errors.push(format!("{}: {}", path.display(), error));
                Self::restore_directory_permissions(path, permissions, report);
            }
        }
    }

    #[cfg(windows)]
    fn delete_windows_entry(path: &Path, expected: &fs::Metadata) -> io::Result<()> {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

        let expected_identity = ToctouGuard::capture(path).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "could not capture filesystem identity before deletion",
            )
        })?;
        let handle = WindowsDeleteHandle::open(path)?;
        let expected_is_reparse = expected.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        if handle.device != expected_identity.device
            || handle.inode != expected_identity.inode
            || handle.is_reparse_point != expected_is_reparse
            || (!expected_is_reparse && handle.is_dir != expected.is_dir())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "entry changed between verification and deletion",
            ));
        }
        handle.delete()
    }

    /// Removes a single file/symlink relative to its verified parent
    /// directory descriptor opened with O_DIRECTORY | O_NOFOLLOW.
    #[cfg(unix)]
    fn remove_file_via_verified_parent(path: &Path) -> io::Result<()> {
        let (parent, name) = Self::open_parent_nofollow(path)?;
        Self::unlink_via_parent(&parent, &name, false)
    }

    /// Removes an empty directory relative to its verified parent descriptor.
    #[cfg(unix)]
    fn remove_dir_via_verified_parent(path: &Path) -> io::Result<()> {
        let (parent, name) = Self::open_parent_nofollow(path)?;
        Self::unlink_via_parent(&parent, &name, true)
    }

    /// Opens the parent directory without following symlinks and verifies it
    /// is still a directory. Returns the parent handle plus the child basename.
    #[cfg(unix)]
    fn open_parent_nofollow(path: &Path) -> io::Result<(fs::File, std::ffi::OsString)> {
        let parent_path = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "path has no parent directory")
        })?;
        let file_name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
        if file_name == "." || file_name == ".." {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to unlink dot path",
            ));
        }
        let parent = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(parent_path)?;
        let meta = parent.metadata()?;
        if !meta.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "parent is no longer a directory",
            ));
        }
        Ok((parent, file_name.to_os_string()))
    }

    /// Make a user-owned directory traversable and writable for the duration
    /// of recursive deletion. Only missing owner bits are added; group/other
    /// permissions and special bits are preserved.
    fn prepare_directory(
        path: &Path,
        expected_metadata: &fs::Metadata,
    ) -> Result<PermissionSnapshot, String> {
        #[cfg(unix)]
        {
            let directory = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)
                .map_err(|error| {
                    format!(
                        "Permission denied: could not safely open cleanup directory ({}): {}",
                        path.display(),
                        error
                    )
                })?;
            let metadata = directory.metadata().map_err(|error| {
                format!(
                    "Could not verify cleanup directory {}: {}",
                    path.display(),
                    error
                )
            })?;
            if metadata.dev() != expected_metadata.dev()
                || metadata.ino() != expected_metadata.ino()
                || !metadata.is_dir()
            {
                return Err(format!(
                    "Directory changed during cleanup: {}",
                    path.display()
                ));
            }

            // fchmod is intentionally limited to a no-follow descriptor for a
            // directory owned by the effective user. Zenith never escalates
            // privileges or chmods a replacement symlink.
            let effective_uid = unsafe { libc::geteuid() } as u32;
            if metadata.uid() != effective_uid {
                return Err(format!(
                    "Permission denied: cleanup directory is not owned by the current user: {}",
                    path.display()
                ));
            }

            let original_mode = metadata.mode() & 0o7777;
            let required_mode = original_mode | 0o700;
            if required_mode != original_mode {
                directory
                    .set_permissions(fs::Permissions::from_mode(required_mode))
                    .map_err(|error| {
                        format!(
                            "Permission denied: could not make cleanup directory writable ({}): {}",
                            path.display(),
                            error
                        )
                    })?;
            }

            let snapshot = PermissionSnapshot {
                original_mode: (required_mode != original_mode).then_some(original_mode),
                directory: Some(directory),
                device: metadata.dev(),
                inode: metadata.ino(),
            };

            Ok(snapshot)
        }

        #[cfg(not(unix))]
        {
            // Windows holds the same no-follow, non-share-delete handle from
            // identity verification through recursive traversal and final
            // disposition. The verified directory therefore cannot be
            // renamed or replaced while cleanup is in progress.
            let current = fs::symlink_metadata(path).map_err(|error| {
                format!(
                    "Could not verify cleanup directory {}: {}",
                    path.display(),
                    error
                )
            })?;
            if current.file_type().is_symlink() || !current.is_dir() {
                return Err(format!(
                    "Directory changed during cleanup: {}",
                    path.display()
                ));
            }
            #[cfg(windows)]
            {
                let expected_identity = ToctouGuard::capture(path).ok_or_else(|| {
                    format!(
                        "Could not capture cleanup directory identity: {}",
                        path.display()
                    )
                })?;
                let mut directory = WindowsDeleteHandle::open(path)
                    .map_err(|error| format_io_error(path, &error))?;
                if directory.device != expected_identity.device
                    || directory.inode != expected_identity.inode
                    || !directory.is_dir
                    || directory.is_reparse_point
                    || expected_metadata.is_dir() != directory.is_dir
                {
                    return Err(format!(
                        "Directory changed during cleanup: {}",
                        path.display()
                    ));
                }
                if let Err(error) = directory.clear_readonly() {
                    return Err(format_io_error(path, &error));
                }
                Ok(PermissionSnapshot {
                    directory: Some(directory),
                })
            }
            #[cfg(not(windows))]
            Ok(PermissionSnapshot::default())
        }
    }

    fn restore_directory_permissions(
        path: &Path,
        snapshot: PermissionSnapshot,
        report: &mut TreeDeleteReport,
    ) {
        #[cfg(unix)]
        if let Some(original_mode) = snapshot.original_mode {
            // Never restore through a replacement or symlink. If the entry
            // changed while cleanup was running, leave it untouched and let
            // the next scan surface the change.
            let Ok(metadata) = fs::symlink_metadata(path) else {
                return;
            };
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || metadata.dev() != snapshot.device
                || metadata.ino() != snapshot.inode
            {
                return;
            }

            let Some(directory) = snapshot.directory.as_ref() else {
                return;
            };
            if let Err(error) = directory.set_permissions(fs::Permissions::from_mode(original_mode))
            {
                report.errors.push(format!(
                    "Permission denied: could not restore directory permissions ({}): {}",
                    path.display(),
                    error
                ));
            }
        }

        #[cfg(windows)]
        {
            if let Some(ref directory) = snapshot.directory {
                if directory.was_readonly {
                    if let Err(error) = directory.restore_readonly() {
                        report.errors.push(format!(
                            "Could not restore read-only attribute on directory ({}): {}",
                            path.display(),
                            error
                        ));
                    }
                }
            }
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = (path, snapshot, report);
        }
    }

    fn validate_entry_owner(path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
        #[cfg(unix)]
        {
            let effective_uid = unsafe { libc::geteuid() } as u32;
            if metadata.uid() != effective_uid {
                return Err(format!(
                    "Permission denied: cleanup entry is not owned by the current user: {}",
                    path.display()
                ));
            }
        }

        #[cfg(not(unix))]
        let _ = (path, metadata);

        Ok(())
    }

    fn verify_entry_identity(path: &Path, expected: &fs::Metadata) -> Result<(), String> {
        let current = fs::symlink_metadata(path).map_err(|error| {
            format!(
                "Could not re-verify cleanup entry {}: {}",
                path.display(),
                error
            )
        })?;

        #[cfg(unix)]
        if current.dev() != expected.dev() || current.ino() != expected.ino() {
            return Err(format!("Entry changed during cleanup: {}", path.display()));
        }

        if current.file_type() != expected.file_type() {
            return Err(format!("Entry changed during cleanup: {}", path.display()));
        }
        Ok(())
    }

    fn verify_directory_identity(path: &Path, snapshot: &PermissionSnapshot) -> Result<(), String> {
        #[cfg(unix)]
        {
            let current = fs::symlink_metadata(path).map_err(|error| {
                format!(
                    "Could not re-verify cleanup directory {}: {}",
                    path.display(),
                    error
                )
            })?;
            if current.file_type().is_symlink()
                || !current.is_dir()
                || current.dev() != snapshot.device
                || current.ino() != snapshot.inode
            {
                return Err(format!(
                    "Directory changed during cleanup: {}",
                    path.display()
                ));
            }
        }

        #[cfg(windows)]
        {
            let directory = snapshot.directory.as_ref().ok_or_else(|| {
                format!(
                    "Verified directory handle is unavailable: {}",
                    path.display()
                )
            })?;
            let is_link = SymlinkGuard::is_symlink_strict(path).map_err(|e| e.to_string())?;
            if is_link {
                return Err(format!(
                    "Directory changed during cleanup: {}",
                    path.display()
                ));
            }
            let current = ToctouGuard::capture(path).ok_or_else(|| {
                format!(
                    "Could not re-verify cleanup directory {}: {}",
                    path.display(),
                    "filesystem identity unavailable"
                )
            })?;
            if !current.is_dir
                || current.device != directory.device
                || current.inode != directory.inode
                || directory.is_reparse_point
            {
                return Err(format!(
                    "Directory changed during cleanup: {}",
                    path.display()
                ));
            }
        }

        #[cfg(not(any(unix, windows)))]
        let _ = (path, snapshot);

        Ok(())
    }

    fn validate_verified_scope(path: &Path, verified_root: &Path) -> Result<(), String> {
        let normalized_path = Blacklist::normalize_path(path);
        let normalized_root = Blacklist::normalize_path(verified_root);
        if normalized_path != normalized_root && !normalized_path.starts_with(&normalized_root) {
            return Err(format!(
                "Path escaped the verified cleanup target: {}",
                path.display()
            ));
        }

        // A final symlink is removed as a link and is never traversed. For all
        // real files/directories, also compare canonical paths so a replaced
        // parent symlink cannot redirect cleanup outside the planned root.
        // Canonicalization failure fails closed on mutation paths.
        let is_link = SymlinkGuard::is_symlink_strict(path).map_err(|error| error.to_string())?;
        if is_link {
            return Ok(());
        }
        let canonical_root = fs::canonicalize(verified_root).map_err(|error| {
            format!(
                "Could not verify cleanup root {}: {}",
                verified_root.display(),
                error
            )
        })?;
        let canonical_path = fs::canonicalize(path).map_err(|error| {
            format!(
                "Could not verify cleanup path {}: {}",
                path.display(),
                error
            )
        })?;
        if canonical_path != canonical_root && !canonical_path.starts_with(&canonical_root) {
            return Err(format!(
                "Path escaped the verified cleanup target: {}",
                path.display()
            ));
        }
        Ok(())
    }

    fn is_excluded(path: &Path, exclusions: &[String], environment: &PlatformEnvironment) -> bool {
        exclusions.iter().any(|exclusion| {
            let exclusion_path = Path::new(exclusion);
            if (exclusion.starts_with('~') || exclusion_path.is_absolute())
                && SignatureLoader::expand_path(exclusion, environment).is_some_and(|expanded| {
                    Self::paths_equal(path, &expanded) || Self::path_starts_with(path, &expanded)
                })
            {
                return true;
            }
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == exclusion)
        })
    }

    fn paths_equal(left: &Path, right: &Path) -> bool {
        #[cfg(windows)]
        {
            Self::windows_path_key(left) == Self::windows_path_key(right)
        }
        #[cfg(not(windows))]
        {
            left == right
        }
    }

    fn path_starts_with(path: &Path, base: &Path) -> bool {
        #[cfg(windows)]
        {
            let path_key = Self::windows_path_key(path);
            let base_key = Self::windows_path_key(base);
            path_key
                .strip_prefix(&base_key)
                .is_some_and(|suffix| suffix.starts_with('/'))
        }
        #[cfg(not(windows))]
        {
            path.starts_with(base)
        }
    }

    #[cfg(windows)]
    fn windows_path_key(path: &Path) -> String {
        Blacklist::normalize_path(path)
            .to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_ascii_lowercase()
    }
}

fn allocated_bytes(metadata: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        metadata.blocks().saturating_mul(512)
    }
    #[cfg(not(unix))]
    {
        metadata.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::path_algebra::PathFlavor;
    use crate::platform::PlatformEnvironment;

    /// Deletion tests exercise argument threading, not path resolution: no
    /// exclusion in these tests needs the environment to expand.
    fn environment() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::current())
    }

    #[cfg(windows)]
    #[test]
    fn windows_verified_handle_blocks_replacement_until_disposition() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("payload.bin");
        let renamed = dir.path().join("renamed.bin");
        std::fs::write(&target, b"payload").unwrap();

        let handle = WindowsDeleteHandle::open(&target).unwrap();
        assert!(
            std::fs::rename(&target, &renamed).is_err(),
            "a retained non-share-delete handle must prevent path replacement"
        );
        handle.delete().unwrap();
        assert!(!target.exists());
        assert!(!renamed.exists());
    }

    #[cfg(unix)]
    #[test]
    fn parent_replacement_between_verification_and_unlink_leaves_outside_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let parent = root.join("parent");
        std::fs::create_dir_all(&parent).unwrap();
        let child = parent.join("child.bin");
        std::fs::write(&child, b"inside").unwrap();

        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let outside_target = outside.join("child.bin");
        std::fs::write(&outside_target, b"outside").unwrap();

        // Open the verified parent descriptor before replacement.
        let parent_file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&parent)
            .unwrap();

        // Simulate a parent-component race: replace `parent` with a symlink to
        // the outside directory after validation.
        std::fs::remove_file(&child).unwrap();
        std::fs::remove_dir(&parent).unwrap();
        std::os::unix::fs::symlink(&outside, &parent).unwrap();

        // The verified descriptor still points at the original directory, so
        // unlinking a stale basename through it must fail without touching
        // the outside target.
        let result = SafeTreeDeleter::unlink_via_parent(
            &parent_file,
            std::ffi::OsStr::new("child.bin"),
            false,
        );
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(&outside_target).unwrap(),
            b"outside",
            "outside target must remain untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_relative_file_removal_deletes_inside_target() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let payload = root.join("payload.bin");
        std::fs::write(&payload, b"payload").unwrap();

        let report = SafeTreeDeleter::delete_contents(&root, &[], &environment());
        assert!(report.is_success(), "errors: {:?}", report.errors);
        assert!(!payload.exists());
        assert!(root.is_dir());
    }

    #[test]
    fn symlink_metadata_failure_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist-link");
        assert!(SymlinkGuard::is_symlink_strict(&missing).is_err());
        let environment = crate::platform::PlatformEnvironment::native();
        assert!(SymlinkGuard::validate_canonical_blacklist_strict(&missing, &environment).is_err());
    }

    #[test]
    fn deletes_readonly_files_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ro_root");
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let file1 = root.join("file1.txt");
        let file2 = sub.join("file2.txt");
        std::fs::write(&file1, b"file 1").unwrap();
        std::fs::write(&file2, b"file 2").unwrap();

        let mut perms = std::fs::metadata(&file1).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&file1, perms).unwrap();

        let report = SafeTreeDeleter::delete_path(&root, &[], &environment());
        assert!(report.is_success(), "errors: {:?}", report.errors);
        assert!(!root.exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_deletes_readonly_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("readonly.bin");
        std::fs::write(&target, b"readonly payload").unwrap();

        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&target, perms).unwrap();
        assert!(std::fs::metadata(&target).unwrap().permissions().readonly());

        let report = SafeTreeDeleter::delete_contents(dir.path(), &[], &environment());
        assert!(report.is_success(), "errors: {:?}", report.errors);
        assert!(!target.exists(), "readonly file must be deleted");
    }

    #[cfg(windows)]
    #[test]
    fn windows_deletes_readonly_directory_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("readonly_root");
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let file1 = root.join("file1.txt");
        let file2 = sub.join("file2.txt");
        std::fs::write(&file1, b"file 1").unwrap();
        std::fs::write(&file2, b"file 2").unwrap();

        for f in [&file1, &file2] {
            let mut p = std::fs::metadata(f).unwrap().permissions();
            p.set_readonly(true);
            std::fs::set_permissions(f, p).unwrap();
            assert!(std::fs::metadata(f).unwrap().permissions().readonly());
        }

        let mut sub_p = std::fs::metadata(&sub).unwrap().permissions();
        sub_p.set_readonly(true);
        std::fs::set_permissions(&sub, sub_p).unwrap();

        let mut root_p = std::fs::metadata(&root).unwrap().permissions();
        root_p.set_readonly(true);
        std::fs::set_permissions(&root, root_p).unwrap();

        let report = SafeTreeDeleter::delete_path(&root, &[], &environment());
        assert!(report.is_success(), "errors: {:?}", report.errors);
        assert!(!root.exists(), "readonly directory tree must be deleted");
    }

    #[cfg(windows)]
    #[test]
    fn windows_sharing_violation_retry_and_reporting() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("locked.bin");
        std::fs::write(&target, b"locked payload").unwrap();

        // Hold an exclusive lock with share_mode(0)
        let lock_handle = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&target)
            .unwrap();

        let report = SafeTreeDeleter::delete_contents(dir.path(), &[], &environment());
        assert!(
            !report.is_success(),
            "deletion must fail while file is exclusively locked"
        );
        assert_eq!(report.errors.len(), 1);
        let error_msg = &report.errors[0];
        assert!(
            error_msg.contains("Sharing violation")
                || error_msg.contains("used by another process")
                || error_msg.contains("os error 32"),
            "Error must report Win32 sharing violation distinctly, got: {}",
            error_msg
        );

        drop(lock_handle);

        let report2 = SafeTreeDeleter::delete_contents(dir.path(), &[], &environment());
        assert!(report2.is_success(), "errors: {:?}", report2.errors);
        assert!(!target.exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_sharing_violation_retries_transient_lock() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("transient.bin");
        std::fs::write(&target, b"transient payload").unwrap();

        let lock_handle = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&target)
            .unwrap();

        let handle = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            drop(lock_handle);
        });

        let report = SafeTreeDeleter::delete_contents(dir.path(), &[], &environment());
        handle.join().unwrap();

        assert!(
            report.is_success(),
            "transient lock must succeed after retry, got errors: {:?}",
            report.errors
        );
        assert!(!target.exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_readonly_rollback_on_deletion_failure() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("readonly_fail.bin");
        std::fs::write(&target, b"content").unwrap();
        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&target, perms).unwrap();
        assert!(std::fs::metadata(&target).unwrap().permissions().readonly());

        let mut handle = WindowsDeleteHandle::open(&target).unwrap();
        assert!(handle.was_readonly);
        handle.clear_readonly().unwrap();
        assert!(!std::fs::metadata(&target).unwrap().permissions().readonly());

        // Restore readonly on failure
        handle.restore_readonly().unwrap();
        assert!(std::fs::metadata(&target).unwrap().permissions().readonly());
    }
}
