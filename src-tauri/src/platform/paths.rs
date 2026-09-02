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
    if path == Path::new("/") {
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
}

impl PlatformPathsProvider for NativePlatformPaths {
    fn user_home(&self) -> Option<PathBuf> {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
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
