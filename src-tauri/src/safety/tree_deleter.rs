use crate::safety::{Blacklist, SymlinkGuard};
use crate::signatures::SignatureLoader;
use std::fs;
use std::io;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub struct SafeTreeDeleter;

impl SafeTreeDeleter {
    pub fn delete_contents(root: &Path, exclusions: &[String]) -> io::Result<u64> {
        if !root.exists() && !SymlinkGuard::is_symlink(root) {
            return Ok(0);
        }
        if !root.is_dir() || SymlinkGuard::is_symlink(root) {
            return Self::delete_entry(root, exclusions);
        }

        Blacklist::validate(root).map_err(io::Error::other)?;
        let mut reclaimed = 0;
        for entry in fs::read_dir(root)? {
            reclaimed += Self::delete_entry(&entry?.path(), exclusions)?;
        }
        Ok(reclaimed)
    }

    pub fn delete_path(root: &Path, exclusions: &[String]) -> io::Result<u64> {
        if !root.exists() && !SymlinkGuard::is_symlink(root) {
            return Ok(0);
        }
        Blacklist::validate(root).map_err(io::Error::other)?;
        Self::delete_entry(root, exclusions)
    }

    fn delete_entry(path: &Path, exclusions: &[String]) -> io::Result<u64> {
        if Self::is_excluded(path, exclusions) || Blacklist::is_blacklisted(path) {
            return Ok(0);
        }

        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || metadata.is_file() {
            let bytes = allocated_bytes(&metadata);
            fs::remove_file(path)?;
            return Ok(bytes);
        }
        if !metadata.is_dir() {
            return Ok(0);
        }

        let mut reclaimed = 0;
        for entry in fs::read_dir(path)? {
            reclaimed += Self::delete_entry(&entry?.path(), exclusions)?;
        }
        match fs::remove_dir(path) {
            Ok(()) => Ok(reclaimed),
            Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(reclaimed),
            Err(error) => Err(error),
        }
    }

    fn is_excluded(path: &Path, exclusions: &[String]) -> bool {
        exclusions.iter().any(|exclusion| {
            if (exclusion.starts_with('~') || exclusion.starts_with('/'))
                && SignatureLoader::expand_path(exclusion)
                    .is_some_and(|expanded| path == expanded || path.starts_with(expanded))
            {
                return true;
            }
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == exclusion)
        })
    }
}

fn allocated_bytes(metadata: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        let allocated = metadata.blocks().saturating_mul(512);
        if allocated == 0 && metadata.len() > 0 {
            metadata.len()
        } else {
            allocated
        }
    }
    #[cfg(not(unix))]
    {
        metadata.len()
    }
}
