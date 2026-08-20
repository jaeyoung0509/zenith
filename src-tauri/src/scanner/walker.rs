use crate::models::{FileSize, ScanItem, Signature};
use crate::scanner::SizeCalculator;
use crate::signatures::SignatureLoader;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub struct DirectoryScanner;

impl DirectoryScanner {
    /// Scans all configured paths for a given signature and returns discovered ScanItems.
    pub fn scan_signature(signature: &Signature) -> Vec<ScanItem> {
        let mut items = Vec::new();

        // If signature has no explicit file paths (e.g. Docker commands), return early or handle in Docker adapter
        if signature.paths.is_empty() {
            return items;
        }

        for (idx, pattern) in signature.paths.iter().enumerate() {
            let path_buf = match SignatureLoader::expand_path(pattern) {
                Some(p) => p,
                None => continue,
            };

            let exists = path_buf.exists() || crate::safety::SymlinkGuard::is_symlink(&path_buf);

            if let Some(min_age_days) = signature.min_age_days {
                items.extend(Self::scan_aged_children(
                    signature,
                    &path_buf,
                    idx,
                    min_age_days,
                ));
                continue;
            }

            let (size, file_count) = if exists {
                SizeCalculator::measure_path(&path_buf, &signature.exclusions)
            } else {
                (FileSize::default(), 0)
            };

            let last_modified = if exists {
                fs::metadata(&path_buf)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
            } else {
                None
            };

            let item_id = if signature.paths.len() > 1 {
                format!("{}.{}", signature.id, idx)
            } else {
                signature.id.clone()
            };

            let display_name = if signature.paths.len() > 1 {
                format!(
                    "{} ({})",
                    signature.name,
                    path_buf
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(pattern)
                )
            } else {
                signature.name.clone()
            };

            // Only auto-select if RiskTier is Safe
            let is_selected = signature.risk.is_auto_selectable() && size.reclaimable() > 0;

            items.push(ScanItem {
                id: item_id,
                signature_id: signature.id.clone(),
                name: display_name,
                category: signature.category,
                risk: signature.risk,
                path: path_buf.to_string_lossy().to_string(),
                size,
                file_count,
                description: signature.description.clone(),
                is_selected,
                last_modified,
                exists,
            });
        }

        items
    }

    fn scan_aged_children(
        signature: &Signature,
        root: &std::path::Path,
        path_index: usize,
        min_age_days: u32,
    ) -> Vec<ScanItem> {
        let Ok(entries) = fs::read_dir(root) else {
            return vec![];
        };
        let minimum_age = Duration::from_secs(u64::from(min_age_days) * 86_400);
        let now = SystemTime::now();
        let mut items = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !signature.include_prefixes.is_empty()
                && !signature
                    .include_prefixes
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
            {
                continue;
            }

            // Single-pass fail-closed tree measurement
            let stats = Self::measure_tree_stats(&path, &signature.exclusions, 0, 32);
            // Fail-closed: If scan encountered permission errors or depth cutoff, exclude from stale cleanup
            if !stats.complete {
                continue;
            }

            let Some(modified) = stats.newest_mtime else {
                continue;
            };
            if now.duration_since(modified).unwrap_or_default() < minimum_age {
                continue;
            }

            let size = FileSize::new(stats.logical, Some(stats.allocated));
            if size.reclaimable() == 0 {
                continue;
            }
            items.push(ScanItem {
                id: format!("{}.{}.{}", signature.id, path_index, name),
                signature_id: signature.id.clone(),
                name,
                category: signature.category,
                risk: signature.risk,
                path: path.to_string_lossy().to_string(),
                size,
                file_count: stats.file_count,
                description: format!(
                    "{} (unchanged for at least {} days)",
                    signature.description, min_age_days
                ),
                is_selected: signature.risk.is_auto_selectable(),
                last_modified: modified
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .ok()
                    .map(|duration| duration.as_secs()),
                exists: true,
            });
        }

        items
    }

    /// Measures directory statistics (size, count, newest mtime) in a single recursive pass.
    /// Marks complete = false if any error, symlink escape, or depth cutoff occurs.
    pub fn measure_tree_stats(
        path: &Path,
        exclusions: &[String],
        current_depth: usize,
        max_depth: usize,
    ) -> TreeStats {
        let mut stats = TreeStats {
            logical: 0,
            allocated: 0,
            file_count: 0,
            newest_mtime: None,
            complete: true,
        };

        if current_depth > max_depth {
            stats.complete = false;
            return stats;
        }

        let meta = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(_) => {
                stats.complete = false;
                return stats;
            }
        };

        if let Ok(modified) = meta.modified() {
            stats.newest_mtime = Some(match stats.newest_mtime {
                Some(existing) => existing.max(modified),
                None => modified,
            });
        }

        if meta.file_type().is_symlink() || meta.is_file() {
            let len = meta.len();
            stats.logical = len;
            #[cfg(unix)]
            {
                stats.allocated = meta.blocks() * 512;
            }
            #[cfg(not(unix))]
            {
                stats.allocated = len;
            }
            stats.file_count = 1;
            return stats;
        }

        if !meta.is_dir() {
            return stats;
        }

        let entries = match fs::read_dir(path) {
            Ok(e) => e,
            Err(_) => {
                stats.complete = false;
                return stats;
            }
        };

        for entry in entries {
            let ent = match entry {
                Ok(e) => e,
                Err(_) => {
                    stats.complete = false;
                    continue;
                }
            };
            let child_path = ent.path();

            if crate::safety::Blacklist::is_blacklisted(&child_path) {
                continue;
            }

            let child_str = child_path.to_string_lossy();
            if exclusions.iter().any(|ex| {
                child_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n == ex)
                    .unwrap_or(false)
                    || child_str.contains(ex)
            }) {
                continue;
            }

            let sub_stats =
                Self::measure_tree_stats(&child_path, exclusions, current_depth + 1, max_depth);
            if !sub_stats.complete {
                stats.complete = false;
            }
            stats.logical += sub_stats.logical;
            stats.allocated += sub_stats.allocated;
            stats.file_count += sub_stats.file_count;
            if let Some(sub_mtime) = sub_stats.newest_mtime {
                stats.newest_mtime = Some(match stats.newest_mtime {
                    Some(existing) => existing.max(sub_mtime),
                    None => sub_mtime,
                });
            }
        }

        stats
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeStats {
    pub logical: u64,
    pub allocated: u64,
    pub file_count: usize,
    pub newest_mtime: Option<SystemTime>,
    pub complete: bool,
}
