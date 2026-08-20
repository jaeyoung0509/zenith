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
                backup_corrupted_settings(config_dir, &contents, &err.to_string());
                ZenithSettings::default().sanitize()
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

fn backup_corrupted_settings(config_dir: &Path, _contents: &str, err_msg: &str) {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let original = settings_path(config_dir);
    let backup = config_dir.join(format!("settings.corrupt.{timestamp}.json"));
    let _ = fs::copy(&original, &backup);
    let msg = format!(
        "Corrupted settings file backed up to {} (Error: {})",
        backup.display(),
        err_msg
    );
    crate::diagnostics::log_error("settings", &msg);
}

pub fn has_corrupted_backup(config_dir: &Path) -> bool {
    if let Ok(entries) = fs::read_dir(config_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("settings.corrupt.") && name.ends_with(".json") {
                    return true;
                }
            }
        }
    }
    false
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
    use super::{has_corrupted_backup, load, save};
    use crate::models::{QuickPanelSection, ZenithSettings};

    #[test]
    fn settings_round_trip_through_config_directory() {
        let directory = tempfile::tempdir().unwrap();
        let settings = ZenithSettings {
            quick_panel_sections: vec![QuickPanelSection::AiUsage],
            quick_panel_ai_providers: vec!["opencode".into()],
            ..ZenithSettings::default()
        };

        save(directory.path(), &settings).unwrap();
        assert_eq!(load(directory.path()), settings);
    }

    #[test]
    fn corrupt_settings_fall_back_to_safe_defaults_and_creates_backup() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("settings.json"), b"not json").unwrap();
        assert_eq!(load(directory.path()), ZenithSettings::default());
        assert!(has_corrupted_backup(directory.path()));
    }
}
