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

    /// Strips Windows verbatim prefix (`\\?\` or `\\?\UNC\server\share` -> `\\server\share`).
    #[cfg(target_os = "windows")]
    pub fn normalize_verbatim_path(path: &Path) -> PathBuf {
        use std::ffi::OsString;
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        let wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        let slash = b'\\' as u16;
        let question = b'?' as u16;
        let verbatim_prefix = [slash, slash, question, slash];
        if !wide.starts_with(&verbatim_prefix) {
            return path.to_path_buf();
        }

        let unc_prefix = [
            slash,
            slash,
            question,
            slash,
            b'U' as u16,
            b'N' as u16,
            b'C' as u16,
            slash,
        ];
        if wide.len() >= unc_prefix.len()
            && wide[..unc_prefix.len()]
                .iter()
                .zip(unc_prefix)
                .all(|(actual, expected)| windows_ascii_eq(*actual, expected))
        {
            let mut normalized = vec![slash, slash];
            normalized.extend_from_slice(&wide[unc_prefix.len()..]);
            return PathBuf::from(OsString::from_wide(&normalized));
        }

        let remainder = &wide[verbatim_prefix.len()..];
        if remainder.len() >= 3
            && is_windows_drive_letter(remainder[0])
            && remainder[1] == b':' as u16
            && matches!(remainder[2], value if value == b'\\' as u16 || value == b'/' as u16)
        {
            return PathBuf::from(OsString::from_wide(remainder));
        }

        path.to_path_buf()
    }

    /// Strips Windows verbatim prefix when running on non-Windows (e.g. unit tests).
    #[cfg(not(target_os = "windows"))]
    pub fn normalize_verbatim_path(path: &Path) -> PathBuf {
        let path_str = path.to_string_lossy();
        if let Some(rest) = path_str.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{}", rest));
        }
        if let Some(rest) = path_str.strip_prefix(r"\\?\") {
            if rest.len() >= 2 && rest.as_bytes()[1] == b':' {
                return PathBuf::from(rest);
            }
        }
        path.to_path_buf()
    }

    /// Prepares a UTF-16 wide representation of a path with verbatim prefix if needed.
    #[cfg(target_os = "windows")]
    pub fn to_verbatim_wide(path: &Path) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        let path_text = path.to_string_lossy();
        if path_text.starts_with(r"\\?\") {
            path.as_os_str().encode_wide().chain([0]).collect()
        } else if let Some(unc_path) = path_text.strip_prefix(r"\\") {
            format!(r"\\?\UNC\{}", unc_path)
                .encode_utf16()
                .chain([0])
                .collect()
        } else if path.as_os_str().encode_wide().count() > 240 {
            format!(r"\\?\{}", path.display())
                .encode_utf16()
                .chain([0])
                .collect()
        } else {
            path.as_os_str().encode_wide().chain([0]).collect()
        }
    }

    /// Prepares a UTF-16 wide representation of a path (stub for non-Windows).
    #[cfg(not(target_os = "windows"))]
    pub fn to_verbatim_wide(path: &Path) -> Vec<u16> {
        let path_str = path.to_string_lossy();
        path_str.encode_utf16().chain([0]).collect()
    }

    /// Returns shared toolchain and package manager installation roots.
    pub fn tool_roots() -> Vec<PathBuf> {
        #[cfg(target_os = "windows")]
        {
            let mut roots = Vec::new();
            for var in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
                if let Some(val) = std::env::var_os(var).map(PathBuf::from) {
                    roots.push(val.clone());
                    roots.push(val.join("Docker\\Docker\\resources\\bin"));
                    roots.push(val.join("Git\\cmd"));
                    roots.push(val.join("Git\\bin"));
                    roots.push(val.join("nodejs"));
                }
            }
            if let Some(program_data) = std::env::var_os("ProgramData").map(PathBuf::from) {
                roots.push(program_data.clone());
                roots.push(program_data.join("chocolatey\\bin"));
                roots.push(program_data.join("scoop\\shims"));
            }
            if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
                roots.push(local.clone());
                roots.push(local.join("Programs"));
                roots.push(local.join("Programs\\Ollama"));
                roots.push(local.join("Programs\\Python\\Launcher"));
                roots.push(local.join("Volta\\bin"));
                roots.push(local.join("Microsoft\\WinGet\\Links"));
                roots.push(local.join("npm"));
                roots.push(local.join("pnpm"));
            }
            if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
                roots.push(roaming.clone());
                roots.push(roaming.join("npm"));
                roots.push(roaming.join("pnpm"));
                roots.push(roaming.join("nodejs"));
                roots.push(roaming.join("nvm"));
            }
            for env_var in [
                "PNPM_HOME",
                "VOLTA_HOME",
                "FNM_DIR",
                "NVM_HOME",
                "NVM_SYMLINK",
                "CARGO_HOME",
            ] {
                if let Some(val) = std::env::var_os(env_var).map(PathBuf::from) {
                    roots.push(val);
                }
            }
            if let Some(profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
                roots.push(profile.join(".cargo\\bin"));
                roots.push(profile.join("AppData\\Roaming\\npm"));
                roots.push(profile.join("AppData\\Local\\pnpm"));
                roots.push(profile.join("scoop\\shims"));
                roots.push(profile.join("scoop\\apps"));
                roots.push(profile.join(".gemini\\antigravity-cli\\bin"));
            }
            roots.retain(|p| p.is_absolute());
            roots.sort();
            roots.dedup();
            roots
        }
        #[cfg(target_os = "macos")]
        {
            let mut roots = vec![
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/Applications"),
                PathBuf::from("/Applications/Docker.app/Contents/Resources/bin"),
                PathBuf::from("/Applications/Ollama.app/Contents/Resources"),
            ];
            if let Some(home) = NativePlatformPaths::new().home() {
                roots.extend([
                    home.join(".local/bin"),
                    home.join(".cargo/bin"),
                    home.join(".npm-global/bin"),
                    home.join(".volta/bin"),
                    home.join(".asdf/shims"),
                    home.join("Library/pnpm"),
                ]);
            }
            roots.retain(|p| p.is_absolute());
            roots.sort();
            roots.dedup();
            roots
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            vec![PathBuf::from("/usr/bin"), PathBuf::from("/usr/local/bin")]
        }
    }

    /// Case-folds a Windows path into a canonical key.
    pub fn windows_path_key(path: &Path) -> String {
        Self::normalize_verbatim_path(path)
            .to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_uppercase()
    }

    /// Compares two Windows paths case-insensitively.
    pub fn windows_path_eq(left: &Path, right: &Path) -> bool {
        Self::windows_path_key(left) == Self::windows_path_key(right)
    }

    /// Compares two paths with the platform's case rules.
    pub fn paths_equal(left: &Path, right: &Path) -> bool {
        #[cfg(target_os = "windows")]
        {
            Self::windows_path_eq(left, right)
        }
        #[cfg(not(target_os = "windows"))]
        {
            left == right
        }
    }

    /// Checks if a candidate path starts with a base path case-insensitively.
    pub fn windows_path_starts_with(path: &Path, base: &Path) -> bool {
        let path_key = Self::windows_path_key(path);
        let base_key = Self::windows_path_key(base);
        if path_key == base_key {
            return true;
        }
        path_key
            .strip_prefix(&base_key)
            .is_some_and(|suffix| suffix.starts_with('/'))
    }
}

#[cfg(target_os = "windows")]
fn is_windows_drive_letter(value: u16) -> bool {
    (value >= b'A' as u16 && value <= b'Z' as u16) || (value >= b'a' as u16 && value <= b'z' as u16)
}

#[cfg(target_os = "windows")]
fn windows_ascii_eq(actual: u16, expected_uppercase: u16) -> bool {
    actual == expected_uppercase
        || (expected_uppercase >= b'A' as u16
            && expected_uppercase <= b'Z' as u16
            && actual == expected_uppercase + (b'a' - b'A') as u16)
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
    fn normalizes_verbatim_drive_and_unc_paths() {
        assert_eq!(
            NativePlatformPaths::normalize_verbatim_path(Path::new(
                r"\\?\C:\Users\tester\file.txt"
            )),
            PathBuf::from(r"C:\Users\tester\file.txt")
        );
        assert_eq!(
            NativePlatformPaths::normalize_verbatim_path(Path::new(
                r"\\?\UNC\server\share\file.txt"
            )),
            PathBuf::from(r"\\server\share\file.txt")
        );
        assert_eq!(
            NativePlatformPaths::normalize_verbatim_path(Path::new(r"C:\Users\tester")),
            PathBuf::from(r"C:\Users\tester")
        );
    }

    #[test]
    fn windows_path_comparison_folds_case_and_verbatim_prefix() {
        let verbatim = Path::new(r"\\?\C:\Users\홍 길동\AppData\Local\npm-cache");
        let plain = Path::new(r"c:\users\홍 길동\appdata\local\NPM-CACHE");
        assert!(NativePlatformPaths::windows_path_eq(verbatim, plain));
        assert!(NativePlatformPaths::windows_path_starts_with(
            plain,
            Path::new(r"C:\Users\홍 길동\AppData\Local")
        ));
        assert!(!NativePlatformPaths::windows_path_starts_with(
            Path::new(r"C:\Users\tester\AppData\LocalBackup"),
            Path::new(r"C:\Users\tester\AppData\Local")
        ));
        // Non-ASCII case folding uses the Unicode uppercase mapping rather
        // than ASCII-only folding.
        assert!(NativePlatformPaths::windows_path_eq(
            Path::new(r"C:\Пользователи\Х"),
            Path::new(r"c:\пользователи\х")
        ));
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
