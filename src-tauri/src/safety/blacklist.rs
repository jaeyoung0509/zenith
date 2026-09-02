use crate::models::ZenithError;
use std::path::{Path, PathBuf};

pub struct Blacklist;

impl Blacklist {
    /// System and user directory paths that must NEVER be deleted under any circumstances.
    pub fn is_blacklisted(path: &Path) -> bool {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from);

        // 1. Exact forbidden root & home
        if path == Path::new("/") {
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
            if path == h {
                return true;
            }
            // Whole AppData itself or Local/Roaming themselves
            if path == h.join("AppData")
                || path == h.join("AppData/Local")
                || path == h.join("AppData\\Local")
                || path == h.join("AppData/Roaming")
                || path == h.join("AppData\\Roaming")
            {
                return true;
            }
        }

        // Whole temp dir itself
        if path == std::env::temp_dir() {
            return true;
        }

        // 2. Universal Git protection, ADS, and Windows alias defense
        let path_str = path.to_string_lossy();
        for part in path_str.split(['/', '\\']) {
            if part == ".git" {
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
            if path.starts_with(h) {
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
                    if path == sensitive_path || path.starts_with(&sensitive_path) {
                        return true;
                    }
                }

                return false;
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
            if path == Path::new("/tmp")
                || path == Path::new("/private/tmp")
                || path == Path::new("/var/folders")
                || path == Path::new("/private/var/folders")
            {
                return true;
            }
            return false;
        }

        // 5. Exact users root directory
        let normalized_path_str = path.to_string_lossy().replace('\\', "/");
        if normalized_path_str == "C:/Users" || path == Path::new("/Users") {
            return true;
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
            "C:/Windows",
            "C:/Program Files",
            "C:/Program Files (x86)",
            "C:/ProgramData",
        ];

        for sys in &system_prefixes {
            let sys_path = Path::new(sys);
            if path == sys_path
                || path.starts_with(sys_path)
                || normalized_path_str == *sys
                || normalized_path_str.starts_with(&format!("{sys}/"))
            {
                return true;
            }
        }

        false
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
}
