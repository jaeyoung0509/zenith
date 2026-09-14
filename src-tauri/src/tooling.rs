use crate::safety::symlink::SymlinkGuard;
#[cfg(windows)]
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use zenith_platform::PlatformEnvironment;

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

/// Every `git` invocation in Zenith goes through this constructor, which also
/// refuses the invocation when the repository's own content can make it run a
/// program Zenith cannot neutralize.
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
/// (`--no-ext-diff` and `--no-textconv` for `git diff`); this constructor owns
/// everything that applies to every invocation.
///
/// `diff.external` is the one key Git has no "unset" spelling for: an empty
/// value is still a value, so a content diff that omits `--no-ext-diff` fails
/// instead of running the repository's program. That is the intended direction
/// — a forgotten flag must not become an executed program — and
/// `tests/git_boundary_tests.rs` asserts the program is never reached either way.
pub fn git_command(root: &Path) -> Result<Command, String> {
    if let Some(refusal) = git_inspection_refusal(root) {
        return Err(refusal);
    }
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
        // The system attributes file is a second source `.gitattributes`-style
        // drivers can come from; `GIT_CONFIG_NOSYSTEM` only covers config.
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C");
    Ok(command)
}

/// Byte cap for the files a repository pointer makes Zenith read: `<root>/.git`
/// when it is a pointer, the `HEAD` it resolves to, and the attribute file that
/// is checked before any invocation. The size is stated before the read,
/// following the stat-then-cap convention the tree already uses for hashed and
/// audited files.
pub(crate) const MAX_GIT_METADATA_BYTES: u64 = 4_096;

/// Outcome of a capped read.
pub(crate) enum CappedRead {
    Read(String),
    Absent,
    OverCap,
}

/// Reads one Git metadata file under [`MAX_GIT_METADATA_BYTES`], without
/// following a link at the path itself.
///
/// The limit is enforced on the open handle rather than on an earlier `stat`, so
/// a file that grows between the check and the read cannot exceed the cap: the
/// read stops one byte past it and reports the oversized case.
pub(crate) fn read_capped(path: &Path) -> CappedRead {
    if SymlinkGuard::is_symlink_metadata(path).unwrap_or(true) {
        // A link, an unreadable entry, or a Windows reparse point is not read:
        // its content would be whatever it names rather than what it is.
        return CappedRead::Absent;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return CappedRead::Absent;
    };
    let Ok(metadata) = file.metadata() else {
        return CappedRead::Absent;
    };
    if !metadata.is_file() {
        return CappedRead::Absent;
    }
    let mut contents = String::new();
    let mut bounded = std::io::Read::take(file, MAX_GIT_METADATA_BYTES + 1);
    match std::io::Read::read_to_string(&mut bounded, &mut contents) {
        Ok(read) if read as u64 <= MAX_GIT_METADATA_BYTES => CappedRead::Read(contents),
        Ok(_) => CappedRead::OverCap,
        Err(_) => CappedRead::Absent,
    }
}

/// A path recorded in a Git metadata file, resolved against `base`.
fn recorded_path(base: &Path, raw: &str) -> Option<PathBuf> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    let path = Path::new(value);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    })
}

/// The path a `gitdir:` pointer file names, resolved against `root`.
fn gitfile_target(root: &Path, contents: &str) -> Option<PathBuf> {
    recorded_path(root, contents.trim().strip_prefix("gitdir:")?)
}

/// The git directory that describes a project root.
pub enum GitDirectory {
    /// Usable: `git_dir` holds the `HEAD` this project reads, and `identity` is
    /// the path hashed into the repository id. The two differ for a linked
    /// worktree, whose own `HEAD` sits in the repository's git directory.
    Resolved { git_dir: PathBuf, identity: PathBuf },
    /// No `.git` entry, or one that names nothing.
    Absent,
    /// Present but refused. The reason is reported instead of the state being
    /// downgraded to "not a repository".
    Refused(String),
}

impl GitDirectory {
    /// A git directory that is both the `HEAD` source and the identity, which is
    /// the ordinary case: the repository's own `.git`.
    fn resolved_whole(git_dir: PathBuf) -> Self {
        GitDirectory::Resolved {
            identity: git_dir.clone(),
            git_dir,
        }
    }
}

/// The git directory for `root`, contained and bounded.
///
/// `.git` may be a *file* holding a `gitdir:` pointer — how a linked worktree
/// and a submodule record where their git directory actually lives. That is a
/// second pointer mechanism, and its legitimate target is usually *outside* the
/// project, so `SymlinkGuard`'s containment rule cannot be the whole decision:
///
/// - a target inside the project is validated by `SymlinkGuard`, the guard that
///   already governs pointer following in this tree;
/// - a target outside it is accepted only when Git's own backlink ties it to
///   this checkout: a linked worktree records `gitdir` naming this project's
///   `.git`, and a submodule records `core.worktree` naming this project;
/// - anything else is refused with a reason, because a path that merely *looks*
///   like a worktree or submodule git directory is a shape, not a relationship.
///
/// `root` is workspace content, not a nominated directory: it comes from an
/// observed agent process working directory.
pub fn git_directory(root: &Path, environment: &PlatformEnvironment) -> GitDirectory {
    let marker = root.join(".git");
    if SymlinkGuard::is_symlink_metadata(&marker).unwrap_or(true) {
        // A link is not followed on faith, and a Windows directory junction is
        // not a `FileType::is_symlink` either, so the guard decides both.
        return match std::fs::canonicalize(&marker) {
            Ok(target)
                if SymlinkGuard::validate_components_between(&target, root, environment)
                    .is_ok() =>
            {
                GitDirectory::resolved_whole(target)
            }
            Ok(target) => match external_git_directory(&target, root) {
                Some((git_dir, identity)) => GitDirectory::Resolved { git_dir, identity },
                None => GitDirectory::Refused(format!(
                    "The .git entry is a link to {}, which is not this project's git directory",
                    target.display()
                )),
            },
            Err(_) => GitDirectory::Absent,
        };
    }
    if marker.is_dir() {
        return GitDirectory::resolved_whole(marker);
    }
    match read_capped(&marker) {
        CappedRead::Absent => GitDirectory::Absent,
        CappedRead::OverCap => GitDirectory::Refused(format!(
            "The .git pointer file is larger than {MAX_GIT_METADATA_BYTES} bytes and was not read"
        )),
        CappedRead::Read(contents) => {
            let Some(target) = gitfile_target(root, &contents) else {
                return GitDirectory::Absent;
            };
            if SymlinkGuard::validate_components_between(&target, root, environment).is_ok() {
                return GitDirectory::resolved_whole(target);
            }
            match external_git_directory(&target, root) {
                Some((git_dir, identity)) => GitDirectory::Resolved { git_dir, identity },
                None => GitDirectory::Refused(format!(
                    "The .git pointer resolves to {}, which is outside the project and is not this project's linked-worktree or submodule git directory",
                    target.display()
                )),
            }
        }
    }
}

/// The `(HEAD source, identity)` pair for a `gitdir:` pointer that leaves the
/// project, when Git's own backlink ties it to `root`.
fn external_git_directory(target: &Path, root: &Path) -> Option<(PathBuf, PathBuf)> {
    // The git directory the target descends from: the nearest ancestor holding a
    // `HEAD`, since `worktrees/<name>` and `modules/<path>` both sit directly
    // under one.
    let git_dir = target
        .ancestors()
        .skip(1)
        .find(|candidate| candidate.is_dir() && candidate.join("HEAD").is_file())?;
    let shape = target.strip_prefix(git_dir).ok()?;
    let mut components = shape.components();
    match components.next()?.as_os_str().to_str()? {
        "worktrees" => {
            if components.count() != 1 || !worktree_belongs_to(target, root) {
                return None;
            }
            // A linked worktree reads its own HEAD but is identified by the
            // repository's git directory, exactly as identity resolution did
            // before this check existed.
            Some((target.to_path_buf(), git_dir.to_path_buf()))
        }
        "modules" => {
            components.next()?;
            if !submodule_belongs_to(target, root) {
                return None;
            }
            // A submodule keeps its own git directory and its own identity.
            Some((target.to_path_buf(), target.to_path_buf()))
        }
        _ => None,
    }
}

/// Whether `<target>/gitdir` names this project's `.git`, which Git records when
/// it creates a linked worktree.
fn worktree_belongs_to(target: &Path, root: &Path) -> bool {
    let CappedRead::Read(contents) = read_capped(&target.join("gitdir")) else {
        return false;
    };
    let Some(recorded) = recorded_path(target, &contents) else {
        return false;
    };
    same_location(&recorded, &root.join(".git"))
}

/// Whether the submodule git directory records this project as its working tree,
/// which `git submodule` writes as `core.worktree`.
fn submodule_belongs_to(target: &Path, root: &Path) -> bool {
    let CappedRead::Read(contents) = read_capped(&target.join("config")) else {
        return false;
    };
    let Some(worktree) = core_config_value(&contents, "worktree") else {
        return false;
    };
    let Some(recorded) = recorded_path(target, &worktree) else {
        return false;
    };
    same_location(&recorded, root)
}

/// Compares two paths by what they resolve to, so a recorded relative path and
/// the checkout it names are compared as the same location.
fn same_location(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// The first `key = value` entry in the `[core]` section of a Git configuration
/// file.
///
/// The file is repository content, so the parse is deliberately small and
/// section-aware: a value a different section carries is not one Git reads.
fn core_config_value(contents: &str, key: &str) -> Option<String> {
    let mut in_core = false;
    for line in contents.lines() {
        let line = line.trim();
        if let Some(section) = line.strip_prefix('[') {
            in_core = section
                .split(']')
                .next()
                .is_some_and(|name| name.trim().eq_ignore_ascii_case("core"));
            continue;
        }
        if !in_core || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(key) {
            let value = value.trim().trim_matches('"');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Whether Zenith may run `git` in `root`, or the reason it must not.
///
/// Two things are checked before any invocation, because both are conditions the
/// command line above cannot neutralize:
///
/// - the machine's `git` must honor `attr.tree` (Git 2.43+), or a repository
///   `.gitattributes` file can still select a `filter.*` driver;
/// - the repository must not carry a non-empty `$GIT_DIR/info/attributes`, the
///   one attribute source Git reads from a fixed location that no command-line
///   configuration replaces. It outranks every other source, so a repository
///   that has one can name a filter or textconv program there.
///
/// In both cases the state is refused and the reason is reported, rather than
/// being read and presented as if it were measured.
pub fn git_inspection_refusal(root: &Path) -> Option<String> {
    if !git_supports_attribute_pin() {
        return Some(
            "The git on this machine does not support attr.tree (Git 2.43 or newer is required), so a repository's attribute-selected programs cannot be neutralized".to_string(),
        );
    }
    for git_dir in effective_git_directories(root) {
        match read_capped(&git_dir.join("info").join("attributes")) {
            CappedRead::Absent => {}
            CappedRead::Read(contents) if contents.trim().is_empty() => {}
            CappedRead::Read(_) => {
                return Some(format!(
                    "{} exists and can select a program for git to run, which Zenith does not run",
                    git_dir.join("info").join("attributes").display()
                ));
            }
            CappedRead::OverCap => {
                return Some(format!(
                    "{} is larger than {MAX_GIT_METADATA_BYTES} bytes and cannot be inspected",
                    git_dir.join("info").join("attributes").display()
                ));
            }
        }
    }
    None
}

/// The git directories a repository's attribute lookup can reach: the one
/// `.git` names, plus the common directory a linked worktree records, because a
/// worktree shares its repository's `info/attributes`.
fn effective_git_directories(root: &Path) -> Vec<PathBuf> {
    let marker = root.join(".git");
    let mut directories = Vec::new();
    match SymlinkGuard::is_symlink_metadata(&marker) {
        Ok(true) => {
            if let Ok(target) = std::fs::canonicalize(&marker) {
                directories.push(target);
            }
        }
        Ok(false) if marker.is_dir() => directories.push(marker),
        Ok(false) => {
            if let CappedRead::Read(contents) = read_capped(&marker) {
                if let Some(target) = gitfile_target(root, &contents) {
                    directories.push(target);
                }
            }
        }
        Err(_) => {}
    }
    for directory in directories.clone() {
        if let CappedRead::Read(contents) = read_capped(&directory.join("commondir")) {
            if let Some(common) = recorded_path(&directory, &contents) {
                directories.push(common);
            }
        }
    }
    directories
}

/// Whether the `git` on this machine honors `attr.tree`.
///
/// One probe per process: the answer cannot change under a running process, and
/// the probe runs without a repository, so no repository content is involved.
/// A `git` that cannot be run answers `false`, which refuses inspection rather
/// than running a command whose configuration is unknown.
static GIT_SUPPORTS_ATTRIBUTE_PIN: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
    let mut command = command("git");
    command.arg("--version");
    let Ok(output) =
        zenith_platform::subprocess::run_with_timeout(command, std::time::Duration::from_secs(2))
    else {
        return false;
    };
    parse_git_version(&String::from_utf8_lossy(&output.stdout))
        .is_some_and(|(major, minor)| major > 2 || (major == 2 && minor >= 43))
});

fn git_supports_attribute_pin() -> bool {
    *GIT_SUPPORTS_ATTRIBUTE_PIN
}

/// The `major.minor` of a `git --version` line, as in `git version 2.50.1`.
fn parse_git_version(text: &str) -> Option<(u32, u32)> {
    let version = text.trim().strip_prefix("git version ")?.trim();
    let mut parts = version.split(['.', '-', ' ']);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor))
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

        let command = super::git_command(Path::new("/workspace/project"))
            .expect("a directory without a repository is not refused");
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
