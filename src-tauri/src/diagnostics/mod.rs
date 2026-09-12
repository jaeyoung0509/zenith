use crate::models::{DiagnosticsSnapshot, ZenithSettings};
use crate::platform::path_algebra::PathFlavor as PlatformFlavor;
use crate::platform::PlatformEnvironment;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

pub mod doctor;

static LOG_MUTEX: Mutex<()> = Mutex::new(());

const MAX_LOG_BYTES: u64 = 1_000_000; // 1 MB rotation threshold

/// Directory holding the log file, resolved through the described environment:
/// Windows `LOCAL_APPDATA/Zenith/Logs`, macOS `~/Library/Logs/Zenith`, other
/// Unix `~/.local/share/zenith/logs`, and the temporary directory when the
/// platform exposes no usable root. The flavor — not the host — selects the
/// Windows branch, so a redirected `LOCAL_APPDATA` can be simulated anywhere.
pub fn log_dir(environment: &PlatformEnvironment) -> PathBuf {
    if environment.flavor().is_windows() {
        // The flavor, not the host, decides the spelling: a simulated Windows
        // environment produces Windows-shaped paths on any runner.
        // Normalize with the environment's own flavor, exactly as
        // `expand_placeholder` does, so the Windows branch yields Windows-shaped
        // paths on any runner instead of the host's separator.
        let join = |root: &Path, relative: &str| {
            PathBuf::from(crate::platform::path_algebra::normalize(
                &root.join(relative).to_string_lossy(),
                PlatformFlavor::Windows,
            ))
        };
        if let Some(local) = environment.local_app_data() {
            return join(&local, "Zenith/Logs");
        }
        // No stated application-data root: fall back to the profile-relative
        // location Windows would use rather than a POSIX-shaped one.
        if let Some(home) = environment.user_home() {
            return join(&home, "AppData/Local/Zenith/Logs");
        }
    } else if let Some(home) = environment.user_home() {
        #[cfg(target_os = "macos")]
        {
            return home.join("Library/Logs/Zenith");
        }
        #[cfg(not(target_os = "macos"))]
        {
            return home.join(".local/share/zenith/logs");
        }
    }
    environment.temp_dir().join("zenith_logs")
}

/// Log file of the running process. The global log sink always follows the real
/// environment; `--doctor` and the environment-aware entry points take the
/// environment explicitly.
pub fn log_file_path() -> PathBuf {
    log_dir(&PlatformEnvironment::native()).join("zenith.log")
}

/// Redacts known credential shapes and masks absolute paths from log messages.
pub fn sanitize_log(msg: &str) -> String {
    let redacted = crate::privacy::secrets::redact(msg);
    crate::privacy::paths::mask_paths_in_text(&redacted)
}

#[cfg(unix)]
fn restrict_permissions(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path, _mode: u32) -> std::io::Result<()> {
    // Windows files inherit the user-profile ACL; there is no portable mode.
    Ok(())
}

pub fn log_error(category: &str, message: &str) {
    let _guard = LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    // The global log sink follows the running environment; resolving it once
    // keeps the directory and the file from disagreeing.
    let dir = log_dir(&PlatformEnvironment::native());
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    // Best effort for the directory; the file below is fail-closed.
    let _ = restrict_permissions(&dir, 0o700);

    let file_path = dir.join("zenith.log");

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

    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Create owner-only from the first byte instead of chmod-after-write.
        options.mode(0o600);
    }
    if let Ok(mut file) = options.open(&file_path) {
        // A pre-existing wider mode must be repaired before anything is
        // written; if it cannot be, drop the line rather than leak it.
        if restrict_permissions(&file_path, 0o600).is_err() {
            return;
        }
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
            // Re-sanitize on read as well as write: entries written by an older
            // build must not reach the clipboard export unmasked.
            lines.push(sanitize_log(&line));
        }
    }

    if lines.len() > limit {
        lines.split_off(lines.len() - limit)
    } else {
        lines
    }
}

pub fn normalized_log_path() -> String {
    normalized_log_path_of(&PlatformEnvironment::native())
}

/// Environment-aware [`normalized_log_path`].
pub fn normalized_log_path_of(environment: &PlatformEnvironment) -> String {
    display_log_path(environment, &log_dir(environment).join("zenith.log"))
}

/// Display form of the directory holding the current log file, with the
/// profile masked. The interface shows this instead of a platform-specific
/// literal such as `~/Library/Logs/Zenith`, which is wrong off macOS.
pub fn log_directory_display() -> String {
    log_directory_display_of(&PlatformEnvironment::native())
}

/// Environment-aware [`log_directory_display`].
pub fn log_directory_display_of(environment: &PlatformEnvironment) -> String {
    display_log_path(environment, &log_dir(environment))
}

/// Masks a log location against the environment's own profile, so a redirected
/// or simulated home is masked with its own spelling.
fn display_log_path(environment: &PlatformEnvironment, path: &Path) -> String {
    crate::privacy::paths::display_path_with_home(path, environment.user_home().as_deref())
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
    features.push(format!("intensive_cleanup: {}", settings.intensive_cleanup));
    features.push(format!(
        "awake_rules: total={}, active={}",
        settings.awake_rules.len(),
        settings.awake_rules.iter().filter(|r| r.enabled).count()
    ));

    let environment = crate::platform::environment::current();
    features.extend(environment.diagnostics_lines());
    let os_version = environment.os_version_label();

    DiagnosticsSnapshot {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os_version,
        arch: environment.process_architecture.clone(),
        log_path: normalized_log_path(),
        enabled_features: features,
        recent_errors: get_recent_errors(20),
        settings_corrupt_recovered: crate::settings_store::has_corrupted_backup(config_dir),
    }
}

pub fn open_logs_folder() -> Result<(), String> {
    let dir = log_dir(&PlatformEnvironment::native());
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create log directory: {e}"))?;

    use crate::platform::SystemActionProvider;
    crate::platform::NativeSystemActions::new().open_folder(&dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::path_algebra::PathFlavor;
    use crate::platform::paths::SimulatedPaths;
    use std::sync::Arc;

    #[test]
    fn secret_sanitizer_redacts_tokens() {
        let dummy_sk_or = format!("sk-or-v1-{}", "testmockkey123");
        let dummy_sk_proj = format!("sk-proj-{}", "testmockkey123");
        let dummy_sk_ant = format!("sk-ant-{}", "testmockkey123");
        let dummy_sk_basic = format!("sk-{}", "abcdef123456");
        let dummy_ghp = format!("ghp_{}", "mockpat1234567890123456");
        let dummy_glpat = format!("glpat-{}", "mockpat1234567890123456");

        let cases = vec![
            (
                "Bearer secret-token-xyz".to_string(),
                "Bearer [REDACTED]".to_string(),
            ),
            (
                "token=secret123".to_string(),
                "token=[REDACTED]".to_string(),
            ),
            (
                "TOKEN=SECRET_VAL".to_string(),
                "TOKEN=[REDACTED]".to_string(),
            ),
            (
                format!("OPENROUTER_API_KEY={dummy_sk_or}"),
                "OPENROUTER_API_KEY=[REDACTED]".to_string(),
            ),
            (
                format!("OPENAI_API_KEY={dummy_sk_proj}"),
                "OPENAI_API_KEY=[REDACTED]".to_string(),
            ),
            (
                format!("ANTHROPIC_API_KEY={dummy_sk_ant}"),
                "ANTHROPIC_API_KEY=[REDACTED]".to_string(),
            ),
            (
                "Authorization: Bearer my-jwt-token".to_string(),
                "Authorization: Bearer [REDACTED]".to_string(),
            ),
            (
                format!(r#"{{"api_key":"{dummy_sk_basic}"}}"#),
                r#"{"api_key":"[REDACTED]"}"#.to_string(),
            ),
            (
                r#"{"token": "dummy-token", "password": "dummy-password"}"#.to_string(),
                r#"{"token": "[REDACTED]", "password": "[REDACTED]"}"#.to_string(),
            ),
            (
                "https://foo.com?token=secret123&other=val".to_string(),
                // The generic assignment rule deliberately over-redacts from
                // the credential to the next whitespace.
                "https://foo.com?token=[REDACTED]".to_string(),
            ),
            (
                format!("api_key:{dummy_sk_basic}"),
                "api_key:[REDACTED]".to_string(),
            ),
            (dummy_ghp, "[REDACTED]".to_string()),
            (dummy_glpat, "[REDACTED]".to_string()),
        ];

        for (input, expected) in cases {
            let sanitized = sanitize_log(&input);
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
        // The snapshot's own path is masked on every runner: `~/…` under the
        // profile, `.../name` for a location outside it. No host is assumed.
        assert!(
            snapshot.log_path.starts_with("~/") || snapshot.log_path.starts_with(".../"),
            "Expected a masked log path, got {}",
            snapshot.log_path
        );

        // A stated home makes the normalization assertion unconditional.
        let environment =
            PlatformEnvironment::simulated(PathFlavor::Posix).with_home("/home/tester");
        let normalized = normalized_log_path_of(&environment);
        assert!(
            normalized.starts_with("~/"),
            "Expected a profile-masked log path, got {normalized}"
        );
        assert!(normalized.ends_with("zenith.log"), "{normalized}");
        assert!(!normalized.contains("/home/tester"), "{normalized}");
    }

    #[test]
    fn log_dir_follows_the_described_environment() {
        // Windows: the stated LOCAL_APPDATA is authoritative, not the literal
        // profile spelling, and the flavor (not the host) picks the branch.
        let windows = PlatformEnvironment::simulated(PathFlavor::Windows).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::Windows)
                .with_home(r"C:\Users\me")
                .with_local_app_data(r"D:\AppData\Local"),
        ));
        assert_eq!(
            log_dir(&windows),
            PathBuf::from(r"D:\AppData\Local\Zenith\Logs")
        );
        assert_eq!(
            log_directory_display_of(&windows),
            ".../Logs",
            "a location outside the stated profile is reduced to its basename"
        );

        // The same environment with application data inside the stated profile
        // masks with the stated profile's own spelling.
        let in_profile = PlatformEnvironment::simulated(PathFlavor::Windows).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::Windows)
                .with_home(r"D:\Users\me")
                .with_local_app_data(r"D:\Users\me\AppData\Local"),
        ));
        assert_eq!(
            log_dir(&in_profile),
            PathBuf::from(r"D:\Users\me\AppData\Local\Zenith\Logs")
        );
        assert_eq!(
            log_directory_display_of(&in_profile),
            "~/AppData/Local/Zenith/Logs"
        );

        // A Windows environment with no stated application data still resolves
        // a Windows-shaped location instead of a POSIX one.
        let windows_without_appdata =
            PlatformEnvironment::simulated(PathFlavor::Windows).with_home(r"D:\Users\me");
        assert_eq!(
            log_dir(&windows_without_appdata),
            PathBuf::from(r"D:\Users\me\AppData\Local\Zenith\Logs")
        );

        // Posix: the profile location, then the temporary directory.
        let posix = PlatformEnvironment::simulated(PathFlavor::Posix).with_home("/home/tester");
        #[cfg(target_os = "macos")]
        assert_eq!(
            log_dir(&posix),
            PathBuf::from("/home/tester/Library/Logs/Zenith")
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            log_dir(&posix),
            PathBuf::from("/home/tester/.local/share/zenith/logs")
        );

        let flavor = PathFlavor::Posix;
        let temp = PathBuf::from("/stated/temp");
        let no_home = PlatformEnvironment::simulated(flavor).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(flavor)
                .with_temp_dir(&temp),
        ));
        assert_eq!(log_dir(&no_home), temp.join("zenith_logs"));
    }

    #[test]
    fn secret_sanitizer_redacts_full_credential_values() {
        let aws = format!(
            "AWS_SECRET_ACCESS_KEY={}",
            concat!("wJalrXUtnFEMI", "/K7MDENG/", "bPxRfiCYEXAMPLEKEY")
        );
        let google = format!("AIzaSy{}", "abcdefghijklmnopqrstuvwxyz0123456");
        let cases = vec![
            (
                "refresh_token=1//0gABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890",
                "refresh_token=[REDACTED]",
            ),
            (
                "client_secret=YWJjZGVmZ2hpamtsbW5vcC+/cXJzdHV2d3h5ejAxMjM0NTY=",
                "client_secret=[REDACTED]",
            ),
            (aws.as_str(), "AWS_SECRET_ACCESS_KEY=[REDACTED]"),
            (
                "Authorization: Basic dXNlcjpwYXNz",
                "Authorization: Basic [REDACTED]",
            ),
            (
                "github_pat_abcdefghijklmnopqrstuvwxyz0123456789ABCD",
                "[REDACTED]",
            ),
            (google.as_str(), "[REDACTED]"),
            (
                "https://user:password-value@example.com/path",
                "https://user:[REDACTED]@example.com/path",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(sanitize_log(input), expected, "Failed on input: {input}");
        }
    }

    #[test]
    fn sanitizer_masks_paths_before_they_reach_diagnostics() {
        // A stated home keeps the assertion unconditional: the masking rule is
        // exercised on every runner, not only where a profile is configured.
        let home = std::path::Path::new("/home/tester");
        let message = "failed to read /home/tester/.claude/settings.json";
        let sanitized = crate::privacy::paths::mask_paths_with_home(message, Some(home));
        assert!(!sanitized.contains("/home/tester"), "{sanitized}");
        assert!(
            sanitized.contains("~/.claude/settings.json") || sanitized.contains("settings.json"),
            "{sanitized}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn owner_only_permissions_are_applied() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zenith.log");
        std::fs::write(&path, b"line").unwrap();
        restrict_permissions(&path, 0o600).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
