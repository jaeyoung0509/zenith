use crate::models::{Signature, SignatureManifest, ZenithError};
use crate::safety::Blacklist;
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

    /// Expands `~` and the current user's `$TMPDIR` without reading arbitrary variables.
    pub fn expand_path(pattern: &str) -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        let path = if pattern == "$TMPDIR" {
            std::env::temp_dir()
        } else if let Some(relative) = pattern.strip_prefix("~/") {
            PathBuf::from(&home).join(relative)
        } else if pattern == "~" {
            PathBuf::from(&home)
        } else {
            PathBuf::from(pattern)
        };

        // Normalize path
        let normalized = Blacklist::normalize_path(&path);
        Some(normalized)
    }
}
