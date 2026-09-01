use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum SubprocessError {
    Timeout(String, Duration),
    SpawnFailed(String, std::io::Error),
    WaitFailed(String, std::io::Error),
}

impl std::fmt::Display for SubprocessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout(cmd, dur) => write!(f, "Command `{cmd}` timed out after {dur:?}"),
            Self::SpawnFailed(cmd, err) => write!(f, "Failed to spawn `{cmd}`: {err}"),
            Self::WaitFailed(cmd, err) => write!(f, "Failed to wait on `{cmd}`: {err}"),
        }
    }
}

impl std::error::Error for SubprocessError {}

/// Resolves command-line tools from both the inherited PATH and common macOS
/// installation locations. Finder-launched applications receive a minimal PATH,
/// so relying on `Command::new("tool")` alone makes installed tools disappear.
pub fn command(name: &str) -> Command {
    Command::new(resolve(name).unwrap_or_else(|| PathBuf::from(name)))
}

#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::sync::mpsc;

/// Runs a command in an isolated process group or Windows Job Object with a strict timeout and pipe draining to prevent deadlock.
pub fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<Output, SubprocessError> {
    let program = cmd.get_program().to_string_lossy().to_string();
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    #[cfg(unix)]
    cmd.process_group(0);

    #[cfg(target_os = "windows")]
    let job_handle = unsafe {
        use windows_sys::Win32::System::JobObjects::*;
        let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if !handle.is_null() {
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
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
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx_out.send(chunk[..n].to_vec()).is_err() {
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
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx_err.send(chunk[..n].to_vec()).is_err() {
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
                Ok(chunk) => collected.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    finished = true;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }

        while let Ok(chunk) = rx.try_recv() {
            collected.extend_from_slice(&chunk);
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

pub fn resolve(name: &str) -> Option<PathBuf> {
    let mut candidates = env::var_os("PATH")
        .map(|value| env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();

    #[cfg(target_os = "macos")]
    {
        candidates.extend([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
        ]);

        if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
            candidates.extend([
                home.join(".local/bin"),
                home.join(".cargo/bin"),
                home.join(".npm-global/bin"),
            ]);
        }

        match name {
            "docker" => candidates.push(PathBuf::from(
                "/Applications/Docker.app/Contents/Resources/bin",
            )),
            "ollama" => {
                candidates.push(PathBuf::from("/Applications/Ollama.app/Contents/Resources"))
            }
            _ => {}
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(prog_files) = env::var_os("ProgramFiles").map(PathBuf::from) {
            candidates.extend([
                prog_files.join("Docker\\Docker\\resources\\bin"),
                prog_files.join("Git\\cmd"),
                prog_files.join("Git\\bin"),
                prog_files.join("nodejs"),
            ]);
        }

        if let Some(local_appdata) = env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            candidates.extend([
                local_appdata.join("Programs\\Ollama"),
                local_appdata.join("Programs\\Python\\Launcher"),
            ]);
        }

        if let Some(user_profile) = env::var_os("USERPROFILE").map(PathBuf::from) {
            candidates.extend([
                user_profile.join(".cargo\\bin"),
                user_profile.join("AppData\\Roaming\\npm"),
                user_profile.join(".gemini\\antigravity-cli\\bin"),
            ]);
        }
    }

    // Direct match or with standard extensions on Windows
    let name_variations: Vec<String> = if cfg!(windows) && !name.contains('.') {
        vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            format!("{name}.bat"),
            name.to_string(),
        ]
    } else {
        vec![name.to_string()]
    };

    for directory in candidates {
        for variation in &name_variations {
            let candidate = directory.join(variation);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
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

#[cfg(all(test, unix))]
mod tests {
    use super::is_executable;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

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
}
