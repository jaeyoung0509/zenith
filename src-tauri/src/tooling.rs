#[cfg(windows)]
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Resolves command-line tools from both the inherited PATH and common platform
/// installation locations. Desktop-launched applications can receive a minimal
/// PATH, so relying on `Command::new("tool")` alone makes installed tools disappear.
pub fn command(name: &str) -> Command {
    command_with(name, &zenith_platform::PlatformEnvironment::native())
}

/// Environment-aware [`command`].
pub fn command_with(name: &str, environment: &zenith_platform::PlatformEnvironment) -> Command {
    let mut command =
        Command::new(resolve_with(name, environment).unwrap_or_else(|| PathBuf::from(name)));
    zenith_platform::subprocess::configure_background_command(&mut command);
    command
}

/// The empty tree object, used as the attribute source for every Git call.
///
/// `attr.tree` (Git 2.43+) decides which tree `.gitattributes` files are read
/// from. Pinning it to the empty tree keeps a repository that Zenith did not
/// create from naming a filter driver: `filter.<driver>.clean` and `.smudge`
/// are programs Git runs while reading working-tree content, and the driver is
/// selected by a `.gitattributes` file that the repository supplies. Git 2.43's
/// own release notes document this variable, and pointing it at the empty tree
/// is the documented way to stop attribute lookup from reaching the repository.
/// Older Git ignores the key, which `docs/THREAT_MODEL.md` records as the one
/// residual in the neutralization below.
const EMPTY_TREE_OBJECT: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// A path no repository can turn into a hooks or attributes directory: `/dev/null`
/// and `NUL` are character devices, not directories, on their platforms.
#[cfg(windows)]
const INERT_PATH: &str = "NUL";
#[cfg(not(windows))]
const INERT_PATH: &str = "/dev/null";

/// A program name that cannot resolve to anything. `core.sshCommand` is only
/// consulted for transport, which Zenith never uses; if that ever changes, the
/// invocation fails closed instead of running a repository-supplied program.
const DISABLED_SSH_COMMAND: &str = "zenith-disabled-ssh";

/// Every `git` invocation in Zenith goes through this constructor.
///
/// Git reads the repository's own `.git/config` whenever it operates on a
/// repository, and several documented keys name a program Git then runs:
/// `core.fsmonitor` is consulted by `git status` specifically, and
/// `core.pager`, `core.hooksPath`, `diff.external`, `core.sshCommand`, and the
/// `filter.*` clean/smudge pair are the same class. The directories are not
/// nominated by the user — `root` comes from an observed agent process working
/// directory — so the invocation neutralizes each of those keys on the command
/// line, where Git gives them precedence over the repository's configuration,
/// and skips the system configuration entirely.
///
/// Command-line flags that are specific to a subcommand stay at the call site
/// (`--no-ext-diff` for `git diff`); this constructor owns everything that
/// applies to every invocation.
///
/// `diff.external` is the one key Git has no "unset" spelling for: an empty
/// value is still a value, so a content diff that omits `--no-ext-diff` fails
/// instead of running the repository's program. That is the intended direction
/// — a forgotten flag must not become an executed program — and
/// `tests/git_boundary_tests.rs` asserts the program is never reached either way.
pub fn git_command(root: &Path) -> Command {
    let mut command = command("git");
    command.arg("--no-pager");
    for entry in [
        "core.fsmonitor=false".to_string(),
        format!("core.hooksPath={INERT_PATH}"),
        "core.pager=".to_string(),
        format!("core.sshCommand={DISABLED_SSH_COMMAND}"),
        "diff.external=".to_string(),
        format!("core.attributesFile={INERT_PATH}"),
        format!("attr.tree={EMPTY_TREE_OBJECT}"),
    ] {
        command.arg("-c").arg(entry);
    }
    command
        .arg("-C")
        .arg(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C");
    command
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

/// Native tool resolution: the PATH and tool roots of the running process.
pub fn resolve(name: &str) -> Option<PathBuf> {
    resolve_with(name, &zenith_platform::PlatformEnvironment::native())
}

/// Environment-aware tool resolution.
///
/// A resolution the environment states is the authority: `Found` is returned
/// as stated, and `NotFound` is reported as not found rather than being
/// re-discovered from the host, so a simulated environment cannot be answered
/// by whatever the runner happens to have installed. The PATH and tool-root
/// search runs only when the environment states nothing for `name`.
pub fn resolve_with(
    name: &str,
    environment: &zenith_platform::PlatformEnvironment,
) -> Option<PathBuf> {
    if let Some(stated) = environment.tool(name) {
        return stated.path().map(Path::to_path_buf);
    }

    let name_variations = executable_name_variations(name);

    for directory in search_candidates(environment) {
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
    search_candidates(&zenith_platform::PlatformEnvironment::native())
}

fn search_candidates(environment: &zenith_platform::PlatformEnvironment) -> Vec<PathBuf> {
    // The stated PATH is the primary search path; the OS-owned tool roots below
    // stay host-derived, which is what "fall back to the platform search" means.
    let mut candidates = environment.path_entries().to_vec();

    // One shared root set for discovery. `tool_search_locations` reads
    // ProgramW6432/ProgramFiles(x86), package-manager environment variables,
    // Chocolatey/Scoop shims, WinGet Links, and nvm-windows on Windows. The
    // profile comes from the description, so a stated machine searches its own
    // roots instead of the runner's.
    candidates.extend(zenith_platform::NativePlatformPaths::tool_search_locations(
        environment.user_home().as_deref(),
    ));

    #[cfg(target_os = "macos")]
    if let Some(home) = environment.user_home() {
        // Version-manager installs (nvm) keep one `bin` dir per Node version;
        // scan them bounded so resolution work cannot grow without limit.
        candidates.extend(nvm_node_bin_dirs(&home.join(".nvm/versions/node")));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = environment.roaming_app_data().or_else(|| {
            environment
                .user_home()
                .map(|home| home.join("AppData/Roaming"))
        }) {
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
    // Used by portable tests as well as the Unix ones, so it must not be
    // gated on `unix`: gating it made the Windows job fail to compile.
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

    /// The tool that exists on every supported host, used to prove a stated
    /// resolution is not answered by host discovery.
    const HOST_TOOL: &str = if cfg!(windows) { "cmd.exe" } else { "sh" };

    #[test]
    fn a_stated_tool_resolution_is_the_authority() {
        use std::path::PathBuf;
        use zenith_platform::path_algebra::PathFlavor;
        use zenith_platform::PlatformEnvironment;

        let stated = if cfg!(windows) {
            PathBuf::from(r"D:\tools\npm.cmd")
        } else {
            PathBuf::from("/stated/bin/npm")
        };
        let environment =
            PlatformEnvironment::simulated(PathFlavor::current()).with_tool("npm", &stated);
        assert_eq!(super::resolve_with("npm", &environment), Some(stated));
    }

    #[test]
    fn a_stated_missing_tool_is_never_re_discovered_from_the_host() {
        use zenith_platform::path_algebra::PathFlavor;
        use zenith_platform::PlatformEnvironment;

        // The host resolves this tool; the environment states it is absent.
        assert!(
            super::resolve(HOST_TOOL).is_some(),
            "the host is expected to provide {HOST_TOOL}"
        );
        let missing =
            PlatformEnvironment::simulated(PathFlavor::current()).with_missing_tool(HOST_TOOL);
        assert_eq!(super::resolve_with(HOST_TOOL, &missing), None);

        // A stated PATH is searched when the environment states no tool.
        let directory = tempfile::tempdir().unwrap();
        let tool = directory.path().join(HOST_TOOL);
        fs::write(&tool, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        let stated_path =
            PlatformEnvironment::simulated(PathFlavor::current()).with_path_entry(directory.path());
        assert_eq!(super::resolve_with(HOST_TOOL, &stated_path), Some(tool));
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

    #[test]
    fn git_command_carries_the_neutralizing_configuration() {
        use std::path::Path;

        let command = super::git_command(Path::new("/workspace/project"));
        let args = command
            .get_args()
            .map(|argument| argument.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let config = args
            .windows(2)
            .filter(|pair| pair[0] == "-c")
            .map(|pair| pair[1].clone())
            .collect::<Vec<_>>();

        // Every key a repository can set to name a program, plus the attribute
        // source that selects a `filter.<driver>` in the first place.
        for expected in [
            "core.fsmonitor=false".to_string(),
            format!("core.hooksPath={}", super::INERT_PATH),
            "core.pager=".to_string(),
            format!("core.sshCommand={}", super::DISABLED_SSH_COMMAND),
            "diff.external=".to_string(),
            format!("core.attributesFile={}", super::INERT_PATH),
            format!("attr.tree={}", super::EMPTY_TREE_OBJECT),
        ] {
            assert!(
                config.contains(&expected),
                "`git` is invoked without `{expected}`; got {config:?}"
            );
        }
        assert!(
            args.contains(&"--no-pager".to_string()),
            "a repository `core.pager` must not be reachable: {args:?}"
        );
        // The project directory is the one the caller hands over, not the
        // process's own working directory.
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-C", "/workspace/project"]));

        let environment = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().to_string(),
                    value.map(|value| value.to_string_lossy().to_string()),
                )
            })
            .collect::<Vec<_>>();
        assert!(environment.contains(&("GIT_CONFIG_NOSYSTEM".into(), Some("1".into()))));
        assert!(environment.contains(&("GIT_OPTIONAL_LOCKS".into(), Some("0".into()))));
    }
}
