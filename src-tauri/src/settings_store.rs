use crate::models::ZenithSettings;
use std::fs;
use std::path::{Path, PathBuf};

const SETTINGS_FILE: &str = "settings.json";

pub fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SETTINGS_FILE)
}

pub fn load(config_dir: &Path) -> ZenithSettings {
    let path = settings_path(config_dir);
    if !path.exists() {
        return ZenithSettings::default().sanitize();
    }

    match fs::read_to_string(&path) {
        Ok(contents) => match serde_json::from_str::<ZenithSettings>(&contents) {
            Ok(settings) => settings.sanitize(),
            Err(err) => {
                let defaults = ZenithSettings::default().sanitize();
                backup_and_recover_corrupted_settings(config_dir, &err.to_string(), &defaults);
                defaults
            }
        },
        Err(err) => {
            crate::diagnostics::log_error(
                "settings",
                &format!("Failed to read settings file: {err}"),
            );
            ZenithSettings::default().sanitize()
        }
    }
}

pub const MAX_CORRUPT_BACKUPS: usize = 5;

fn backup_and_recover_corrupted_settings(
    config_dir: &Path,
    err_msg: &str,
    defaults: &ZenithSettings,
) {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let original = settings_path(config_dir);
    let backup = config_dir.join(format!("settings.corrupt.{timestamp}.json"));

    // 1. Move corrupted settings.json to backup. Only prune older backups
    // after the current corrupt payload was actually preserved.
    let backup_preserved = match fs::rename(&original, &backup) {
        Ok(()) => true,
        Err(rename_error) => match fs::copy(&original, &backup) {
            Ok(_) => {
                crate::diagnostics::log_error(
                    "settings",
                    &format!(
                        "Failed to rename corrupted settings; preserved a copy instead: {rename_error}"
                    ),
                );
                // Windows rename does not replace an existing destination. Once
                // the corrupt payload is safely copied, remove the original so
                // the atomic temporary-file rename in `save` can restore it.
                if let Err(remove_error) = fs::remove_file(&original) {
                    crate::diagnostics::log_error(
                        "settings",
                        &format!(
                            "Failed to remove copied corrupt settings before recovery: {remove_error}"
                        ),
                    );
                }
                true
            }
            Err(copy_error) => {
                crate::diagnostics::log_error(
                    "settings",
                    &format!(
                        "Failed to preserve corrupted settings (rename: {rename_error}; copy: {copy_error})"
                    ),
                );
                false
            }
        },
    };

    // 2. Atomically write default settings back into settings.json to recover
    if let Err(e) = save(config_dir, defaults) {
        crate::diagnostics::log_error(
            "settings",
            &format!("Failed to save recovered default settings: {e}"),
        );
    }

    // 3. Prune old corrupted backups beyond the retention limit only after
    // preserving this recovery event.
    if backup_preserved {
        prune_corrupt_backups(config_dir, MAX_CORRUPT_BACKUPS);
    }

    let preservation = if backup_preserved {
        format!("preserved at {}", backup.display())
    } else {
        "could not be preserved".to_string()
    };
    let msg = format!(
        "Corrupted settings file {preservation}; recovery with defaults was attempted (Error: {err_msg})"
    );
    crate::diagnostics::log_error("settings", &msg);
}

fn parse_backup_timestamp(path: &Path) -> Option<u128> {
    let name = path.file_name()?.to_str()?;
    let without_prefix = name.strip_prefix("settings.corrupt.")?;
    let ts_str = without_prefix.strip_suffix(".json")?;
    ts_str.parse::<u128>().ok()
}

pub fn list_corrupted_backups(config_dir: &Path) -> Vec<PathBuf> {
    let mut backups = Vec::new();
    if let Ok(entries) = fs::read_dir(config_dir) {
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|file_type| file_type.is_file()) {
                continue;
            }
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("settings.corrupt.") && name.ends_with(".json") {
                    backups.push(path);
                }
            }
        }
    }
    backups.sort_by(
        |a, b| match (parse_backup_timestamp(a), parse_backup_timestamp(b)) {
            (Some(ts_a), Some(ts_b)) => ts_a.cmp(&ts_b),
            _ => {
                let a_time = a.metadata().and_then(|m| m.modified()).ok();
                let b_time = b.metadata().and_then(|m| m.modified()).ok();
                match (a_time, b_time) {
                    (Some(at), Some(bt)) => at.cmp(&bt),
                    _ => a.cmp(b),
                }
            }
        },
    );
    backups
}

pub fn prune_corrupt_backups(config_dir: &Path, max_backups: usize) {
    let backups = list_corrupted_backups(config_dir);
    if backups.len() > max_backups {
        let remove_count = backups.len() - max_backups;
        for backup in backups.into_iter().take(remove_count) {
            let _ = fs::remove_file(backup);
        }
    }
}

pub fn count_corrupted_backups(config_dir: &Path) -> usize {
    list_corrupted_backups(config_dir).len()
}

pub fn has_corrupted_backup(config_dir: &Path) -> bool {
    count_corrupted_backups(config_dir) > 0
}

pub fn save(config_dir: &Path, settings: &ZenithSettings) -> Result<(), String> {
    fs::create_dir_all(config_dir).map_err(|error| error.to_string())?;
    let path = settings_path(config_dir);
    let temporary_path = config_dir.join(format!("{SETTINGS_FILE}.tmp"));
    let contents = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    fs::write(&temporary_path, contents).map_err(|error| error.to_string())?;
    fs::rename(&temporary_path, &path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        count_corrupted_backups, has_corrupted_backup, load, prune_corrupt_backups, save,
        settings_path, MAX_CORRUPT_BACKUPS,
    };
    use crate::models::{QuickPanelSection, ZenithSettings};

    #[test]
    fn settings_round_trip_through_config_directory() {
        let directory = tempfile::tempdir().unwrap();
        let settings = ZenithSettings {
            quick_panel_sections: vec![QuickPanelSection::AgentActivity],
            quick_panel_ai_providers: vec!["opencode".into()],
            ai_accounts_quota_providers: vec!["cursor".into(), "grok".into()],
            ..ZenithSettings::default()
        };

        save(directory.path(), &settings).unwrap();
        assert_eq!(load(directory.path()), settings);
    }

    #[test]
    fn corrupt_settings_recovers_and_does_not_retrigger_on_subsequent_loads() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("settings.json"), b"not json").unwrap();

        // First load triggers recovery: moves corrupted to backup and saves default settings.json
        let loaded = load(directory.path());
        assert_eq!(loaded, ZenithSettings::default());
        assert!(has_corrupted_backup(directory.path()));

        // settings.json must now be a valid JSON file on disk
        let disk_contents = std::fs::read_to_string(settings_path(directory.path())).unwrap();
        let parsed = serde_json::from_str::<ZenithSettings>(&disk_contents);
        assert!(
            parsed.is_ok(),
            "Recovered settings.json on disk must be valid JSON"
        );

        // Second load must load normally and NOT create additional corrupt backup files
        let backups_before = count_corrupted_backups(directory.path());
        assert_eq!(backups_before, 1);

        let second_load = load(directory.path());
        assert_eq!(second_load, ZenithSettings::default());
        assert_eq!(count_corrupted_backups(directory.path()), backups_before);
    }

    #[test]
    fn corrupt_backups_cap_at_max_and_prune_oldest() {
        let directory = tempfile::tempdir().unwrap();
        let dir_path = directory.path();

        // Create 8 backups with known timestamps (1000..1007)
        for i in 1000..1008 {
            let file = dir_path.join(format!("settings.corrupt.{i}.json"));
            std::fs::write(&file, b"corrupt").unwrap();
        }

        assert_eq!(count_corrupted_backups(dir_path), 8);

        // Prune to MAX_CORRUPT_BACKUPS (5)
        prune_corrupt_backups(dir_path, MAX_CORRUPT_BACKUPS);

        assert_eq!(count_corrupted_backups(dir_path), 5);

        // The oldest 3 (1000, 1001, 1002) should have been pruned
        assert!(!dir_path.join("settings.corrupt.1000.json").exists());
        assert!(!dir_path.join("settings.corrupt.1001.json").exists());
        assert!(!dir_path.join("settings.corrupt.1002.json").exists());

        // The newest 5 (1003..1007) must remain
        for i in 1003..1008 {
            assert!(
                dir_path.join(format!("settings.corrupt.{i}.json")).exists(),
                "Expected backup {i} to remain"
            );
        }
    }

    #[test]
    fn prune_preserves_unrelated_files() {
        let directory = tempfile::tempdir().unwrap();
        let dir_path = directory.path();

        let unrelated_files = [
            "settings.json",
            "settings.json.tmp",
            "settings.corrupt.notjson.txt",
            "unrelated.log",
            "notes.md",
        ];

        for name in &unrelated_files {
            std::fs::write(dir_path.join(name), b"data").unwrap();
        }

        // Create 7 backups
        for i in 100..107 {
            let file = dir_path.join(format!("settings.corrupt.{i}.json"));
            std::fs::write(&file, b"corrupt").unwrap();
        }

        prune_corrupt_backups(dir_path, 3);

        assert_eq!(count_corrupted_backups(dir_path), 3);

        // All unrelated files must remain intact
        for name in &unrelated_files {
            assert!(
                dir_path.join(name).exists(),
                "Unrelated file {} must be preserved",
                name
            );
        }

        let lookalike_directory = dir_path.join("settings.corrupt.50.json");
        std::fs::create_dir(&lookalike_directory).unwrap();
        prune_corrupt_backups(dir_path, 2);
        assert!(
            lookalike_directory.is_dir(),
            "Backup pruning must never treat lookalike directories as files"
        );
    }
}
