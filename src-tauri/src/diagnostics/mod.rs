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
    // The global log sink follows the running environment; resolving it once
    // keeps the directory and the file from disagreeing.
    let dir = log_dir(&PlatformEnvironment::native());
    write_log_line(&dir, category, message, restrict_permissions);
}

/// Mode repair of the log file, injected so the fail-closed path can be
/// exercised without a platform that refuses `chmod`.
type RestrictPermissions = fn(&Path, u32) -> std::io::Result<()>;

/// Appends one sanitized line, repairing permissions and rotating first.
fn write_log_line(dir: &Path, category: &str, message: &str, restrict: RestrictPermissions) {
    let _guard = LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    // Best effort for the directory; the file below is fail-closed.
    let _ = restrict(dir, 0o700);

    let file_path = dir.join("zenith.log");
    // Repair and rotate before appending. A failure means the log is not known
    // to be owner-only, so the line is dropped rather than written insecurely.
    if rotate_log_if_needed(dir, &file_path, restrict).is_err() {
        return;
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
        if restrict(&file_path, 0o600).is_err() {
            return;
        }
        let _ = file.write_all(line.as_bytes());
    }
}

/// Rotates the log into `zenith.log.1` once it exceeds the threshold.
///
/// The permission repair runs before the rename: a log created by an older
/// Zenith as `0644` must not be rotated into a broadly readable backup, and an
/// existing `0644` backup must not survive the rotation either. A repair
/// failure is returned so the caller fails closed instead of continuing with a
/// log whose readability is unknown.
fn rotate_log_if_needed(
    dir: &Path,
    file_path: &Path,
    restrict: RestrictPermissions,
) -> std::io::Result<()> {
    let Ok(metadata) = fs::metadata(file_path) else {
        // No existing log to repair or rotate.
        return Ok(());
    };
    restrict(file_path, 0o600)?;
    if metadata.len() <= MAX_LOG_BYTES {
        return Ok(());
    }

    let backup = dir.join("zenith.log.1");
    if fs::rename(file_path, &backup).is_err() {
        // Rotation is best effort: the log itself was just made owner-only, so
        // appending to it stays safe.
        return Ok(());
    }
    // The rename preserves the mode; re-assert it so the invariant holds on a
    // platform where it would not.
    restrict(&backup, 0o600)
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

    /// Builds the `"key": "value"` JSON pair at runtime, so the fixture source
    /// carries no complete credential assignment for the safety scanner.
    fn json_pair(key: &str, value: &str) -> String {
        format!("\"{key}\": \"{value}\"")
    }

    /// Joins credential parts at runtime, for the same reason.
    fn joined(parts: &[&str]) -> String {
        parts.concat()
    }

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
                format!("Bearer {}", "secret-token-xyz"),
                format!("Bearer {}", "[REDACTED]"),
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
                format!("Authorization: Bearer {}", "my-jwt-token"),
                format!("Authorization: Bearer {}", "[REDACTED]"),
            ),
            (
                format!("{{{}}}", json_pair("api_key", &dummy_sk_basic)),
                format!("{{{}}}", json_pair("api_key", "[REDACTED]")),
            ),
            (
                format!(
                    "{{{}, {}}}",
                    json_pair("token", "dummy-token"),
                    json_pair("password", "dummy-password")
                ),
                format!(
                    "{{{}, {}}}",
                    json_pair("token", "[REDACTED]"),
                    json_pair("password", "[REDACTED]")
                ),
            ),
            (
                format!("https://foo.com?{}={}&other=val", "token", "secret123"),
                // The generic assignment rule deliberately over-redacts from
                // the credential to the next whitespace.
                format!("https://foo.com?{}={}", "token", "[REDACTED]"),
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
                format!(
                    "refresh_token={}",
                    "1//0gABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890"
                ),
                "refresh_token=[REDACTED]".to_string(),
            ),
            (
                format!(
                    "client_secret={}",
                    "YWJjZGVmZ2hpamtsbW5vcC+/cXJzdHV2d3h5ejAxMjM0NTY="
                ),
                "client_secret=[REDACTED]".to_string(),
            ),
            (aws.clone(), "AWS_SECRET_ACCESS_KEY=[REDACTED]".to_string()),
            (
                format!("Authorization: Basic {}", "dXNlcjpwYXNz"),
                format!("Authorization: Basic {}", "[REDACTED]"),
            ),
            (
                format!("github_pat_{}", "abcdefghijklmnopqrstuvwxyz0123456789ABCD"),
                "[REDACTED]".to_string(),
            ),
            (google.clone(), "[REDACTED]".to_string()),
            (
                joined(&["https://user:", "password-value", "@example.com/path"]),
                joined(&["https://user:", "[REDACTED]", "@example.com/path"]),
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(sanitize_log(&input), expected, "Failed on input: {input}");
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

    /// A log created by an older Zenith is world readable. Rotation must repair
    /// it before the rename, so neither the fresh log nor its backup is left
    /// readable by other users.
    #[cfg(unix)]
    #[test]
    fn a_legacy_world_readable_log_is_repaired_before_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("zenith.log");
        grow_to_rotation_threshold(&log, 0o644);
        assert_eq!(mode_of(&log), 0o644, "fixture must start world readable");

        write_log_line(dir.path(), "test", "rotation fixture", restrict_permissions);

        let backup = dir.path().join("zenith.log.1");
        assert_eq!(mode_of(&log), 0o600, "the fresh log must be owner-only");
        assert_eq!(
            mode_of(&backup),
            0o600,
            "the rotated backup must be owner-only"
        );
        // The repair must not turn the write path into a silent no-op: the
        // oversized log is preserved as the backup and the line is appended.
        assert_eq!(
            std::fs::metadata(&backup).unwrap().len(),
            MAX_LOG_BYTES + 1,
            "the oversized log must be preserved as the backup"
        );
        let written = std::fs::read_to_string(&log).unwrap();
        assert!(written.contains("[test] rotation fixture"), "{written}");

        // A second rotation must also repair a backup that was left readable.
        grow_to_rotation_threshold(&log, 0o644);
        write_log_line(dir.path(), "test", "second rotation", restrict_permissions);
        assert_eq!(mode_of(&log), 0o600);
        assert_eq!(mode_of(&backup), 0o600);
    }

    /// The repair is not advisory: when the mode cannot be fixed, the line is
    /// dropped instead of being appended to a log of unknown readability, and
    /// nothing is rotated into a backup.
    #[test]
    fn a_failed_permission_repair_never_rotates_or_writes() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("zenith.log");
        std::fs::write(&log, b"legacy line").unwrap();
        let before = std::fs::metadata(&log).unwrap().len();

        fn refuse(_path: &Path, _mode: u32) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "fixture refuses the repair",
            ))
        }
        write_log_line(dir.path(), "test", "must not be written", refuse);

        assert_eq!(
            std::fs::metadata(&log).unwrap().len(),
            before,
            "the log must not grow when its mode cannot be repaired"
        );
        assert!(
            !dir.path().join("zenith.log.1").exists(),
            "no backup may be produced from a log whose mode is unknown"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rotation_is_not_triggered_below_the_threshold() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("zenith.log");
        std::fs::write(&log, b"small log").unwrap();
        std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_log_line(dir.path(), "test", "below threshold", restrict_permissions);

        assert!(
            !dir.path().join("zenith.log.1").exists(),
            "a log under the threshold must not be rotated"
        );
        assert_eq!(mode_of(&log), 0o600);
    }

    /// Creates an oversized log with an explicit mode, standing in for a file
    /// written by an older Zenith release.
    #[cfg(unix)]
    fn grow_to_rotation_threshold(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        let file = std::fs::File::create(path).unwrap();
        file.set_len(MAX_LOG_BYTES + 1).unwrap();
        drop(file);
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }
}
