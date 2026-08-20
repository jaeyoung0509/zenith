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

use regex::Regex;
use std::sync::LazyLock;

static SECRET_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        // 0. sk-... (OpenAI, Anthropic, OpenRouter API keys)
        Regex::new(r"sk-[a-zA-Z0-9_\-]{8,}").unwrap(),
        // 1. GitHub personal access tokens
        Regex::new(r"ghp_[a-zA-Z0-9]{20,}").unwrap(),
        // 2. GitLab personal access tokens
        Regex::new(r"glpat-[a-zA-Z0-9_\-]{20,}").unwrap(),
        // 3. Slack tokens
        Regex::new(r"xox[baprs]-[a-zA-Z0-9_\-]{10,}").unwrap(),
        // 4. Authorization headers (Bearer, Token, or raw)
        Regex::new(r#"(?i)((?:authorization\s*[:=]\s*(?:bearer\s+|token\s+)?|auth\s*[:=]\s*["']?))[a-zA-Z0-9_\.\-]+"#).unwrap(),
        // 5. Standalone Bearer <token>
        Regex::new(r"(?i)(bearer\s+)[a-zA-Z0-9_\.\-]+").unwrap(),
        // 6. Key-value pairs (e.g. token=..., api_key: "...", "api_key": "...", OPENAI_API_KEY=..., password=...)
        Regex::new(r#"(?i)(["']?(?:api[_-]?key|token|secret|password)[a-zA-Z0-9_\-]*["']?\s*[:=]\s*["']?)[a-zA-Z0-9_\.\-]+"#).unwrap(),
        // 7. URL query parameters (e.g. ?token=..., &key=..., &api_key=...)
        Regex::new(r"(?i)([?&](?:token|key|api_key|secret|password)=)[^&\s]+").unwrap(),
    ]
});

/// Redacts known secret patterns (API keys, bearer tokens, passwords, query params) from log messages.
pub fn sanitize_log(msg: &str) -> String {
    let mut sanitized = msg.to_string();

    // 1. Exact token formats (sk-..., ghp_..., glpat-..., xox-...)
    for idx in 0..4 {
        sanitized = SECRET_PATTERNS[idx]
            .replace_all(&sanitized, "[REDACTED]")
            .to_string();
    }
    // 2. Authorization header tokens
    sanitized = SECRET_PATTERNS[4]
        .replace_all(&sanitized, "${1}[REDACTED]")
        .to_string();
    // 3. Standalone Bearer tokens
    sanitized = SECRET_PATTERNS[5]
        .replace_all(&sanitized, "${1}[REDACTED]")
        .to_string();
    // 4. Key-value pairs (token=..., api_key: ..., password: ..., etc.)
    sanitized = SECRET_PATTERNS[6]
        .replace_all(&sanitized, "${1}[REDACTED]")
        .to_string();
    // 5. URL query parameters
    sanitized = SECRET_PATTERNS[7]
        .replace_all(&sanitized, "${1}[REDACTED]")
        .to_string();

    sanitized
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

pub fn normalized_log_path() -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let full = log_file_path();
        if let Ok(rel) = full.strip_prefix(&home) {
            return format!("~/{}", rel.display());
        }
    }
    log_file_path().to_string_lossy().to_string()
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
        log_path: normalized_log_path(),
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
        let cases = [
            ("Bearer secret-token-xyz", "Bearer [REDACTED]"),
            ("token=secret123", "token=[REDACTED]"),
            ("TOKEN=SECRET_VAL", "TOKEN=[REDACTED]"),
            (
                "OPENROUTER_API_KEY=sk-or-v1-abcdef123456",
                "OPENROUTER_API_KEY=[REDACTED]",
            ),
            (
                "OPENAI_API_KEY=sk-proj-998877665544",
                "OPENAI_API_KEY=[REDACTED]",
            ),
            (
                "ANTHROPIC_API_KEY=sk-ant-112233445566",
                "ANTHROPIC_API_KEY=[REDACTED]",
            ),
            (
                "Authorization: Bearer my-jwt-secret-token",
                "Authorization: Bearer [REDACTED]",
            ),
            (
                r#"{"api_key":"sk-abcdef123456"}"#,
                r#"{"api_key":"[REDACTED]"}"#,
            ),
            (
                r#"{"token": "my-secret-token", "password": "super-secret-password"}"#,
                r#"{"token": "[REDACTED]", "password": "[REDACTED]"}"#,
            ),
            (
                "https://foo.com?token=secret123&other=val",
                "https://foo.com?token=[REDACTED]&other=val",
            ),
            ("api_key:sk-abcdef123456", "api_key:[REDACTED]"),
            ("ghp_123456789012345678901234567890", "[REDACTED]"),
            ("glpat-123456789012345678901234567890", "[REDACTED]"),
        ];

        for (input, expected) in cases {
            let sanitized = sanitize_log(input);
            assert_eq!(sanitized, expected, "Failed on input: {input}");
        }
    }

    #[test]
    fn diagnostics_snapshot_contains_system_info_and_normalized_path() {
        let dir = tempfile::tempdir().unwrap();
        let settings = ZenithSettings::default();
        let snapshot = get_snapshot(&settings, dir.path());
        assert_eq!(snapshot.app_version, env!("CARGO_PKG_VERSION"));
        assert!(!snapshot.arch.is_empty());
        assert!(!snapshot.enabled_features.is_empty());
        if std::env::var_os("HOME").is_some() {
            assert!(
                snapshot.log_path.starts_with("~/"),
                "Expected normalized log path starting with ~/, got {}",
                snapshot.log_path
            );
        }
    }
}
