use crate::large_files::FileIdentity;
use crate::models::{
    AppInstallSource, AppRelatedConfidence, AppRelatedItem, AppRelatedKind, AppUninstallInspection,
    InstalledApp,
};
use crate::safety::Blacklist;
use plist::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::{ProcessesToUpdate, System};
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[derive(Debug, Clone)]
pub struct AppRecord {
    pub app: InstalledApp,
    pub path: PathBuf,
    pub identity: FileIdentity,
}

#[derive(Debug, Clone)]
pub struct AppInventory {
    pub inventory_id: String,
    pub records: HashMap<String, AppRecord>,
    pub created_at: u64,
}

#[derive(Debug, Clone)]
pub struct RelatedRecord {
    pub item: AppRelatedItem,
    pub path: PathBuf,
    pub identity: FileIdentity,
}

#[derive(Debug, Clone)]
pub struct AppInspectionRecord {
    pub inspection: AppUninstallInspection,
    pub app_path: PathBuf,
    pub app_identity: FileIdentity,
    pub related: HashMap<String, RelatedRecord>,
    pub created_at: u64,
}

pub struct ApplicationScanner;

impl ApplicationScanner {
    pub fn scan() -> AppInventory {
        let mut records = HashMap::new();
        let home = crate::platform::paths::NativePlatformPaths::new()
            .home()
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from));

        #[cfg(not(target_os = "windows"))]
        let mut roots = vec![PathBuf::from("/Applications")];
        #[cfg(not(target_os = "windows"))]
        if let Some(home) = &home {
            roots.push(home.join("Applications"));
        }

        #[cfg(target_os = "windows")]
        let mut roots = vec![
            PathBuf::from("C:\\Program Files"),
            PathBuf::from("C:\\Program Files (x86)"),
        ];
        #[cfg(target_os = "windows")]
        if let Some(home) = &home {
            roots.push(home.join("AppData\\Local\\Programs"));
        }

        let mut system = System::new_all();
        system.refresh_processes(ProcessesToUpdate::All, true);
        let running_paths = system
            .processes()
            .values()
            .filter_map(|process| process.exe().map(Path::to_path_buf))
            .collect::<Vec<_>>();

        for root in roots {
            if fs::symlink_metadata(&root)
                .map(|metadata| !metadata.is_dir() || metadata.file_type().is_symlink())
                .unwrap_or(true)
            {
                continue;
            }
            let Ok(entries) = fs::read_dir(root) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                #[cfg(not(target_os = "windows"))]
                if path.extension().and_then(|value| value.to_str()) != Some("app") {
                    continue;
                }
                #[cfg(target_os = "windows")]
                if !path.is_dir() {
                    continue;
                }

                let Some(identity) = FileIdentity::from_path(&path) else {
                    continue;
                };

                #[cfg(not(target_os = "windows"))]
                let (metadata, name) = {
                    let metadata = read_bundle_metadata(&path);
                    let name = metadata
                        .display_name
                        .clone()
                        .or_else(|| {
                            path.file_stem()
                                .and_then(|value| value.to_str())
                                .map(str::to_string)
                        })
                        .unwrap_or_else(|| "Unknown App".to_string());
                    (metadata, name)
                };

                #[cfg(target_os = "windows")]
                let (metadata, name) = {
                    let name = path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or("Unknown App")
                        .to_string();
                    if name.eq_ignore_ascii_case("WindowsApps")
                        || name.eq_ignore_ascii_case("Common Files")
                        || name.eq_ignore_ascii_case("Internet Explorer")
                    {
                        continue;
                    }
                    (
                        BundleMetadata {
                            display_name: Some(name.clone()),
                            bundle_id: None,
                            version: None,
                            executable: Some(format!("{name}.exe")),
                        },
                        name,
                    )
                };

                let (logical_size, allocated_size) = measure_path_without_symlinks(&path);
                let is_system_protected = is_zenith_identity(&name, metadata.bundle_id.as_deref());
                let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
                let is_running = running_paths.iter().any(|exe| exe.starts_with(&canonical));
                let id = Uuid::new_v4().to_string();
                let app = InstalledApp {
                    id: id.clone(),
                    name,
                    bundle_id: metadata.bundle_id,
                    version: metadata.version,
                    display_path: path.to_string_lossy().to_string(),
                    executable_name: metadata.executable,
                    logical_size,
                    allocated_size,
                    modified_at: fs::metadata(&path)
                        .ok()
                        .and_then(|value| value.modified().ok())
                        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                        .map(|value| value.as_secs()),
                    install_source: detect_install_source(&path),
                    is_running,
                    is_system_protected,
                };
                records.insert(
                    id,
                    AppRecord {
                        app,
                        path,
                        identity,
                    },
                );
            }
        }

        AppInventory {
            inventory_id: Uuid::new_v4().to_string(),
            records,
            created_at: unix_timestamp(),
        }
    }

    pub fn inspect(inventory: &AppInventory, app_id: &str) -> Result<AppInspectionRecord, String> {
        let record = inventory
            .records
            .get(app_id)
            .ok_or_else(|| "Application inventory is stale. Refresh applications.".to_string())?;
        if is_zenith_app(&record.app) {
            return Err("Zenith cannot uninstall itself.".to_string());
        }
        if record.app.is_running {
            return Err(format!(
                "Quit {} before reviewing uninstall data.",
                record.app.name
            ));
        }
        if FileIdentity::from_path(&record.path).as_ref() != Some(&record.identity) {
            return Err(
                "The application changed after inventory. Refresh applications.".to_string(),
            );
        }

        let home = crate::platform::paths::NativePlatformPaths::new()
            .home()
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .ok_or_else(|| "Could not resolve the user home directory".to_string())?;
        let bundle_id = record.app.bundle_id.clone();
        let normalized_name = record.app.name.trim().to_string();
        let mut related = HashMap::new();
        let mut incomplete = false;
        let mut warnings = Vec::new();

        #[cfg(not(target_os = "windows"))]
        let roots = [
            (
                "Library/Application Support",
                AppRelatedKind::ApplicationSupport,
            ),
            ("Library/Caches", AppRelatedKind::Cache),
            ("Library/Logs", AppRelatedKind::Log),
            ("Library/Preferences", AppRelatedKind::Preference),
            (
                "Library/Saved Application State",
                AppRelatedKind::SavedState,
            ),
            ("Library/Containers", AppRelatedKind::Container),
            ("Library/Group Containers", AppRelatedKind::GroupContainer),
            (
                "Library/Application Scripts",
                AppRelatedKind::ApplicationScripts,
            ),
            ("Library/HTTPStorages", AppRelatedKind::HttpStorage),
            ("Library/WebKit", AppRelatedKind::WebKit),
        ];

        #[cfg(target_os = "windows")]
        let roots = [
            ("AppData/Local", AppRelatedKind::Cache),
            ("AppData/Roaming", AppRelatedKind::ApplicationSupport),
        ];

        for (relative, kind) in roots {
            let root = home.join(relative);
            if fs::symlink_metadata(&root)
                .map(|metadata| !metadata.is_dir() || metadata.file_type().is_symlink())
                .unwrap_or(true)
            {
                if root.exists() {
                    incomplete = true;
                }
                continue;
            }
            let Ok(entries) = fs::read_dir(&root) else {
                if root.exists() {
                    incomplete = true;
                }
                continue;
            };
            for entry in entries {
                let Ok(entry) = entry else {
                    incomplete = true;
                    continue;
                };
                let path = entry.path();
                if Blacklist::is_blacklisted(&path)
                    || fs::symlink_metadata(&path)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(true)
                {
                    continue;
                }
                let filename = entry.file_name().to_string_lossy().to_string();
                let Some((confidence, evidence)) =
                    match_candidate(kind, &filename, bundle_id.as_deref(), &normalized_name)
                else {
                    continue;
                };
                let Some(identity) = FileIdentity::from_path(&path) else {
                    continue;
                };
                let (logical_size, allocated_size) = measure_path_without_symlinks(&path);
                let id = Uuid::new_v4().to_string();
                let selected_by_default = confidence == AppRelatedConfidence::High;
                let item = AppRelatedItem {
                    id: id.clone(),
                    name: filename,
                    display_path: path.to_string_lossy().to_string(),
                    kind,
                    confidence,
                    evidence,
                    logical_size,
                    allocated_size,
                    selected_by_default,
                };
                related.insert(
                    id,
                    RelatedRecord {
                        item,
                        path,
                        identity,
                    },
                );
            }
        }

        if bundle_id.is_none() {
            warnings.push("This app has no readable CFBundleIdentifier. Only exact app-name matches are shown and none are selected automatically.".to_string());
        }
        if incomplete {
            warnings.push(
                "Some protected or unreadable Library locations could not be inspected."
                    .to_string(),
            );
        }

        let inspection_id = Uuid::new_v4().to_string();
        let mut related_items = related
            .values()
            .map(|record| record.item.clone())
            .collect::<Vec<_>>();
        related_items.sort_by_key(|left| std::cmp::Reverse(left.allocated_size));
        let inspection = AppUninstallInspection {
            inspection_id,
            app: record.app.clone(),
            related_items,
            incomplete,
            warnings,
        };
        Ok(AppInspectionRecord {
            inspection,
            app_path: record.path.clone(),
            app_identity: record.identity.clone(),
            related,
            created_at: unix_timestamp(),
        })
    }
}

fn is_zenith_app(app: &InstalledApp) -> bool {
    app.is_system_protected || is_zenith_identity(&app.name, app.bundle_id.as_deref())
}

fn is_zenith_identity(name: &str, bundle_id: Option<&str>) -> bool {
    name == "Zenith" || bundle_id == Some("com.zenith.desktop")
}

fn measure_path_without_symlinks(path: &Path) -> (u64, u64) {
    let Ok(root_metadata) = fs::symlink_metadata(path) else {
        return (0, 0);
    };
    if root_metadata.file_type().is_symlink() {
        return (0, 0);
    }

    #[cfg(unix)]
    let root_device = root_metadata.dev();
    let mut logical_size = 0u64;
    let mut allocated_size = 0u64;
    let mut stack = vec![path.to_path_buf()];

    while let Some(current) = stack.pop() {
        let Ok(metadata) = fs::symlink_metadata(&current) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        #[cfg(unix)]
        if metadata.dev() != root_device {
            continue;
        }
        if metadata.is_file() {
            logical_size = logical_size.saturating_add(metadata.len());
            #[cfg(unix)]
            {
                allocated_size =
                    allocated_size.saturating_add(metadata.blocks().saturating_mul(512));
            }
            #[cfg(not(unix))]
            {
                allocated_size = allocated_size.saturating_add(metadata.len());
            }
            continue;
        }
        if metadata.is_dir() {
            if let Ok(entries) = fs::read_dir(&current) {
                stack.extend(entries.flatten().map(|entry| entry.path()));
            }
        }
    }

    (logical_size, allocated_size)
}

#[derive(Default)]
struct BundleMetadata {
    display_name: Option<String>,
    bundle_id: Option<String>,
    version: Option<String>,
    executable: Option<String>,
}

fn read_bundle_metadata(path: &Path) -> BundleMetadata {
    let plist_path = path.join("Contents/Info.plist");
    let Ok(value) = Value::from_file(plist_path) else {
        return BundleMetadata::default();
    };
    let Some(dict) = value.as_dictionary() else {
        return BundleMetadata::default();
    };
    let get = |key: &str| dict.get(key).and_then(Value::as_string).map(str::to_string);
    BundleMetadata {
        display_name: get("CFBundleDisplayName").or_else(|| get("CFBundleName")),
        bundle_id: get("CFBundleIdentifier"),
        version: get("CFBundleShortVersionString").or_else(|| get("CFBundleVersion")),
        executable: get("CFBundleExecutable"),
    }
}

fn match_candidate(
    kind: AppRelatedKind,
    filename: &str,
    bundle_id: Option<&str>,
    app_name: &str,
) -> Option<(AppRelatedConfidence, String)> {
    if let Some(bundle_id) = bundle_id {
        let exact_bundle_match = filename == bundle_id
            || filename == format!("{bundle_id}.plist")
            || filename == format!("{bundle_id}.savedState");
        if exact_bundle_match {
            let shared = kind == AppRelatedKind::GroupContainer;
            return Some((
                if shared {
                    AppRelatedConfidence::Shared
                } else {
                    AppRelatedConfidence::High
                },
                if shared {
                    "Exact group/container identifier; treated as shared until exclusive ownership is proven"
                } else {
                    "Exact CFBundleIdentifier match"
                }
                .to_string(),
            ));
        }
    }
    if !app_name.is_empty() && filename.eq_ignore_ascii_case(app_name) {
        return Some((
            AppRelatedConfidence::Medium,
            "Exact application display-name match".to_string(),
        ));
    }
    None
}

fn detect_install_source(path: &Path) -> AppInstallSource {
    let text = path.to_string_lossy();
    if text.contains("/Caskroom/") {
        AppInstallSource::HomebrewCask
    } else {
        AppInstallSource::ApplicationBundle
    }
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
    use std::io::Write;

    #[test]
    fn exact_bundle_identifier_is_high_confidence() {
        let (confidence, _) = match_candidate(
            AppRelatedKind::Preference,
            "com.example.Editor.plist",
            Some("com.example.Editor"),
            "Editor",
        )
        .unwrap();
        assert_eq!(confidence, AppRelatedConfidence::High);
    }

    #[test]
    fn substring_match_is_rejected() {
        assert!(match_candidate(
            AppRelatedKind::ApplicationSupport,
            "Editor Pro Backup",
            Some("com.example.Editor"),
            "Editor",
        )
        .is_none());
    }

    #[test]
    fn group_container_is_never_high_confidence() {
        let (confidence, _) = match_candidate(
            AppRelatedKind::GroupContainer,
            "group.com.example.Editor",
            Some("group.com.example.Editor"),
            "Editor",
        )
        .unwrap();
        assert_eq!(confidence, AppRelatedConfidence::Shared);
    }

    #[test]
    fn recognizes_the_configured_zenith_bundle_identifier() {
        let app = InstalledApp {
            id: "zenith".to_string(),
            name: "Renamed App".to_string(),
            bundle_id: Some("com.zenith.desktop".to_string()),
            version: None,
            display_path: "/Applications/Renamed App.app".to_string(),
            executable_name: None,
            logical_size: 0,
            allocated_size: 0,
            modified_at: None,
            install_source: AppInstallSource::ApplicationBundle,
            is_running: false,
            is_system_protected: false,
        };
        assert!(is_zenith_app(&app));
    }

    #[test]
    fn dedicated_app_size_measurement_counts_files_without_following_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("Example.app");
        fs::create_dir_all(bundle.join("Contents")).unwrap();
        let mut file = fs::File::create(bundle.join("Contents/payload.bin")).unwrap();
        file.write_all(&[7; 4096]).unwrap();
        drop(file);

        #[cfg(unix)]
        std::os::unix::fs::symlink(temp.path(), bundle.join("Contents/escape")).unwrap();

        let (logical, allocated) = measure_path_without_symlinks(&bundle);
        assert_eq!(logical, 4096);
        assert!(allocated > 0);
    }
}
