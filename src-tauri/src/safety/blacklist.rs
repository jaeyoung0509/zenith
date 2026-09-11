use crate::models::ZenithError;
use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
use std::ffi::OsString;
#[cfg(target_os = "windows")]
use std::os::windows::ffi::{OsStrExt, OsStringExt};

pub struct Blacklist;

impl Blacklist {
    /// System and user directory paths that must NEVER be deleted under any circumstances.
    pub fn is_blacklisted(path: &Path) -> bool {
        #[cfg(target_os = "windows")]
        if Self::has_unsupported_windows_namespace(path) {
            return true;
        }

        // Windows canonicalization returns verbatim paths (`\\?\C:\...`).
        // Normalize them before drive-root, home, system-prefix, and ADS checks
        // so the prefix itself is never mistaken for an alternate data stream.
        #[cfg(target_os = "windows")]
        let normalized_path = Self::normalize_path(path);
        #[cfg(target_os = "windows")]
        let path = normalized_path.as_path();
        let home = crate::platform::NativePlatformPaths::new().home();
        #[cfg(target_os = "windows")]
        let home = home.map(|value| Self::normalize_path(&value));

        // 1. Exact forbidden root & home
        if Self::paths_equal(path, Path::new("/")) {
            return true;
        }

        // Drive roots like C:\ or D:\
        if let Some(path_str) = path.to_str() {
            let trimmed = path_str.trim_end_matches(['\\', '/']);
            if trimmed.len() == 2 && trimmed.ends_with(':') {
                return true;
            }
        }

        if let Some(ref h) = home {
            if Self::paths_equal(path, h) {
                return true;
            }
            // Whole AppData itself or Local/Roaming themselves
            if Self::paths_equal(path, &h.join("AppData"))
                || Self::paths_equal(path, &h.join("AppData/Local"))
                || Self::paths_equal(path, &h.join("AppData\\Local"))
                || Self::paths_equal(path, &h.join("AppData/Roaming"))
                || Self::paths_equal(path, &h.join("AppData\\Roaming"))
            {
                return true;
            }
        }

        // Whole temp dir itself
        let temp = std::env::temp_dir();
        #[cfg(target_os = "windows")]
        let temp = Self::normalize_path(&temp);
        if Self::paths_equal(path, &temp) {
            return true;
        }

        // 2. Universal Git protection, ADS, and Windows alias defense
        let path_str = path.to_string_lossy();
        for part in path_str.split(['/', '\\']) {
            if part == ".git" || cfg!(windows) && part.eq_ignore_ascii_case(".git") {
                return true;
            }
            if part.ends_with('.') || part.ends_with(' ') {
                return true;
            }
        }

        // Reject alternate data streams (e.g. file.txt:stream or C:\path\file.txt:stream)
        if let Some(colon_pos) = path_str.rfind(':') {
            if colon_pos != 1 {
                return true;
            }
        }

        // 3. User sensitive directories (credentials, keychains, user content)
        if let Some(ref h) = home {
            if Self::path_starts_with(path, h) {
                let sensitive_relative = [
                    ".ssh",
                    ".gnupg",
                    ".aws",
                    ".azure",
                    ".kube",
                    ".config/gcloud",
                    "Library/Keychains",
                    "Library/Accounts",
                    "Library/Mail",
                    "Library/Messages",
                    "Library/IdentityServices",
                    "Library/Containers/com.apple.mail",
                    "Desktop",
                    "Documents",
                    "Pictures",
                    "Movies",
                    "Music",
                    "Videos",
                    "Contacts",
                    "Searches",
                    "Links",
                    "Saved Games",
                ];

                for rel in &sensitive_relative {
                    let sensitive_path = h.join(rel);
                    if Self::paths_equal(path, &sensitive_path)
                        || Self::path_starts_with(path, &sensitive_path)
                    {
                        return true;
                    }
                }

                return false;
            }
        }

        // 3b. Dynamically resolved known folders (e.g. OneDrive Known Folder Move, redirected Documents/Desktop)
        let platform_paths = crate::platform::NativePlatformPaths::new();
        for folder_token in ["downloads", "desktop", "documents", "movies"] {
            if let Some(known_dir) = platform_paths.content_dir(folder_token) {
                let norm_known =
                    crate::platform::NativePlatformPaths::normalize_verbatim_path(&known_dir);
                if Self::paths_equal(path, &norm_known) || Self::path_starts_with(path, &norm_known)
                {
                    return true;
                }
            }
        }

        // 4. Allow safe temp directories (/tmp, /private/tmp, /var/folders, /private/var/folders)
        let path_str = path.to_string_lossy();
        if path_str.starts_with("/var/folders")
            || path_str.starts_with("/private/var/folders")
            || path_str.starts_with("/tmp")
            || path_str.starts_with("/private/tmp")
        {
            // Protect root temp folders themselves from direct deletion
            if Self::paths_equal(path, Path::new("/tmp"))
                || Self::paths_equal(path, Path::new("/private/tmp"))
                || Self::paths_equal(path, Path::new("/var/folders"))
                || Self::paths_equal(path, Path::new("/private/var/folders"))
            {
                return true;
            }
            return false;
        }

        // 5. Exact users root directory
        let normalized_path_str = path.to_string_lossy().replace('\\', "/");
        if normalized_path_str.eq_ignore_ascii_case("C:/Users")
            || Self::paths_equal(path, Path::new("/Users"))
        {
            return true;
        }

        // Drive-agnostic protection for Windows system and users directories on any drive letter
        let path_normalized_str = normalized_path_str.trim_end_matches('/');
        if path_normalized_str.len() >= 2 && path_normalized_str.as_bytes()[1] == b':' {
            let tail = &path_normalized_str[2..];
            for denied in [
                "/Users",
                "/Windows",
                "/Program Files",
                "/Program Files (x86)",
                "/ProgramData",
            ] {
                if tail.eq_ignore_ascii_case(denied)
                    || tail
                        .to_ascii_lowercase()
                        .starts_with(&format!("{}/", denied.to_ascii_lowercase()))
                {
                    return true;
                }
            }
        }

        // Environment-derived system roots
        for env_var in [
            "SystemRoot",
            "windir",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramW6432",
            "ProgramData",
        ] {
            if let Some(val) = std::env::var_os(env_var).map(PathBuf::from) {
                let norm_sys = crate::platform::NativePlatformPaths::normalize_verbatim_path(&val);
                if Self::paths_equal(path, &norm_sys) || Self::path_starts_with(path, &norm_sys) {
                    return true;
                }
            }
        }

        // 6. System critical prefixes outside user home and temp
        let system_prefixes = [
            "/System",
            "/bin",
            "/sbin",
            "/usr",
            "/etc",
            "/var",
            "/private",
            "/Applications",
            "/Library",
            "/Network",
            "/dev",
            "/cores",
            "/opt",
        ];

        for sys in &system_prefixes {
            let sys_path = Path::new(sys);
            let normalized_system = sys.to_ascii_lowercase();
            let normalized_candidate = normalized_path_str.to_ascii_lowercase();
            if Self::paths_equal(path, sys_path)
                || Self::path_starts_with(path, sys_path)
                || normalized_candidate == normalized_system
                || normalized_candidate.starts_with(&format!("{normalized_system}/"))
            {
                return true;
            }
        }

        false
    }

    fn paths_equal(left: &Path, right: &Path) -> bool {
        #[cfg(target_os = "windows")]
        {
            crate::platform::NativePlatformPaths::windows_path_eq(left, right)
        }
        #[cfg(not(target_os = "windows"))]
        {
            left == right
        }
    }

    fn path_starts_with(path: &Path, base: &Path) -> bool {
        #[cfg(target_os = "windows")]
        {
            crate::platform::NativePlatformPaths::windows_path_starts_with(path, base)
        }
        #[cfg(not(target_os = "windows"))]
        {
            path.starts_with(base)
        }
    }

    #[cfg(target_os = "windows")]
    fn windows_path_key(path: &Path) -> String {
        crate::platform::NativePlatformPaths::windows_path_key(path)
    }

    /// Verifies that a target path is completely safe from the blacklist.
    pub fn validate(path: &Path) -> Result<(), ZenithError> {
        // Resolve parent components to catch ../ attacks
        let normalized = Self::normalize_path(path);

        if Self::is_blacklisted(&normalized) {
            return Err(ZenithError::BlacklistedPath(
                path.to_string_lossy().to_string(),
            ));
        }

        Ok(())
    }

    /// Normalizes path without following symlinks (prevents path traversal `..`)
    pub fn normalize_path(path: &Path) -> PathBuf {
        #[cfg(target_os = "windows")]
        let platform_path = crate::platform::NativePlatformPaths::normalize_verbatim_path(path);
        #[cfg(target_os = "windows")]
        let path = platform_path.as_path();

        let mut components = Vec::new();
        for comp in path.components() {
            match comp {
                std::path::Component::Prefix(p) => components.push(std::path::Component::Prefix(p)),
                std::path::Component::RootDir => components.push(std::path::Component::RootDir),
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    if let Some(last) = components.last() {
                        if !matches!(
                            last,
                            std::path::Component::RootDir | std::path::Component::Prefix(_)
                        ) {
                            components.pop();
                        }
                    }
                }
                std::path::Component::Normal(n) => components.push(std::path::Component::Normal(n)),
            }
        }
        components.into_iter().collect()
    }

    #[cfg(target_os = "windows")]
    fn has_unsupported_windows_namespace(path: &Path) -> bool {
        let wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        let slash = b'\\' as u16;
        let verbatim_prefix = [slash, slash, b'?' as u16, slash];
        let device_prefix = [slash, slash, b'.' as u16, slash];
        let nt_prefix = [slash, b'?' as u16, b'?' as u16, slash];

        if wide.starts_with(&device_prefix) || wide.starts_with(&nt_prefix) {
            return true;
        }
        if !wide.starts_with(&verbatim_prefix) {
            return false;
        }

        let remainder = &wide[verbatim_prefix.len()..];
        let is_drive = remainder.len() >= 3
            && Self::is_windows_drive_letter(remainder[0])
            && remainder[1] == b':' as u16
            && matches!(remainder[2], value if value == slash || value == b'/' as u16);
        let is_unc = remainder.len() >= 4
            && remainder[..4]
                .iter()
                .zip([b'U' as u16, b'N' as u16, b'C' as u16, slash])
                .all(|(actual, expected)| Self::windows_ascii_eq(*actual, expected));

        !is_drive && !is_unc
    }

    #[cfg(target_os = "windows")]
    fn is_windows_drive_letter(value: u16) -> bool {
        (value >= b'A' as u16 && value <= b'Z' as u16)
            || (value >= b'a' as u16 && value <= b'z' as u16)
    }

    #[cfg(target_os = "windows")]
    fn windows_ascii_eq(actual: u16, expected_uppercase: u16) -> bool {
        actual == expected_uppercase
            || (expected_uppercase >= b'A' as u16
                && expected_uppercase <= b'Z' as u16
                && actual == expected_uppercase + (b'a' - b'A') as u16)
    }
}
