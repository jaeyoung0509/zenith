#[cfg(windows)]
use crate::safety::ToctouGuard;
use crate::safety::{Blacklist, SymlinkGuard};
#[cfg(unix)]
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use zenith_platform::PlatformEnvironment;

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeDeleteReport {
    pub reclaimed_bytes: u64,
    pub deleted_files: usize,
    pub skipped_files: usize,
    pub errors: Vec<String>,
    /// Policy skips are not mutation failures, but callers still need a stable
    /// explanation when an entire target was deliberately left untouched.
    pub skip_reasons: Vec<String>,
    /// Raw OS error codes recorded alongside `errors`, in push order, so
    /// failure classification can use the code instead of localized text.
    pub os_error_codes: Vec<i32>,
    pub(crate) protect_structured_state: bool,
}

impl TreeDeleteReport {
    pub fn is_success(&self) -> bool {
        self.errors.is_empty()
    }
}

pub struct SafeTreeDeleter;

/// The lexical and canonical boundary captured once before a cleanup walk.
///
/// The canonical root is a value, not a path that gets resolved again while
/// entries are being removed. If an ancestor is renamed or replaced during
/// cleanup, later checks therefore continue to compare against the object the
/// execution guard approved instead of following the replacement.
#[derive(Debug, Clone)]
struct VerifiedCleanupScope {
    lexical_root: PathBuf,
    canonical_root: Option<PathBuf>,
}

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

/// Metadata read with `fstatat(..., AT_SYMLINK_NOFOLLOW)` from a verified
/// parent directory descriptor. Keeping only the facts deletion policy needs
/// avoids resolving the display pathname to a different filesystem object.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UnixEntrySnapshot {
    device: u64,
    inode: u64,
    mode: u32,
    uid: libc::uid_t,
    blocks: u64,
    modified_seconds: libc::time_t,
    modified_nanoseconds: u32,
}

#[cfg(unix)]
impl UnixEntrySnapshot {
    fn from_stat(stat: &libc::stat) -> Self {
        Self {
            device: u64::try_from(stat.st_dev).unwrap_or(u64::MAX),
            inode: stat.st_ino,
            mode: u32::from(stat.st_mode),
            uid: stat.st_uid,
            blocks: u64::try_from(stat.st_blocks.max(0)).unwrap_or(u64::MAX),
            modified_seconds: stat.st_mtime,
            modified_nanoseconds: u32::try_from(stat.st_mtime_nsec.max(0)).unwrap_or(999_999_999),
        }
    }

    fn file_type_bits(self) -> u32 {
        self.mode & libc::S_IFMT as u32
    }

    fn is_directory(self) -> bool {
        self.file_type_bits() == libc::S_IFDIR as u32
    }

    fn is_file(self) -> bool {
        self.file_type_bits() == libc::S_IFREG as u32
    }

    fn is_symlink(self) -> bool {
        self.file_type_bits() == libc::S_IFLNK as u32
    }

    fn entry_kind(self) -> crate::models::EntryKind {
        if self.is_directory() {
            crate::models::EntryKind::Directory
        } else if self.is_file() {
            crate::models::EntryKind::File
        } else {
            crate::models::EntryKind::Other
        }
    }

    fn is_executable(self) -> bool {
        self.is_file() && self.mode & 0o111 != 0
    }

    fn allocated_bytes(self) -> u64 {
        self.blocks.saturating_mul(512)
    }

    fn modified(self) -> Option<std::time::SystemTime> {
        if self.modified_seconds >= 0 {
            std::time::SystemTime::UNIX_EPOCH.checked_add(std::time::Duration::new(
                self.modified_seconds as u64,
                self.modified_nanoseconds.min(999_999_999),
            ))
        } else {
            std::time::SystemTime::UNIX_EPOCH.checked_sub(std::time::Duration::from_secs(
                self.modified_seconds.unsigned_abs(),
            ))
        }
    }
}

/// A stable pre-mutation observation for one whole-directory Trash operation.
/// The second observation must match the first before the path-based platform
/// adapter is invoked, so changed descendants are refused instead of silently
/// inheriting an earlier plan estimate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TrashTreeObservation {
    measured_bytes: u64,
    identities: Vec<(PathBuf, crate::models::CleanupIdentity)>,
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

        let wide = zenith_platform::NativePlatformPaths::to_verbatim_wide(path);

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

        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
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
    #[cfg(test)]
    fn delete_contents_unchecked(
        root: &Path,
        exclusions: &[String],
        environment: &PlatformEnvironment,
    ) -> TreeDeleteReport {
        Self::delete_contents_with_policy(root, exclusions, environment, false, None)
    }

    fn delete_contents_with_policy(
        root: &Path,
        exclusions: &[String],
        environment: &PlatformEnvironment,
        protect_structured_state: bool,
        stale_policy: Option<super::StaleEntryPolicy>,
    ) -> TreeDeleteReport {
        let mut report = TreeDeleteReport {
            protect_structured_state,
            ..Default::default()
        };
        let root_metadata = match fs::symlink_metadata(root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return report,
            Err(error) => {
                // Only an explicit NotFound is an already-satisfied
                // postcondition. `Path::exists` cannot be used here because it
                // collapses permission and other metadata errors into false.
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
            let verified_scope = match Self::capture_verified_scope(root) {
                Ok(scope) => scope,
                Err(error) => {
                    report.errors.push(error);
                    return report;
                }
            };
            Self::delete_entry(
                root,
                &verified_scope,
                exclusions,
                environment,
                stale_policy,
                &mut report,
            );
            return report;
        }

        if let Err(e) = Blacklist::validate_with(root, environment) {
            report.errors.push(e.to_string());
            return report;
        }
        if let Err(e) = SymlinkGuard::validate_canonical_blacklist_strict(root, environment) {
            // A root that vanished after the tolerant metadata read above is
            // already in the desired state, so the empty report is the honest
            // answer rather than a mutation failure.
            if crate::safety::is_already_absent(&e) {
                return report;
            }
            report.errors.push(e.to_string());
            return report;
        }
        let verified_scope = match Self::capture_verified_scope(root) {
            Ok(scope) => scope,
            Err(error) => {
                report.errors.push(error);
                return report;
            }
        };

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
                    &verified_scope,
                    exclusions,
                    environment,
                    stale_policy,
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
                Ok(ent) => Self::delete_entry(
                    &ent.path(),
                    &verified_scope,
                    exclusions,
                    environment,
                    stale_policy,
                    &mut report,
                ),
                Err(e) => report.errors.push(e.to_string()),
            }
        }
        Self::restore_directory_permissions(root, permissions, &mut report);
        report
    }

    #[cfg(test)]
    fn delete_path_unchecked(
        root: &Path,
        exclusions: &[String],
        environment: &PlatformEnvironment,
    ) -> TreeDeleteReport {
        Self::delete_path_with_policy(root, exclusions, environment, false, None)
    }

    fn delete_path_with_policy(
        root: &Path,
        exclusions: &[String],
        environment: &PlatformEnvironment,
        protect_structured_state: bool,
        stale_policy: Option<super::StaleEntryPolicy>,
    ) -> TreeDeleteReport {
        let mut report = TreeDeleteReport {
            protect_structured_state,
            ..Default::default()
        };
        match fs::symlink_metadata(root) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return report,
            Err(error) => {
                // Fail closed for every metadata failure except an explicit
                // NotFound; `exists()` would hide access refusals here.
                report.errors.push(format!("{}: {}", root.display(), error));
                return report;
            }
        }
        if let Err(e) = Blacklist::validate_with(root, environment) {
            report.errors.push(e.to_string());
            return report;
        }
        if let Err(e) = SymlinkGuard::validate_canonical_blacklist_strict(root, environment) {
            // A root that vanished during the strict check is already gone, so
            // the empty report is the honest answer rather than a failure.
            if crate::safety::is_already_absent(&e) {
                return report;
            }
            report.errors.push(e.to_string());
            return report;
        }
        let verified_scope = match Self::capture_verified_scope(root) {
            Ok(scope) => scope,
            Err(error) => {
                report.errors.push(error);
                return report;
            }
        };
        Self::delete_entry(
            root,
            &verified_scope,
            exclusions,
            environment,
            stale_policy,
            &mut report,
        );
        report
    }

    /// Mutates the filesystem by deleting directory contents, requiring a verified deletion authority.
    pub fn delete_contents_validated<'a>(
        authority: impl Into<super::FilesystemDeleteAuthority<'a>>,
        environment: &PlatformEnvironment,
    ) -> TreeDeleteReport {
        let auth = authority.into();
        Self::delete_contents_with_policy(
            auth.path(),
            auth.exclusions(),
            environment,
            auth.protect_structured_state(),
            auth.stale_policy(),
        )
    }

    /// Removes the entries under a path whose own age satisfies the policy.
    ///
    /// This is the same walk as [`Self::delete_contents_validated`] with the
    /// entry policy applied: every file is judged on its own timestamp and on
    /// its own classification, directories are descended into (never through a
    /// link) and removed only when they end up empty, and a file that is too
    /// recent or that holds structured state is counted as skipped rather than
    /// as an error.
    pub fn prune_stale_contents_validated<'a>(
        authority: impl Into<super::FilesystemDeleteAuthority<'a>>,
        environment: &PlatformEnvironment,
    ) -> TreeDeleteReport {
        let auth = authority.into();
        let Some(policy) = auth.stale_policy() else {
            let mut report = TreeDeleteReport::default();
            report
                .errors
                .push("A stale-entry cleanup was requested without an age policy".to_string());
            return report;
        };
        Self::delete_contents_with_policy(
            auth.path(),
            auth.exclusions(),
            environment,
            auth.protect_structured_state(),
            Some(policy),
        )
    }

    /// Mutates the filesystem by deleting a path, requiring a verified deletion authority.
    pub fn delete_path_validated<'a>(
        authority: impl Into<super::FilesystemDeleteAuthority<'a>>,
        environment: &PlatformEnvironment,
    ) -> TreeDeleteReport {
        let auth = authority.into();
        Self::delete_path_with_policy(
            auth.path(),
            auth.exclusions(),
            environment,
            auth.protect_structured_state(),
            auth.stale_policy(),
        )
    }

    /// Moves a reviewed, non-Safe filesystem target through the platform
    /// Trash adapter instead of permanently unlinking it.
    ///
    /// `DeleteDirectory` moves the authorized unit as one object. Content
    /// strategies walk without following links and move only entries they
    /// authorize, preserving exclusions, recent entries, and structured state.
    pub fn move_to_trash_validated(
        target: &super::ValidatedTarget,
        environment: &PlatformEnvironment,
        backend: &dyn zenith_platform::TrashBackend,
    ) -> TreeDeleteReport {
        let mut report = TreeDeleteReport {
            protect_structured_state: true,
            ..Default::default()
        };
        let scope = match Self::capture_verified_scope(target.path()) {
            Ok(scope) => scope,
            Err(error) => {
                report.errors.push(error);
                return report;
            }
        };

        match target.strategy() {
            crate::models::CleanStrategy::DeleteDirectory => {
                let metadata = match Self::validate_trash_entry(
                    target.path(),
                    &scope,
                    target.exclusions(),
                    environment,
                    None,
                ) {
                    Ok(Some(metadata)) => metadata,
                    Ok(None) => {
                        report.skipped_files += 1;
                        report.skip_reasons.push(format!(
                            "{} was skipped by cleanup policy; the directory was not moved to Trash",
                            target.path().display()
                        ));
                        return report;
                    }
                    Err(error) => {
                        report.errors.push(error);
                        return report;
                    }
                };
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    report.errors.push(format!(
                        "{} changed during cleanup; whole-directory Trash requires a real directory",
                        target.path().display()
                    ));
                    return report;
                }

                let first = match Self::observe_trash_tree(
                    target.path(),
                    &scope,
                    target.exclusions(),
                    environment,
                ) {
                    Ok(observation) => observation,
                    Err(error) => {
                        report.errors.push(error);
                        return report;
                    }
                };
                let second = match Self::observe_trash_tree(
                    target.path(),
                    &scope,
                    target.exclusions(),
                    environment,
                ) {
                    Ok(observation) => observation,
                    Err(error) => {
                        report.errors.push(error);
                        return report;
                    }
                };
                if first != second {
                    report.errors.push(format!(
                        "{} changed during cleanup; refusing whole-directory Trash movement",
                        target.path().display()
                    ));
                    return report;
                }

                // TrashBackend is intentionally path-based on both supported
                // platforms. Two identical no-follow observations, followed by
                // a final root identity check, detect replacements before the
                // call. POSIX does not offer a conditional whole-tree Trash
                // primitive, so a same-user leaf replacement after this final
                // check remains outside the guarantee and must never be
                // described as an atomic descriptor-relative deletion.
                if let Err(error) = Self::verify_entry_identity(target.path(), &metadata) {
                    report.errors.push(error);
                    return report;
                }
                match backend.move_to_trash(target.path()) {
                    Ok(()) => {
                        report.reclaimed_bytes = second.measured_bytes;
                        report.deleted_files = 1;
                    }
                    Err(error) => report.errors.push(error),
                }
            }
            crate::models::CleanStrategy::DeleteContents
            | crate::models::CleanStrategy::DeleteStaleContents => {
                Self::move_directory_contents_to_trash(
                    target.path(),
                    &scope,
                    target.exclusions(),
                    environment,
                    target.stale_policy(),
                    backend,
                    &mut report,
                );
            }
            _ => report.errors.push(
                "Target strategy does not authorize a recoverable filesystem mutation".to_string(),
            ),
        }
        report
    }

    /// Observes every entry a whole-directory Trash move would carry. A policy
    /// skip below the root refuses the whole unit: the path-based Trash API
    /// cannot preserve one excluded child while moving its parent.
    fn observe_trash_tree(
        root: &Path,
        scope: &VerifiedCleanupScope,
        exclusions: &[String],
        environment: &PlatformEnvironment,
    ) -> Result<TrashTreeObservation, String> {
        fn visit(
            path: &Path,
            root: &Path,
            scope: &VerifiedCleanupScope,
            exclusions: &[String],
            environment: &PlatformEnvironment,
            observation: &mut TrashTreeObservation,
        ) -> Result<(), String> {
            if SafeTreeDeleter::is_excluded(path, exclusions, environment) {
                return Err(format!(
                    "{} is excluded; refusing to move the whole directory {} to Trash",
                    path.display(),
                    root.display()
                ));
            }
            if Blacklist::is_blacklisted_with(path, environment) {
                return Err(format!(
                    "{} is a protected location; refusing to move the whole directory {} to Trash",
                    path.display(),
                    root.display()
                ));
            }

            let metadata = match SafeTreeDeleter::validate_trash_entry(
                path,
                scope,
                exclusions,
                environment,
                None,
            )? {
                Some(metadata) => metadata,
                None => {
                    return Err(format!(
                        "{} changed or was skipped during cleanup; refusing to move the whole directory {} to Trash",
                        path.display(),
                        root.display()
                    ))
                }
            };
            let identity = crate::safety::ToctouGuard::capture(path).ok_or_else(|| {
                format!(
                    "Could not verify filesystem identity for {}; refusing whole-directory Trash movement",
                    path.display()
                )
            })?;
            observation.identities.push((path.to_path_buf(), identity));

            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                let mut children = fs::read_dir(path)
                    .map_err(|error| format!("{}: {}", path.display(), error))?
                    .map(|entry| {
                        entry
                            .map(|entry| entry.path())
                            .map_err(|error| error.to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                children.sort();
                for child in children {
                    visit(&child, root, scope, exclusions, environment, observation)?;
                }
            } else {
                observation.measured_bytes = observation
                    .measured_bytes
                    .saturating_add(allocated_bytes(&metadata));
            }
            Ok(())
        }

        let mut observation = TrashTreeObservation::default();
        visit(root, root, scope, exclusions, environment, &mut observation)?;
        Ok(observation)
    }

    #[allow(clippy::too_many_arguments)]
    fn move_directory_contents_to_trash(
        directory: &Path,
        scope: &VerifiedCleanupScope,
        exclusions: &[String],
        environment: &PlatformEnvironment,
        stale_policy: Option<super::StaleEntryPolicy>,
        backend: &dyn zenith_platform::TrashBackend,
        report: &mut TreeDeleteReport,
    ) {
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries.collect::<Vec<_>>(),
            Err(error) => {
                report
                    .errors
                    .push(format!("{}: {}", directory.display(), error));
                return;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    report.errors.push(error.to_string());
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match Self::validate_trash_entry(
                &path,
                scope,
                exclusions,
                environment,
                stale_policy,
            ) {
                Ok(Some(metadata)) => metadata,
                Ok(None) => {
                    report.skipped_files += 1;
                    continue;
                }
                Err(error) => {
                    report.errors.push(error);
                    continue;
                }
            };

            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                Self::move_directory_contents_to_trash(
                    &path,
                    scope,
                    exclusions,
                    environment,
                    stale_policy,
                    backend,
                    report,
                );
                match fs::read_dir(&path) {
                    Ok(mut remaining) => {
                        if remaining.next().is_none() {
                            if let Err(error) = Self::validate_verified_scope(&path, scope) {
                                report.errors.push(error);
                                continue;
                            }
                            match backend.move_to_trash(&path) {
                                Ok(()) => report.deleted_files += 1,
                                Err(error) => report.errors.push(error),
                            }
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        report.skipped_files += 1;
                    }
                    Err(error) => report.errors.push(format!("{}: {}", path.display(), error)),
                }
                continue;
            }

            let bytes = allocated_bytes(&metadata);
            match backend.move_to_trash(&path) {
                Ok(()) => {
                    report.reclaimed_bytes = report.reclaimed_bytes.saturating_add(bytes);
                    report.deleted_files += 1;
                }
                Err(error) => report.errors.push(error),
            }
        }
    }

    /// Returns metadata for an entry that may move, `None` for a deliberate
    /// policy skip, and an error for an incomplete safety check.
    fn validate_trash_entry(
        path: &Path,
        scope: &VerifiedCleanupScope,
        exclusions: &[String],
        environment: &PlatformEnvironment,
        stale_policy: Option<super::StaleEntryPolicy>,
    ) -> Result<Option<fs::Metadata>, String> {
        if Self::is_excluded(path, exclusions, environment)
            || Blacklist::is_blacklisted_with(path, environment)
        {
            return Ok(None);
        }
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("{}: {}", path.display(), error)),
        };
        if let Some((kind, _)) = super::structured_state_at(path) {
            return Err(format!(
                "{} is {}; refusing structured state",
                path.display(),
                kind.display_name()
            ));
        }
        Self::validate_verified_scope(path, scope)?;
        SymlinkGuard::validate_canonical_blacklist_strict(path, environment)
            .map_err(|error| format!("{}: {}", path.display(), error))?;
        Self::validate_entry_owner(path, &metadata)?;
        Self::verify_entry_identity(path, &metadata)?;

        if !metadata.is_dir() {
            if let Some(policy) = stale_policy {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if !policy.allows(
                    &name,
                    super::stale::entry_kind(&metadata),
                    super::stale::is_executable(&metadata),
                    metadata.modified().ok(),
                    std::time::SystemTime::now(),
                ) {
                    return Ok(None);
                }
            }
        }
        Ok(Some(metadata))
    }

    #[cfg(unix)]
    fn child_name_cstring(name: &OsStr) -> io::Result<CString> {
        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes.contains(&b'/') || bytes == b"." || bytes == b".." {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing unsafe child name",
            ));
        }
        CString::new(bytes).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing child name containing NUL",
            )
        })
    }

    #[cfg(unix)]
    fn clear_readdir_errno() {
        #[cfg(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        unsafe {
            *libc::__error() = 0;
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        unsafe {
            *libc::__errno_location() = 0;
        }
    }

    /// Enumerates basenames through a duplicate of the verified directory
    /// descriptor. `fdopendir` owns the duplicate, while the caller retains the
    /// original descriptor for metadata, child opens, and mutation.
    #[cfg(unix)]
    fn read_dir_names(dir_file: &fs::File) -> io::Result<Vec<OsString>> {
        let duplicate = unsafe { libc::dup(dir_file.as_raw_fd()) };
        if duplicate < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::fcntl(duplicate, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            let error = io::Error::last_os_error();
            unsafe {
                libc::close(duplicate);
            }
            return Err(error);
        }
        let stream = unsafe { libc::fdopendir(duplicate) };
        if stream.is_null() {
            let error = io::Error::last_os_error();
            unsafe {
                libc::close(duplicate);
            }
            return Err(error);
        }

        let mut names = Vec::new();
        let read_result = loop {
            Self::clear_readdir_errno();
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                break if error.raw_os_error().unwrap_or(0) == 0 {
                    Ok(())
                } else {
                    Err(error)
                };
            }
            let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            names.push(OsString::from_vec(bytes.to_vec()));
        };
        let close_result = unsafe { libc::closedir(stream) };
        read_result?;
        if close_result != 0 {
            return Err(io::Error::last_os_error());
        }
        names.sort();
        Ok(names)
    }

    /// Reads a child's no-follow metadata relative to the verified parent.
    #[cfg(unix)]
    fn metadata_at(parent: &fs::File, name: &OsStr) -> io::Result<UnixEntrySnapshot> {
        let name = Self::child_name_cstring(name)?;
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result == 0 {
            Ok(UnixEntrySnapshot::from_stat(&stat))
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Reads a symlink target from the verified parent descriptor without
    /// resolving the display pathname to a possibly replaced directory.
    #[cfg(unix)]
    fn read_link_at(parent: &fs::File, name: &OsStr) -> io::Result<PathBuf> {
        let name = Self::child_name_cstring(name)?;
        let mut capacity = 256usize;
        loop {
            let mut buffer = vec![0u8; capacity];
            let length = unsafe {
                libc::readlinkat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    buffer.as_mut_ptr() as *mut libc::c_char,
                    buffer.len(),
                )
            };
            if length < 0 {
                return Err(io::Error::last_os_error());
            }
            let length = length as usize;
            if length < buffer.len() {
                buffer.truncate(length);
                return Ok(PathBuf::from(OsString::from_vec(buffer)));
            }
            capacity = capacity.checked_mul(2).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "symlink target is too large")
            })?;
            if capacity > 64 * 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "symlink target exceeds the safety bound",
                ));
            }
        }
    }

    /// Applies the existing symlink-target blacklist to the target read from
    /// the verified descriptor. `Ok(false)` means the target vanished and the
    /// link is conservatively retained as a neutral skip.
    #[cfg(unix)]
    fn validate_symlink_target_at(
        parent: &fs::File,
        name: &OsStr,
        display_path: &Path,
        environment: &PlatformEnvironment,
    ) -> Result<bool, String> {
        let target = Self::read_link_at(parent, name)
            .map_err(|error| format!("{}: {}", display_path.display(), error))?;
        let resolved = if target.is_absolute() {
            target
        } else {
            display_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(target)
        };
        Blacklist::validate_with(&resolved, environment).map_err(|error| error.to_string())?;
        let canonical = match fs::canonicalize(&resolved) {
            Ok(canonical) => canonical,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(format!(
                    "Could not verify symlink target for {}: {}",
                    display_path.display(),
                    error
                ))
            }
        };
        Blacklist::validate_with(&canonical, environment).map_err(|error| error.to_string())?;
        Ok(true)
    }

    #[cfg(unix)]
    fn verify_child_identity(
        parent: &fs::File,
        name: &OsStr,
        expected: UnixEntrySnapshot,
        display_path: &Path,
    ) -> Result<bool, String> {
        let current = match Self::metadata_at(parent, name) {
            Ok(current) => current,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(format!(
                    "Could not re-verify cleanup entry {}: {}",
                    display_path.display(),
                    error
                ))
            }
        };
        if current.device != expected.device
            || current.inode != expected.inode
            || current.file_type_bits() != expected.file_type_bits()
        {
            return Err(format!(
                "Entry changed during cleanup: {}",
                display_path.display()
            ));
        }
        Ok(true)
    }

    /// Confirms that the display pathname still names the directory held by
    /// the descriptor. This is a refusal check, not the traversal primitive:
    /// enumeration and all child opens remain descriptor-relative.
    #[cfg(unix)]
    fn verify_directory_path_binding(path: &Path, directory: &fs::File) -> Result<(), String> {
        let current = fs::symlink_metadata(path).map_err(|error| {
            format!(
                "Directory changed during cleanup: {} ({})",
                path.display(),
                error
            )
        })?;
        let held = directory.metadata().map_err(|error| {
            format!(
                "Could not re-verify cleanup directory {}: {}",
                path.display(),
                error
            )
        })?;
        if current.file_type().is_symlink()
            || !current.is_dir()
            || current.dev() != held.dev()
            || current.ino() != held.ino()
        {
            return Err(format!(
                "Directory changed during cleanup: {}",
                path.display()
            ));
        }
        Ok(())
    }

    /// Opens a child directory relative to the verified parent and compares the
    /// resulting descriptor with the no-follow metadata observed for that same
    /// basename before applying temporary owner permissions.
    #[cfg(unix)]
    fn prepare_directory_at(
        parent: &fs::File,
        name: &OsStr,
        expected: UnixEntrySnapshot,
        display_path: &Path,
    ) -> Result<PermissionSnapshot, String> {
        let name = Self::child_name_cstring(name).map_err(|error| error.to_string())?;
        let descriptor = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return Err(format!(
                "Permission denied: could not safely open cleanup directory ({}): {}",
                display_path.display(),
                io::Error::last_os_error()
            ));
        }
        let directory = unsafe { fs::File::from_raw_fd(descriptor) };
        let metadata = directory.metadata().map_err(|error| {
            format!(
                "Could not verify cleanup directory {}: {}",
                display_path.display(),
                error
            )
        })?;
        if metadata.dev() != expected.device
            || metadata.ino() != expected.inode
            || !metadata.is_dir()
        {
            return Err(format!(
                "Directory changed during cleanup: {}",
                display_path.display()
            ));
        }

        let effective_uid = unsafe { libc::geteuid() };
        if expected.uid != effective_uid {
            return Err(format!(
                "Permission denied: cleanup directory is not owned by the current user: {}",
                display_path.display()
            ));
        }
        let original_mode = expected.mode & 0o7777;
        let required_mode = original_mode | 0o700;
        if required_mode != original_mode {
            directory
                .set_permissions(fs::Permissions::from_mode(required_mode))
                .map_err(|error| {
                    format!(
                        "Permission denied: could not make cleanup directory writable ({}): {}",
                        display_path.display(),
                        error
                    )
                })?;
        }
        Ok(PermissionSnapshot {
            original_mode: (required_mode != original_mode).then_some(original_mode),
            directory: Some(directory),
            device: expected.device,
            inode: expected.inode,
        })
    }

    /// Deletes directory children through one verified descriptor chain.
    /// Enumeration (`readdir`), metadata (`fstatat`), recursive child opens
    /// (`openat`), and mutation (`unlinkat`) all use the same parent handle.
    #[cfg(unix)]
    fn delete_dir_contents_via_fd(
        dir_path: &Path,
        dir_file: &fs::File,
        verified_scope: &VerifiedCleanupScope,
        exclusions: &[String],
        environment: &PlatformEnvironment,
        stale_policy: Option<super::StaleEntryPolicy>,
        report: &mut TreeDeleteReport,
    ) {
        if let Err(error) = Self::verify_directory_path_binding(dir_path, dir_file) {
            report.errors.push(error);
            return;
        }
        let parent_metadata = match dir_file.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                report.errors.push(format!(
                    "Could not read cleanup directory {}: {}",
                    dir_path.display(),
                    error
                ));
                return;
            }
        };
        let entries = match Self::read_dir_names(dir_file) {
            Ok(entries) => entries,
            Err(error) => {
                report
                    .errors
                    .push(format!("{}: {}", dir_path.display(), error));
                return;
            }
        };

        for file_name in entries {
            if let Err(error) = Self::verify_directory_path_binding(dir_path, dir_file) {
                report.errors.push(error);
                return;
            }
            let child_path = dir_path.join(&file_name);

            if Self::is_excluded(&child_path, exclusions, environment)
                || Blacklist::is_blacklisted_with(&child_path, environment)
            {
                report.skipped_files += 1;
                continue;
            }

            if let Err(error) = Self::validate_lexical_scope(&child_path, verified_scope) {
                report.errors.push(error);
                continue;
            }

            let metadata = match Self::metadata_at(dir_file, &file_name) {
                Ok(metadata) => metadata,
                // A vanished entry is already in the desired state: the
                // deletion postcondition holds, so it is a neutral skip rather
                // than an error. Nothing was reclaimed.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    report.skipped_files += 1;
                    continue;
                }
                Err(error) => {
                    report
                        .errors
                        .push(format!("{}: {}", child_path.display(), error));
                    continue;
                }
            };

            let name = file_name.to_string_lossy();
            if report.protect_structured_state {
                let facts = crate::models::PathFacts::new(&name, metadata.entry_kind())
                    .executable(metadata.is_executable());
                if let Some(kind) = crate::models::classify_structured_state(facts) {
                    report.errors.push(format!(
                        "{} is {}; refusing structured state",
                        child_path.display(),
                        kind.display_name()
                    ));
                    continue;
                }
            }

            let effective_uid = unsafe { libc::geteuid() };
            if metadata.uid != effective_uid {
                report.errors.push(format!(
                    "Permission denied: cleanup entry is not owned by the current user: {}",
                    child_path.display()
                ));
                continue;
            }

            if metadata.is_symlink() {
                match Self::validate_symlink_target_at(
                    dir_file,
                    &file_name,
                    &child_path,
                    environment,
                ) {
                    Ok(true) => {}
                    Ok(false) => {
                        report.skipped_files += 1;
                        continue;
                    }
                    Err(error) => {
                        report
                            .errors
                            .push(format!("{}: {}", child_path.display(), error));
                        continue;
                    }
                }
            }

            if metadata.is_symlink() || metadata.is_file() {
                // An entry is removed only when its own age satisfies the
                // policy. A directory is still descended into: its entries are
                // judged one by one, and it disappears only if it ends up empty.
                if let Some(policy) = stale_policy {
                    if !policy.allows(
                        &name,
                        metadata.entry_kind(),
                        metadata.is_executable(),
                        metadata.modified(),
                        std::time::SystemTime::now(),
                    ) {
                        report.skipped_files += 1;
                        continue;
                    }
                }
                match Self::verify_child_identity(dir_file, &file_name, metadata, &child_path) {
                    Ok(true) => {}
                    Ok(false) => {
                        report.skipped_files += 1;
                        continue;
                    }
                    Err(error) => {
                        report.errors.push(error);
                        continue;
                    }
                }
                if let Err(error) = Self::verify_directory_path_binding(dir_path, dir_file) {
                    report.errors.push(error);
                    return;
                }
                let bytes = metadata.allocated_bytes();
                match Self::unlink_via_parent(dir_file, &file_name, false) {
                    Ok(()) => {
                        report.reclaimed_bytes = report.reclaimed_bytes.saturating_add(bytes);
                        report.deleted_files += 1;
                    }
                    // The entry vanished between verification and unlink. It is
                    // already gone, so nothing was reclaimed and nothing failed.
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        report.skipped_files += 1;
                    }
                    Err(e) => {
                        report
                            .errors
                            .push(format!("{}: {}", child_path.display(), e));
                    }
                }
                continue;
            }

            if !metadata.is_directory() {
                continue;
            }
            if metadata.device != parent_metadata.dev() {
                report.errors.push(format!(
                    "{} crosses a mount boundary; refusing recursive cleanup",
                    child_path.display()
                ));
                continue;
            }

            // Recurse with the child's own verified descriptor.
            let child_permissions =
                match Self::prepare_directory_at(dir_file, &file_name, metadata, &child_path) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        report.errors.push(error);
                        continue;
                    }
                };
            match Self::verify_child_identity(dir_file, &file_name, metadata, &child_path) {
                Ok(true) => {}
                Ok(false) => {
                    report.skipped_files += 1;
                    Self::restore_directory_permissions(&child_path, child_permissions, report);
                    continue;
                }
                Err(error) => {
                    report.errors.push(error);
                    Self::restore_directory_permissions(&child_path, child_permissions, report);
                    continue;
                }
            }
            if let Some(ref child_file) = child_permissions.directory {
                Self::delete_dir_contents_via_fd(
                    &child_path,
                    child_file,
                    verified_scope,
                    exclusions,
                    environment,
                    stale_policy,
                    report,
                );
            }
            if let Err(error) = Self::verify_directory_identity(&child_path, &child_permissions) {
                report.errors.push(error);
                Self::restore_directory_permissions(&child_path, child_permissions, report);
                continue;
            }
            match Self::verify_child_identity(dir_file, &file_name, metadata, &child_path) {
                Ok(true) => {}
                Ok(false) => {
                    report.errors.push(format!(
                        "Directory changed during cleanup: {} is no longer attached to its verified parent",
                        child_path.display()
                    ));
                    Self::restore_directory_permissions(&child_path, child_permissions, report);
                    continue;
                }
                Err(error) => {
                    report.errors.push(error);
                    Self::restore_directory_permissions(&child_path, child_permissions, report);
                    continue;
                }
            }
            if let Err(error) = Self::verify_directory_path_binding(dir_path, dir_file) {
                report.errors.push(error);
                Self::restore_directory_permissions(&child_path, child_permissions, report);
                return;
            }
            // Remove the now-empty child directory relative to the verified
            // parent descriptor. The child's own permissions are restored
            // implicitly by removing it; only restore on failure.
            match Self::unlink_via_parent(dir_file, &file_name, true) {
                Ok(()) => {}
                // The child directory vanished after its contents were walked.
                // It is already gone, so this is neutral and needs no
                // permission restore.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    report.skipped_files += 1;
                }
                Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => {
                    report.errors.push(format!(
                        "{}: directory remains after cleanup; retained entries were not removed",
                        child_path.display()
                    ));
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
    /// descriptor. The caller performs a no-follow `fstatat` identity check
    /// immediately before this call. POSIX has no conditional unlink-by-inode,
    /// so this does not claim to eliminate a hostile same-user replacement in
    /// the final check-to-unlink instruction window; it does ensure pathname
    /// replacement cannot redirect traversal to another parent object.
    #[cfg(unix)]
    fn unlink_via_parent(parent: &fs::File, name: &OsStr, is_dir: bool) -> io::Result<()> {
        let name = Self::child_name_cstring(name)?;
        let flags = if is_dir { libc::AT_REMOVEDIR } else { 0 };
        // SAFETY: `name` is a NUL-terminated non-empty basename without `/`,
        // and `parent` is a verified directory fd opened with O_NOFOLLOW.
        let res = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if res == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn delete_entry(
        path: &Path,
        verified_scope: &VerifiedCleanupScope,
        exclusions: &[String],
        environment: &PlatformEnvironment,
        stale_policy: Option<super::StaleEntryPolicy>,
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
            // A vanished entry is already in the desired state: the deletion
            // postcondition holds, so it is a neutral skip instead of an error.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                report.skipped_files += 1;
                return;
            }
            Err(e) => {
                report.errors.push(format!("{}: {}", path.display(), e));
                return;
            }
        };

        if let Some((kind, _)) = report
            .protect_structured_state
            .then(|| super::structured_state_at(path))
            .flatten()
        {
            report.errors.push(format!(
                "{} is {}; refusing structured state",
                path.display(),
                kind.display_name()
            ));
            return;
        }

        if let Err(error) = Self::validate_verified_scope(path, verified_scope) {
            report.errors.push(error);
            return;
        }

        // Re-check the canonical location at every recursive entry before any
        // permission change or deletion. Symlink entries are still removed as
        // links, never traversed. Canonicalization failure fails closed.
        if let Err(error) = SymlinkGuard::validate_canonical_blacklist_strict(path, environment) {
            // The entry is gone, so there is nothing to mutate and nothing to
            // report: absence is neutral here too.
            if crate::safety::is_already_absent(&error) {
                report.skipped_files += 1;
                return;
            }
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
            // An entry is removed only when its own age satisfies the policy.
            // This path walker and the descriptor-relative one must refuse the
            // same entries, or the guard's promise would depend on which route
            // the filesystem allowed.
            if let Some(policy) = stale_policy {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if !policy.allows(
                    &name,
                    super::stale::entry_kind(&metadata),
                    super::stale::is_executable(&metadata),
                    metadata.modified().ok(),
                    std::time::SystemTime::now(),
                ) {
                    report.skipped_files += 1;
                    return;
                }
            }
            let bytes = allocated_bytes(&metadata);
            #[cfg(unix)]
            {
                // Prefer descriptor-relative unlink via the verified parent.
                // Falls back to path removal only when the parent cannot be
                // opened safely, in which case the error fails closed.
                match Self::remove_file_via_verified_parent(path, &metadata) {
                    Ok(()) => {
                        report.reclaimed_bytes += bytes;
                        report.deleted_files += 1;
                    }
                    // Either the parent or the entry itself vanished after
                    // verification. The deletion postcondition already holds,
                    // so this is a neutral skip: nothing reclaimed, no error.
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        report.skipped_files += 1;
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
                verified_scope,
                exclusions,
                environment,
                stale_policy,
                report,
            );
            if let Err(error) = Self::verify_directory_identity(path, &permissions) {
                report.errors.push(error);
                Self::restore_directory_permissions(path, permissions, report);
                return;
            }
            if let Some(ref directory) = permissions.directory {
                if let Err(error) = Self::verify_directory_path_binding(path, directory) {
                    report.errors.push(error);
                    Self::restore_directory_permissions(path, permissions, report);
                    return;
                }
            }
            // The top-level directory itself is removed relative to its own
            // verified parent so a replaced parent cannot redirect it.
            // `delete_contents` never reaches here for its root (it returns
            // after `delete_dir_contents_via_fd`); `delete_path` removes it.
            let remove_result =
                Self::remove_dir_via_verified_parent(path, permissions.device, permissions.inode);
            match remove_result {
                Ok(()) => {}
                // The directory (or its parent) vanished between the walk and
                // the removal. It is already gone, so this is neutral.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    report.skipped_files += 1;
                }
                Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => {
                    report.errors.push(format!(
                        "{}: directory remains after cleanup; retained entries were not removed",
                        path.display()
                    ));
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
            // The directory vanished after its metadata was read. It is already
            // gone, so the walk treats it as a neutral skip instead of pushing
            // a failure that would fail the whole target closed.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                report.skipped_files += 1;
                return;
            }
            Err(e) => {
                report.errors.push(format!("{}: {}", path.display(), e));
                Self::restore_directory_permissions(path, permissions, report);
                return;
            }
        };

        for entry in entries {
            match entry {
                Ok(ent) => Self::delete_entry(
                    &ent.path(),
                    verified_scope,
                    exclusions,
                    environment,
                    stale_policy,
                    report,
                ),
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
                    report.errors.push(format!(
                        "{}: directory remains after cleanup; retained entries were not removed",
                        path.display()
                    ));
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
                report.errors.push(format!(
                    "{}: directory remains after cleanup; retained entries were not removed",
                    path.display()
                ));
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
        if handle.device != expected_identity.entity().device()
            || handle.inode != expected_identity.entity().inode()
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
    fn remove_file_via_verified_parent(path: &Path, expected: &fs::Metadata) -> io::Result<()> {
        let (parent, name) = Self::open_parent_nofollow(path)?;
        let current = Self::metadata_at(&parent, &name)?;
        if current.device != expected.dev()
            || current.inode != expected.ino()
            || current.file_type_bits() != (expected.mode() & libc::S_IFMT as u32)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "entry changed between verification and deletion",
            ));
        }
        Self::unlink_via_parent(&parent, &name, false)
    }

    /// Removes an empty directory relative to its verified parent descriptor.
    #[cfg(unix)]
    fn remove_dir_via_verified_parent(
        path: &Path,
        expected_device: u64,
        expected_inode: u64,
    ) -> io::Result<()> {
        let (parent, name) = Self::open_parent_nofollow(path)?;
        let current = Self::metadata_at(&parent, &name)?;
        if !current.is_directory()
            || current.device != expected_device
            || current.inode != expected_inode
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory changed between verification and deletion",
            ));
        }
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
                if directory.device != expected_identity.entity().device()
                    || directory.inode != expected_identity.entity().inode()
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
            // Restore through the retained descriptor. Resolving `path` here
            // could chmod a replacement, while the handle still identifies
            // exactly the directory whose mode this snapshot changed.
            let Some(directory) = snapshot.directory.as_ref() else {
                return;
            };
            let Ok(metadata) = directory.metadata() else {
                return;
            };
            if !metadata.is_dir()
                || metadata.dev() != snapshot.device
                || metadata.ino() != snapshot.inode
            {
                return;
            }
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
            let directory = snapshot.directory.as_ref().ok_or_else(|| {
                format!(
                    "Verified directory handle is unavailable: {}",
                    path.display()
                )
            })?;
            // `File::metadata` is an fstat of the descriptor opened with
            // O_DIRECTORY | O_NOFOLLOW. It verifies the object we hold rather
            // than resolving `path` again after an ancestor may have moved.
            let current = directory.metadata().map_err(|error| {
                format!(
                    "Could not re-verify cleanup directory {}: {}",
                    path.display(),
                    error
                )
            })?;
            if !current.is_dir()
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
            if !current.is_dir()
                || current.entity().device() != directory.device
                || current.entity().inode() != directory.inode
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

    fn capture_verified_scope(root: &Path) -> Result<VerifiedCleanupScope, String> {
        let lexical_root = zenith_platform::path_algebra::normalize_lexical(root);
        let is_link = SymlinkGuard::is_symlink_strict(root).map_err(|error| error.to_string())?;
        let canonical_root = if is_link {
            // A final link is removed as a link and never traversed. Following
            // it while capturing the boundary would grant authority over its
            // target, which is exactly what the link rule forbids.
            None
        } else {
            Some(fs::canonicalize(root).map_err(|error| {
                format!(
                    "Could not verify cleanup root {}: {}",
                    root.display(),
                    error
                )
            })?)
        };
        Ok(VerifiedCleanupScope {
            lexical_root,
            canonical_root,
        })
    }

    fn validate_verified_scope(
        path: &Path,
        verified_scope: &VerifiedCleanupScope,
    ) -> Result<(), String> {
        Self::validate_lexical_scope(path, verified_scope)?;

        // A final symlink is removed as a link and is never traversed. For all
        // real files/directories, also compare canonical paths so a replaced
        // parent symlink cannot redirect cleanup outside the planned root.
        // Canonicalization failure fails closed on mutation paths.
        let is_link = SymlinkGuard::is_symlink_strict(path).map_err(|error| error.to_string())?;
        if is_link {
            return Ok(());
        }
        let canonical_root = verified_scope.canonical_root.as_ref().ok_or_else(|| {
            format!(
                "Cleanup root changed from a final link to a traversable path: {}",
                path.display()
            )
        })?;
        let canonical_path = fs::canonicalize(path).map_err(|error| {
            format!(
                "Could not verify cleanup path {}: {}",
                path.display(),
                error
            )
        })?;
        if canonical_path.as_path() != canonical_root.as_path()
            && !canonical_path.starts_with(canonical_root)
        {
            return Err(format!(
                "Path escaped the verified cleanup target: {}",
                path.display()
            ));
        }
        Ok(())
    }

    /// Verifies the display spelling remains inside the authorized unit. Unix
    /// descriptor-relative traversal uses this lexical half and relies on the
    /// already-verified descriptor chain, rather than canonicalizing a pathname
    /// that may now name a different object.
    fn validate_lexical_scope(
        path: &Path,
        verified_scope: &VerifiedCleanupScope,
    ) -> Result<(), String> {
        let normalized_path = zenith_platform::path_algebra::normalize_lexical(path);
        if normalized_path != verified_scope.lexical_root
            && !normalized_path.starts_with(&verified_scope.lexical_root)
        {
            return Err(format!(
                "Path escaped the verified cleanup target: {}",
                path.display()
            ));
        }
        Ok(())
    }

    fn is_excluded(path: &Path, exclusions: &[String], environment: &PlatformEnvironment) -> bool {
        crate::signatures::exclusions::is_excluded(path, exclusions, environment)
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
    use crate::models::ZenithError;
    use zenith_platform::path_algebra::PathFlavor;
    use zenith_platform::PlatformEnvironment;

    /// Deletion tests exercise argument threading, not path resolution: no
    /// exclusion in these tests needs the environment to expand.
    fn environment() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::current()).with_home(
            if PathFlavor::current().is_windows() {
                r"Z:\ZenithFixtureHome"
            } else {
                "/zenith-fixture-home"
            },
        )
    }

    #[test]
    fn exclusion_matching_uses_the_stated_path_flavor() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Windows).with_home(
            if PathFlavor::Windows.is_windows() {
                r"Z:\ZenithFixtureHome"
            } else {
                "/zenith-fixture-home"
            },
        );
        let exclusions = vec![r"C:\Users\Alice\Cache".to_string()];

        assert!(SafeTreeDeleter::is_excluded(
            Path::new(r"c:/users/alice/cache/nested/file.bin"),
            &exclusions,
            &environment
        ));
        assert!(!SafeTreeDeleter::is_excluded(
            Path::new(r"C:\Users\Alice\Cache-Other\file.bin"),
            &exclusions,
            &environment
        ));
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
    fn pinned_scope_does_not_follow_a_replaced_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let moved_root = dir.path().join("moved-root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("inside.bin"), b"inside").unwrap();
        let outside_target = outside.join("outside.bin");
        std::fs::write(&outside_target, b"outside").unwrap();

        let scope = SafeTreeDeleter::capture_verified_scope(&root).unwrap();
        std::fs::rename(&root, &moved_root).unwrap();
        std::os::unix::fs::symlink(&outside, &root).unwrap();

        let redirected = root.join("outside.bin");
        assert!(SafeTreeDeleter::validate_verified_scope(&redirected, &scope).is_err());
        let mut report = TreeDeleteReport::default();
        SafeTreeDeleter::delete_entry(&redirected, &scope, &[], &environment(), None, &mut report);

        assert_eq!(std::fs::read(&outside_target).unwrap(), b"outside");
        assert!(moved_root.join("inside.bin").exists());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("escaped the verified cleanup target")));
    }

    #[cfg(unix)]
    #[test]
    fn directory_identity_is_verified_through_the_open_handle() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let moved_root = dir.path().join("moved-root");
        std::fs::create_dir_all(&root).unwrap();
        let metadata = std::fs::symlink_metadata(&root).unwrap();
        let snapshot = SafeTreeDeleter::prepare_directory(&root, &metadata).unwrap();

        std::fs::rename(&root, &moved_root).unwrap();
        std::fs::create_dir(&root).unwrap();

        assert!(SafeTreeDeleter::verify_directory_identity(&root, &snapshot).is_ok());
        drop(snapshot);
    }

    /// The walk must not inspect a replacement at the approved spelling and
    /// then unlink the same basename from the directory handle captured before
    /// the replacement. The explicit sequence between `prepare_directory` and
    /// `delete_dir_contents_via_fd` is the deterministic barrier for the race.
    #[cfg(unix)]
    #[test]
    fn root_replacement_between_handle_acquisition_and_enumeration_is_refused() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("cache");
        let relocated = fixture.path().join("relocated");
        std::fs::create_dir(&root).unwrap();
        let original_payload = root.join("payload.bin");
        std::fs::write(&original_payload, b"original protected payload").unwrap();

        let scope = SafeTreeDeleter::capture_verified_scope(&root).unwrap();
        let metadata = std::fs::symlink_metadata(&root).unwrap();
        let snapshot = SafeTreeDeleter::prepare_directory(&root, &metadata).unwrap();

        // Controlled barrier: the retained descriptor names `relocated`, while
        // the approved spelling now names a different ordinary directory.
        std::fs::rename(&root, &relocated).unwrap();
        std::fs::create_dir(&root).unwrap();
        let replacement_payload = root.join("payload.bin");
        std::fs::write(&replacement_payload, b"replacement payload").unwrap();

        let relocated_payload = relocated.join("payload.bin");
        let mut executable = std::fs::metadata(&relocated_payload).unwrap().permissions();
        executable.set_mode(executable.mode() | 0o100);
        std::fs::set_permissions(&relocated_payload, executable).unwrap();

        let mut report = TreeDeleteReport {
            protect_structured_state: true,
            ..Default::default()
        };
        SafeTreeDeleter::delete_dir_contents_via_fd(
            &root,
            snapshot.directory.as_ref().unwrap(),
            &scope,
            &[],
            &environment(),
            None,
            &mut report,
        );

        assert!(
            relocated_payload.exists(),
            "the protected entry behind the retained handle must remain"
        );
        assert!(
            replacement_payload.exists(),
            "the replacement entry inspected through the pathname must remain"
        );
        assert_eq!(report.reclaimed_bytes, 0);
        assert_eq!(report.deleted_files, 0);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("changed during cleanup")),
            "the refusal must name the identity change: {:?}",
            report.errors
        );
    }

    #[cfg(unix)]
    #[test]
    fn recursive_child_open_is_bound_to_the_parent_observation() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("cache");
        let child = root.join("nested");
        let relocated = root.join("relocated");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(child.join("original.bin"), b"original").unwrap();

        let root_metadata = std::fs::symlink_metadata(&root).unwrap();
        let root_snapshot = SafeTreeDeleter::prepare_directory(&root, &root_metadata).unwrap();
        let root_handle = root_snapshot.directory.as_ref().unwrap();
        let observed = SafeTreeDeleter::metadata_at(root_handle, OsStr::new("nested")).unwrap();

        std::fs::rename(&child, &relocated).unwrap();
        std::fs::create_dir(&child).unwrap();
        std::fs::write(child.join("replacement.bin"), b"replacement").unwrap();

        let result = SafeTreeDeleter::prepare_directory_at(
            root_handle,
            OsStr::new("nested"),
            observed,
            &child,
        );
        assert!(result
            .expect_err("a replacement child must not inherit the prior observation")
            .contains("changed during cleanup"));
        assert!(relocated.join("original.bin").exists());
        assert!(child.join("replacement.bin").exists());
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_relative_file_removal_deletes_inside_target() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let payload = root.join("payload.bin");
        std::fs::write(&payload, b"payload").unwrap();

        let report = SafeTreeDeleter::delete_contents_unchecked(&root, &[], &environment());
        assert!(report.is_success(), "errors: {:?}", report.errors);
        assert!(!payload.exists());
        assert!(root.is_dir());
    }

    #[test]
    fn symlink_metadata_absence_reports_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist-link");
        assert!(matches!(
            SymlinkGuard::is_symlink_strict(&missing),
            Err(ZenithError::Missing(_))
        ));
        let environment = zenith_platform::PlatformEnvironment::native();
        assert!(matches!(
            SymlinkGuard::validate_canonical_blacklist_strict(&missing, &environment),
            Err(ZenithError::Missing(_))
        ));
    }

    /// A nested entry that vanished before the walk reached it is already in
    /// the desired state: it is counted as skipped, never as an error, so the
    /// surrounding target stays successful.
    #[test]
    fn vanished_entry_is_a_neutral_skip_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("vanished.bin");

        let mut report = TreeDeleteReport::default();
        let scope = SafeTreeDeleter::capture_verified_scope(dir.path()).unwrap();
        SafeTreeDeleter::delete_entry(&missing, &scope, &[], &environment(), None, &mut report);

        assert!(report.is_success(), "errors: {:?}", report.errors);
        assert_eq!(report.skipped_files, 1);
        assert_eq!(report.deleted_files, 0);
        assert_eq!(report.reclaimed_bytes, 0);
    }

    /// A child that vanished before the walk reached it must not fail the whole
    /// recursive target, and its bytes must not be claimed as reclaimed.
    #[cfg(unix)]
    #[test]
    fn vanished_child_is_excluded_from_recursive_reclaim() {
        use std::os::unix::fs::MetadataExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("cache");
        std::fs::create_dir(&root).unwrap();
        let vanished = root.join("gone.bin");
        std::fs::write(&vanished, vec![7u8; 64 * 1024]).unwrap();
        let kept = root.join("kept.bin");
        std::fs::write(&kept, vec![7u8; 8 * 1024]).unwrap();
        let nested_dir = root.join("nested");
        std::fs::create_dir(&nested_dir).unwrap();
        let nested = nested_dir.join("inner.bin");
        std::fs::write(&nested, vec![7u8; 4 * 1024]).unwrap();

        let expected_bytes = std::fs::metadata(&kept).unwrap().blocks() * 512
            + std::fs::metadata(&nested).unwrap().blocks() * 512;

        std::fs::remove_file(&vanished).unwrap();

        let report = SafeTreeDeleter::delete_contents_unchecked(&root, &[], &environment());
        assert!(report.is_success(), "errors: {:?}", report.errors);
        assert!(report.errors.is_empty());
        assert_eq!(report.reclaimed_bytes, expected_bytes);
        assert!(!kept.exists());
        assert!(!nested_dir.exists());
        // `delete_contents` preserves the cache root itself.
        assert!(root.is_dir());
    }

    /// Removing an already-absent path is a success report, not an error.
    #[test]
    fn delete_path_on_an_already_removed_path_is_a_success_report() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("already-gone");
        std::fs::create_dir(&missing).unwrap();
        std::fs::remove_dir(&missing).unwrap();

        let report = SafeTreeDeleter::delete_path_unchecked(&missing, &[], &environment());
        assert!(report.is_success());
        assert!(report.errors.is_empty());
        assert_eq!(report.reclaimed_bytes, 0);
        assert_eq!(report.deleted_files, 0);
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

        let report = SafeTreeDeleter::delete_path_unchecked(&root, &[], &environment());
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

        let report = SafeTreeDeleter::delete_contents_unchecked(dir.path(), &[], &environment());
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

        let report = SafeTreeDeleter::delete_path_unchecked(&root, &[], &environment());
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
        // The rest of the directory is what a locked file must not cost: a
        // sibling file and a nested directory that are freely removable.
        let sibling = dir.path().join("reclaimable.bin");
        std::fs::write(&sibling, vec![7u8; 4_096]).unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("inner.bin"), vec![9u8; 2_048]).unwrap();

        // Hold an exclusive lock with share_mode(0)
        let lock_handle = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&target)
            .unwrap();

        let report = SafeTreeDeleter::delete_contents_unchecked(dir.path(), &[], &environment());
        assert!(
            !report.is_success(),
            "deletion must fail while file is exclusively locked"
        );
        assert_eq!(
            report.errors.len(),
            1,
            "only the locked entry is a failure: {:?}",
            report.errors
        );
        let error_msg = &report.errors[0];
        assert!(
            error_msg.contains("Sharing violation")
                || error_msg.contains("used by another process")
                || error_msg.contains("os error 32"),
            "Error must report Win32 sharing violation distinctly, got: {}",
            error_msg
        );
        // The locked file is the remainder, and the rest of the directory is
        // reclaimed around it rather than abandoned with it.
        assert!(target.exists(), "the locked file stays");
        assert!(!sibling.exists(), "a removable sibling is still removed");
        assert!(
            !nested.exists(),
            "a nested directory that became empty is still removed"
        );
        assert!(
            report.reclaimed_bytes >= 4_096 + 2_048,
            "the reclaimed amount states what was removed beside the lock: {report:?}"
        );

        drop(lock_handle);

        let report2 = SafeTreeDeleter::delete_contents_unchecked(dir.path(), &[], &environment());
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

        let report = SafeTreeDeleter::delete_contents_unchecked(dir.path(), &[], &environment());
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

    #[cfg(unix)]
    #[test]
    fn the_path_walker_refuses_recent_files_and_keeps_recent_directories() {
        use std::time::{Duration, SystemTime};

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("cache");
        let old_dir = root.join("old");
        let recent_dir = root.join("recent");
        std::fs::create_dir_all(&old_dir).unwrap();
        std::fs::create_dir_all(&recent_dir).unwrap();
        let old_file = old_dir.join("old.bin");
        let recent_file = old_dir.join("recent.bin");
        let recent_only = recent_dir.join("recent.bin");
        std::fs::write(&old_file, vec![0u8; 64]).unwrap();
        std::fs::write(&recent_file, vec![0u8; 64]).unwrap();
        std::fs::write(&recent_only, vec![0u8; 64]).unwrap();

        let old = SystemTime::now() - Duration::from_secs(40 * 24 * 60 * 60);
        for path in [&old_file, &recent_file, &recent_only] {
            let handle = std::fs::File::options().write(true).open(path).unwrap();
            let modified = if path == &old_file {
                old
            } else {
                SystemTime::now()
            };
            handle.set_modified(modified).unwrap();
        }
        for path in [&old_dir, &recent_dir] {
            let handle = std::fs::File::open(path).unwrap();
            handle
                .set_modified(SystemTime::now() - Duration::from_secs(10 * 24 * 60 * 60))
                .unwrap();
        }

        let environment = environment();
        let policy = super::super::StaleEntryPolicy::from_days(7);
        let report =
            SafeTreeDeleter::delete_path_with_policy(&root, &[], &environment, true, Some(policy));

        assert!(!old_file.exists(), "a stale file is removed");
        assert!(recent_file.exists(), "a recent file beside it stays");
        assert!(
            recent_dir.exists(),
            "a directory is descended into and kept"
        );
        assert!(recent_only.exists(), "its recent entry stays");
        assert_eq!(report.skipped_files, 2);
        assert!(!report.is_success());
        assert_eq!(report.errors.len(), 3);
        assert!(root.is_dir());
        assert!(report.reclaimed_bytes > 0);
    }

    /// The path walker judges a file on its own age. It is the route a
    /// filesystem that refuses a parent descriptor falls back to, and it must
    /// refuse the same entries as the descriptor-relative walk.
    #[cfg(unix)]
    #[test]
    fn a_path_entry_is_judged_on_its_own_age() {
        use std::time::{Duration, SystemTime};

        let fixture = tempfile::tempdir().unwrap();
        let dir = fixture.path().join("cache");
        std::fs::create_dir_all(&dir).unwrap();
        let recent = dir.join("recent.bin");
        let old = dir.join("old.bin");
        std::fs::write(&recent, vec![0u8; 64]).unwrap();
        std::fs::write(&old, vec![0u8; 64]).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(40 * 24 * 60 * 60))
            .unwrap();

        let environment = environment();
        let policy = super::super::StaleEntryPolicy::from_days(7);
        let mut report = TreeDeleteReport {
            protect_structured_state: true,
            ..Default::default()
        };
        let scope = SafeTreeDeleter::capture_verified_scope(&dir).unwrap();
        SafeTreeDeleter::delete_entry(
            &recent,
            &scope,
            &[],
            &environment,
            Some(policy),
            &mut report,
        );
        assert!(recent.exists(), "a recent entry is refused");
        assert_eq!(report.skipped_files, 1);
        assert!(report.errors.is_empty(), "{:?}", report.errors);

        SafeTreeDeleter::delete_entry(&old, &scope, &[], &environment, Some(policy), &mut report);
        assert!(!old.exists(), "a stale entry on the same route is removed");
        assert_eq!(report.deleted_files, 1);
    }
}
