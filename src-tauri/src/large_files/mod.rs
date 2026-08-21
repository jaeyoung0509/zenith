use crate::models::{
    LargeFileItem, LargeFileKind, LargeFileScanEvent, LargeFileScanRequest, LargeFileScanResult,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

const MAX_RESULTS: usize = 10_000;
const MIN_THRESHOLD: u64 = 100 * 1024 * 1024;
const MAX_THRESHOLD: u64 = 64 * 1024 * 1024 * 1024;
const LARGE_FILE_ROOTS: [&str; 4] = ["Downloads", "Desktop", "Documents", "Movies"];

#[derive(Debug, Clone)]
pub struct LargeFileRecord {
    pub item: LargeFileItem,
    pub path: PathBuf,
    pub identity: FileIdentity,
}

#[derive(Debug, Clone)]
pub struct LargeFileInventory {
    pub scan_id: String,
    pub records: HashMap<String, LargeFileRecord>,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
    pub size: u64,
    pub modified: Option<u64>,
}

impl FileIdentity {
    pub fn from_path(path: &Path) -> Option<Self> {
        let meta = fs::symlink_metadata(path).ok()?;
        if meta.file_type().is_symlink() {
            return None;
        }
        #[cfg(unix)]
        let (device, inode) = (meta.dev(), meta.ino());
        #[cfg(not(unix))]
        let (device, inode) = (0, 0);
        Some(Self {
            device,
            inode,
            size: meta.len(),
            modified: modified_secs(&meta),
        })
    }
}

pub fn is_allowed_large_file_path(path: &Path) -> bool {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return false;
    };
    let inside_allowed_root = LARGE_FILE_ROOTS
        .iter()
        .any(|root| path.starts_with(home.join(root)));
    if !inside_allowed_root {
        return false;
    }
    !path.components().any(|component| {
        matches!(
            component,
            std::path::Component::Normal(value) if value == ".git"
        )
    })
}

pub struct LargeFileScanner;

impl LargeFileScanner {
    pub fn scan<F>(
        request: &LargeFileScanRequest,
        cancel: Arc<AtomicBool>,
        mut on_event: F,
    ) -> Result<LargeFileInventory, String>
    where
        F: FnMut(LargeFileScanEvent),
    {
        let threshold = request.min_size_bytes.clamp(MIN_THRESHOLD, MAX_THRESHOLD);
        let roots = resolve_roots(&request.roots)?;
        let scan_id = Uuid::new_v4().to_string();
        on_event(LargeFileScanEvent::Started {
            scan_id: scan_id.clone(),
        });

        let mut records = HashMap::new();
        let mut entries_scanned = 0u64;
        let mut skipped_entries = 0u64;

        for root in roots {
            if cancel.load(Ordering::Relaxed) {
                on_event(LargeFileScanEvent::Cancelled {
                    scan_id: scan_id.clone(),
                });
                return Ok(LargeFileInventory {
                    scan_id,
                    records,
                    created_at: unix_timestamp(),
                });
            }

            let display_root = root.to_string_lossy().to_string();
            on_event(LargeFileScanEvent::RootStarted {
                root: display_root.clone(),
            });

            let Ok(root_meta) = fs::symlink_metadata(&root) else {
                skipped_entries += 1;
                continue;
            };
            #[cfg(unix)]
            let root_device = root_meta.dev();
            #[cfg(not(unix))]
            let root_device = 0u64;

            let mut stack = vec![root.clone()];
            while let Some(dir) = stack.pop() {
                if cancel.load(Ordering::Relaxed) {
                    on_event(LargeFileScanEvent::Cancelled {
                        scan_id: scan_id.clone(),
                    });
                    return Ok(LargeFileInventory {
                        scan_id,
                        records,
                        created_at: unix_timestamp(),
                    });
                }

                let entries = match fs::read_dir(&dir) {
                    Ok(entries) => entries,
                    Err(_) => {
                        skipped_entries += 1;
                        continue;
                    }
                };

                for entry in entries {
                    let Ok(entry) = entry else {
                        skipped_entries += 1;
                        continue;
                    };
                    let path = entry.path();
                    entries_scanned += 1;

                    if entries_scanned.is_multiple_of(500) {
                        on_event(LargeFileScanEvent::Progress {
                            root: display_root.clone(),
                            entries_scanned,
                            matches_found: records.len() as u64,
                        });
                    }

                    if !is_allowed_large_file_path(&path) {
                        skipped_entries += 1;
                        continue;
                    }

                    let meta = match fs::symlink_metadata(&path) {
                        Ok(meta) => meta,
                        Err(_) => {
                            skipped_entries += 1;
                            continue;
                        }
                    };

                    if meta.file_type().is_symlink() {
                        skipped_entries += 1;
                        continue;
                    }

                    #[cfg(unix)]
                    if meta.dev() != root_device {
                        skipped_entries += 1;
                        continue;
                    }

                    if meta.is_dir() {
                        if should_skip_directory(&path) {
                            continue;
                        }
                        stack.push(path);
                        continue;
                    }
                    if !meta.is_file() || meta.len() < threshold {
                        continue;
                    }

                    let id = Uuid::new_v4().to_string();
                    #[cfg(unix)]
                    let allocated_size = meta.blocks().saturating_mul(512);
                    #[cfg(not(unix))]
                    let allocated_size = meta.len();
                    let extension = path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|value| value.to_ascii_lowercase());
                    let item = LargeFileItem {
                        id: id.clone(),
                        name: path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("Unknown file")
                            .to_string(),
                        display_parent: path
                            .parent()
                            .map(|parent| parent.to_string_lossy().to_string())
                            .unwrap_or_default(),
                        logical_size: meta.len(),
                        allocated_size,
                        modified_at: modified_secs(&meta),
                        kind: classify(extension.as_deref()),
                        extension,
                    };
                    let identity = FileIdentity {
                        #[cfg(unix)]
                        device: meta.dev(),
                        #[cfg(not(unix))]
                        device: 0,
                        #[cfg(unix)]
                        inode: meta.ino(),
                        #[cfg(not(unix))]
                        inode: 0,
                        size: meta.len(),
                        modified: modified_secs(&meta),
                    };
                    records.insert(
                        id,
                        LargeFileRecord {
                            item: item.clone(),
                            path,
                            identity,
                        },
                    );
                    on_event(LargeFileScanEvent::ItemFound { item });

                    if records.len() >= MAX_RESULTS {
                        break;
                    }
                }
                if records.len() >= MAX_RESULTS {
                    break;
                }
            }
            on_event(LargeFileScanEvent::RootFinished { root: display_root });
            if records.len() >= MAX_RESULTS {
                break;
            }
        }

        let mut items = records
            .values()
            .map(|record| record.item.clone())
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            right
                .allocated_size
                .cmp(&left.allocated_size)
                .then_with(|| right.logical_size.cmp(&left.logical_size))
                .then_with(|| left.name.cmp(&right.name))
        });
        let result = LargeFileScanResult {
            scan_id: scan_id.clone(),
            items,
            entries_scanned,
            skipped_entries,
            cancelled: false,
        };
        on_event(LargeFileScanEvent::Finished {
            result: result.clone(),
        });
        Ok(LargeFileInventory {
            scan_id,
            records,
            created_at: unix_timestamp(),
        })
    }
}

fn resolve_roots(tokens: &[String]) -> Result<Vec<PathBuf>, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "Could not resolve the user home directory".to_string())?;
    let requested = if tokens.is_empty() {
        vec!["downloads", "desktop", "documents", "movies"]
    } else {
        tokens.iter().map(String::as_str).collect()
    };
    let mut roots = Vec::new();
    for token in requested {
        let name = match token.to_ascii_lowercase().as_str() {
            "downloads" => "Downloads",
            "desktop" => "Desktop",
            "documents" => "Documents",
            "movies" => "Movies",
            _ => return Err(format!("Unsupported large-file scan root: {token}")),
        };
        let root = home.join(name);
        if root.is_dir() {
            roots.push(root);
        }
    }
    Ok(roots)
}

fn should_skip_directory(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());
    matches!(
        extension.as_deref(),
        Some("app" | "photoslibrary" | "photolibrary" | "musiclibrary" | "imovielibrary")
    )
}

fn classify(extension: Option<&str>) -> LargeFileKind {
    match extension.unwrap_or_default() {
        "mov" | "mp4" | "mkv" | "avi" | "webm" | "m4v" => LargeFileKind::Video,
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" => LargeFileKind::Archive,
        "dmg" | "iso" => LargeFileKind::DiskImage,
        "qcow2" | "vmdk" | "vdi" | "pvm" => LargeFileKind::VmImage,
        "gguf" | "safetensors" | "ckpt" | "onnx" => LargeFileKind::AiModel,
        "db" | "sqlite" | "sqlite3" | "dump" | "sql" => LargeFileKind::Database,
        "o" | "a" | "wasm" | "jar" => LargeFileKind::DeveloperArtifact,
        _ => LargeFileKind::Other,
    }
}

fn modified_secs(meta: &fs::Metadata) -> Option<u64> {
    meta.modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_secs())
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_developer_large_files() {
        assert_eq!(classify(Some("gguf")), LargeFileKind::AiModel);
        assert_eq!(classify(Some("qcow2")), LargeFileKind::VmImage);
        assert_eq!(classify(Some("mkv")), LargeFileKind::Video);
    }

    #[test]
    fn package_directories_are_not_descended() {
        assert!(should_skip_directory(Path::new("/tmp/Test.app")));
        assert!(should_skip_directory(Path::new(
            "/tmp/Photos.photoslibrary"
        )));
        assert!(!should_skip_directory(Path::new("/tmp/project")));
    }

    #[test]
    fn dedicated_scope_allows_reviewed_user_content_but_protects_git() {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            assert!(is_allowed_large_file_path(
                &home.join("Documents/video.mov")
            ));
            assert!(is_allowed_large_file_path(
                &home.join("Desktop/archive.zip")
            ));
            assert!(!is_allowed_large_file_path(
                &home.join("Documents/project/.git/objects/pack.bin")
            ));
            assert!(!is_allowed_large_file_path(
                &home.join("Library/Caches/cache.bin")
            ));
        }
    }
}
