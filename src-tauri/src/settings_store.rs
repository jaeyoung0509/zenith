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

fn backup_and_recover_corrupted_settings(
    config_dir: &Path,
    err_msg: &str,
    defaults: &ZenithSettings,
) {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let original = settings_path(config_dir);
    let backup = config_dir.join(format!("settings.corrupt.{timestamp}.json"));

    // 1. Move corrupted settings.json to backup
    if let Err(e) = fs::rename(&original, &backup) {
        let _ = fs::copy(&original, &backup);
        crate::diagnostics::log_error(
            "settings",
            &format!("Failed to rename corrupted settings: {e}"),
        );
    }

    // 2. Atomically write default settings back into settings.json to recover
    if let Err(e) = save(config_dir, defaults) {
        crate::diagnostics::log_error(
            "settings",
            &format!("Failed to save recovered default settings: {e}"),
        );
    }

    let msg = format!(
        "Corrupted settings file moved to {} and recovered with defaults (Error: {})",
        backup.display(),
        err_msg
    );
    crate::diagnostics::log_error("settings", &msg);
}

pub fn count_corrupted_backups(config_dir: &Path) -> usize {
    if let Ok(entries) = fs::read_dir(config_dir) {
        entries
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .map(|n| n.starts_with("settings.corrupt.") && n.ends_with(".json"))
                    .unwrap_or(false)
            })
            .count()
    } else {
        0
    }
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
    use super::{count_corrupted_backups, has_corrupted_backup, load, save, settings_path};
    use crate::models::{QuickPanelSection, ZenithSettings};

    #[test]
    fn settings_round_trip_through_config_directory() {
        let directory = tempfile::tempdir().unwrap();
        let settings = ZenithSettings {
            quick_panel_sections: vec![QuickPanelSection::AgentActivity],
            quick_panel_ai_providers: vec!["opencode".into()],
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
}
