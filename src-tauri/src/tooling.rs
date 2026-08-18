use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Resolves command-line tools from both the inherited PATH and common macOS
/// installation locations. Finder-launched applications receive a minimal PATH,
/// so relying on `Command::new("tool")` alone makes installed tools disappear.
pub fn command(name: &str) -> Command {
    Command::new(resolve(name).unwrap_or_else(|| PathBuf::from(name)))
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
}
