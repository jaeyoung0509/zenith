use crate::platform::path_algebra::PathFlavor;
use std::path::{Path, PathBuf};

/// Platform-owned path resolution interface for user directories and reviewed system roots.
pub trait PlatformPathsProvider: Send + Sync {
    fn user_home(&self) -> Option<PathBuf>;
    fn local_app_data(&self) -> Option<PathBuf>;
    fn roaming_app_data(&self) -> Option<PathBuf>;
    fn temp_dir(&self) -> PathBuf;
    fn program_files(&self) -> Option<PathBuf>;
    fn program_data(&self) -> Option<PathBuf>;

    /// Resolves a user-content folder token.
    ///
    /// The literal profile path is only a fallback: Known Folder Move,
    /// administrator redirection, and UNC profiles all move the real folder
    /// without leaving a trace in `$HOME`. Platform implementations that can
    /// ask the operating system must do so.
    fn content_dir(&self, token: &str) -> Option<PathBuf> {
        let name = match token {
            "downloads" => "Downloads",
            "desktop" => "Desktop",
            "documents" => "Documents",
            "movies" => "Movies",
            _ => return None,
        };
        self.user_home().map(|home| home.join(name))
    }

    /// Expands allowlisted placeholders:
    /// - `${USER_HOME}` or `~`
    /// - `${LOCAL_APP_DATA}`
    /// - `${ROAMING_APP_DATA}`
    /// - `${TEMP}` or `$TMPDIR`
    /// - `${PROGRAM_FILES}`
    /// - `${PROGRAM_DATA}`
    ///
    /// Rejects arbitrary environment variables, empty roots, and broad filesystem roots.
    ///
    /// The result is normalized with the environment's own [`PathFlavor`] and
    /// not with the host's `Path` rules, so a simulated Windows environment
    /// produces Windows-shaped absolute paths on any runner.
    fn expand_placeholder(&self, pattern: &str) -> Option<PathBuf> {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return None;
        }
        let flavor = self.flavor();

        // The host's own `Path::join` is used only when the described
        // environment *is* the host's, which keeps POSIX byte-exactness. A
        // simulated environment joins with its own separator rules instead, so
        // a POSIX root stays POSIX on a Windows runner.
        let joined = |base: PathBuf, tail: &str| join_with_flavor(base, tail, flavor);

        let raw_path = if pattern == "$TMPDIR" || pattern == "${TEMP}" {
            self.temp_dir()
        } else if let Some(rest) = pattern
            .strip_prefix("${TEMP}/")
            .or_else(|| pattern.strip_prefix("${TEMP}\\"))
            .or_else(|| pattern.strip_prefix("$TMPDIR/"))
        {
            joined(self.temp_dir(), rest)
        } else if let Some(rest) = pattern
            .strip_prefix("${USER_HOME}/")
            .or_else(|| pattern.strip_prefix("${USER_HOME}\\"))
        {
            joined(self.user_home()?, rest)
        } else if pattern == "${USER_HOME}" || pattern == "~" {
            self.user_home()?
        } else if let Some(rest) = pattern
            .strip_prefix("~/")
            .or_else(|| pattern.strip_prefix("~\\"))
        {
            // `~\Documents` is the Windows spelling of the same profile
            // reference, and the built-in signatures may use either.
            joined(self.user_home()?, rest)
        } else if let Some(rest) = pattern
            .strip_prefix("${LOCAL_APP_DATA}/")
            .or_else(|| pattern.strip_prefix("${LOCAL_APP_DATA}\\"))
        {
            joined(self.local_app_data()?, rest)
        } else if pattern == "${LOCAL_APP_DATA}" {
            self.local_app_data()?
        } else if let Some(rest) = pattern
            .strip_prefix("${ROAMING_APP_DATA}/")
            .or_else(|| pattern.strip_prefix("${ROAMING_APP_DATA}\\"))
        {
            joined(self.roaming_app_data()?, rest)
        } else if pattern == "${ROAMING_APP_DATA}" {
            self.roaming_app_data()?
        } else if let Some(rest) = pattern
            .strip_prefix("${PROGRAM_FILES}/")
            .or_else(|| pattern.strip_prefix("${PROGRAM_FILES}\\"))
        {
            joined(self.program_files()?, rest)
        } else if pattern == "${PROGRAM_FILES}" {
            self.program_files()?
        } else if let Some(rest) = pattern
            .strip_prefix("${PROGRAM_DATA}/")
            .or_else(|| pattern.strip_prefix("${PROGRAM_DATA}\\"))
        {
            joined(self.program_data()?, rest)
        } else if pattern == "${PROGRAM_DATA}" {
            self.program_data()?
        } else if pattern.starts_with("${") {
            // Reject any unapproved arbitrary placeholder
            return None;
        } else {
            // A literal pattern must already be absolute in its own spelling.
            // Normalization cannot be trusted to decide this: `C:cache` is
            // drive-relative while `C:\cache` is rooted, and a normalizer that
            // inserted a separator would accept the relative form.
            if !crate::platform::path_algebra::is_absolute(pattern, self.flavor()) {
                return None;
            }
            PathBuf::from(pattern)
        };

        // The host's own `Path` rules apply only when the described environment
        // is the host's: that keeps POSIX normalization byte-exact. A simulated
        // environment is normalized by the algebra for its own flavor, so a
        // POSIX environment produces POSIX spellings on a Windows runner.
        let normalized = if flavor == crate::platform::path_algebra::PathFlavor::current() {
            crate::safety::Blacklist::normalize_path(&raw_path)
        } else {
            PathBuf::from(crate::platform::path_algebra::normalize(
                &raw_path.to_string_lossy(),
                flavor,
            ))
        };

        // Safety: the path must be absolute under its own flavor's rules and
        // must not be a broad root such as `C:\` or `/`.
        if flavor == crate::platform::path_algebra::PathFlavor::current() {
            if !normalized.is_absolute() || is_broad_root(&normalized) {
                return None;
            }
        } else {
            let text = normalized.to_string_lossy();
            if !crate::platform::path_algebra::is_absolute(&text, flavor) {
                return None;
            }
            if crate::platform::path_algebra::is_root(&text, flavor) {
                return None;
            }
        }

        Some(normalized)
    }

    /// Path flavor of the described environment. Implementations that simulate
    /// another platform override this; the default is the compiled target.
    fn flavor(&self) -> crate::platform::path_algebra::PathFlavor {
        crate::platform::path_algebra::PathFlavor::current()
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

    /// Returns directories searched for tools under `home`, the profile the
    /// caller is describing. Discovery convenience only: this list is not an
    /// execution trust boundary.
    pub fn tool_search_locations(home: Option<&Path>) -> Vec<PathBuf> {
        // Only the macOS branch consults the profile: the Windows branch reads
        // the machine's own Program Files and package-manager roots, and the
        // remaining hosts have fixed tool paths. One signature covers all three.
        let _ = home;
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
            if let Some(home) = home {
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

    /// Returns executable trust roots: only platform install locations Zenith
    /// is willing to execute. User-writable containers (`%LOCALAPPDATA%`,
    /// `%APPDATA%`, `%ProgramData%` themselves) are excluded; only their
    /// documented tool/package-manager children are trusted.
    pub fn trusted_tool_roots(home: Option<&Path>) -> Vec<PathBuf> {
        // Only the macOS branch consults the profile; see
        // [`Self::tool_search_locations`].
        let _ = home;
        #[cfg(target_os = "windows")]
        {
            let mut roots = Vec::new();
            // System install roots: administrator-writable only.
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
                // The ProgramData root itself is intentionally not trusted.
                roots.push(program_data.join("chocolatey\\bin"));
                roots.push(program_data.join("scoop\\shims"));
            }
            if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
                // The LOCALAPPDATA root itself is intentionally not trusted.
                roots.extend([
                    local.join("Programs"),
                    local.join("Programs\\Ollama"),
                    local.join("Programs\\Python\\Launcher"),
                    local.join("Volta\\bin"),
                    local.join("Microsoft\\WinGet\\Links"),
                    local.join("npm"),
                    local.join("pnpm"),
                    local.join("nvm"),
                ]);
            }
            if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
                // The APPDATA root itself is intentionally not trusted.
                roots.extend([
                    roaming.join("npm"),
                    roaming.join("pnpm"),
                    roaming.join("nodejs"),
                    roaming.join("nvm"),
                ]);
            }
            // Explicitly configured toolchain roots.
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
                roots.push(profile.join(".local\\bin"));
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
            Self::tool_search_locations(home)
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

    /// Only fixed user-content tokens may cross the IPC boundary. On Windows
    /// the shell's known-folder API is authoritative because Known Folder Move
    /// and redirection change the real location.
    fn content_dir(&self, token: &str) -> Option<PathBuf> {
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
            Some(self.user_home()?.join(name))
        }
    }

    fn program_files(&self) -> Option<PathBuf> {
        #[cfg(target_os = "windows")]
        {
            std::env::var_os("ProgramFiles")
                .map(PathBuf::from)
                .or_else(|| Some(windows_program_files()))
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
                .or_else(|| Some(windows_program_data()))
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

/// The administrator-writable install root a Windows machine resolves when the
/// environment does not state one.
///
/// The fallback is a fact of the *platform*, so it lives in the flavor's branch
/// instead of inside the `#[cfg(target_os = "windows")]` block that used to hold
/// the literal: a stated Windows environment is protected exactly like a real
/// one, and the value is asserted on every runner.
pub fn windows_program_files() -> PathBuf {
    PathBuf::from(r"C:\Program Files")
}

/// The shared application-data root a Windows machine resolves when the
/// environment does not state one.
pub fn windows_program_data() -> PathBuf {
    PathBuf::from(r"C:\ProgramData")
}

fn resolve_user_home(
    windows: bool,
    get: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    get(if windows { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && !is_broad_root(path))
}

/// Joins a placeholder tail onto a resolved root.
///
/// When the described environment is the host's, the host join is byte-exact
/// and is preferred. When it is a simulation of another platform, the join must
/// follow the described flavor instead of the host's separator rules.
fn join_with_flavor(base: PathBuf, tail: &str, flavor: PathFlavor) -> PathBuf {
    if flavor == PathFlavor::current() {
        return base.join(tail);
    }
    PathBuf::from(crate::platform::path_algebra::join(
        &base.to_string_lossy(),
        tail,
        flavor,
    ))
}

/// A [`PlatformPathsProvider`] whose roots the caller states.
///
/// Tests and the `--doctor` self-check use this to present an environment that
/// is not the host's — a non-`C:` system drive, a UNC profile, a redirected
/// app-data root — without reading the process environment. Nothing here
/// consults `std::env`, so a simulated environment cannot silently inherit a
/// fact the caller did not state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulatedPaths {
    flavor: PathFlavor,
    home: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
    roaming_app_data: Option<PathBuf>,
    temp_dir: Option<PathBuf>,
    program_files: Option<PathBuf>,
    program_data: Option<PathBuf>,
}

impl Default for SimulatedPaths {
    fn default() -> Self {
        Self {
            flavor: PathFlavor::current(),
            home: None,
            local_app_data: None,
            roaming_app_data: None,
            temp_dir: None,
            program_files: None,
            program_data: None,
        }
    }
}

impl SimulatedPaths {
    pub fn new() -> Self {
        Self::default()
    }

    /// States which platform's path rules the described roots follow.
    pub fn with_flavor(mut self, flavor: PathFlavor) -> Self {
        self.flavor = flavor;
        self
    }

    pub fn with_home(mut self, path: impl Into<PathBuf>) -> Self {
        self.home = Some(path.into());
        self
    }

    pub fn with_local_app_data(mut self, path: impl Into<PathBuf>) -> Self {
        self.local_app_data = Some(path.into());
        self
    }

    pub fn with_roaming_app_data(mut self, path: impl Into<PathBuf>) -> Self {
        self.roaming_app_data = Some(path.into());
        self
    }

    pub fn with_temp_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.temp_dir = Some(path.into());
        self
    }

    pub fn with_program_files(mut self, path: impl Into<PathBuf>) -> Self {
        self.program_files = Some(path.into());
        self
    }

    pub fn with_program_data(mut self, path: impl Into<PathBuf>) -> Self {
        self.program_data = Some(path.into());
        self
    }
}

impl PlatformPathsProvider for SimulatedPaths {
    fn user_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }

    fn local_app_data(&self) -> Option<PathBuf> {
        self.local_app_data.clone()
    }

    fn roaming_app_data(&self) -> Option<PathBuf> {
        self.roaming_app_data.clone()
    }

    fn temp_dir(&self) -> PathBuf {
        self.temp_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
    }

    fn program_files(&self) -> Option<PathBuf> {
        // POSIX protection comes from the blacklist's own system rules, while a
        // Windows machine's administrator-writable roots are environment facts,
        // so the platform default is what keeps a stated Windows machine
        // protected rather than a default that silently widens a POSIX one.
        self.program_files.clone().or_else(|| {
            self.flavor
                .is_windows()
                .then(crate::platform::paths::windows_program_files)
        })
    }

    fn program_data(&self) -> Option<PathBuf> {
        self.program_data.clone().or_else(|| {
            self.flavor
                .is_windows()
                .then(crate::platform::paths::windows_program_data)
        })
    }

    fn flavor(&self) -> PathFlavor {
        self.flavor
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

    /// A stated Windows machine keeps the platform's administrator-writable
    /// roots even when the caller states none, and a POSIX one never inherits
    /// them. Both directions are asserted on every runner.
    #[test]
    fn stated_platforms_resolve_their_own_install_roots() {
        let windows = SimulatedPaths::new().with_flavor(PathFlavor::Windows);
        assert_eq!(windows.program_files(), Some(windows_program_files()));
        assert_eq!(windows.program_data(), Some(windows_program_data()));

        // A stated root wins over the platform default: Known Folder Move and a
        // non-`C:` system drive both relocate these.
        let relocated = SimulatedPaths::new()
            .with_flavor(PathFlavor::Windows)
            .with_program_files(r"D:\Program Files")
            .with_program_data(r"D:\ProgramData");
        assert_eq!(
            relocated.program_files(),
            Some(PathBuf::from(r"D:\Program Files"))
        );
        assert_eq!(
            relocated.program_data(),
            Some(PathBuf::from(r"D:\ProgramData"))
        );

        let posix = SimulatedPaths::new().with_flavor(PathFlavor::Posix);
        assert_eq!(posix.program_files(), None);
        assert_eq!(posix.program_data(), None);
    }

    #[test]
    fn simulated_environment_expands_allowlisted_placeholders() {
        let dir = tempdir().unwrap();
        let environment = SimulatedPaths::new()
            .with_home(dir.path().join("home"))
            .with_local_app_data(dir.path().join("home/AppData/Local"))
            .with_roaming_app_data(dir.path().join("home/AppData/Roaming"))
            .with_temp_dir(dir.path().join("temp"));

        assert_eq!(
            environment.expand_placeholder("${USER_HOME}/.cargo/registry"),
            Some(dir.path().join("home/.cargo/registry"))
        );
        assert_eq!(
            environment.expand_placeholder("${LOCAL_APP_DATA}/Zenith/Cache"),
            Some(dir.path().join("home/AppData/Local/Zenith/Cache"))
        );
        assert_eq!(
            environment.expand_placeholder("${TEMP}/codex-session"),
            Some(dir.path().join("temp/codex-session"))
        );
    }

    #[test]
    fn a_redirected_windows_profile_resolves_placeholders_without_the_host() {
        // The same pure expansion logic must serve a machine whose system
        // drive is not `C:` — the fact is stated, not read from the host.
        let environment = SimulatedPaths::new()
            .with_flavor(PathFlavor::Windows)
            .with_home(r"D:\Users\홍 길동")
            .with_local_app_data(r"D:\Users\홍 길동\AppData\Local")
            .with_temp_dir(r"D:\Users\홍 길동\AppData\Local\Temp");

        assert_eq!(
            environment.expand_placeholder("${USER_HOME}/.cursor/extensions"),
            Some(PathBuf::from(r"D:\Users\홍 길동\.cursor\extensions"))
        );
        assert_eq!(
            environment.expand_placeholder("${LOCAL_APP_DATA}\\npm-cache"),
            Some(PathBuf::from(r"D:\Users\홍 길동\AppData\Local\npm-cache"))
        );
        assert_eq!(
            environment.expand_placeholder("${TEMP}"),
            Some(PathBuf::from(r"D:\Users\홍 길동\AppData\Local\Temp"))
        );
    }

    #[test]
    fn an_unstated_root_is_absent_rather_than_invented() {
        let environment = SimulatedPaths::new().with_home(r"D:\Users\me");

        assert_eq!(environment.program_files(), None);
        assert_eq!(environment.program_data(), None);
        assert_eq!(environment.expand_placeholder("${PROGRAM_FILES}/Git"), None);
        assert_eq!(
            environment.expand_placeholder("${PROGRAM_DATA}/scoop"),
            None
        );
        // An unstated home is also absent, so expansion fails closed.
        let empty = SimulatedPaths::new();
        assert_eq!(empty.user_home(), None);
        assert_eq!(empty.expand_placeholder("~/Downloads"), None);
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
        let environment = SimulatedPaths::new()
            .with_home(dir.path().join("home"))
            .with_temp_dir(dir.path().join("temp"));

        assert_eq!(
            environment.expand_placeholder("~/.npm"),
            Some(dir.path().join("home/.npm"))
        );
        assert_eq!(
            environment.expand_placeholder("$TMPDIR"),
            Some(dir.path().join("temp"))
        );
    }

    #[test]
    fn a_simulated_environment_never_leaks_the_hosts_separators() {
        // A POSIX environment simulated on a Windows runner used to normalize
        // through the host's `Path` and come back with backslashes, which then
        // stopped matching anything. Each case asserts the described flavor's
        // spelling, so the macOS and Windows runners check the two directions
        // of the same invariant.
        let cases = [
            (
                PathFlavor::Posix,
                "/home/tester",
                "Downloads/cache",
                "/home/tester/Downloads/cache",
                '\\',
            ),
            (
                PathFlavor::Windows,
                r"D:\Users\tester",
                r"Documents\cache",
                r"D:\Users\tester\Documents\cache",
                '/',
            ),
        ];
        for (flavor, home, tail, expected, foreign) in cases {
            let environment = SimulatedPaths::new().with_flavor(flavor).with_home(home);
            let expanded = environment
                .expand_placeholder(&format!("~/{tail}"))
                .unwrap_or_else(|| panic!("{flavor} environment did not expand ~/{tail}"));
            assert_eq!(expanded.to_string_lossy(), expected);
            assert!(
                !expanded.to_string_lossy().contains(foreign),
                "{flavor} expansion leaked the host separator: {expanded:?}"
            );
        }
    }

    #[test]
    fn a_literal_drive_relative_pattern_is_refused() {
        let environment = SimulatedPaths::new()
            .with_flavor(PathFlavor::Windows)
            .with_home(r"D:\Users\me");

        // `D:cache` means "relative to the current directory on D:" and must
        // not be accepted as an absolute cleanup target.
        assert_eq!(environment.expand_placeholder("D:cache"), None);
        assert_eq!(environment.expand_placeholder(r"D:..\Windows"), None);
        assert_eq!(environment.expand_placeholder("D:."), None);
        // The rooted spelling is still accepted.
        assert_eq!(
            environment.expand_placeholder(r"D:\cache"),
            Some(PathBuf::from(r"D:\cache"))
        );
    }

    #[test]
    fn rejects_unauthorized_and_arbitrary_placeholders() {
        let dir = tempdir().unwrap();
        let environment = SimulatedPaths::new()
            .with_home(dir.path().join("home"))
            .with_temp_dir(dir.path().join("temp"));

        assert_eq!(environment.expand_placeholder("${SECRET_KEY}"), None);
        assert_eq!(environment.expand_placeholder("${AWS_CREDENTIALS}"), None);
        assert_eq!(environment.expand_placeholder(""), None);
        assert_eq!(
            environment.expand_placeholder("relative/path/not/allowed"),
            None
        );
    }
}
