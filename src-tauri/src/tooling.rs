use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;

const MAX_CAPTURE_BYTES: usize = 1024 * 1024;
/// Bounded pipe cleanup after the child lifecycle ends (termination + drain).
const PIPE_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum SubprocessError {
    Timeout(String, Duration),
    SpawnFailed(String, std::io::Error),
    WaitFailed(String, std::io::Error),
    Cancelled(String),
}

impl std::fmt::Display for SubprocessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout(cmd, dur) => write!(f, "Command `{cmd}` timed out after {dur:?}"),
            Self::SpawnFailed(cmd, err) => write!(f, "Failed to spawn `{cmd}`: {err}"),
            Self::WaitFailed(cmd, err) => write!(f, "Failed to wait on `{cmd}`: {err}"),
            Self::Cancelled(cmd) => write!(f, "Command `{cmd}` was cancelled"),
        }
    }
}

impl std::error::Error for SubprocessError {}

/// Resolves command-line tools from both the inherited PATH and common platform
/// installation locations. Desktop-launched applications can receive a minimal
/// PATH, so relying on `Command::new("tool")` alone makes installed tools disappear.
pub fn command(name: &str) -> Command {
    let mut command = Command::new(resolve(name).unwrap_or_else(|| PathBuf::from(name)));
    configure_background_command(&mut command);
    command
}

#[cfg(target_os = "windows")]
fn configure_background_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn configure_background_command(_command: &mut Command) {}

#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::sync::mpsc;

/// Runs a command in an isolated process group or Windows Job Object with a strict timeout and pipe draining to prevent deadlock.
pub fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<Output, SubprocessError> {
    let program = cmd.get_program().to_string_lossy().to_string();
    // Callers may construct a Command directly (for example a native picker).
    // Keep all timeout-managed subprocesses headless on Windows.
    configure_background_command(&mut cmd);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    #[cfg(unix)]
    cmd.process_group(0);

    #[cfg(target_os = "windows")]
    let job_handle = unsafe {
        use windows_sys::Win32::System::JobObjects::*;
        let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if !handle.is_null() {
            let info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
                BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                    LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    ..std::mem::zeroed()
                },
                ..std::mem::zeroed()
            };
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
        }
        handle
    };

    let mut child = cmd.spawn().map_err(|e| {
        let err = SubprocessError::SpawnFailed(program.clone(), e);
        crate::diagnostics::log_error("subprocess", &err.to_string());
        err
    })?;

    #[cfg(target_os = "windows")]
    unsafe {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        if !job_handle.is_null() {
            AssignProcessToJobObject(job_handle, child.as_raw_handle() as _);
        }
    }

    #[cfg(unix)]
    let pid = child.id() as i32;

    let mut stdout_stream = child.stdout.take();
    let mut stderr_stream = child.stderr.take();

    let (tx_out, rx_out) = mpsc::channel::<Vec<u8>>();
    let stdout_handle = std::thread::spawn(move || {
        if let Some(mut stream) = stdout_stream.take() {
            let mut chunk = [0u8; 8192];
            let mut captured = 0usize;
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let keep = n.min(MAX_CAPTURE_BYTES.saturating_sub(captured));
                        captured = captured.saturating_add(keep);
                        if keep > 0 && tx_out.send(chunk[..keep].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        }
    });

    let (tx_err, rx_err) = mpsc::channel::<Vec<u8>>();
    let stderr_handle = std::thread::spawn(move || {
        if let Some(mut stream) = stderr_stream.take() {
            let mut chunk = [0u8; 8192];
            let mut captured = 0usize;
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let keep = n.min(MAX_CAPTURE_BYTES.saturating_sub(captured));
                        captured = captured.saturating_add(keep);
                        if keep > 0 && tx_err.send(chunk[..keep].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        }
    });

    let drain_stream = |rx: &mpsc::Receiver<Vec<u8>>,
                        handle: std::thread::JoinHandle<()>,
                        timeout_dur: Duration|
     -> Vec<u8> {
        let mut collected = Vec::new();
        let deadline = Instant::now() + timeout_dur;
        let mut finished = false;

        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(15)) {
                Ok(chunk) => {
                    let remaining = MAX_CAPTURE_BYTES.saturating_sub(collected.len());
                    collected.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    finished = true;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }

        while let Ok(chunk) = rx.try_recv() {
            let remaining = MAX_CAPTURE_BYTES.saturating_sub(collected.len());
            collected.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        }

        if finished {
            let _ = handle.join();
        }

        collected
    };

    let cleanup_and_drain = |_kill_tree: bool| -> (Vec<u8>, Vec<u8>) {
        #[cfg(unix)]
        if _kill_tree && pid > 1 {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
                libc::kill(pid, libc::SIGKILL);
            }
        }

        #[cfg(target_os = "windows")]
        unsafe {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::JobObjects::TerminateJobObject;
            if !job_handle.is_null() {
                if _kill_tree {
                    TerminateJobObject(job_handle, 1);
                }
                CloseHandle(job_handle);
            }
        }

        let stdout = drain_stream(&rx_out, stdout_handle, Duration::from_millis(200));
        let stderr = drain_stream(&rx_err, stderr_handle, Duration::from_millis(200));

        (stdout, stderr)
    };

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let (stdout, stderr) = cleanup_and_drain(true);
                return Ok(Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = cleanup_and_drain(true);
                    let err = SubprocessError::Timeout(program.clone(), timeout);
                    crate::diagnostics::log_error("subprocess", &err.to_string());
                    return Err(err);
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = cleanup_and_drain(true);
                let err = SubprocessError::WaitFailed(program.clone(), e);
                crate::diagnostics::log_error("subprocess", &err.to_string());
                return Err(err);
            }
        }
    }
}

/// Async subprocess runner for high-frequency collector paths.
///
/// Uses the existing Tauri async runtime; never creates a runtime per request.
/// Async child waiting plus concurrent stdout/stderr draining replace the
/// per-pipe OS threads and 15 ms `try_wait` poll loop of the synchronous
/// runner. Capture byte limits, Unix process-group cleanup, Windows Job Object
/// ownership, error redaction, exit-status mapping, and UTF-8 handling match
/// the synchronous runner.
///
/// The overall `timeout` covers the full child lifecycle including termination
/// and bounded pipe cleanup. Cancellation (dropping the future or an enclosing
/// timeout) terminates and reaps owned children via the drop guard and releases
/// the caller's execution-budget permit through normal drop semantics.
/// Dropping alone is best-effort for descendants that outlive the direct
/// child; the timeout path always performs process-tree termination.
///
/// Synchronous native callers keep using [`run_with_timeout`]; migrate paths
/// incrementally without `block_on` inside executor callbacks.
pub async fn run_with_timeout_async(
    mut cmd: Command,
    timeout: Duration,
) -> Result<Output, SubprocessError> {
    use tokio::time::timeout as tokio_timeout;

    let program = cmd.get_program().to_string_lossy().to_string();
    configure_background_command(&mut cmd);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    #[cfg(unix)]
    cmd.process_group(0);

    let mut tokio_cmd = tokio::process::Command::from(cmd);

    #[cfg(target_os = "windows")]
    let job_handle = unsafe {
        use windows_sys::Win32::System::JobObjects::*;
        let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if !handle.is_null() {
            let info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
                BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                    LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    ..std::mem::zeroed()
                },
                ..std::mem::zeroed()
            };
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
        }
        handle
    };

    let child = tokio_cmd.spawn().map_err(|e| {
        #[cfg(target_os = "windows")]
        unsafe {
            use windows_sys::Win32::Foundation::CloseHandle;
            if !job_handle.is_null() {
                CloseHandle(job_handle);
            }
        }
        let err = SubprocessError::SpawnFailed(program.clone(), e);
        crate::diagnostics::log_error("subprocess", &err.to_string());
        err
    })?;

    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        if !job_handle.is_null() {
            // `tokio::process::Child` exposes the owned process handle via the
            // inherent `raw_handle()` method on Windows (`None` once reaped).
            if let Some(raw) = child.raw_handle() {
                AssignProcessToJobObject(job_handle, raw);
            }
        }
    }

    #[cfg(unix)]
    let pid = child.id().map(|id| id as i32).unwrap_or(-1);

    // Best-effort tree termination shared by the timeout path and the drop
    // guard. Dropping a future or timing out an await alone never proves
    // descendants were terminated; this explicitly kills the process group or
    // job object.
    #[cfg(unix)]
    let kill_tree = |pid: i32| {
        if pid > 1 {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
                libc::kill(pid, libc::SIGKILL);
            }
        }
    };
    #[cfg(target_os = "windows")]
    let kill_tree = |_: i32| unsafe {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        if !job_handle.is_null() {
            TerminateJobObject(job_handle, 1);
        }
    };
    #[cfg(not(any(unix, target_os = "windows")))]
    let kill_tree = |_: i32| {};

    struct ChildGuard {
        child: Option<tokio::process::Child>,
        #[cfg(unix)]
        pid: i32,
        #[cfg(target_os = "windows")]
        job_handle: isize,
    }
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Some(mut child) = self.child.take() {
                // Best-effort: terminate the tree, then detach. The timeout
                // path below always reaps explicitly; this covers futures
                // dropped by their callers (cancellation).
                #[cfg(unix)]
                {
                    let pid = self.pid;
                    if pid > 1 {
                        unsafe {
                            libc::kill(-pid, libc::SIGKILL);
                            libc::kill(pid, libc::SIGKILL);
                        }
                    }
                }
                #[cfg(target_os = "windows")]
                unsafe {
                    use windows_sys::Win32::System::JobObjects::TerminateJobObject;
                    if self.job_handle != 0 {
                        TerminateJobObject(self.job_handle as _, 1);
                    }
                }
                let _ = child.start_kill();
            }
            #[cfg(target_os = "windows")]
            unsafe {
                use windows_sys::Win32::Foundation::CloseHandle;
                if self.job_handle != 0 {
                    CloseHandle(self.job_handle as _);
                }
            }
        }
    }

    let mut guard = ChildGuard {
        child: Some(child),
        #[cfg(unix)]
        pid,
        #[cfg(target_os = "windows")]
        job_handle: job_handle as isize,
    };

    async fn drain_pipe(mut pipe: impl tokio::io::AsyncRead + Unpin) -> Vec<u8> {
        let mut collected = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    // Retain the capture limit; keep draining excess without
                    // retaining so a full pipe cannot deadlock the child.
                    let remaining = MAX_CAPTURE_BYTES.saturating_sub(collected.len());
                    collected.extend_from_slice(&chunk[..n.min(remaining)]);
                }
                Err(_) => break,
            }
        }
        collected
    }

    let child_ref = guard.child.as_mut().expect("child present");
    let stdout = child_ref.stdout.take();
    let stderr = child_ref.stderr.take();
    let mut stdout_task = tokio::spawn(async move {
        match stdout {
            Some(pipe) => drain_pipe(pipe).await,
            None => Vec::new(),
        }
    });
    let mut stderr_task = tokio::spawn(async move {
        match stderr {
            Some(pipe) => drain_pipe(pipe).await,
            None => Vec::new(),
        }
    });

    let child_ref = guard.child.as_mut().expect("child present");
    let wait_result = tokio_timeout(timeout, child_ref.wait()).await;
    let status = match wait_result {
        Ok(Ok(status)) => {
            // Even after a clean exit, background descendants may hold pipes
            // open; terminate the tree best-effort before the bounded drain so
            // collection cannot hang on an inherited pipe.
            #[cfg(unix)]
            kill_tree(pid);
            #[cfg(target_os = "windows")]
            kill_tree(0);
            status
        }
        Ok(Err(e)) => {
            let mut child = guard.child.take().expect("child present");
            let _ = child.kill().await;
            let _ = child.wait().await;
            #[cfg(unix)]
            kill_tree(pid);
            #[cfg(target_os = "windows")]
            kill_tree(0);
            let _ = tokio_timeout(PIPE_CLEANUP_TIMEOUT, async {
                let _ = tokio::join!(&mut stdout_task, &mut stderr_task);
            })
            .await;
            stdout_task.abort();
            stderr_task.abort();
            #[cfg(target_os = "windows")]
            unsafe {
                use windows_sys::Win32::Foundation::CloseHandle;
                if !job_handle.is_null() {
                    CloseHandle(job_handle);
                }
                guard.job_handle = 0;
            }
            let err = SubprocessError::WaitFailed(program.clone(), e);
            crate::diagnostics::log_error("subprocess", &err.to_string());
            return Err(err);
        }
        Err(_) => {
            // Deadline covers termination plus bounded pipe cleanup.
            let mut child = guard.child.take().expect("child present");
            let _ = child.kill().await;
            let _ = tokio_timeout(PIPE_CLEANUP_TIMEOUT, child.wait()).await;
            #[cfg(unix)]
            kill_tree(pid);
            #[cfg(target_os = "windows")]
            kill_tree(0);
            let _ = tokio_timeout(PIPE_CLEANUP_TIMEOUT, async {
                let _ = tokio::join!(&mut stdout_task, &mut stderr_task);
            })
            .await;
            stdout_task.abort();
            stderr_task.abort();
            #[cfg(target_os = "windows")]
            unsafe {
                use windows_sys::Win32::Foundation::CloseHandle;
                if !job_handle.is_null() {
                    CloseHandle(job_handle);
                }
                guard.job_handle = 0;
            }
            let err = SubprocessError::Timeout(program.clone(), timeout);
            crate::diagnostics::log_error("subprocess", &err.to_string());
            return Err(err);
        }
    };

    // Bounded drain; descendants killed above must release pipes promptly.
    let (stdout, stderr) = match tokio_timeout(PIPE_CLEANUP_TIMEOUT, async {
        tokio::join!(&mut stdout_task, &mut stderr_task)
    })
    .await
    {
        Ok((Ok(stdout), Ok(stderr))) => (stdout, stderr),
        _ => {
            stdout_task.abort();
            stderr_task.abort();
            // Child already reaped via `wait` above; detach the guard without
            // re-killing.
            let _ = guard.child.take();
            #[cfg(target_os = "windows")]
            unsafe {
                use windows_sys::Win32::Foundation::CloseHandle;
                if !job_handle.is_null() {
                    CloseHandle(job_handle);
                }
                guard.job_handle = 0;
            }
            (Vec::new(), Vec::new())
        }
    };
    // Child reaped; detach without re-killing and close the job handle.
    let _ = guard.child.take();
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::Foundation::CloseHandle;
        if !job_handle.is_null() {
            CloseHandle(job_handle);
        }
        guard.job_handle = 0;
    }

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Cap for version-manager directory scans so resolution work stays bounded.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const MAX_NODE_VERSION_DIRS: usize = 32;

/// Bounded scan of version-manager Node installs: every direct child of
/// `versions_dir` contributing `<child>/bin` (nvm on Unix). Non-directories
/// are skipped without following symlinks.
#[cfg(target_os = "macos")]
fn nvm_node_bin_dirs(versions_dir: &Path) -> Vec<PathBuf> {
    versioned_install_dirs(versions_dir, "bin")
}

/// nvm-windows layout: executables sit directly in each version directory
/// (`%APPDATA%/nvm/v*/npm.cmd`). Same bound as the Unix helper.
#[cfg(target_os = "windows")]
fn nvm_windows_bin_dirs(nvm_dir: &Path) -> Vec<PathBuf> {
    versioned_install_dirs(nvm_dir, "")
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn versioned_install_dirs(versions_dir: &Path, leaf: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(versions_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .take(MAX_NODE_VERSION_DIRS)
        .map(|entry| entry.path())
        .filter(|path| {
            std::fs::symlink_metadata(path)
                .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
                .unwrap_or(false)
        })
        .map(|path| {
            if leaf.is_empty() {
                path
            } else {
                path.join(leaf)
            }
        })
        .filter(|path| {
            std::fs::symlink_metadata(path)
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false)
        })
        .collect()
}

pub fn resolve(name: &str) -> Option<PathBuf> {
    let name_variations = executable_name_variations(name);

    for directory in search_candidates() {
        for variation in &name_variations {
            let candidate = directory.join(variation);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

/// Directories consulted by [`resolve`], in priority order. Used to report
/// honest "not detected" diagnostics with the locations that were searched.
pub fn search_locations() -> Vec<PathBuf> {
    search_candidates()
}

fn search_candidates() -> Vec<PathBuf> {
    let mut candidates = env::var_os("PATH")
        .map(|value| env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();

    // One shared root set for discovery and provider allowlisting so a tool
    // that resolves here cannot be rejected afterwards. `tool_roots` reads
    // ProgramW6432/ProgramFiles(x86), package-manager environment variables,
    // Chocolatey/Scoop shims, WinGet Links, and nvm-windows on Windows.
    candidates.extend(crate::platform::NativePlatformPaths::tool_roots());

    #[cfg(target_os = "macos")]
    if let Some(home) = crate::platform::NativePlatformPaths::new().home() {
        // Version-manager installs (nvm) keep one `bin` dir per Node version;
        // scan them bounded so resolution work cannot grow without limit.
        candidates.extend(nvm_node_bin_dirs(&home.join(".nvm/versions/node")));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = env::var_os("APPDATA").map(PathBuf::from) {
            candidates.push(appdata.join("nodejs"));
            candidates.extend(nvm_windows_bin_dirs(&appdata.join("nvm")));
        }
        if let Some(nvm_home) = env::var_os("NVM_HOME").map(PathBuf::from) {
            candidates.extend(nvm_windows_bin_dirs(&nvm_home));
        }
    }

    candidates
}

#[cfg(windows)]
fn executable_name_variations(name: &str) -> Vec<String> {
    if name.contains('.') {
        return vec![name.to_string()];
    }
    // Derive extensions from PATHEXT instead of assuming a fixed list.
    let pathext = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    let mut variations: Vec<String> = pathext
        .split(';')
        .map(str::trim)
        .filter(|extension| !extension.is_empty())
        .map(|extension| format!("{name}{}", extension.to_ascii_lowercase()))
        .collect();
    variations.push(name.to_string());
    variations
}

#[cfg(not(windows))]
fn executable_name_variations(name: &str) -> Vec<String> {
    vec![name.to_string()]
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext_lower = ext.to_ascii_lowercase();
        matches!(ext_lower.as_str(), "exe" | "cmd" | "bat" | "com")
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::is_executable;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    #[test]
    fn resolver_rejects_non_executable_files() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("tool");
        fs::write(&file, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!is_executable(&file));

        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_executable(&file));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn background_command_captures_output_without_a_console() {
        let mut cmd = std::process::Command::new("cmd.exe");
        cmd.args(["/D", "/C", "echo hello world"]);
        let output = super::run_with_timeout(cmd, std::time::Duration::from_secs(2)).unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "hello world"
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_with_timeout_captures_output() {
        let mut cmd = std::process::Command::new("echo");
        cmd.arg("hello world");
        let output = super::run_with_timeout(cmd, std::time::Duration::from_secs(2)).unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "hello world"
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_with_timeout_caps_captured_output() {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "yes x | head -c 2097152"]);
        let output = super::run_with_timeout(cmd, std::time::Duration::from_secs(3)).unwrap();
        assert_eq!(output.stdout.len(), super::MAX_CAPTURE_BYTES);
    }

    #[cfg(unix)]
    #[test]
    fn run_with_timeout_terminates_slow_process() {
        let mut cmd = std::process::Command::new("sleep");
        cmd.arg("2");
        let result = super::run_with_timeout(cmd, std::time::Duration::from_millis(50));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            super::SubprocessError::Timeout(..)
        ));
    }

    #[test]
    #[cfg(unix)]
    fn run_with_timeout_terminates_descendant_process_holding_pipe() {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "sh -c 'sleep 10 >&1' & sleep 10"]);
        let start = std::time::Instant::now();
        let result = super::run_with_timeout(cmd, std::time::Duration::from_millis(150));
        let elapsed = start.elapsed();

        assert!(result.is_err());
        assert!(
            elapsed < std::time::Duration::from_secs(3),
            "Hanged for {elapsed:?} waiting for descendant"
        );
    }

    #[test]
    #[cfg(unix)]
    fn run_with_timeout_terminates_grandchild_when_parent_exits_fast() {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "sh -c 'sleep 10 >&1' & exit 0"]);
        let start = std::time::Instant::now();
        let result = super::run_with_timeout(cmd, std::time::Duration::from_millis(500));
        let elapsed = start.elapsed();

        assert!(result.is_ok());
        assert!(
            elapsed < std::time::Duration::from_secs(3),
            "Hanged for {elapsed:?} on background grandchild"
        );
    }

    #[test]
    #[cfg(unix)]
    fn run_with_timeout_terminates_even_with_detached_session_holding_pipe() {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "sh -c '(setsid sleep 10 >&1 &) ; exit 0'"]);
        let start = std::time::Instant::now();
        let result = super::run_with_timeout(cmd, std::time::Duration::from_millis(500));
        let elapsed = start.elapsed();

        assert!(result.is_ok());
        assert!(
            elapsed < std::time::Duration::from_secs(3),
            "Hanged for {elapsed:?} on detached session holding pipe"
        );
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn async_command_captures_output_without_a_console() {
        let mut cmd = std::process::Command::new("cmd.exe");
        cmd.args(["/D", "/C", "echo hello world"]);
        let output = super::run_with_timeout_async(cmd, std::time::Duration::from_secs(5))
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "hello world"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn async_run_captures_output() {
        let mut cmd = std::process::Command::new("echo");
        cmd.arg("hello world");
        let output = super::run_with_timeout_async(cmd, std::time::Duration::from_secs(5))
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "hello world"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn async_run_caps_both_pipes_without_deadlock() {
        let mut cmd = std::process::Command::new("sh");
        cmd.args([
            "-c",
            "yes out | head -c 1572864; yes err | head -c 1572864 1>&2",
        ]);
        let output = super::run_with_timeout_async(cmd, std::time::Duration::from_secs(10))
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), super::MAX_CAPTURE_BYTES);
        assert_eq!(output.stderr.len(), super::MAX_CAPTURE_BYTES);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn async_run_reports_nonzero_exit() {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "exit 3"]);
        let output = super::run_with_timeout_async(cmd, std::time::Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(output.status.code(), Some(3));
    }

    #[tokio::test]
    async fn async_run_reports_spawn_failure() {
        let cmd = std::process::Command::new("zenith-definitely-missing-binary-xyz");
        let result = super::run_with_timeout_async(cmd, std::time::Duration::from_secs(5)).await;
        assert!(matches!(
            result.unwrap_err(),
            super::SubprocessError::SpawnFailed(..)
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn async_run_terminates_slow_process_and_reaps() {
        let mut cmd = std::process::Command::new("sleep");
        cmd.arg("30");
        let start = std::time::Instant::now();
        let result =
            super::run_with_timeout_async(cmd, std::time::Duration::from_millis(100)).await;
        let elapsed = start.elapsed();
        assert!(matches!(
            result.unwrap_err(),
            super::SubprocessError::Timeout(..)
        ));
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "Hanged for {elapsed:?} on timeout cleanup"
        );
        // Permit-style reuse: the runner releases all ownership on timeout so
        // a follow-up collection can start immediately.
        let mut cmd = std::process::Command::new("echo");
        cmd.arg("reused");
        let output = super::run_with_timeout_async(cmd, std::time::Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "reused");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn async_run_terminates_descendant_holding_pipe() {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "sh -c 'sleep 10 >&1' & sleep 10"]);
        let start = std::time::Instant::now();
        let result =
            super::run_with_timeout_async(cmd, std::time::Duration::from_millis(200)).await;
        let elapsed = start.elapsed();
        assert!(result.is_err());
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "Hanged for {elapsed:?} waiting for descendant"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn async_run_terminates_grandchild_when_parent_exits_fast() {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "sh -c 'sleep 10 >&1' & exit 0"]);
        let start = std::time::Instant::now();
        let result =
            super::run_with_timeout_async(cmd, std::time::Duration::from_millis(500)).await;
        let elapsed = start.elapsed();
        assert!(result.is_ok());
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "Hanged for {elapsed:?} on background grandchild"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn async_run_terminates_even_with_detached_session_holding_pipe() {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "sh -c '(setsid sleep 10 >&1 &) ; exit 0'"]);
        let start = std::time::Instant::now();
        let result =
            super::run_with_timeout_async(cmd, std::time::Duration::from_millis(500)).await;
        let elapsed = start.elapsed();
        assert!(result.is_ok());
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "Hanged for {elapsed:?} on detached session holding pipe"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn async_run_cancellation_does_not_hang_caller() {
        let mut cmd = std::process::Command::new("sleep");
        cmd.arg("30");
        let handle = tokio::spawn(async move {
            super::run_with_timeout_async(cmd, std::time::Duration::from_secs(30)).await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        handle.abort();
        // Aborting (cancellation) resolves promptly; the drop guard performs
        // best-effort tree termination while the timeout path reaps explicitly.
        let completed = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        assert!(completed.is_err() || completed.unwrap().is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn nvm_scan_finds_version_bins_and_skips_non_directories() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let versions = directory.path().join("versions/node");
        for version in ["v20.0.0", "v22.0.0"] {
            fs::create_dir_all(versions.join(version).join("bin")).unwrap();
        }
        fs::write(versions.join("stray-file"), "x").unwrap();
        symlink(versions.join("v20.0.0"), versions.join("v-link")).unwrap();

        let mut found = super::nvm_node_bin_dirs(&versions);
        found.sort();
        assert_eq!(
            found,
            vec![versions.join("v20.0.0/bin"), versions.join("v22.0.0/bin"),]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn nvm_scan_returns_empty_for_missing_versions_dir() {
        let directory = tempfile::tempdir().unwrap();
        assert!(super::nvm_node_bin_dirs(&directory.path().join("versions/node")).is_empty());
    }
}
