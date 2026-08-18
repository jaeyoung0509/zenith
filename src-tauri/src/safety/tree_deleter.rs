use crate::safety::{Blacklist, SymlinkGuard};
use crate::signatures::SignatureLoader;
use std::fs;
use std::io;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeDeleteReport {
    pub reclaimed_bytes: u64,
    pub deleted_files: usize,
    pub skipped_files: usize,
    pub errors: Vec<String>,
}

impl TreeDeleteReport {
    pub fn is_success(&self) -> bool {
        self.errors.is_empty()
    }
}

pub struct SafeTreeDeleter;

impl SafeTreeDeleter {
    pub fn delete_contents(root: &Path, exclusions: &[String]) -> TreeDeleteReport {
        let mut report = TreeDeleteReport::default();
        if !root.exists() && !SymlinkGuard::is_symlink(root) {
            return report;
        }
        if !root.is_dir() || SymlinkGuard::is_symlink(root) {
            Self::delete_entry(root, exclusions, &mut report);
            return report;
        }

        if let Err(e) = Blacklist::validate(root) {
            report.errors.push(e.to_string());
            return report;
        }

        let entries = match fs::read_dir(root) {
            Ok(e) => e,
            Err(e) => {
                report.errors.push(e.to_string());
                return report;
            }
        };

        for entry in entries {
            match entry {
                Ok(ent) => Self::delete_entry(&ent.path(), exclusions, &mut report),
                Err(e) => report.errors.push(e.to_string()),
            }
        }
        report
    }

    pub fn delete_path(root: &Path, exclusions: &[String]) -> TreeDeleteReport {
        let mut report = TreeDeleteReport::default();
        if !root.exists() && !SymlinkGuard::is_symlink(root) {
            return report;
        }
        if let Err(e) = Blacklist::validate(root) {
            report.errors.push(e.to_string());
            return report;
        }
        Self::delete_entry(root, exclusions, &mut report);
        report
    }

    fn delete_entry(path: &Path, exclusions: &[String], report: &mut TreeDeleteReport) {
        if Self::is_excluded(path, exclusions) || Blacklist::is_blacklisted(path) {
            report.skipped_files += 1;
            return;
        }

        let metadata = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                report.errors.push(format!("{}: {}", path.display(), e));
                return;
            }
        };

        if metadata.file_type().is_symlink() || metadata.is_file() {
            let bytes = allocated_bytes(&metadata);
            match fs::remove_file(path) {
                Ok(()) => {
                    report.reclaimed_bytes += bytes;
                    report.deleted_files += 1;
                }
                Err(e) => {
                    report.errors.push(format!("{}: {}", path.display(), e));
                }
            }
            return;
        }

        if !metadata.is_dir() {
            return;
        }

        let entries = match fs::read_dir(path) {
            Ok(e) => e,
            Err(e) => {
                report.errors.push(format!("{}: {}", path.display(), e));
                return;
            }
        };

        for entry in entries {
            match entry {
                Ok(ent) => Self::delete_entry(&ent.path(), exclusions, report),
                Err(e) => report.errors.push(format!("{}: {}", path.display(), e)),
            }
        }

        match fs::remove_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => {}
            Err(error) => report.errors.push(format!("{}: {}", path.display(), error)),
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
        metadata.blocks().saturating_mul(512)
    }
    #[cfg(not(unix))]
    {
        metadata.len()
    }
}
