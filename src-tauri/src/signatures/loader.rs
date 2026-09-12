use crate::models::{Signature, SignatureManifest, ZenithError};
use crate::platform::path_algebra::PathFlavor;
use crate::platform::{KnownFolder, PlatformEnvironment};
use std::fs;
use std::path::{Path, PathBuf};

pub struct SignatureLoader;

impl SignatureLoader {
    /// Loads a signature manifest from a TOML file on disk.
    pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Vec<Signature>, ZenithError> {
        let content = fs::read_to_string(path.as_ref()).map_err(|e| {
            ZenithError::Io(format!("Failed to read {}: {}", path.as_ref().display(), e))
        })?;
        Self::load_str(&content)
    }

    /// Loads signatures from a TOML string.
    pub fn load_str(content: &str) -> Result<Vec<Signature>, ZenithError> {
        let manifest: SignatureManifest = toml::from_str(content)
            .map_err(|e| ZenithError::Io(format!("Failed to parse TOML signature: {}", e)))?;

        let mut valid_signatures = Vec::new();
        for sig in manifest.signatures {
            if sig.id.trim().is_empty() {
                continue;
            }
            valid_signatures.push(sig);
        }

        Ok(valid_signatures)
    }

    /// Expands platform placeholders (`${USER_HOME}`, `${LOCAL_APP_DATA}`, `~`, `$TMPDIR`)
    /// safely against the described environment.
    ///
    /// A user-content folder is owned by the platform, not by its literal
    /// profile spelling: Known Folder Move and administrator redirection move
    /// the real folder without leaving a trace in `$HOME`, so a resolved known
    /// folder (`~/Downloads`, `${USER_HOME}\Documents`) is expanded through the
    /// environment's own resolution instead of `home.join(name)`.
    ///
    /// This is the single resolution boundary for signature paths: the scanner,
    /// the plan verifier, and the cache providers all expand through it, so a
    /// simulated environment resolves exactly the paths it states.
    pub fn expand_path(pattern: &str, environment: &PlatformEnvironment) -> Option<PathBuf> {
        match Self::substitute_known_folder(pattern, environment) {
            Some(substituted) => environment.expand_placeholder(&substituted),
            None => environment.expand_placeholder(pattern),
        }
    }

    /// Rewrites a user-content folder pattern (`~/Documents/x`) to the
    /// environment's resolved folder when it states one. `None` when the
    /// pattern does not name a known folder or nothing is stated for it, so the
    /// caller falls back to the placeholder expansion.
    fn substitute_known_folder(pattern: &str, environment: &PlatformEnvironment) -> Option<String> {
        let (folder, tail) = Self::split_known_folder(pattern, environment.flavor())?;
        let root = environment.known_folder(folder)?;
        if tail.is_empty() {
            return Some(root.to_string_lossy().into_owned());
        }
        Some(root.join(tail).to_string_lossy().into_owned())
    }

    fn split_known_folder(pattern: &str, flavor: PathFlavor) -> Option<(KnownFolder, String)> {
        let trimmed = pattern.trim();
        let rest = trimmed
            .strip_prefix("~/")
            .or_else(|| trimmed.strip_prefix("${USER_HOME}/"))
            .or_else(|| trimmed.strip_prefix("${USER_HOME}\\"))?;
        let (first, tail) = match rest.split_once(['/', '\\']) {
            Some((first, tail)) => (first, Some(tail)),
            None => (rest, None),
        };
        let folder = known_folder_component(first, flavor)?;
        Some((folder, tail.unwrap_or_default().replace('\\', "/")))
    }
}

/// Maps a profile-relative component to a known folder. POSIX folder names are
/// case sensitive (`~/downloads` is not `~/Downloads`); Windows resolves the
/// shell folder case-insensitively.
fn known_folder_component(component: &str, flavor: PathFlavor) -> Option<KnownFolder> {
    KnownFolder::ALL.into_iter().find(|folder| {
        let token = folder.token();
        if flavor.is_windows() {
            return component.eq_ignore_ascii_case(token);
        }
        let mut name = token.to_string();
        if let Some(first) = name.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        component == name
    })
}

#[cfg(test)]
mod tests {
    use super::SignatureLoader;
    use crate::platform::path_algebra::PathFlavor;
    use crate::platform::paths::SimulatedPaths;
    use crate::platform::{KnownFolder, PlatformEnvironment};
    use std::path::PathBuf;
    use std::sync::Arc;

    #[test]
    fn expand_path_preserves_absolute_paths_without_home_lookup() {
        let abs = if cfg!(windows) {
            r"C:\test\folder"
        } else {
            "/tmp/test_folder"
        };
        let environment = PlatformEnvironment::simulated(PathFlavor::current());
        let expanded = SignatureLoader::expand_path(abs, &environment);
        assert_eq!(expanded, Some(PathBuf::from(abs)));
    }

    #[test]
    fn expand_path_expands_the_stated_temp_directory() {
        let temp = if cfg!(windows) {
            PathBuf::from(r"D:\Temp\zenith")
        } else {
            PathBuf::from("/tmp/zenith-stated")
        };
        let flavor = PathFlavor::current();
        let environment = PlatformEnvironment::simulated(flavor).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(flavor)
                .with_temp_dir(&temp),
        ));

        assert_eq!(
            SignatureLoader::expand_path("$TMPDIR", &environment),
            Some(temp.clone())
        );
        assert_eq!(
            SignatureLoader::expand_path("${TEMP}", &environment),
            Some(temp.clone())
        );
        assert_eq!(
            SignatureLoader::expand_path("${TEMP}/nested", &environment),
            Some(temp.join("nested"))
        );
    }

    #[test]
    fn expand_path_prefers_the_redirected_known_folder_over_the_profile_spelling() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Windows)
            .with_home(r"C:\Users\me")
            .with_known_folder(KnownFolder::Downloads, r"D:\Cache\Downloads")
            .with_known_folder(KnownFolder::Documents, r"\\fileserver\home\docs");

        assert_eq!(
            SignatureLoader::expand_path("~/Downloads/uv", &environment),
            Some(PathBuf::from(r"D:\Cache\Downloads\uv"))
        );
        assert_eq!(
            SignatureLoader::expand_path("${USER_HOME}/Downloads", &environment),
            Some(PathBuf::from(r"D:\Cache\Downloads"))
        );
        assert_eq!(
            SignatureLoader::expand_path(r"${USER_HOME}\Documents\cache", &environment),
            Some(PathBuf::from(r"\\fileserver\home\docs\cache"))
        );

        // A folder the environment does not state still resolves through the
        // stated profile: the redirect is only authoritative when it is known.
        assert_eq!(
            SignatureLoader::expand_path("~/Movies", &environment),
            Some(PathBuf::from(r"C:\Users\me\Movies"))
        );
    }

    #[test]
    fn a_posix_known_folder_component_stays_case_sensitive() {
        let environment = PlatformEnvironment::simulated(PathFlavor::Posix)
            .with_home("/home/tester")
            .with_known_folder(KnownFolder::Downloads, "/mnt/data/downloads");

        assert_eq!(
            SignatureLoader::expand_path("~/Downloads/cache", &environment),
            Some(PathBuf::from("/mnt/data/downloads/cache"))
        );
        // `~/downloads` names a different directory on POSIX, so it is not the
        // known folder and resolves through the literal profile spelling.
        assert_eq!(
            SignatureLoader::expand_path("~/downloads/cache", &environment),
            Some(PathBuf::from("/home/tester/downloads/cache"))
        );
    }
}
