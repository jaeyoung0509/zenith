use crate::models::{
    LargeFileItem, LargeFileKind, LargeFileScanEvent, LargeFileScanRequest, LargeFileScanResult,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

const MAX_RESULTS: usize = 10_000;
const MAX_THRESHOLD: u64 = 64 * 1024 * 1024 * 1024;
const LARGE_FILE_ROOTS: [&str; 4] = ["downloads", "desktop", "documents", "movies"];

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
    pub entries_scanned: u64,
    pub skipped_entries: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIdentity {
    device: u64,
    inode: u64,
    size: u64,
    modified: Option<u64>,
}

impl FileIdentity {
    pub fn is_zero(&self) -> bool {
        self.device == 0 && self.inode == 0
    }

    pub fn same_entity(&self, other: &Self) -> bool {
        !self.is_zero()
            && !other.is_zero()
            && self.device == other.device
            && self.inode == other.inode
    }

    pub fn device(&self) -> u64 {
        self.device
    }

    pub fn inode(&self) -> u64 {
        self.inode
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn modified(&self) -> Option<u64> {
        self.modified
    }

    #[cfg(test)]
    pub fn for_test(device: u64, inode: u64, size: u64, modified: Option<u64>) -> Self {
        Self {
            device,
            inode,
            size,
            modified,
        }
    }

    #[cfg(test)]
    pub fn with_size(&self, size: u64) -> Self {
        Self {
            size,
            ..self.clone()
        }
    }

    #[cfg(test)]
    pub fn with_modified(&self, modified: Option<u64>) -> Self {
        Self {
            modified,
            ..self.clone()
        }
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        let meta = fs::symlink_metadata(path).ok()?;
        if crate::safety::SymlinkGuard::is_symlink(path) {
            return None;
        }
        #[cfg(unix)]
        let (device, inode) = (meta.dev(), meta.ino());
        #[cfg(windows)]
        let (device, inode) = {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
            use windows_sys::Win32::Storage::FileSystem::{
                CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
                FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
                FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
                OPEN_EXISTING,
            };

            let path_text = path.to_string_lossy();
            let wide: Vec<u16> = if path_text.starts_with(r"\\?\") {
                path.as_os_str().encode_wide().chain([0]).collect()
            } else if let Some(unc_path) = path_text.strip_prefix(r"\\") {
                format!(r"\\?\UNC\{}", unc_path)
                    .encode_utf16()
                    .chain([0])
                    .collect()
            } else if path.as_os_str().encode_wide().count() > 240 {
                format!(r"\\?\{}", path.display())
                    .encode_utf16()
                    .chain([0])
                    .collect()
            } else {
                path.as_os_str().encode_wide().chain([0]).collect()
            };

            unsafe {
                let handle = CreateFileW(
                    wide.as_ptr(),
                    0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                    std::ptr::null_mut(),
                );
                if handle == INVALID_HANDLE_VALUE || handle.is_null() {
                    return None;
                }
                let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
                let ok = GetFileInformationByHandle(handle, &mut info);
                CloseHandle(handle);
                if ok == 0 || (info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0) {
                    return None;
                }
                let dev = info.dwVolumeSerialNumber as u64;
                let ino = ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64);
                (dev, ino)
            }
        };
        #[cfg(not(any(unix, windows)))]
        let (device, inode) = (0, 0);

        if device == 0 && inode == 0 {
            return None;
        }

        Some(Self {
            device,
            inode,
            size: meta.len(),
            modified: modified_secs(&meta),
        })
    }
}

pub fn is_allowed_large_file_path(path: &Path) -> bool {
    if allowed_large_file_root(path).is_none() {
        return false;
    }
    !path.components().any(|component| {
        matches!(
            component,
            std::path::Component::Normal(value) if value == ".git"
        )
    })
}

pub fn allowed_large_file_root(path: &Path) -> Option<PathBuf> {
    let paths = crate::platform::NativePlatformPaths::new();
    let normalized = crate::safety::Blacklist::normalize_path(path);
    LARGE_FILE_ROOTS
        .iter()
        .filter_map(|token| paths.content_dir(token))
        .find(|root| normalized.starts_with(crate::safety::Blacklist::normalize_path(root)))
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
        let threshold = request
            .min_size_bytes
            .clamp(request.filter.minimum_threshold(), MAX_THRESHOLD);
        let roots = resolve_roots(&request.roots)?;
        let approved_roots = roots.clone();
        let scan_id = Uuid::new_v4().to_string();
        on_event(LargeFileScanEvent::Started {
            scan_id: scan_id.clone(),
        });

        let mut retained = BTreeMap::new();
        let mut entries_scanned = 0u64;
        let mut skipped_entries = 0u64;
        let mut matches_found = 0u64;
        let mut truncated = false;

        for root in roots {
            if cancel.load(Ordering::Relaxed) {
                on_event(LargeFileScanEvent::Cancelled {
                    scan_id: scan_id.clone(),
                });
                return Ok(inventory_from_retained(
                    scan_id,
                    retained,
                    entries_scanned,
                    skipped_entries,
                    truncated,
                ));
            }

            let display_root = root.to_string_lossy().to_string();
            on_event(LargeFileScanEvent::RootStarted {
                root: display_root.clone(),
            });

            let Some(_root_meta) = safe_scan_root_metadata(&root) else {
                skipped_entries += 1;
                continue;
            };
            #[cfg(unix)]
            let root_device = _root_meta.dev();

            let mut stack = vec![root.clone()];
            while let Some(dir) = stack.pop() {
                if cancel.load(Ordering::Relaxed) {
                    on_event(LargeFileScanEvent::Cancelled {
                        scan_id: scan_id.clone(),
                    });
                    return Ok(inventory_from_retained(
                        scan_id,
                        retained,
                        entries_scanned,
                        skipped_entries,
                        truncated,
                    ));
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
                            matches_found,
                        });
                    }

                    if !approved_roots.iter().any(|root| path.starts_with(root))
                        || path
                            .components()
                            .any(|component| component.as_os_str().eq_ignore_ascii_case(".git"))
                    {
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

                    if crate::safety::SymlinkGuard::is_symlink(&path) {
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
                    if !meta.is_file() {
                        continue;
                    }

                    let extension = path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|value| value.to_ascii_lowercase());
                    if !request.filter.matches_extension(extension.as_deref())
                        || meta.len() < threshold
                    {
                        continue;
                    }
                    let id = Uuid::new_v4().to_string();
                    #[cfg(unix)]
                    let allocated_size = meta.blocks().saturating_mul(512);
                    #[cfg(not(unix))]
                    let allocated_size = meta.len();
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
                    let Some(identity) = FileIdentity::from_path(&path) else {
                        continue;
                    };
                    matches_found = matches_found.saturating_add(1);
                    let rank = (allocated_size, meta.len(), id);
                    let record = LargeFileRecord {
                        item: item.clone(),
                        path,
                        identity,
                    };
                    if retain_largest(&mut retained, rank, record, MAX_RESULTS) {
                        on_event(LargeFileScanEvent::ItemFound { item });
                    } else {
                        truncated = true;
                    }
                }
            }
            on_event(LargeFileScanEvent::RootFinished { root: display_root });
        }

        let inventory = inventory_from_retained(
            scan_id.clone(),
            retained,
            entries_scanned,
            skipped_entries,
            truncated,
        );
        let mut items = inventory
            .records
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
            truncated,
        };
        on_event(LargeFileScanEvent::Finished {
            result: result.clone(),
        });
        Ok(inventory)
    }
}

fn retain_largest(
    retained: &mut BTreeMap<(u64, u64, String), LargeFileRecord>,
    rank: (u64, u64, String),
    record: LargeFileRecord,
    limit: usize,
) -> bool {
    if retained.len() < limit {
        retained.insert(rank, record);
        return true;
    }
    let should_replace = retained
        .first_key_value()
        .map(|(smallest, _)| &rank > smallest)
        .unwrap_or(false);
    if should_replace {
        retained.pop_first();
        retained.insert(rank, record);
    }
    false
}

fn inventory_from_retained(
    scan_id: String,
    retained: BTreeMap<(u64, u64, String), LargeFileRecord>,
    entries_scanned: u64,
    skipped_entries: u64,
    truncated: bool,
) -> LargeFileInventory {
    let records = retained
        .into_values()
        .map(|record| (record.item.id.clone(), record))
        .collect();
    LargeFileInventory {
        scan_id,
        records,
        created_at: unix_timestamp(),
        entries_scanned,
        skipped_entries,
        truncated,
    }
}

fn resolve_roots(tokens: &[String]) -> Result<Vec<PathBuf>, String> {
    let paths = crate::platform::NativePlatformPaths::new();
    resolve_roots_with(tokens, |token| paths.content_dir(token))
}

#[cfg(test)]
fn resolve_roots_for_home(tokens: &[String], home: &Path) -> Result<Vec<PathBuf>, String> {
    resolve_roots_with(tokens, |token| {
        let name = match token {
            "downloads" => "Downloads",
            "desktop" => "Desktop",
            "documents" => "Documents",
            "movies" => "Movies",
            _ => return None,
        };
        Some(home.join(name))
    })
}

fn resolve_roots_with(
    tokens: &[String],
    resolve: impl Fn(&str) -> Option<PathBuf>,
) -> Result<Vec<PathBuf>, String> {
    let requested = if tokens.is_empty() {
        vec!["downloads", "desktop", "documents", "movies"]
    } else {
        tokens.iter().map(String::as_str).collect()
    };
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    for token in requested {
        let token = token.to_ascii_lowercase();
        if !LARGE_FILE_ROOTS.contains(&token.as_str()) {
            return Err(format!("Unsupported large-file scan root: {token}"));
        }
        let Some(root) = resolve(&token) else {
            continue;
        };
        if seen.insert(root.clone()) && safe_scan_root_metadata(&root).is_some() {
            roots.push(root);
        }
    }
    if roots.is_empty() {
        return Err(
            "None of the selected Large Files folders exist or are safe to scan.".to_string(),
        );
    }
    Ok(roots)
}

fn safe_scan_root_metadata(path: &Path) -> Option<fs::Metadata> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_dir() || crate::safety::SymlinkGuard::is_symlink(path) {
        return None;
    }
    crate::safety::SymlinkGuard::validate_anchored_path(path).ok()?;
    Some(metadata)
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
        "pkg" | "mpkg" | "xip" => LargeFileKind::Installer,
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
    #[test]
    fn redirected_korean_content_roots_remain_token_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let documents = dir.path().join("다른 드라이브/사용자 하나/OneDrive/문서");
        let other = dir.path().join("사용자 둘/문서");
        std::fs::create_dir_all(&documents).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let resolve = |token: &str| (token == "documents").then(|| documents.clone());
        assert_eq!(
            super::resolve_roots_with(&["documents".into()], resolve).unwrap(),
            vec![documents.clone()]
        );
        assert!(super::resolve_roots_with(&[other.display().to_string()], resolve).is_err());
        assert!(super::resolve_roots_with(&["../사용자 둘".into()], resolve).is_err());
    }

    #[test]
    fn directory_identities_distinguish_korean_profile_fixtures() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("계정 하나");
        let second = dir.path().join("계정 둘");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let first_id = super::FileIdentity::from_path(&first).unwrap();
        let second_id = super::FileIdentity::from_path(&second).unwrap();
        assert_ne!(
            (first_id.device(), first_id.inode()),
            (second_id.device(), second_id.inode())
        );
        assert!(!first_id.same_entity(&second_id));
        assert!(super::FileIdentity::from_path(&dir.path().join("없는 폴더")).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn junction_cannot_become_a_large_file_or_workspace_root() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("다른 계정");
        let link = dir.path().join("연결");
        std::fs::create_dir(&target).unwrap();
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(super::safe_scan_root_metadata(&link).is_none());
        std::fs::create_dir(target.join("문서")).unwrap();
        assert!(super::safe_scan_root_metadata(&link.join("문서")).is_none());
        assert!(super::FileIdentity::from_path(&link).is_none());
        assert!(crate::developer_artifacts::validate_workspace_root(&link, dir.path()).is_err());
    }
    use super::*;
    use crate::models::LargeFileFilter;

    #[test]
    fn classifies_developer_large_files() {
        assert_eq!(classify(Some("gguf")), LargeFileKind::AiModel);
        assert_eq!(classify(Some("qcow2")), LargeFileKind::VmImage);
        assert_eq!(classify(Some("mkv")), LargeFileKind::Video);
    }

    #[test]
    fn installer_filter_has_a_lower_floor_and_strict_extensions() {
        assert_eq!(
            LargeFileFilter::Installers.minimum_threshold(),
            10 * 1024 * 1024
        );
        assert!(LargeFileFilter::Installers.matches_extension(Some("pkg")));
        assert!(LargeFileFilter::Installers.matches_extension(Some("dmg")));
        assert!(!LargeFileFilter::Installers.matches_extension(Some("zip")));
        assert_eq!(classify(Some("pkg")), LargeFileKind::Installer);
        assert_eq!(classify(Some("dmg")), LargeFileKind::DiskImage);
    }

    #[test]
    fn missing_filter_keeps_existing_large_file_requests_on_all_files() {
        let request: LargeFileScanRequest =
            serde_json::from_str(r#"{"roots":["downloads"],"min_size_bytes":104857600}"#)
                .expect("legacy request should deserialize");
        assert_eq!(request.filter, LargeFileFilter::All);
        assert!(request.filter.matches_extension(Some("zip")));
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

    #[cfg(unix)]
    #[test]
    fn symlinked_scan_roots_are_not_safe_directories() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        let linked = temp.path().join("linked");
        fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &linked).unwrap();

        assert!(safe_scan_root_metadata(&real).is_some());
        assert!(safe_scan_root_metadata(&linked).is_none());
    }

    #[test]
    fn root_resolution_rejects_an_empty_or_missing_selection() {
        let temp = tempfile::tempdir().unwrap();
        let error = resolve_roots_for_home(&["movies".to_string()], temp.path()).unwrap_err();
        assert!(error.contains("None of the selected"));
    }

    #[test]
    fn bounded_results_retain_the_largest_candidates() {
        fn record(id: &str, allocated_size: u64) -> LargeFileRecord {
            LargeFileRecord {
                item: LargeFileItem {
                    id: id.to_string(),
                    name: format!("{id}.bin"),
                    display_parent: "/tmp".to_string(),
                    logical_size: allocated_size,
                    allocated_size,
                    modified_at: None,
                    kind: LargeFileKind::Other,
                    extension: Some("bin".to_string()),
                },
                path: PathBuf::from(format!("/tmp/{id}.bin")),
                identity: FileIdentity::for_test(1, allocated_size, allocated_size, None),
            }
        }

        let mut retained = BTreeMap::new();
        assert!(retain_largest(
            &mut retained,
            (10, 10, "small".to_string()),
            record("small", 10),
            2,
        ));
        assert!(retain_largest(
            &mut retained,
            (20, 20, "medium".to_string()),
            record("medium", 20),
            2,
        ));
        assert!(!retain_largest(
            &mut retained,
            (30, 30, "large".to_string()),
            record("large", 30),
            2,
        ));

        let ids = retained
            .values()
            .map(|candidate| candidate.item.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["medium", "large"]);
    }

    #[test]
    fn file_identity_from_path_roundtrips_and_is_never_zero() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test_file.bin");
        std::fs::write(&file, b"test content").unwrap();

        let id = FileIdentity::from_path(&file)
            .expect("FileIdentity::from_path must succeed for existing file");
        assert!(!id.is_zero(), "Identity must not be zero");
        assert_eq!(id.size(), 12);
        assert_eq!(FileIdentity::from_path(&file), Some(id.clone()));
        assert!(id.same_entity(&id));

        // Zero identity must not be considered same entity or valid
        let zero = FileIdentity::for_test(0, 0, 12, None);
        assert!(zero.is_zero());
        assert!(!id.same_entity(&zero));
        assert!(!zero.same_entity(&zero));
    }
}
