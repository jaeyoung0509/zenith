use crate::models::{DiagnosticsSnapshot, ZenithSettings};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

static LOG_MUTEX: Mutex<()> = Mutex::new(());

const MAX_LOG_BYTES: u64 = 1_000_000; // 1 MB rotation threshold

pub fn log_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        #[cfg(target_os = "macos")]
        {
            return home.join("Library/Logs/Zenith");
        }
        #[cfg(not(target_os = "macos"))]
        {
            return home.join(".local/share/zenith/logs");
        }
    }
    std::env::temp_dir().join("zenith_logs")
}

pub fn log_file_path() -> PathBuf {
    log_dir().join("zenith.log")
}

/// Redacts known secret patterns (e.g. `sk-...`, `Bearer ...`, `token=...`) from log messages.
pub fn sanitize_log(msg: &str) -> String {
    let mut result = Vec::new();
    let mut redact_next = false;

    for word in msg.split_whitespace() {
        if redact_next {
            result.push("[REDACTED]");
            redact_next = false;
            continue;
        }

        let lower = word.to_ascii_lowercase();
        if lower.starts_with("sk-")
            || lower.starts_with("ghp_")
            || lower.starts_with("glpat-")
            || lower.starts_with("token=")
            || lower.starts_with("key=")
            || lower.starts_with("secret=")
            || lower.starts_with("password=")
            || lower.starts_with("api_key=")
            || lower.starts_with("apikey=")
        {
            result.push("[REDACTED]");
        } else if lower == "bearer" || lower == "token" || lower == "key" || lower == "secret" {
            result.push(word);
            redact_next = true;
        } else {
            result.push(word);
        }
    }
    result.join(" ")
}

pub fn log_error(category: &str, message: &str) {
    let _guard = LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let dir = log_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }

    let file_path = log_file_path();

    // Check size for rotation
    if let Ok(meta) = fs::metadata(&file_path) {
        if meta.len() > MAX_LOG_BYTES {
            let backup = dir.join("zenith.log.1");
            let _ = fs::rename(&file_path, backup);
        }
    }

    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let sanitized = sanitize_log(message);
    let line = format!("[{timestamp}] [{category}] {sanitized}\n");

    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

pub fn get_recent_errors(limit: usize) -> Vec<String> {
    let _guard = LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let file_path = log_file_path();
    let Ok(file) = fs::File::open(file_path) else {
        return Vec::new();
    };

    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        if !line.trim().is_empty() {
            lines.push(line);
        }
    }

    if lines.len() > limit {
        lines.split_off(lines.len() - limit)
    } else {
        lines
    }
}

pub fn get_snapshot(settings: &ZenithSettings, config_dir: &Path) -> DiagnosticsSnapshot {
    let mut features = Vec::new();

    features.push(format!(
        "dashboard_tabs: {}",
        settings
            .dashboard_tabs
            .iter()
            .map(|t| format!("{t:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    features.push(format!(
        "quick_panel_sections: {}",
        settings
            .quick_panel_sections
            .iter()
            .map(|s| format!("{s:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    features.push(format!(
        "clean_categories: ai={}, dev={}, docker={}, models={}",
        settings.clean_ai_tools,
        settings.clean_developer_tools,
        settings.clean_docker,
        settings.clean_local_models
    ));
    features.push(format!(
        "awake_rules: total={}, active={}",
        settings.awake_rules.len(),
        settings.awake_rules.iter().filter(|r| r.enabled).count()
    ));

    #[cfg(target_os = "macos")]
    let os_version = {
        let mut cmd = std::process::Command::new("sw_vers");
        cmd.arg("-productVersion");
        crate::tooling::run_with_timeout(cmd, std::time::Duration::from_secs(2))
            .ok()
            .map(|o| format!("macOS {}", String::from_utf8_lossy(&o.stdout).trim()))
            .unwrap_or_else(|| "macOS".to_string())
    };
    #[cfg(not(target_os = "macos"))]
    let os_version = std::env::consts::OS.to_string();

    DiagnosticsSnapshot {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os_version,
        arch: std::env::consts::ARCH.to_string(),
        log_path: log_file_path().to_string_lossy().to_string(),
        enabled_features: features,
        recent_errors: get_recent_errors(20),
        settings_corrupt_recovered: crate::settings_store::has_corrupted_backup(config_dir),
    }
}

pub fn open_logs_folder() -> Result<(), String> {
    let dir = log_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create log directory: {e}"))?;

    let mut cmd = std::process::Command::new("open");
    cmd.arg(&dir);
    crate::tooling::run_with_timeout(cmd, std::time::Duration::from_secs(5))
        .map_err(|e| format!("Failed to open logs folder: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_sanitizer_redacts_tokens() {
        let text = "Failed with key sk-123456789abcdef and Bearer secret-token-xyz during auth";
        let clean = sanitize_log(text);
        assert!(!clean.contains("sk-123456789abcdef"));
        assert!(!clean.contains("secret-token-xyz"));
        assert!(clean.contains("[REDACTED]"));
    }

    #[test]
    fn diagnostics_snapshot_contains_system_info() {
        let dir = tempfile::tempdir().unwrap();
        let settings = ZenithSettings::default();
        let snapshot = get_snapshot(&settings, dir.path());
        assert_eq!(snapshot.app_version, env!("CARGO_PKG_VERSION"));
        assert!(!snapshot.arch.is_empty());
        assert!(!snapshot.enabled_features.is_empty());
    }
}
