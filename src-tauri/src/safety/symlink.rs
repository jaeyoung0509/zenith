use crate::models::{CanonicalPath, PathViolation, ZenithError};
use crate::platform::path_algebra;
use std::fs;
use std::path::{Path, PathBuf};

pub struct SymlinkGuard;

/// The target's components below `base`, when `base` contains it under the
/// flavor's own rules (separators, drive/UNC prefixes, and case folding).
///
/// Pure: no filesystem access, so the same input is judged the same way on
/// every runner.
fn relative_components(
    base: &str,
    target: &str,
    flavor: path_algebra::PathFlavor,
) -> Option<Vec<String>> {
    let separator = flavor.separator();
    let parts = |text: &str| -> Vec<String> {
        // Either separator spelling names the same component on Windows, so the
        // comparison runs on the flavor's canonical spelling.
        path_algebra::canonical_separators(&path_algebra::strip_verbatim(text, flavor), flavor)
            .split(separator)
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect()
    };
    let base_parts = parts(base);
    let target_parts = parts(target);
    if target_parts.len() < base_parts.len() {
        return None;
    }
    let contains = target_parts
        .iter()
        .zip(base_parts.iter())
        .all(|(target, base)| {
            path_algebra::fold(target, flavor) == path_algebra::fold(base, flavor)
        });
    contains.then(|| target_parts[base_parts.len()..].to_vec())
}

/// The drive or UNC root a Windows path descends from, or the path itself when
/// it has none.
fn windows_root(text: &str) -> String {
    let flavor = crate::platform::path_algebra::PathFlavor::Windows;
    let canonical = crate::platform::path_algebra::canonical_separators(
        &crate::platform::path_algebra::strip_verbatim(text, flavor),
        flavor,
    );
    let trimmed = crate::platform::path_algebra::trim_trailing_separators(&canonical, flavor);
    let mut candidate = trimmed.as_str();
    loop {
        if crate::platform::path_algebra::is_root(candidate, flavor) {
            // `path_algebra::is_root` intentionally treats both `C:` and
            // `C:\` as root spellings, but only the latter is absolute.
            // This anchor is fed back through `is_absolute`, so keep the
            // separator for drive roots instead of turning a relocated
            // workspace such as `D:\dev` into the drive-relative `D:`.
            return if candidate.len() == 2 && candidate.ends_with(':') {
                format!("{candidate}\\")
            } else {
                candidate.to_string()
            };
        }
        match candidate.rfind('\\') {
            Some(index) if index > 0 => candidate = &candidate[..index],
            _ => return trimmed,
        }
    }
}

/// Metadata seam for symlink and canonicalization inspection.
///
/// Native code reads the host filesystem. Tests can provide a simulated inspector
/// so that Windows-shaped paths (including 8.3 alias fallback and reparse points)
/// can be verified on macOS and Linux runners.
pub trait SymlinkInspector: Send + Sync {
    fn is_symlink(&self, path: &Path) -> std::io::Result<bool>;
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf>;
}

pub struct NativeSymlinkInspector;

impl SymlinkInspector for NativeSymlinkInspector {
    fn is_symlink(&self, path: &Path) -> std::io::Result<bool> {
        SymlinkGuard::is_symlink_metadata(path)
    }

    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
        fs::canonicalize(path)
    }
}

fn fail_closed_link_result(result: std::io::Result<bool>) -> bool {
    match result {
        Ok(is_symlink) => is_symlink,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

impl SymlinkGuard {
    /// Checks whether the path is a symbolic link or reparse point (junction, mount point) without following it.
    /// Cloud placeholders (OneDrive), deduplication, and WOF compression are treated as regular entries.
    /// For inspection pipelines that must fail closed on access/permission errors, use `is_symlink_metadata`.
    pub fn is_symlink(path: &Path) -> bool {
        fail_closed_link_result(Self::is_symlink_metadata(path))
    }

    /// Reads metadata to check whether the path is a symbolic link or name-surrogate reparse point.
    /// Preserves I/O and permission errors so verification paths can fail closed.
    pub fn is_symlink_metadata(path: &Path) -> std::io::Result<bool> {
        let meta = fs::symlink_metadata(path)?;
        if meta.file_type().is_symlink() {
            return Ok(true);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return classify_name_surrogate_reparse_point(path);
            }
        }
        Ok(false)
    }

    /// Resolves the trusted base anchor for a given target path.
    /// E.g., user home directory (`/Users/username` or `C:\Users\username`), `/tmp`, or temp dir.
    ///
    /// The anchors come from the described environment, and the comparison uses
    /// its path rules: the host's profile is never substituted for a stated one,
    /// and a Windows-shaped target resolves to its own drive or UNC root instead
    /// of collapsing to the POSIX `/`.
    pub fn resolve_trusted_anchor(
        target: &Path,
        environment: &crate::platform::PlatformEnvironment,
    ) -> PathBuf {
        let flavor = environment.flavor();
        let text = target.to_string_lossy();
        if let Some(home) = environment.user_home() {
            if path_algebra::contains(&home.to_string_lossy(), &text, flavor) {
                return home;
            }
        }
        let temp = environment.temp_dir();
        if path_algebra::contains(&temp.to_string_lossy(), &text, flavor) {
            return temp;
        }
        if flavor.is_windows() {
            return PathBuf::from(windows_root(&text));
        }
        if !text.starts_with('/') {
            // A relative target has no anchor to descend from; it is returned
            // unchanged so the component check still refuses it.
            return target.to_path_buf();
        }
        for anchor in ["/private/tmp", "/tmp", "/private/var", "/var"] {
            if text.starts_with(anchor) {
                return PathBuf::from(anchor);
            }
        }
        PathBuf::from("/")
    }

    /// Validates all path components from `base` down to `target`.
    ///
    /// The lexical judgments — absolute, parent traversal, and containment — are
    /// decided by the described flavor's algebra, so a Windows-shaped path is
    /// judged by Windows rules on every runner. Only the per-component check
    /// reads filesystem metadata, which defaults to the native provider and
    /// can be injected for cross-platform fixture verification.
    pub fn validate_components_between(
        target: &Path,
        base: &Path,
        environment: &crate::platform::PlatformEnvironment,
    ) -> Result<(), ZenithError> {
        Self::validate_components_between_with(target, base, environment, &NativeSymlinkInspector)
    }

    pub fn validate_components_between_with(
        target: &Path,
        base: &Path,
        environment: &crate::platform::PlatformEnvironment,
        inspector: &impl SymlinkInspector,
    ) -> Result<(), ZenithError> {
        let flavor = environment.flavor();
        let target_text = target.to_string_lossy();
        let base_text = base.to_string_lossy();
        // Do not erase a link/.. traversal before checking its components.
        if !path_algebra::is_absolute(&target_text, flavor)
            || !path_algebra::is_absolute(&base_text, flavor)
            || path_algebra::has_parent_traversal(&target_text, flavor)
            || path_algebra::has_parent_traversal(&base_text, flavor)
        {
            return Err(ZenithError::SymlinkEscape(
                "Expected an absolute path without parent traversal".into(),
            ));
        }
        let normalized_target = path_algebra::normalize(&target_text, flavor);
        let normalized_base = path_algebra::normalize(&base_text, flavor);
        let outside_base = || {
            ZenithError::SymlinkEscape(format!(
                "Target {} is not within base {}",
                target.display(),
                base.display()
            ))
        };
        let (relative, start_base) =
            match relative_components(&normalized_base, &normalized_target, flavor) {
                Some(relative) => (relative, normalized_base),
                None => {
                    if flavor.is_windows() {
                        // TEMP can contain an 8.3 account alias (RUNNER~1) while
                        // canonicalize returns its long name. Check both original
                        // paths from their roots BEFORE resolving aliases, so a
                        // junction cannot disappear during canonicalization.
                        for path in [target, base] {
                            let root = PathBuf::from(windows_root(&path.to_string_lossy()));
                            Self::validate_components_between_with(
                                path,
                                &root,
                                environment,
                                inspector,
                            )?;
                        }
                        let canonical_target =
                            inspector
                                .canonicalize(target)
                                .map_err(|err| match err.kind() {
                                    std::io::ErrorKind::PermissionDenied => {
                                        ZenithError::PermissionDenied(target.display().to_string())
                                    }
                                    std::io::ErrorKind::NotFound => outside_base(),
                                    _ => ZenithError::Io(err.to_string()),
                                })?;
                        let canonical_base =
                            inspector
                                .canonicalize(base)
                                .map_err(|err| match err.kind() {
                                    std::io::ErrorKind::PermissionDenied => {
                                        ZenithError::PermissionDenied(base.display().to_string())
                                    }
                                    std::io::ErrorKind::NotFound => outside_base(),
                                    _ => ZenithError::Io(err.to_string()),
                                })?;
                        let norm_canonical_base =
                            path_algebra::normalize(&canonical_base.to_string_lossy(), flavor);
                        let norm_canonical_target =
                            path_algebra::normalize(&canonical_target.to_string_lossy(), flavor);
                        let relative = relative_components(
                            &norm_canonical_base,
                            &norm_canonical_target,
                            flavor,
                        )
                        .ok_or_else(outside_base)?;
                        (relative, norm_canonical_base)
                    } else {
                        return Err(outside_base());
                    }
                }
            };

        let mut current_text = start_base;
        for component in relative {
            if !current_text.ends_with(flavor.separator()) {
                current_text.push(flavor.separator());
            }
            current_text.push_str(&component);
            let current = PathBuf::from(&current_text);
            match inspector.is_symlink(&current) {
                Ok(true) => {
                    return Err(ZenithError::SymlinkEscape(format!(
                        "Path component is a symlink or reparse escape: {}",
                        current.display()
                    )));
                }
                Ok(false) => {}
                Err(err) => {
                    return Err(match err.kind() {
                        std::io::ErrorKind::PermissionDenied => {
                            ZenithError::PermissionDenied(current.display().to_string())
                        }
                        std::io::ErrorKind::NotFound => {
                            ZenithError::Missing(current.display().to_string())
                        }
                        _ => ZenithError::Io(err.to_string()),
                    });
                }
            }
        }

        Ok(())
    }

    /// Verifies that no intermediate component between trusted base anchor, trusted_root,
    /// and target (inclusive) is a symlink.
    pub fn validate_no_symlink_ancestors(
        target: &Path,
        trusted_root: &Path,
        environment: &crate::platform::PlatformEnvironment,
    ) -> Result<(), ZenithError> {
        Self::validate_no_symlink_ancestors_with(
            target,
            trusted_root,
            environment,
            &NativeSymlinkInspector,
        )
    }

    pub fn validate_no_symlink_ancestors_with(
        target: &Path,
        trusted_root: &Path,
        environment: &crate::platform::PlatformEnvironment,
        inspector: &impl SymlinkInspector,
    ) -> Result<(), ZenithError> {
        let anchor = Self::resolve_trusted_anchor(trusted_root, environment);
        if !path_algebra::equal(
            &anchor.to_string_lossy(),
            &trusted_root.to_string_lossy(),
            environment.flavor(),
        ) {
            Self::validate_components_between_with(trusted_root, &anchor, environment, inspector)?;
        }

        Self::validate_components_between_with(target, trusted_root, environment, inspector)?;
        Ok(())
    }

    /// Validates that target has no symlink ancestors from its system anchor (home/temp/root)
    pub fn validate_anchored_path(
        target: &Path,
        environment: &crate::platform::PlatformEnvironment,
    ) -> Result<(), ZenithError> {
        Self::validate_anchored_path_with(target, environment, &NativeSymlinkInspector)
    }

    pub fn validate_anchored_path_with(
        target: &Path,
        environment: &crate::platform::PlatformEnvironment,
        inspector: &impl SymlinkInspector,
    ) -> Result<(), ZenithError> {
        let anchor = Self::resolve_trusted_anchor(target, environment);
        Self::validate_components_between_with(target, &anchor, environment, inspector)
    }

    /// Verifies that the path itself is safe. If it is a symlink, ensures its target does not point to a blacklisted destination.
    pub fn validate_symlink_target(
        path: &Path,
        environment: &crate::platform::PlatformEnvironment,
    ) -> Result<(), ZenithError> {
        if Self::is_symlink(path) {
            // Read link destination
            if let Ok(target) = fs::read_link(path) {
                let resolved_target = if target.is_relative() {
                    path.parent().unwrap_or(Path::new("")).join(target)
                } else {
                    target
                };

                // Target cannot be blacklisted
                crate::safety::Blacklist::validate_with(&resolved_target, environment)?;
            }
        }
        Ok(())
    }

    /// Canonicalizes the path and verifies that its canonical location does not violate Blacklist.
    ///
    /// The blacklist is applied to the resolved location. Every spelling of a
    /// file resolves to one path, so a target cannot be named around the rule,
    /// and a link is judged by where it points rather than by what it is
    /// called.
    pub fn validate_canonical_blacklist(
        path: &Path,
        environment: &crate::platform::PlatformEnvironment,
    ) -> Result<(), ZenithError> {
        if let Ok(canonical) = CanonicalPath::resolve(path) {
            crate::safety::Blacklist::validate_with(canonical.as_path(), environment)?;
        }
        Ok(())
    }

    /// Strict variant for mutation paths: canonicalization failure aborts the
    /// affected mutation instead of being treated as safe.
    pub fn validate_canonical_blacklist_strict(
        path: &Path,
        environment: &crate::platform::PlatformEnvironment,
    ) -> Result<(), ZenithError> {
        let canonical = CanonicalPath::resolve(path).map_err(|violation| {
            // A path that no longer resolves is already absent; every other
            // resolution failure is a mutation blocker.
            match &violation {
                PathViolation::Unresolvable { kind, .. }
                    if *kind == std::io::ErrorKind::NotFound =>
                {
                    ZenithError::Missing(path.display().to_string())
                }
                _ => ZenithError::ChangedSinceScan(violation.to_string()),
            }
        })?;
        crate::safety::Blacklist::validate_with(canonical.as_path(), environment)?;
        Ok(())
    }

    /// Strict symlink/reparse-point check for mutation paths. Metadata failure
    /// fails closed instead of reporting "not a symlink".
    pub fn is_symlink_strict(path: &Path) -> Result<bool, ZenithError> {
        let meta = fs::symlink_metadata(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                return ZenithError::Missing(path.display().to_string());
            }
            ZenithError::ChangedSinceScan(format!(
                "Could not read link metadata for {}: {}",
                path.display(),
                error
            ))
        })?;
        if meta.file_type().is_symlink() {
            return Ok(true);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                // A reparse point whose tag cannot be read is unknown, not
                // safe: mutation paths must fail closed.
                return classify_name_surrogate_reparse_point(path).map_err(|error| {
                    ZenithError::ChangedSinceScan(format!(
                        "Could not classify reparse point {}: {}",
                        path.display(),
                        error
                    ))
                });
            }
        }
        Ok(false)
    }
}

/// `IsReparseTagNameSurrogate(tag)` from winnt.h. Name surrogates are the
/// reparse points that act as path indirections (symbolic links and mount
/// points/junctions). Other reparse points — cloud placeholders, WOF
/// compression, deduplication — are ordinary entries for traversal purposes.
#[cfg(any(windows, test))]
fn is_reparse_tag_name_surrogate(tag: u32) -> bool {
    tag & 0x2000_0000 != 0
}

/// Opens the reparse point without following it and classifies its tag.
/// Every failure is an error so callers cannot mistake "unknown" for "safe".
#[cfg(windows)]
fn classify_name_surrogate_reparse_point(path: &Path) -> std::io::Result<bool> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FileAttributeTagInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_TAG_INFO,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let wide = crate::platform::NativePlatformPaths::to_verbatim_wide(path);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(std::io::Error::last_os_error());
    }

    let mut tag_info: FILE_ATTRIBUTE_TAG_INFO = unsafe { std::mem::zeroed() };
    let queried = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            &mut tag_info as *mut _ as *mut std::ffi::c_void,
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    // Capture the error before closing the handle so it cannot be overwritten.
    let query_error = if queried == 0 {
        Some(std::io::Error::last_os_error())
    } else {
        None
    };
    unsafe { CloseHandle(handle) };
    if let Some(error) = query_error {
        return Err(error);
    }
    Ok(is_reparse_tag_name_surrogate(tag_info.ReparseTag))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lexical judgments follow the stated flavor, so a Windows-shaped pair
    /// is decided by Windows rules on this runner: separators, drive prefixes,
    /// and case folding.
    #[test]
    fn component_validation_judges_windows_shaped_paths_with_the_stated_flavor() {
        use crate::platform::path_algebra::PathFlavor;

        // Containment is component-wise and folds case on Windows.
        assert_eq!(
            relative_components(
                r"C:\Users\Tester\cache",
                r"C:\Users\tester\cache\npm\_cacache",
                PathFlavor::Windows,
            ),
            Some(vec!["npm".to_string(), "_cacache".to_string()])
        );
        // A sibling directory is not contained, and a prefix lookalike is not
        // either.
        assert_eq!(
            relative_components(
                r"C:\Users\tester\cache",
                r"C:\Users\tester\cache-two\x",
                PathFlavor::Windows,
            ),
            None
        );
        // Forward slashes are separators on Windows and characters on POSIX.
        assert_eq!(
            relative_components(
                "C:/Users/tester/cache",
                "C:/Users/tester/cache/npm",
                PathFlavor::Windows,
            ),
            Some(vec!["npm".to_string()])
        );
        assert_eq!(
            relative_components("/Users/tester", "/Users/tester/cache", PathFlavor::Posix),
            Some(vec!["cache".to_string()])
        );
        // POSIX keeps case sensitivity.
        assert_eq!(
            relative_components("/Users/Tester", "/Users/tester/cache", PathFlavor::Posix),
            None
        );
    }

    #[test]
    fn windows_drive_and_unc_anchors_remain_absolute() {
        use crate::platform::path_algebra::PathFlavor;

        for (target, expected) in [
            (r"D:\dev\zenith", "D:\\"),
            (r"\\server\share\projects\zenith", r"\\server\share"),
        ] {
            let anchor = windows_root(target);
            assert_eq!(anchor, expected);
            assert!(
                path_algebra::is_absolute(&anchor, PathFlavor::Windows),
                "anchor must be absolute: {anchor}"
            );
        }
    }

    #[test]
    fn normalization_never_hides_parent_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("link/../cache");
        assert!(SymlinkGuard::validate_components_between(
            &path,
            dir.path(),
            &crate::platform::PlatformEnvironment::native(),
        )
        .is_err());
    }

    #[test]
    fn reparse_tags_classify_name_surrogates_separately() {
        // winnt.h: IO_REPARSE_TAG_SYMLINK and IO_REPARSE_TAG_MOUNT_POINT.
        assert!(is_reparse_tag_name_surrogate(0xA000_000C));
        assert!(is_reparse_tag_name_surrogate(0xA000_0003));
        // Cloud placeholder, WOF compression, and dedup are not surrogates:
        // they are ordinary entries for traversal purposes.
        assert!(!is_reparse_tag_name_surrogate(0x9000_001A));
        assert!(!is_reparse_tag_name_surrogate(0x8000_0017));
        assert!(!is_reparse_tag_name_surrogate(0x8000_0007));
        assert!(!is_reparse_tag_name_surrogate(0));
    }

    #[test]
    fn a_path_that_cannot_be_inspected_is_never_verified_by_the_strict_checks() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("not-created-yet");

        // Read-side helpers report "no link to follow" for a missing path.
        assert!(!SymlinkGuard::is_symlink(&missing));
        let environment = crate::platform::PlatformEnvironment::native();
        assert!(SymlinkGuard::validate_symlink_target(&missing, &environment).is_ok());

        // Mutation-side helpers never mistake a missing path for a
        // verified-safe one: they report the distinct absence signal, which
        // callers classify as already-absent. Any other metadata failure still
        // returns `ChangedSinceScan` and fails closed.
        assert!(matches!(
            SymlinkGuard::is_symlink_strict(&missing),
            Err(ZenithError::Missing(_))
        ));
        assert!(matches!(
            SymlinkGuard::validate_canonical_blacklist_strict(&missing, &environment),
            Err(ZenithError::Missing(_))
        ));
    }

    #[test]
    fn boolean_link_check_only_treats_not_found_as_an_ordinary_absence() {
        assert!(!fail_closed_link_result(Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "missing",
        ))));
        assert!(fail_closed_link_result(Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "refused",
        ))));
        assert!(fail_closed_link_result(Err(std::io::Error::other(
            "reparse classification failed",
        ))));
    }

    #[cfg(unix)]
    #[test]
    fn links_are_detected_even_when_their_targets_do_not_exist() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("plain.txt");
        fs::write(&file, b"payload").unwrap();
        let link = dir.path().join("link");
        symlink(&file, &link).unwrap();
        let dangling = dir.path().join("dangling");
        symlink(dir.path().join("missing-target"), &dangling).unwrap();

        assert!(SymlinkGuard::is_symlink(&link));
        assert!(SymlinkGuard::is_symlink_strict(&link).unwrap());
        assert!(!SymlinkGuard::is_symlink(&file));
        assert!(!SymlinkGuard::is_symlink_strict(&file).unwrap());

        // A link is a link even when following it leads nowhere, so a dangling
        // link is still refused as a traversal path.
        assert!(SymlinkGuard::is_symlink(&dangling));
        assert!(SymlinkGuard::is_symlink_strict(&dangling).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn reparse_classification_returns_error_for_unopenable_paths() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing-reparse-target");
        assert!(classify_name_surrogate_reparse_point(&missing).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn selected_plain_path_matches_canonical_profile_and_outside_roots_are_allowed() {
        let dir = tempfile::tempdir().unwrap();
        // `validate_workspace_root` compares the canonicalized path against the
        // stated roots, so the fixture states the canonical root: a temporary
        // path spelled with an 8.3 alias would otherwise fall back to walking in
        // from the drive root.
        let base = dir.path().canonicalize().unwrap();
        let profile = base.join("사용자 하나");
        let workspace = profile.join("프로젝트");
        let other = base.join("사용자 둘");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&other).unwrap();
        let canonical_profile = profile.canonicalize().unwrap();
        let plain_workspace = crate::safety::Blacklist::normalize_path(&workspace);
        SymlinkGuard::validate_components_between(
            &plain_workspace,
            &canonical_profile,
            &crate::platform::PlatformEnvironment::simulated(
                crate::platform::path_algebra::PathFlavor::Windows,
            ),
        )
        .unwrap_or_else(|error| {
            panic!("target={plain_workspace:?}, base={canonical_profile:?}: {error}")
        });
        assert!(SymlinkGuard::validate_components_between(
            &other,
            &canonical_profile,
            &crate::platform::PlatformEnvironment::simulated(
                crate::platform::path_algebra::PathFlavor::Windows,
            ),
        )
        .is_err());
        let environment = crate::platform::PlatformEnvironment::simulated(
            crate::platform::path_algebra::PathFlavor::Windows,
        )
        .with_home(profile.clone())
        // The fixture lives in the temporary directory, which is the root the
        // relocated workspace is anchored at; the profile above is the selected
        // one, not the containing one.
        .with_temp_dir(base.clone());
        crate::developer_artifacts::validate_workspace_root(&environment, &plain_workspace)
            .unwrap_or_else(|error| panic!("the selected workspace must be allowed: {error}"));
        // A relocated workspace outside the selected profile (D:\dev-style)
        // is accepted under anchored symlink validation; the cross-account
        // traversal check above still rejects reading through another profile.
        crate::developer_artifacts::validate_workspace_root(&environment, &other)
            .unwrap_or_else(|error| panic!("a relocated workspace must be allowed: {error}"));
    }

    #[cfg(windows)]
    #[test]
    fn alias_fallback_does_not_hide_a_junction_into_the_trusted_profile() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("사용자 하나");
        let workspace = profile.join("프로젝트");
        let link = dir.path().join("다른 경로");
        fs::create_dir_all(&workspace).unwrap();
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&link)
            .arg(&profile)
            .output()
            .unwrap();
        assert!(output.status.success());
        let environment = crate::platform::PlatformEnvironment::simulated(
            crate::platform::path_algebra::PathFlavor::Windows,
        )
        .with_home(profile.clone());
        assert!(SymlinkGuard::validate_components_between(
            &link.join("프로젝트"),
            &profile.canonicalize().unwrap(),
            &environment,
        )
        .is_err());
    }

    #[derive(Default)]
    struct TestSymlinkInspector {
        symlinks: std::collections::HashSet<PathBuf>,
        errors: std::collections::HashMap<PathBuf, std::io::ErrorKind>,
        canonical_map: std::collections::HashMap<PathBuf, PathBuf>,
        canonical_errors: std::collections::HashMap<PathBuf, std::io::ErrorKind>,
    }

    impl SymlinkInspector for TestSymlinkInspector {
        fn is_symlink(&self, path: &Path) -> std::io::Result<bool> {
            if let Some(kind) = self.errors.get(path) {
                return Err(std::io::Error::new(*kind, "simulated metadata error"));
            }
            Ok(self.symlinks.contains(path))
        }

        fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
            if let Some(kind) = self.canonical_errors.get(path) {
                return Err(std::io::Error::new(*kind, "simulated canonical error"));
            }
            if let Some(canonical) = self.canonical_map.get(path) {
                Ok(canonical.clone())
            } else {
                Ok(path.to_path_buf())
            }
        }
    }

    #[test]
    fn symlink_guard_verifies_windows_shaped_links_on_any_runner_via_seam() {
        let environment = crate::platform::PlatformEnvironment::simulated(
            crate::platform::path_algebra::PathFlavor::Windows,
        );
        let base = Path::new(r"C:\Users\Tester\AppData\Local\Temp");
        let target = Path::new(r"C:\Users\Tester\AppData\Local\Temp\npm_cache\payload.bin");

        let mut inspector = TestSymlinkInspector::default();
        // Without symlinks, validation succeeds on Windows-shaped path.
        assert!(SymlinkGuard::validate_components_between_with(
            target,
            base,
            &environment,
            &inspector,
        )
        .is_ok());

        // When intermediate component is marked as symlink in the inspector:
        inspector.symlinks.insert(PathBuf::from(
            r"C:\Users\Tester\AppData\Local\Temp\npm_cache",
        ));
        let err =
            SymlinkGuard::validate_components_between_with(target, base, &environment, &inspector)
                .unwrap_err();
        assert!(
            matches!(err, ZenithError::SymlinkEscape(_)),
            "must reject component symlink: {err}"
        );
    }

    #[test]
    fn symlink_guard_fails_closed_when_component_metadata_inspection_fails_with_permission_denied()
    {
        let environment = crate::platform::PlatformEnvironment::simulated(
            crate::platform::path_algebra::PathFlavor::Windows,
        );
        let base = Path::new(r"C:\Users\Tester\AppData\Local\Temp");
        let target = Path::new(r"C:\Users\Tester\AppData\Local\Temp\npm_cache\payload.bin");

        let mut inspector = TestSymlinkInspector::default();
        inspector.errors.insert(
            PathBuf::from(r"C:\Users\Tester\AppData\Local\Temp\npm_cache"),
            std::io::ErrorKind::PermissionDenied,
        );
        let err =
            SymlinkGuard::validate_components_between_with(target, base, &environment, &inspector)
                .unwrap_err();
        assert!(
            matches!(err, ZenithError::PermissionDenied(_)),
            "must fail closed as PermissionDenied, not SymlinkEscape or success: {err:?}"
        );
    }

    #[test]
    fn windows_8_3_alias_fallback_resolves_and_validates_on_any_runner_via_seam() {
        let environment = crate::platform::PlatformEnvironment::simulated(
            crate::platform::path_algebra::PathFlavor::Windows,
        );
        // Base is 8.3 alias RUNNER~1
        let base_alias = Path::new(r"C:\Users\RUNNER~1\AppData\Local\Temp");
        // Target is under the expanded long name
        let target = Path::new(r"C:\Users\runneradmin\AppData\Local\Temp\cache\item.bin");

        let mut inspector = TestSymlinkInspector::default();
        // Canonical map expands RUNNER~1 to runneradmin
        inspector.canonical_map.insert(
            base_alias.to_path_buf(),
            PathBuf::from(r"C:\Users\runneradmin\AppData\Local\Temp"),
        );
        inspector.canonical_map.insert(
            PathBuf::from(r"C:\Users\RUNNER~1"),
            PathBuf::from(r"C:\Users\runneradmin"),
        );

        // Alias resolution allows validation to succeed
        assert!(SymlinkGuard::validate_components_between_with(
            target,
            base_alias,
            &environment,
            &inspector,
        )
        .is_ok());

        // But if an ancestor before canonicalization is a junction/symlink:
        inspector
            .symlinks
            .insert(PathBuf::from(r"C:\Users\RUNNER~1"));
        let err = SymlinkGuard::validate_components_between_with(
            target,
            base_alias,
            &environment,
            &inspector,
        )
        .unwrap_err();
        assert!(
            matches!(err, ZenithError::SymlinkEscape(_)),
            "junction in alias ancestor must fail closed: {err}"
        );
    }

    #[test]
    fn windows_8_3_alias_canonicalization_failure_preserves_permission_denied() {
        let environment = crate::platform::PlatformEnvironment::simulated(
            crate::platform::path_algebra::PathFlavor::Windows,
        );
        let base_alias = Path::new(r"C:\Users\RUNNER~1\AppData\Local\Temp");
        let target = Path::new(r"C:\Users\runneradmin\AppData\Local\Temp\cache\item.bin");

        let mut inspector = TestSymlinkInspector::default();
        inspector.canonical_errors.insert(
            base_alias.to_path_buf(),
            std::io::ErrorKind::PermissionDenied,
        );
        let err = SymlinkGuard::validate_components_between_with(
            target,
            base_alias,
            &environment,
            &inspector,
        )
        .unwrap_err();
        assert!(
            matches!(err, ZenithError::PermissionDenied(_)),
            "canonicalize PermissionDenied must fail closed as PermissionDenied: {err:?}"
        );
    }
}
