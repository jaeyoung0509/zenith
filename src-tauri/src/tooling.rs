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
use std::os::unix::io::AsRawFd;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::sync::mpsc;

/// Runs a command in an isolated process group with a strict timeout and pipe draining to prevent deadlock.
pub fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<Output, SubprocessError> {
    let program = cmd.get_program().to_string_lossy().to_string();
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn().map_err(|e| {
        let err = SubprocessError::SpawnFailed(program.clone(), e);
        crate::diagnostics::log_error("subprocess", &err.to_string());
        err
    })?;

    let pid = child.id() as i32;

    let mut stdout_stream = child.stdout.take();
    let mut stderr_stream = child.stderr.take();

    #[cfg(unix)]
    let stdout_fd = stdout_stream.as_ref().map(|s| s.as_raw_fd());
    #[cfg(unix)]
    let stderr_fd = stderr_stream.as_ref().map(|s| s.as_raw_fd());

    let (tx_out, rx_out) = mpsc::channel();
    let stdout_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut stream) = stdout_stream.take() {
            let _ = stream.read_to_end(&mut buf);
        }
        let _ = tx_out.send(buf);
    });

    let (tx_err, rx_err) = mpsc::channel();
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut stream) = stderr_stream.take() {
            let _ = stream.read_to_end(&mut buf);
        }
        let _ = tx_err.send(buf);
    });

    let cleanup_and_drain = |kill_tree: bool| -> (Vec<u8>, Vec<u8>) {
        #[cfg(unix)]
        if kill_tree && pid > 1 {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
                libc::kill(pid, libc::SIGKILL);
            }
        }

        // Wait up to 100ms for pipes to drain naturally
        let stdout = match rx_out.recv_timeout(Duration::from_millis(100)) {
            Ok(buf) => buf,
            Err(_) => {
                #[cfg(unix)]
                if let Some(fd) = stdout_fd {
                    unsafe {
                        libc::close(fd);
                    }
                }
                rx_out
                    .recv_timeout(Duration::from_millis(50))
                    .unwrap_or_default()
            }
        };

        let stderr = match rx_err.recv_timeout(Duration::from_millis(100)) {
            Ok(buf) => buf,
            Err(_) => {
                #[cfg(unix)]
                if let Some(fd) = stderr_fd {
                    unsafe {
                        libc::close(fd);
                    }
                }
                rx_err
                    .recv_timeout(Duration::from_millis(50))
                    .unwrap_or_default()
            }
        };

        let _ = stdout_handle.join();
        let _ = stderr_handle.join();

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
        "ollama" => candidates.push(PathBuf::from("/Applications/Ollama.app/Contents/Resources")),
        _ => {}
    }

    candidates
        .into_iter()
        .map(|directory| directory.join(name))
        .find(|candidate| is_executable(candidate))
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
    path.is_file()
}

#[cfg(test)]
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
}
