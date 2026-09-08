use std::path::{Path, PathBuf};

/// Platform-owned path resolution interface for user directories and reviewed system roots.
pub trait PlatformPathsProvider: Send + Sync {
    fn user_home(&self) -> Option<PathBuf>;
    fn local_app_data(&self) -> Option<PathBuf>;
    fn roaming_app_data(&self) -> Option<PathBuf>;
    fn temp_dir(&self) -> PathBuf;
    fn program_files(&self) -> Option<PathBuf>;
    fn program_data(&self) -> Option<PathBuf>;

    /// Expands allowlisted placeholders:
    /// - `${USER_HOME}` or `~`
    /// - `${LOCAL_APP_DATA}`
    /// - `${ROAMING_APP_DATA}`
    /// - `${TEMP}` or `$TMPDIR`
    /// - `${PROGRAM_FILES}`
    /// - `${PROGRAM_DATA}`
    ///
    /// Rejects arbitrary environment variables, empty roots, and broad filesystem roots.
    fn expand_placeholder(&self, pattern: &str) -> Option<PathBuf> {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return None;
        }

        let raw_path = if pattern == "$TMPDIR" || pattern == "${TEMP}" {
            self.temp_dir()
        } else if let Some(rest) = pattern
            .strip_prefix("${TEMP}/")
            .or_else(|| pattern.strip_prefix("${TEMP}\\"))
            .or_else(|| pattern.strip_prefix("$TMPDIR/"))
        {
            self.temp_dir().join(rest)
        } else if let Some(rest) = pattern
            .strip_prefix("${USER_HOME}/")
            .or_else(|| pattern.strip_prefix("${USER_HOME}\\"))
        {
            self.user_home()?.join(rest)
        } else if pattern == "${USER_HOME}" || pattern == "~" {
            self.user_home()?
        } else if let Some(rest) = pattern.strip_prefix("~/") {
            self.user_home()?.join(rest)
        } else if let Some(rest) = pattern
            .strip_prefix("${LOCAL_APP_DATA}/")
            .or_else(|| pattern.strip_prefix("${LOCAL_APP_DATA}\\"))
        {
            self.local_app_data()?.join(rest)
        } else if pattern == "${LOCAL_APP_DATA}" {
            self.local_app_data()?
        } else if let Some(rest) = pattern
            .strip_prefix("${ROAMING_APP_DATA}/")
            .or_else(|| pattern.strip_prefix("${ROAMING_APP_DATA}\\"))
        {
            self.roaming_app_data()?.join(rest)
        } else if pattern == "${ROAMING_APP_DATA}" {
            self.roaming_app_data()?
        } else if let Some(rest) = pattern
            .strip_prefix("${PROGRAM_FILES}/")
            .or_else(|| pattern.strip_prefix("${PROGRAM_FILES}\\"))
        {
            self.program_files()?.join(rest)
        } else if pattern == "${PROGRAM_FILES}" {
            self.program_files()?
        } else if let Some(rest) = pattern
            .strip_prefix("${PROGRAM_DATA}/")
            .or_else(|| pattern.strip_prefix("${PROGRAM_DATA}\\"))
        {
            self.program_data()?.join(rest)
        } else if pattern == "${PROGRAM_DATA}" {
            self.program_data()?
        } else if pattern.starts_with("${") {
            // Reject any unapproved arbitrary placeholder
            return None;
        } else {
            PathBuf::from(pattern)
        };

        // Normalize path without following symlinks
        let normalized = crate::safety::Blacklist::normalize_path(&raw_path);

        // Safety: Path must be absolute and not a broad root
        if !normalized.is_absolute() {
            return None;
        }

        // Root protection: Reject drive roots like "C:\" or "/"
        if is_broad_root(&normalized) {
            return None;
        }

        Some(normalized)
    }
}

fn is_broad_root(path: &Path) -> bool {
    if path.parent().is_none() {
        return true;
    }
    #[cfg(windows)]
    {
        if let Some(path_str) = path.to_str() {
            let trimmed = path_str.trim_end_matches(['\\', '/']);
            if trimmed.len() == 2 && trimmed.ends_with(':') {
                return true;
            }
        }
    }
    false
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NativePlatformPaths;

impl NativePlatformPaths {
    pub fn new() -> Self {
        Self
    }

    pub fn home(&self) -> Option<PathBuf> {
        self.user_home()
    }

    /// Only fixed user-content tokens may cross the IPC boundary.
    pub fn content_dir(&self, token: &str) -> Option<PathBuf> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::Shell::*;
            let id = match token {
                "downloads" => FOLDERID_Downloads,
                "desktop" => FOLDERID_Desktop,
                "documents" => FOLDERID_Documents,
                "movies" => FOLDERID_Videos,
                _ => return None,
            };
            windows_known_folder(&id)
        }
        #[cfg(not(windows))]
        {
            let name = match token {
                "downloads" => "Downloads",
                "desktop" => "Desktop",
                "documents" => "Documents",
                "movies" => "Movies",
                _ => return None,
            };
            Some(self.home()?.join(name))
        }
    }

    #[cfg(windows)]
    pub fn application_roots(&self) -> Vec<PathBuf> {
        let mut roots = ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
            .into_iter()
            .filter_map(|key| std::env::var_os(key).map(PathBuf::from))
            .filter(|path| path.is_absolute())
            .collect::<Vec<_>>();
        if let Some(local) = self.local_app_data() {
            roots.push(local.join("Programs"));
        }
        roots.sort();
        roots.dedup();
        roots
    }
}

#[cfg(windows)]
fn windows_known_folder(id: &windows_sys::core::GUID) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{SHGetKnownFolderPath, KF_FLAG_DONT_VERIFY};
    let mut value = std::ptr::null_mut();
    // Null token selects the current account, including redirected known folders.
    let status = unsafe {
        SHGetKnownFolderPath(
            id,
            KF_FLAG_DONT_VERIFY as u32,
            std::ptr::null_mut(),
            &mut value,
        )
    };
    if value.is_null() {
        return None;
    }
    if status < 0 {
        unsafe {
            CoTaskMemFree(value.cast());
        }
        return None;
    }
    let path = unsafe {
        let mut len = 0;
        while *value.add(len) != 0 {
            len += 1;
        }
        let path = PathBuf::from(std::ffi::OsString::from_wide(std::slice::from_raw_parts(
            value, len,
        )));
        CoTaskMemFree(value.cast());
        path
    };
    (path.is_absolute() && !is_broad_root(&path)).then_some(path)
}

impl PlatformPathsProvider for NativePlatformPaths {
    fn user_home(&self) -> Option<PathBuf> {
        // Windows HOME may belong to Git/MSYS or another account. Never use it
        // as an alternate scan authority when USERPROFILE is missing.
        resolve_user_home(cfg!(windows), |key| std::env::var_os(key))
    }

    fn local_app_data(&self) -> Option<PathBuf> {
        #[cfg(target_os = "windows")]
        {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .or_else(|| self.user_home().map(|h| h.join("AppData\\Local")))
        }

        #[cfg(target_os = "macos")]
        {
            self.user_home()
                .map(|h| h.join("Library/Application Support"))
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| self.user_home().map(|h| h.join(".local/share")))
        }
    }

    fn roaming_app_data(&self) -> Option<PathBuf> {
        #[cfg(target_os = "windows")]
        {
            std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .or_else(|| self.user_home().map(|h| h.join("AppData\\Roaming")))
        }

        #[cfg(target_os = "macos")]
        {
            self.user_home()
                .map(|h| h.join("Library/Application Support"))
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| self.user_home().map(|h| h.join(".config")))
        }
    }

    fn temp_dir(&self) -> PathBuf {
        std::env::temp_dir()
    }

    fn program_files(&self) -> Option<PathBuf> {
        #[cfg(target_os = "windows")]
        {
            std::env::var_os("ProgramFiles")
                .map(PathBuf::from)
                .or_else(|| Some(PathBuf::from("C:\\Program Files")))
        }

        #[cfg(target_os = "macos")]
        {
            Some(PathBuf::from("/Applications"))
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            Some(PathBuf::from("/usr/bin"))
        }
    }

    fn program_data(&self) -> Option<PathBuf> {
        #[cfg(target_os = "windows")]
        {
            std::env::var_os("ProgramData")
                .map(PathBuf::from)
                .or_else(|| Some(PathBuf::from("C:\\ProgramData")))
        }

        #[cfg(target_os = "macos")]
        {
            Some(PathBuf::from("/Library"))
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            Some(PathBuf::from("/var/lib"))
        }
    }
}

fn resolve_user_home(
    windows: bool,
    get: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    get(if windows { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && !is_broad_root(path))
}

#[cfg(test)]
pub struct MockPlatformPaths {
    pub home: PathBuf,
    pub local_appdata: PathBuf,
    pub roaming_appdata: PathBuf,
    pub temp: PathBuf,
}

#[cfg(test)]
impl PlatformPathsProvider for MockPlatformPaths {
    fn user_home(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }

    fn local_app_data(&self) -> Option<PathBuf> {
        Some(self.local_appdata.clone())
    }

    fn roaming_app_data(&self) -> Option<PathBuf> {
        Some(self.roaming_appdata.clone())
    }

    fn temp_dir(&self) -> PathBuf {
        self.temp.clone()
    }

    fn program_files(&self) -> Option<PathBuf> {
        Some(self.home.join("ProgramFiles"))
    }

    fn program_data(&self) -> Option<PathBuf> {
        Some(self.home.join("ProgramData"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn windows_profile_does_not_follow_another_accounts_shell_home() {
        let dir = tempdir().unwrap();
        let profile = dir.path().join("사용자 하나");
        let other = dir.path().join("다른 계정");
        let get = |key: &str| match key {
            "USERPROFILE" => Some(profile.clone().into_os_string()),
            "HOME" => Some(other.clone().into_os_string()),
            _ => None,
        };
        assert_eq!(resolve_user_home(true, get), Some(profile.clone()));
        assert_eq!(resolve_user_home(false, get), Some(other.clone()));
        assert_eq!(
            resolve_user_home(true, |key| (key == "HOME")
                .then(|| other.clone().into_os_string())),
            None
        );
        assert_eq!(resolve_user_home(true, |_| Some("".into())), None);
        assert_eq!(
            resolve_user_home(true, |_| Some("relative/profile".into())),
            None
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_profile_preserves_non_c_drive_and_korean_name() {
        assert_eq!(
            resolve_user_home(true, |_| Some(r"D:\Users\홍 길동".into())),
            Some(PathBuf::from(r"D:\Users\홍 길동"))
        );
        assert!(resolve_user_home(true, |_| Some(r"D:\".into())).is_none());
        assert!(resolve_user_home(true, |_| Some(r"\\server\share\".into())).is_none());
        let paths = NativePlatformPaths::new();
        assert!(paths
            .content_dir("documents")
            .is_some_and(|path| path.is_absolute()));
        assert!(paths.content_dir("../../other-user").is_none());
    }

    #[test]
    fn mock_platform_paths_expands_allowlisted_placeholders() {
        let dir = tempdir().unwrap();
        let mock = MockPlatformPaths {
            home: dir.path().join("home"),
            local_appdata: dir.path().join("home/AppData/Local"),
            roaming_appdata: dir.path().join("home/AppData/Roaming"),
            temp: dir.path().join("temp"),
        };

        assert_eq!(
            mock.expand_placeholder("${USER_HOME}/.cargo/registry"),
            Some(dir.path().join("home/.cargo/registry"))
        );
        assert_eq!(
            mock.expand_placeholder("${LOCAL_APP_DATA}/Zenith/Cache"),
            Some(dir.path().join("home/AppData/Local/Zenith/Cache"))
        );
        assert_eq!(
            mock.expand_placeholder("${TEMP}/codex-session"),
            Some(dir.path().join("temp/codex-session"))
        );
    }

    #[test]
    fn expands_legacy_tilde_and_tmpdir() {
        let dir = tempdir().unwrap();
        let mock = MockPlatformPaths {
            home: dir.path().join("home"),
            local_appdata: dir.path().join("local"),
            roaming_appdata: dir.path().join("roaming"),
            temp: dir.path().join("temp"),
        };

        assert_eq!(
            mock.expand_placeholder("~/.npm"),
            Some(dir.path().join("home/.npm"))
        );
        assert_eq!(
            mock.expand_placeholder("$TMPDIR"),
            Some(dir.path().join("temp"))
        );
    }

    #[test]
    fn rejects_unauthorized_and_arbitrary_placeholders() {
        let dir = tempdir().unwrap();
        let mock = MockPlatformPaths {
            home: dir.path().join("home"),
            local_appdata: dir.path().join("local"),
            roaming_appdata: dir.path().join("roaming"),
            temp: dir.path().join("temp"),
        };

        assert_eq!(mock.expand_placeholder("${SECRET_KEY}"), None);
        assert_eq!(mock.expand_placeholder("${AWS_CREDENTIALS}"), None);
        assert_eq!(mock.expand_placeholder(""), None);
        assert_eq!(mock.expand_placeholder("relative/path/not/allowed"), None);
    }
}
