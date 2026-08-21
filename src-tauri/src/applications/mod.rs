use crate::large_files::FileIdentity;
use crate::models::{
    AppInstallSource, AppRelatedConfidence, AppRelatedItem, AppRelatedKind, AppUninstallInspection,
    InstalledApp,
};
use crate::safety::Blacklist;
use crate::scanner::SizeCalculator;
use plist::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::{ProcessesToUpdate, System};
use uuid::Uuid;

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
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = &home {
            roots.push(home.join("Applications"));
        }

        let mut system = System::new_all();
        system.refresh_processes(ProcessesToUpdate::All, true);
        let running_paths = system
            .processes()
            .values()
            .filter_map(|process| process.exe().map(Path::to_path_buf))
            .collect::<Vec<_>>();

        for root in roots {
            let Ok(entries) = fs::read_dir(root) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("app") {
                    continue;
                }
                let Some(identity) = FileIdentity::from_path(&path) else {
                    continue;
                };
                let metadata = read_bundle_metadata(&path);
                let (size, _) = SizeCalculator::measure_path(&path, &[]);
                let name = metadata
                    .display_name
                    .or_else(|| {
                        path.file_stem()
                            .and_then(|value| value.to_str())
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "Unknown App".to_string());
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
                    logical_size: size.logical,
                    allocated_size: size.allocated.unwrap_or(size.logical),
                    modified_at: fs::metadata(&path)
                        .ok()
                        .and_then(|value| value.modified().ok())
                        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                        .map(|value| value.as_secs()),
                    install_source: detect_install_source(&path),
                    is_running,
                    is_system_protected: false,
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
        if record.app.name == "Zenith" || record.app.bundle_id.as_deref() == Some("com.zenith.app")
        {
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

        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "Could not resolve the user home directory".to_string())?;
        let bundle_id = record.app.bundle_id.clone();
        let normalized_name = record.app.name.trim().to_string();
        let mut related = HashMap::new();
        let mut incomplete = false;
        let mut warnings = Vec::new();

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

        for (relative, kind) in roots {
            let root = home.join(relative);
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
                let (size, _) = SizeCalculator::measure_path(&path, &[]);
                let id = Uuid::new_v4().to_string();
                let selected_by_default = confidence == AppRelatedConfidence::High;
                let item = AppRelatedItem {
                    id: id.clone(),
                    name: filename,
                    display_path: path.to_string_lossy().to_string(),
                    kind,
                    confidence,
                    evidence,
                    logical_size: size.logical,
                    allocated_size: size.allocated.unwrap_or(size.logical),
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
}
