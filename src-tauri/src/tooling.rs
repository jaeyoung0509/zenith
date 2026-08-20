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

/// Runs a command with a strict timeout and pipe draining to prevent deadlock.
pub fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<Output, SubprocessError> {
    let program = cmd.get_program().to_string_lossy().to_string();
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| SubprocessError::SpawnFailed(program.clone(), e))?;

    let mut stdout_stream = child.stdout.take();
    let mut stderr_stream = child.stderr.take();

    let stdout_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut stream) = stdout_stream.take() {
            let _ = stream.read_to_end(&mut buf);
        }
        buf
    });

    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut stream) = stderr_stream.take() {
            let _ = stream.read_to_end(&mut buf);
        }
        buf
    });

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = stdout_handle.join().unwrap_or_default();
                let stderr = stderr_handle.join().unwrap_or_default();
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
                    let _ = stdout_handle.join();
                    let _ = stderr_handle.join();
                    return Err(SubprocessError::Timeout(program, timeout));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_handle.join();
                let _ = stderr_handle.join();
                return Err(SubprocessError::WaitFailed(program, e));
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
}
