use crate::models::ZenithSettings;
use std::fs;
use std::path::{Path, PathBuf};

const SETTINGS_FILE: &str = "settings.json";

pub fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SETTINGS_FILE)
}

pub fn load(config_dir: &Path) -> ZenithSettings {
    let path = settings_path(config_dir);
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<ZenithSettings>(&contents).ok())
        .unwrap_or_default()
        .sanitize()
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
    use super::{load, save};
    use crate::models::{QuickPanelSection, ZenithSettings};

    #[test]
    fn settings_round_trip_through_config_directory() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = ZenithSettings::default();
        settings.quick_panel_sections = vec![QuickPanelSection::AiUsage];
        settings.quick_panel_ai_providers = vec!["opencode".into()];

        save(directory.path(), &settings).unwrap();
        assert_eq!(load(directory.path()), settings);
    }

    #[test]
    fn corrupt_settings_fall_back_to_safe_defaults() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("settings.json"), b"not json").unwrap();
        assert_eq!(load(directory.path()), ZenithSettings::default());
    }
}
