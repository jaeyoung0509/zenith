use crate::models::{FileSize, ScanItem, Signature};
use crate::scanner::SizeCalculator;
use crate::signatures::SignatureLoader;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};
use walkdir::WalkDir;

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

            let Some(modified) = latest_modified(&path) else {
                continue;
            };
            if now.duration_since(modified).unwrap_or_default() < minimum_age {
                continue;
            }

            let (size, file_count) = SizeCalculator::measure_path(&path, &signature.exclusions);
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
                file_count,
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
}

/// Uses the newest timestamp in a candidate tree so an actively-written temp
/// directory is never considered stale merely because its root mtime is old.
fn latest_modified(path: &Path) -> Option<SystemTime> {
    WalkDir::new(path)
        .follow_links(false)
        .max_depth(32)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|entry| fs::symlink_metadata(entry.path()).ok()?.modified().ok())
        .max()
}
