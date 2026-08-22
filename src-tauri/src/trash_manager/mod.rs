use crate::applications::AppInspectionRecord;
use crate::large_files::{
    allowed_large_file_root, is_allowed_large_file_path, FileIdentity, LargeFileInventory,
};
use crate::models::{TrashItemResult, TrashPlanPreview, TrashResult};
use crate::safety::{Blacklist, SymlinkGuard};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::{ProcessesToUpdate, System};
use uuid::Uuid;

const PLAN_TTL_SECS: u64 = 300;

#[derive(Debug, Clone)]
pub struct TrashTarget {
    pub item_id: String,
    pub path: PathBuf,
    pub identity: FileIdentity,
    pub logical_size: u64,
    pub allocated_size: u64,
    pub scope: TrashScope,
}

#[derive(Debug, Clone)]
pub enum TrashScope {
    LargeFile { approved_parent: PathBuf },
    AppBundle,
    AppRelated,
}

#[derive(Debug, Clone)]
pub struct TrashPlan {
    pub id: Uuid,
    pub created_at: u64,
    pub inventory_id: String,
    pub targets: Vec<TrashTarget>,
}

impl TrashPlan {
    pub fn preview(&self) -> TrashPlanPreview {
        TrashPlanPreview {
            id: self.id,
            item_count: self.targets.len(),
            logical_size: self.targets.iter().map(|target| target.logical_size).sum(),
            allocated_size: self
                .targets
                .iter()
                .map(|target| target.allocated_size)
                .sum(),
            expires_at: self.created_at + PLAN_TTL_SECS,
        }
    }

    pub fn is_expired(&self) -> bool {
        unix_timestamp().saturating_sub(self.created_at) >= PLAN_TTL_SECS
    }
}

pub struct TrashPlanner;

impl TrashPlanner {
    pub fn from_large_files(
        inventory: &LargeFileInventory,
        selected_ids: &[String],
    ) -> Result<TrashPlan, String> {
        if selected_ids.is_empty() {
            return Err("Select at least one file to move to Trash.".to_string());
        }
        let mut targets = Vec::with_capacity(selected_ids.len());
        let mut seen = HashSet::with_capacity(selected_ids.len());
        for id in selected_ids {
            if !seen.insert(id) {
                continue;
            }
            let record = inventory
                .records
                .get(id)
                .ok_or_else(|| "The large-file inventory changed. Scan again.".to_string())?;
            let parent = record
                .path
                .parent()
                .ok_or_else(|| "Invalid file parent".to_string())?
                .to_path_buf();
            targets.push(TrashTarget {
                item_id: id.clone(),
                path: record.path.clone(),
                identity: record.identity.clone(),
                logical_size: record.item.logical_size,
                allocated_size: record.item.allocated_size,
                scope: TrashScope::LargeFile {
                    approved_parent: parent,
                },
            });
        }
        Ok(TrashPlan {
            id: Uuid::new_v4(),
            created_at: unix_timestamp(),
            inventory_id: inventory.scan_id.clone(),
            targets,
        })
    }

    pub fn from_app_inspection(
        inspection: &AppInspectionRecord,
        selected_related_ids: &[String],
    ) -> Result<TrashPlan, String> {
        let mut targets = Vec::new();
        let app_size = &inspection.inspection.app;
        targets.push(TrashTarget {
            item_id: inspection.inspection.app.id.clone(),
            path: inspection.app_path.clone(),
            identity: inspection.app_identity.clone(),
            logical_size: app_size.logical_size,
            allocated_size: app_size.allocated_size,
            scope: TrashScope::AppBundle,
        });

        let mut seen = HashSet::with_capacity(selected_related_ids.len());
        for id in selected_related_ids {
            if !seen.insert(id) {
                continue;
            }
            let record = inspection.related.get(id).ok_or_else(|| {
                "The app inspection changed. Review the uninstall again.".to_string()
            })?;
            targets.push(TrashTarget {
                item_id: id.clone(),
                path: record.path.clone(),
                identity: record.identity.clone(),
                logical_size: record.item.logical_size,
                allocated_size: record.item.allocated_size,
                scope: TrashScope::AppRelated,
            });
        }

        Ok(TrashPlan {
            id: Uuid::new_v4(),
            created_at: unix_timestamp(),
            inventory_id: inspection.inspection.inspection_id.clone(),
            targets,
        })
    }
}

pub struct TrashExecutor;

impl TrashExecutor {
    pub fn execute(plan: TrashPlan) -> TrashResult {
        Self::execute_with(plan, |path| {
            trash::delete(path).map_err(|error| format!("Could not move to Trash: {error}"))
        })
    }

    fn execute_with<F>(plan: TrashPlan, mut move_to_trash: F) -> TrashResult
    where
        F: FnMut(&Path) -> Result<(), String>,
    {
        let mut result = TrashResult {
            moved_count: 0,
            failed_count: 0,
            skipped_count: 0,
            moved_allocated_size: 0,
            items: Vec::new(),
        };

        let app_uninstall = matches!(
            plan.targets.first().map(|target| &target.scope),
            Some(TrashScope::AppBundle)
        );
        let mut app_bundle_moved = !app_uninstall;

        for (index, target) in plan.targets.into_iter().enumerate() {
            if app_uninstall && index > 0 && !app_bundle_moved {
                result.skipped_count += 1;
                result.items.push(TrashItemResult {
                    item_id: target.item_id,
                    success: false,
                    message: "Skipped because the reviewed app bundle was not moved to Trash."
                        .to_string(),
                });
                continue;
            }
            match validate_target(&target) {
                Ok(()) => match move_to_trash(&target.path) {
                    Ok(()) => {
                        if matches!(target.scope, TrashScope::AppBundle) {
                            app_bundle_moved = true;
                        }
                        result.moved_count += 1;
                        result.moved_allocated_size = result
                            .moved_allocated_size
                            .saturating_add(target.allocated_size);
                        result.items.push(TrashItemResult {
                            item_id: target.item_id,
                            success: true,
                            message: "Moved to Trash".to_string(),
                        });
                    }
                    Err(message) => {
                        result.failed_count += 1;
                        result.items.push(TrashItemResult {
                            item_id: target.item_id,
                            success: false,
                            message,
                        });
                    }
                },
                Err(error) => {
                    result.skipped_count += 1;
                    result.items.push(TrashItemResult {
                        item_id: target.item_id,
                        success: false,
                        message: error,
                    });
                }
            }
        }
        result
    }
}

fn validate_target(target: &TrashTarget) -> Result<(), String> {
    match &target.scope {
        TrashScope::LargeFile { .. } => {
            if !is_allowed_large_file_path(&target.path) {
                return Err(
                    "Skipped because the file moved outside the approved Large Files scope."
                        .to_string(),
                );
            }
            let root = allowed_large_file_root(&target.path).ok_or_else(|| {
                "Skipped because the file has no approved Large Files root.".to_string()
            })?;
            validate_no_symlink_components(&target.path, &root)?;
        }
        TrashScope::AppBundle => {
            let Some(root) = application_root_for_path(&target.path) else {
                return Err("Skipped because the app moved outside Applications.".to_string());
            };
            validate_no_symlink_components(&target.path, &root)?;
            if is_application_running(&target.path) {
                return Err("Skipped because the reviewed application is running.".to_string());
            }
        }
        TrashScope::AppRelated => {
            if Blacklist::is_blacklisted(&target.path) {
                return Err("Skipped because the path is protected by Zenith.".to_string());
            }
            let root = app_data_root_for_path(&target.path).ok_or_else(|| {
                "Skipped because related data moved outside the approved Library scope.".to_string()
            })?;
            validate_no_symlink_components(&target.path, &root)?;
        }
    }

    let current = FileIdentity::from_path(&target.path)
        .ok_or_else(|| "Skipped because the item disappeared or became a symlink.".to_string())?;
    if current != target.identity {
        return Err("Skipped because the filesystem item changed after review.".to_string());
    }

    match &target.scope {
        TrashScope::LargeFile { approved_parent } => {
            if target.path.parent() != Some(approved_parent.as_path()) {
                return Err(
                    "Skipped because the file moved outside the reviewed scope.".to_string()
                );
            }
            if target.path.is_dir() {
                return Err("Large Files only moves reviewed files, not directories.".to_string());
            }
        }
        TrashScope::AppBundle => {
            if target.path.extension().and_then(|value| value.to_str()) != Some("app") {
                return Err(
                    "Skipped because the reviewed app bundle is no longer an app.".to_string(),
                );
            }
        }
        TrashScope::AppRelated => {
            if !is_allowed_app_data_path(&target.path) {
                return Err(
                    "Skipped because related data moved outside the approved Library scope."
                        .to_string(),
                );
            }
        }
    }
    Ok(())
}

fn application_root_for_path(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    if parent == Path::new("/Applications") {
        return Some(PathBuf::from("/Applications"));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Applications"))
        .filter(|root| parent == root)
}

fn is_allowed_app_data_path(path: &Path) -> bool {
    app_data_root_for_path(path).is_some()
}

fn app_data_root_for_path(path: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    const ROOTS: [&str; 10] = [
        "Library/Application Support",
        "Library/Caches",
        "Library/Logs",
        "Library/Preferences",
        "Library/Saved Application State",
        "Library/Containers",
        "Library/Group Containers",
        "Library/Application Scripts",
        "Library/HTTPStorages",
        "Library/WebKit",
    ];
    ROOTS
        .iter()
        .map(|root| home.join(root))
        .find(|root| path.parent() == Some(root.as_path()))
}

fn validate_no_symlink_components(path: &Path, root: &Path) -> Result<(), String> {
    SymlinkGuard::validate_no_symlink_ancestors(path, root)
        .map_err(|_| "Skipped because the reviewed path contains a symbolic link.".to_string())
}

fn is_application_running(path: &Path) -> bool {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system
        .processes()
        .values()
        .filter_map(|process| process.exe())
        .any(|executable| executable.starts_with(&canonical))
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
    use std::collections::HashMap;

    #[test]
    fn personal_documents_are_not_app_data_scope() {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            assert!(!is_allowed_app_data_path(
                &home.join("Documents/App Project")
            ));
            assert!(is_allowed_app_data_path(
                &home.join("Library/Caches/com.example.app")
            ));
        }
    }

    #[test]
    fn app_bundle_must_be_a_direct_child_of_an_application_root() {
        assert!(application_root_for_path(Path::new("/Applications/Example.app")).is_some());
        assert!(application_root_for_path(Path::new("/Applications/Nested/Example.app")).is_none());
        assert!(application_root_for_path(Path::new("/System/Applications/Mail.app")).is_none());
    }

    #[test]
    fn large_file_planner_rejects_forged_item_ids() {
        let inventory = LargeFileInventory {
            scan_id: "scan-1".to_string(),
            records: HashMap::new(),
            created_at: unix_timestamp(),
            entries_scanned: 0,
            skipped_entries: 0,
            truncated: false,
        };
        let error = TrashPlanner::from_large_files(&inventory, &["forged".to_string()])
            .expect_err("unknown frontend ids must be rejected");
        assert!(error.contains("inventory changed"));
    }

    #[test]
    fn large_file_planner_deduplicates_frontend_ids() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("large.bin");
        std::fs::write(&path, [1, 2, 3]).unwrap();
        let item = crate::models::LargeFileItem {
            id: "item".to_string(),
            name: "large.bin".to_string(),
            display_parent: temp.path().to_string_lossy().to_string(),
            logical_size: 3,
            allocated_size: 3,
            modified_at: None,
            kind: crate::models::LargeFileKind::Other,
            extension: Some("bin".to_string()),
        };
        let record = crate::large_files::LargeFileRecord {
            item,
            path: path.clone(),
            identity: FileIdentity::from_path(&path).unwrap(),
        };
        let inventory = LargeFileInventory {
            scan_id: "scan-1".to_string(),
            records: HashMap::from([("item".to_string(), record)]),
            created_at: unix_timestamp(),
            entries_scanned: 1,
            skipped_entries: 0,
            truncated: false,
        };

        let plan =
            TrashPlanner::from_large_files(&inventory, &["item".to_string(), "item".to_string()])
                .unwrap();
        assert_eq!(plan.targets.len(), 1);
    }

    #[test]
    fn dedicated_large_file_scope_intentionally_differs_from_generic_blacklist() {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            let document = home.join("Documents/large-video.mov");
            assert!(Blacklist::is_blacklisted(&document));
            assert!(is_allowed_large_file_path(&document));
            assert!(!is_allowed_large_file_path(
                &home.join("Documents/project/.git/objects/pack.bin")
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn reviewed_targets_reject_symlinked_parent_components() {
        let temp = tempfile::tempdir().unwrap();
        let trusted_root = temp.path().join("trusted");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&trusted_root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, trusted_root.join("redirect")).unwrap();
        let target = trusted_root.join("redirect/file.bin");

        let error = validate_no_symlink_components(&target, &trusted_root).unwrap_err();
        assert!(error.contains("symbolic link"));
    }

    #[test]
    fn app_related_items_are_skipped_when_the_app_bundle_cannot_move() {
        let missing = PathBuf::from("/Applications/Missing Example.app");
        let identity = FileIdentity {
            device: 0,
            inode: 0,
            size: 0,
            modified: None,
        };
        let plan = TrashPlan {
            id: Uuid::new_v4(),
            created_at: unix_timestamp(),
            inventory_id: "inspection".to_string(),
            targets: vec![
                TrashTarget {
                    item_id: "app".to_string(),
                    path: missing,
                    identity: identity.clone(),
                    logical_size: 0,
                    allocated_size: 0,
                    scope: TrashScope::AppBundle,
                },
                TrashTarget {
                    item_id: "related".to_string(),
                    path: PathBuf::from("/unused"),
                    identity,
                    logical_size: 0,
                    allocated_size: 0,
                    scope: TrashScope::AppRelated,
                },
            ],
        };
        let mut move_attempts = 0;
        let result = TrashExecutor::execute_with(plan, |_| {
            move_attempts += 1;
            Ok(())
        });

        assert_eq!(move_attempts, 0);
        assert_eq!(result.skipped_count, 2);
        assert!(result.items[1].message.contains("app bundle was not moved"));
    }

    #[test]
    fn trash_plan_ttl_boundary_and_expiry() {
        let now = unix_timestamp();
        let valid = TrashPlan {
            id: Uuid::new_v4(),
            created_at: now - (PLAN_TTL_SECS - 1),
            inventory_id: "test".to_string(),
            targets: vec![],
        };
        let expired = TrashPlan {
            id: Uuid::new_v4(),
            created_at: now - (PLAN_TTL_SECS + 1),
            inventory_id: "test".to_string(),
            targets: vec![],
        };
        let at_boundary = TrashPlan {
            id: Uuid::new_v4(),
            created_at: now - PLAN_TTL_SECS,
            inventory_id: "test".to_string(),
            targets: vec![],
        };
        assert!(!valid.is_expired());
        assert!(expired.is_expired());
        // At exactly TTL should be considered expired (fail-closed, >=)
        assert!(at_boundary.is_expired());
    }

    #[test]
    fn store_plan_caps_at_64_and_evicts_oldest() {
        use crate::storage_commands::{
            clear_trash_plans_for_test, store_plan, trash_plans_len_for_test, TRASH_PLANS,
        };
        clear_trash_plans_for_test();
        let now = unix_timestamp();
        for i in 0..65 {
            let plan = TrashPlan {
                id: Uuid::new_v4(),
                created_at: now + i,
                inventory_id: format!("inv-{i}"),
                targets: vec![],
            };
            store_plan(plan);
        }
        assert_eq!(trash_plans_len_for_test(), 64);
        let plans = TRASH_PLANS.lock().expect("TRASH_PLANS poisoned");
        assert!(
            !plans.values().any(|p| p.inventory_id == "inv-0"),
            "oldest plan should have been evicted"
        );
        assert!(plans.values().any(|p| p.inventory_id == "inv-64"));
        drop(plans);
        clear_trash_plans_for_test();
    }
}
