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

        if let Some(ref h) = home {
            if path == h {
                return true;
            }
        }

        // 2. Universal Git protection (.git is forbidden anywhere)
        for component in path.components() {
            if let std::path::Component::Normal(os_str) = component {
                if os_str == ".git" {
                    return true;
                }
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

        // 5. System critical prefixes outside user home and temp
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
            if path == Path::new(sys) || path.starts_with(sys) {
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
