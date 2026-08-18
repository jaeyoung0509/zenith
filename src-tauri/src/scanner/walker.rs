use crate::models::{FileSize, ScanItem, Signature};
use crate::scanner::SizeCalculator;
use crate::signatures::SignatureLoader;
use std::fs;
use std::time::SystemTime;

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
}
