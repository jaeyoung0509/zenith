use crate::models::{Signature, SignatureManifest, ZenithError};
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

    /// Expands platform placeholders (`${USER_HOME}`, `${LOCAL_APP_DATA}`, `~`, `$TMPDIR`) safely.
    pub fn expand_path(pattern: &str) -> Option<PathBuf> {
        use crate::platform::paths::{NativePlatformPaths, PlatformPathsProvider};
        NativePlatformPaths::new().expand_placeholder(pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::SignatureLoader;
    use std::path::PathBuf;

    #[test]
    fn expand_path_preserves_absolute_paths_without_home_lookup() {
        let abs = if cfg!(windows) {
            r"C:\test\folder"
        } else {
            "/tmp/test_folder"
        };
        let expanded = SignatureLoader::expand_path(abs);
        assert_eq!(expanded, Some(PathBuf::from(abs)));
    }

    #[test]
    fn expand_path_expands_tmpdir_variable() {
        let expanded = SignatureLoader::expand_path("$TMPDIR");
        assert!(expanded.is_some());
        assert_eq!(expanded.unwrap(), std::env::temp_dir());
    }
}
