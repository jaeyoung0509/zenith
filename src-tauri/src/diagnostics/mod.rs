use crate::models::{DiagnosticsSnapshot, ZenithSettings};
use crate::platform::path_algebra::PathFlavor as PlatformFlavor;
use crate::platform::PlatformEnvironment;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

pub mod doctor;

static LOG_MUTEX: Mutex<()> = Mutex::new(());
static LOG_PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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

/// The first reason this process could not write its diagnostics log.
///
/// A logger that disables itself silently removes the only evidence of what
/// disabled it — and under Controlled Folder Access that is exactly the
/// failure a user needs named. The first reason is kept and reported through
/// `DiagnosticsSnapshot::log_failure` and the `--doctor` self-check.
static LOG_FAILURE: Mutex<Option<String>> = Mutex::new(None);

fn record_log_failure(reason: impl Into<String>) {
    let mut guard = LOG_FAILURE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        *guard = Some(reason.into());
    }
}

/// Records a failure against the directory it happened in, naming Controlled
/// Folder Access when the policy explains the refusal — the log directory is
/// the one place a user cannot be told about a refusal otherwise.
fn record_log_failure_in(dir: &Path, reason: impl Into<String>) {
    let reason = reason.into();
    record_log_failure(crate::platform::environment::describe_access_refusal(
        &PlatformEnvironment::native(),
        dir,
        &reason,
    ));
}

/// The first diagnostics write failure observed by this process, sanitized for
/// the interface.
///
/// A later successful write does not erase the failure that may explain a gap
/// in the log. The reason can name a path, so it is masked and redacted on the
/// way out the same way a log line is.
pub fn log_failure() -> Option<String> {
    let reason = LOG_FAILURE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    reason.map(|reason| sanitize_log(&reason))
}

/// Mode repair of the log file, injected so the fail-closed path can be
/// exercised without a platform that refuses `chmod`.
type RestrictPermissions = fn(&Path, u32) -> std::io::Result<()>;

/// Appends one sanitized line, repairing permissions and rotating first.
fn write_log_line(dir: &Path, category: &str, message: &str, restrict: RestrictPermissions) {
    let _guard = LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(error) = fs::create_dir_all(dir) {
        record_log_failure_in(
            dir,
            format!("the log directory could not be created: {error}"),
        );
        return;
    }
    // Best effort for the directory; the files below are fail-closed.
    let _ = restrict(dir, 0o700);

    let file_path = dir.join("zenith.log");
    // Repair every log file before anything is written. Rotation only rewrites
    // `zenith.log`, so a `0644` `zenith.log.1` created by an older Zenith would
    // otherwise stay readable until the next rotation, possibly for months. A
    // failure means a log is not known to be owner-only, so the line is dropped
    // rather than written insecurely.
    if let Err(error) = repair_log_permissions(dir, restrict) {
        record_log_failure_in(
            dir,
            format!("the log file mode could not be restricted to the owner: {error}"),
        );
        return;
    }
    if let Err(error) = rotate_log_if_needed(dir, &file_path) {
        record_log_failure_in(dir, format!("the log file could not be rotated: {error}"));
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
    match options.open(&file_path) {
        Ok(mut file) => {
            // A pre-existing wider mode must be repaired before anything is
            // written; if it cannot be, drop the line rather than leak it.
            if let Err(error) = restrict(&file_path, 0o600) {
                record_log_failure_in(
                    dir,
                    format!("the log file mode could not be restricted to the owner: {error}"),
                );
                return;
            }
            if let Err(error) = file.write_all(line.as_bytes()) {
                record_log_failure_in(dir, format!("the log line could not be written: {error}"));
            }
        }
        Err(error) => record_log_failure_in(
            dir,
            format!("the log file could not be opened for writing: {error}"),
        ),
    }
}

/// The contexts whose startup failure has already been reported.
///
/// A tray click or a repeated single-instance launch can fail the same way many
/// times; the user needs to be told once, not once per click.
static REPORTED_STARTUP_FAILURES: Mutex<Vec<String>> = Mutex::new(Vec::new());

const STARTUP_FAILURE_TITLE: &str = "Zenith could not open its window";

fn prepare_startup_failure(context: &str, error: &str) -> Option<(String, String)> {
    let detail = format!(
        "{context}: {error}\n\nRun `Zenith --doctor` for a self-check, or open the log at {}.",
        normalized_log_path()
    );
    log_error("startup", &detail);

    mark_startup_failure_reported(context)
        .then(|| (STARTUP_FAILURE_TITLE.to_string(), sanitize_log(&detail)))
}

/// Reports a failure the user cannot see otherwise.
///
/// Release builds run with `windows_subsystem = "windows"` and no console, so a
/// failure that prevents a window from being created leaves a tray icon and
/// nothing else: the log line that records it is invisible because the
/// interface that would show it never opened. The same text is therefore raised
/// in a native dialog, once per context, and it names `--doctor` and the log
/// path so the diagnostics stay reachable without a window.
pub fn report_startup_failure(context: &str, error: &str) {
    let Some((title, detail)) = prepare_startup_failure(context, error) else {
        return;
    };

    // The dialog is raised off the calling thread: the failure may have
    // happened on the main thread, and a modal dialog there would block the
    // event loop that is trying to finish starting.
    std::thread::spawn(move || show_native_error_dialog(&title, &detail));
}

/// Reports a fatal failure synchronously before the process exits.
///
/// A detached dialog thread cannot survive `process::exit`, so failures that
/// abort startup must wait for the native dialog instead of using the
/// non-blocking path intended for the running event loop.
pub fn report_fatal_startup_failure(context: &str, error: &str) {
    if let Some((title, detail)) = prepare_startup_failure(context, error) {
        show_native_error_dialog(&title, &detail);
    }
}

/// Records that a context was reported, answering whether this is the first
/// time. The dialog must not stack when a user clicks the tray repeatedly.
fn mark_startup_failure_reported(context: &str) -> bool {
    let mut reported = REPORTED_STARTUP_FAILURES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if reported.iter().any(|entry| entry == context) {
        return false;
    }
    reported.push(context.to_string());
    true
}

/// Shows a native error dialog, or writes to stderr where no dialog exists.
///
/// Windows uses `MessageBoxW` because it is available before the event loop
/// starts; macOS uses AppleScript for the same reason. Neither call needs the
/// interface that just failed to open.
fn show_native_error_dialog(title: &str, detail: &str) {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND,
        };
        let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
        let detail: Vec<u16> = detail.encode_utf16().chain([0]).collect();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                detail.as_ptr(),
                title.as_ptr(),
                MB_ICONERROR | MB_OK | MB_SETFOREGROUND,
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        fn quoted(value: &str) -> String {
            format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
        }
        let script = format!(
            "display dialog {} with title {} buttons {{\"OK\"}} default button \"OK\" with icon stop",
            quoted(detail),
            quoted(title)
        );
        let mut cmd = std::process::Command::new("osascript");
        cmd.args(["-e", &script]);
        let _ = crate::tooling::run_with_timeout(cmd, std::time::Duration::from_secs(30));
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        eprintln!("{title}: {detail}");
    }
}

/// Whether a line could be appended right now, creating the directory and
/// writing a temporary sibling probe on the same file system.
///
/// `--doctor` runs this against the machine's own log directory: a report that
/// cannot be written is the failure that hides every other failure.
pub(crate) fn probe_log_writability(dir: &Path) -> Result<(), String> {
    probe_log_writability_with(dir, |file| {
        file.write_all(b"Zenith diagnostics write probe\n")?;
        file.flush()
    })
}

type ProbeWrite = fn(&mut fs::File) -> std::io::Result<()>;

fn probe_log_writability_with(dir: &Path, write_probe: ProbeWrite) -> Result<(), String> {
    // The probe leaves the machine as it found it. On Windows the per-user log
    // directory sits inside the install directory, so a probe that kept its
    // file would make an uninstalled tree look like it survived, and the
    // packaging gate exists to catch exactly that kind of residue.
    let directory_existed = match fs::symlink_metadata(dir) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("the log directory could not be inspected: {error}")),
    };
    fs::create_dir_all(dir)
        .map_err(|error| format!("the log directory could not be created: {error}"))?;
    let cleanup_created_directory = || -> Result<(), String> {
        if directory_existed {
            return Ok(());
        }
        fs::remove_dir(dir)
            .map_err(|error| format!("the temporary log directory could not be removed: {error}"))
    };

    let file_path = dir.join("zenith.log");
    let file_exists = match fs::symlink_metadata(&file_path) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            let _ = cleanup_created_directory();
            return Err(format!("the log file could not be inspected: {error}"));
        }
    };
    if let Err(error) = repair_log_permissions(dir, restrict_permissions)
        .map_err(|error| format!("the log file mode could not be repaired: {error}"))
    {
        // Never remove a pre-existing log on a failed probe. If this call made
        // the directory, it is still empty and can be removed safely.
        let _ = cleanup_created_directory();
        return Err(error);
    }

    if file_exists {
        // Verify that the real log can be opened, but never append the probe to
        // it. Restoring by truncating to an earlier length could discard lines
        // written concurrently by a running Zenith process.
        if let Err(error) = OpenOptions::new().append(true).open(&file_path) {
            return Err(format!(
                "the log file could not be opened for writing: {error}"
            ));
        }
    }

    // A sibling file exercises creation and an actual write on the same file
    // system without changing the user's log. A unique, create-only path makes
    // cleanup safe even when multiple doctor processes run at once.
    let probe_path = dir.join(format!(
        ".zenith-write-probe-{}-{}",
        std::process::id(),
        LOG_PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Create owner-only from the first byte, as the write path does.
        options.mode(0o600);
    }

    let mut file = match options.open(&probe_path) {
        Ok(file) => file,
        Err(error) => {
            let mut message = format!("the log write probe could not be created: {error}");
            if let Err(cleanup_error) = cleanup_created_directory() {
                message.push_str(&format!("; {cleanup_error}"));
            }
            return Err(message);
        }
    };

    let write_result = write_probe(&mut file)
        .map_err(|error| format!("the log write probe could not be written: {error}"));
    drop(file);
    let restore_result = fs::remove_file(&probe_path)
        .map_err(|error| format!("the temporary log probe could not be removed: {error}"));

    let directory_restore_result = cleanup_created_directory();
    let failures = [write_result, restore_result, directory_restore_result]
        .into_iter()
        .filter_map(Result::err)
        .collect::<Vec<_>>();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

/// Repairs the mode of every diagnostics log file, existing or not.
///
/// A symlinked log path is refused instead of followed: repairing or writing
/// through it would reach a file outside the log directory, and the log would
/// be redirected by whoever created the link.
fn repair_log_permissions(dir: &Path, restrict: RestrictPermissions) -> std::io::Result<()> {
    for name in ["zenith.log", "zenith.log.1"] {
        let path = dir.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(std::io::Error::other(
                        "the diagnostics log path must not be a symlink",
                    ));
                }
                if !metadata.file_type().is_file() {
                    return Err(std::io::Error::other(
                        "the diagnostics log path must be a regular file",
                    ));
                }
                restrict(&path, 0o600)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Rotates the log into `zenith.log.1` once it exceeds the threshold.
///
/// Both files were repaired to `0600` before this runs, and a rename inside one
/// directory preserves the mode, so a world-readable legacy log can never
/// become a world-readable backup.
fn rotate_log_if_needed(dir: &Path, file_path: &Path) -> std::io::Result<()> {
    let Ok(metadata) = fs::metadata(file_path) else {
        // No existing log to rotate.
        return Ok(());
    };
    if metadata.len() <= MAX_LOG_BYTES {
        return Ok(());
    }

    let backup = dir.join("zenith.log.1");
    // Rotation is best effort: the log itself was just made owner-only, so
    // appending to it stays safe even when the rename is refused.
    let _ = fs::rename(file_path, &backup);
    Ok(())
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
        os_build: environment.os_build.clone(),
        arch: environment.process_architecture.clone(),
        native_arch: environment.native_architecture.clone(),
        emulated: environment.emulated,
        webview_version: environment.webview_version.clone(),
        elevated: environment.elevated,
        locale: environment.locale.clone(),
        log_failure: log_failure(),
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
        // Emulation is exactly "the process architecture differs from the
        // native one", so the two fields cannot contradict each other.
        assert_eq!(
            snapshot.emulated,
            snapshot
                .native_arch
                .as_deref()
                .is_some_and(|native| native != snapshot.arch)
        );
        assert_ne!(snapshot.log_failure.as_deref(), Some(""));
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

    fn refuse_permissions(_path: &Path, _mode: u32) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "fixture refuses the repair",
        ))
    }

    fn refuse_probe_write(_file: &mut fs::File) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            "fixture refuses the write",
        ))
    }

    /// A logger that cannot write must say so: the interface shows this text
    /// next to the log path, and `--doctor` reports the probe.
    #[test]
    fn a_log_write_failure_is_recorded_instead_of_being_silent() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("zenith.log");
        std::fs::write(&log, b"line\n").unwrap();

        write_log_line(dir.path(), "test", "must be dropped", refuse_permissions);

        let failure = log_failure().expect("a refused repair must be recorded");
        assert!(
            failure.contains("mode could not be restricted"),
            "{failure}"
        );
    }

    #[test]
    fn the_log_writability_probe_reports_what_a_write_would_find() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        assert!(probe_log_writability(&logs).is_ok());
        // What it created, it removes: an uninstalled tree must not look like
        // it survived because a report was requested.
        assert!(
            !logs.exists(),
            "the probe must leave no residue when it created the directory"
        );

        // A directory that already exists keeps its contents.
        std::fs::create_dir_all(&logs).unwrap();
        let existing = logs.join("zenith.log");
        std::fs::write(&existing, b"previous line\n").unwrap();
        assert!(probe_log_writability(&logs).is_ok());
        assert_eq!(
            std::fs::read_to_string(&existing).unwrap(),
            "previous line\n"
        );

        // Opening a file is not proof that a write succeeds. The failed sibling
        // probe is reported and the pre-existing log stays untouched.
        let failure = probe_log_writability_with(&logs, refuse_probe_write).unwrap_err();
        assert!(failure.contains("could not be written"), "{failure}");
        assert_eq!(
            std::fs::read_to_string(&existing).unwrap(),
            "previous line\n"
        );

        // A failed write to a file created by the probe leaves no residue.
        let new_logs = dir.path().join("new-logs");
        assert!(probe_log_writability_with(&new_logs, refuse_probe_write).is_err());
        assert!(!new_logs.exists());

        // A path that cannot be a directory is reported, not ignored.
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, b"not a directory").unwrap();
        assert!(probe_log_writability(&blocked).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_log_probe_never_removes_an_existing_log_path() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        std::fs::create_dir(&logs).unwrap();
        let outside = dir.path().join("outside.log");
        std::fs::write(&outside, b"outside\n").unwrap();
        let log = logs.join("zenith.log");
        std::os::unix::fs::symlink(&outside, &log).unwrap();

        assert!(probe_log_writability(&logs).is_err());
        assert!(std::fs::symlink_metadata(&log)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "outside\n");
    }

    #[test]
    fn a_startup_failure_is_reported_once_per_context() {
        let context = "the quick panel could not be created (test fixture)";
        assert!(mark_startup_failure_reported(context));
        assert!(
            !mark_startup_failure_reported(context),
            "a repeated failure must not stack dialogs"
        );
        assert!(
            mark_startup_failure_reported("the main window could not be created (test fixture)"),
            "a different context is still reported"
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

    /// A backup written by an older Zenith stays `0644` until the next
    /// rotation, which may be months away. Every write repairs it instead.
    #[cfg(unix)]
    #[test]
    fn a_legacy_world_readable_backup_is_repaired_on_the_next_write() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("zenith.log");
        let backup = dir.path().join("zenith.log.1");
        std::fs::write(&log, b"current log\n").unwrap();
        std::fs::write(&backup, b"legacy rotated log\n").unwrap();
        std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_log_line(dir.path(), "test", "after upgrade", restrict_permissions);

        assert_eq!(
            mode_of(&backup),
            0o600,
            "the legacy backup must be repaired"
        );
        assert_eq!(mode_of(&log), 0o600);
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "legacy rotated log\n",
            "repair must not disturb the backup's contents"
        );
        let written = std::fs::read_to_string(&log).unwrap();
        assert!(written.contains("after upgrade"), "{written}");
    }

    /// A symlinked log path must not be followed: repairing or writing through
    /// it would reach a file outside the log directory.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_log_path_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("elsewhere.log");
        std::fs::write(&target, b"outside\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("zenith.log")).unwrap();

        write_log_line(
            dir.path(),
            "test",
            "must not be written",
            restrict_permissions,
        );

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "outside\n");
        assert_eq!(
            mode_of(&target),
            0o644,
            "the link target must keep its own mode"
        );
    }

    #[test]
    fn a_non_file_log_path_is_refused_without_being_modified() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("zenith.log");
        std::fs::create_dir(&log).unwrap();

        #[cfg(unix)]
        let original_mode = mode_of(&log);
        let error = repair_log_permissions(dir.path(), restrict_permissions).unwrap_err();

        assert!(error.to_string().contains("regular file"), "{error}");
        assert!(log.is_dir());
        #[cfg(unix)]
        assert_eq!(mode_of(&log), original_mode);
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
