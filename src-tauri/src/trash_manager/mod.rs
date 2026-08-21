use crate::applications::AppInspectionRecord;
use crate::large_files::{is_allowed_large_file_path, FileIdentity, LargeFileInventory};
use crate::models::{TrashItemResult, TrashPlanPreview, TrashResult};
use crate::safety::Blacklist;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
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
        for id in selected_ids {
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

        for id in selected_related_ids {
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
        let mut result = TrashResult {
            moved_count: 0,
            failed_count: 0,
            skipped_count: 0,
            moved_allocated_size: 0,
            items: Vec::new(),
        };

        for target in plan.targets {
            match validate_target(&target) {
                Ok(()) => match trash::delete(&target.path) {
                    Ok(()) => {
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
                    Err(error) => {
                        result.failed_count += 1;
                        result.items.push(TrashItemResult {
                            item_id: target.item_id,
                            success: false,
                            message: format!("Could not move to Trash: {error}"),
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
        }
        TrashScope::AppBundle => {
            if !is_application_root(&target.path) {
                return Err("Skipped because the app moved outside Applications.".to_string());
            }
        }
        TrashScope::AppRelated => {
            if Blacklist::is_blacklisted(&target.path) {
                return Err("Skipped because the path is protected by Zenith.".to_string());
            }
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

fn is_application_root(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    if parent == Path::new("/Applications") {
        return true;
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| parent == home.join("Applications"))
        .unwrap_or(false)
}

fn is_allowed_app_data_path(path: &Path) -> bool {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return false;
    };
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
    ROOTS.iter().any(|root| path.starts_with(home.join(root)))
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
        assert!(is_application_root(Path::new("/Applications/Example.app")));
        assert!(!is_application_root(Path::new(
            "/Applications/Nested/Example.app"
        )));
        assert!(!is_application_root(Path::new(
            "/System/Applications/Mail.app"
        )));
    }

    #[test]
    fn large_file_planner_rejects_forged_item_ids() {
        let inventory = LargeFileInventory {
            scan_id: "scan-1".to_string(),
            records: HashMap::new(),
            created_at: unix_timestamp(),
        };
        let error = TrashPlanner::from_large_files(&inventory, &["forged".to_string()])
            .expect_err("unknown frontend ids must be rejected");
        assert!(error.contains("inventory changed"));
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
}
