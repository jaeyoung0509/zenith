//! The one constructor every `git` invocation in Zenith goes through.

use crate::git::metadata::{read_capped, recorded_path, CappedRead, MAX_GIT_METADATA_BYTES};
use crate::git::repository::{git_directory, GitDirectory};
use crate::privacy::paths::mask_paths_in_text;
use crate::tooling;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use zenith_platform::PlatformEnvironment;

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

/// The driver definitions a repository may not supply.
///
/// Each entry names a program Git runs: `filter.<driver>.clean` and `.smudge`
/// convert working-tree content, `.process` does both directions through one
/// long-running process, `diff.<driver>.command` and `.textconv` render
/// patches, and `merge.<driver>.driver` would run during a merge — which
/// Zenith never starts, so that one is refused for the same reason the rest
/// are: a command line cannot name the key in advance, because the driver name
/// is whatever the repository writes. Git is asked which of them the
/// repository's own configuration defines, and a repository that defines one is
/// refused rather than read.
///
/// The pattern is matched by Git against canonical configuration names, so its
/// section and variable parts are matched case-insensitively there.
const REPOSITORY_PROGRAM_KEYS: &str =
    r"^(filter|diff|merge)\..*\.(clean|smudge|process|command|textconv|driver)$";

/// Bound on the configuration probe's output.
///
/// The probe prints one line per matching key, so the bound is only reachable
/// when a repository defines an absurd number of drivers; an answer that large
/// is refused instead of parsed, because a truncated listing cannot be read as
/// "no program is defined".
const MAX_PROGRAM_PROBE_BYTES: usize = 256 * 1024;

/// Longest repository-supplied key quoted in a refusal message, which crosses
/// IPC and reaches the diagnostics log.
const MAX_KEY_BYTES: usize = 128;

/// Timeout for the configuration probe: a repository's own configuration file
/// is normally kilobytes, so an answer that takes longer is refused.
const CONFIG_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Why Zenith does not run `git` in a root it identified.
///
/// Each variant is a decision an interface can report: the repository's shape
/// was refused, its attribute file could not be read, or its own configuration
/// names a program. The message is what the Control Center carries as the
/// summary's status message and what the diagnostics log records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRefusal {
    /// `.git` resolves to something that is not tied to this checkout.
    GitDirectory(String),
    /// `$GIT_DIR/info/attributes` is present and not empty, or too large to
    /// inspect. It outranks every other attribute source and no command-line
    /// configuration replaces it.
    InfoAttributes(String),
    /// The repository's own configuration defines a driver program.
    RepositorySuppliesProgram { key: String, file: String },
    /// The repository's own configuration could not be inspected, so whether it
    /// names a program is unknown.
    RepositoryConfigurationUnreadable(String),
}

impl GitRefusal {
    /// The reason as a user-facing sentence. Every path and key it carries was
    /// masked and bounded where it was recorded.
    pub fn message(&self) -> String {
        match self {
            GitRefusal::GitDirectory(reason) | GitRefusal::InfoAttributes(reason) => reason.clone(),
            GitRefusal::RepositorySuppliesProgram { key, file } => format!(
                "The repository's own configuration defines a program git would run ({key} in {file}), so its git state is not read"
            ),
            GitRefusal::RepositoryConfigurationUnreadable(reason) => format!(
                "The repository's own configuration could not be inspected, so its git state is not read: {reason}"
            ),
        }
    }
}

/// The repository inspection behind a root's Git invocations.
///
/// One metadata and configuration pass answers both questions an invocation
/// depends on — which git directory describes the root, and whether `git` may
/// run in it — so a capture that runs two commands, or a caller that asks for
/// the identity and the dirty state of the same root, does not repeat the
/// filesystem and configuration work.
pub struct GitInspection {
    root: PathBuf,
    directory: GitDirectory,
    refusal: Option<GitRefusal>,
}

impl GitInspection {
    /// Inspects `root` once.
    pub fn open(root: &Path, environment: &PlatformEnvironment) -> Self {
        let directory = git_directory(root, environment);
        let refusal = match &directory {
            GitDirectory::Resolved { git_dir, .. } => repository_refusal(root, git_dir),
            GitDirectory::Refused(reason) => Some(GitRefusal::GitDirectory(reason.clone())),
            GitDirectory::Absent => None,
        };
        Self {
            root: root.to_path_buf(),
            directory,
            refusal,
        }
    }

    /// The project root this inspection was opened for.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The git directory this root names.
    pub fn directory(&self) -> &GitDirectory {
        &self.directory
    }

    /// Why an invocation is refused, when the repository cannot be read.
    pub fn refusal(&self) -> Option<&GitRefusal> {
        self.refusal.as_ref()
    }

    /// Whether the root is inside a usable repository.
    pub fn is_repository(&self) -> bool {
        matches!(self.directory, GitDirectory::Resolved { .. })
    }

    /// The git directory that holds this project's `HEAD`.
    pub fn git_dir(&self) -> Option<&Path> {
        match &self.directory {
            GitDirectory::Resolved { git_dir, .. } => Some(git_dir),
            _ => None,
        }
    }

    /// The path hashed into the repository id: the repository's own git
    /// directory, which differs from [`Self::git_dir`] for a linked worktree.
    pub fn identity(&self) -> Option<&Path> {
        match &self.directory {
            GitDirectory::Resolved { identity, .. } => Some(identity),
            _ => None,
        }
    }

    /// A `git` command for this root, or the reason none may run.
    ///
    /// The returned command carries the neutralized configuration and the
    /// project directory; the caller adds the subcommand and its arguments.
    pub fn command(&self) -> Result<Command, GitRefusal> {
        if let Some(refusal) = &self.refusal {
            return Err(refusal.clone());
        }
        // Repository metadata is mutable. Repeat the inexpensive refusal pass
        // immediately before each child is built so an inspection cannot be
        // made stale merely by adding a driver definition or info/attributes
        // after `open`. The command-line overrides remain the final protection
        // for the fixed program-naming keys.
        if let GitDirectory::Resolved { git_dir, .. } = &self.directory {
            if let Some(refusal) = repository_refusal(&self.root, git_dir) {
                return Err(refusal);
            }
        }
        Ok(neutralized_command(&self.root))
    }
}

/// Every `git` invocation in Zenith goes through this constructor.
///
/// Git reads a repository's own `.git/config` whenever it operates on a
/// repository, and several documented keys name a program Git then runs:
/// `core.fsmonitor` is consulted by `git status` specifically, and
/// `core.pager`, `core.hooksPath`, `diff.external`, and `core.sshCommand` are
/// the same class. Those keys have fixed names, so the invocation replaces each
/// of them on the command line, where Git gives them precedence over any
/// configuration file. The open-ended driver definitions (`filter.<driver>.*`,
/// `diff.<driver>.*`, `merge.<driver>.*`) cannot be replaced that way — a
/// repository writes the driver name — so a repository that defines one is
/// refused instead.
///
/// Nothing else is neutralized: the machine's configuration, the user's own
/// configuration, and the repository's attribute files decide what a checkout
/// looks like, exactly as they do for the user's own `git status`. Skipping
/// them made Zenith report a clean checkout as modified — `core.autocrlf` from
/// the system configuration and the `text`/`eol` attributes of a repository
/// both change what Git compares — so a clean tree has to read as clean here
/// too.
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
pub fn git_command(root: &Path, environment: &PlatformEnvironment) -> Result<Command, GitRefusal> {
    GitInspection::open(root, environment).command()
}

/// The command line and environment every invocation carries.
///
/// Built without the repository check, so the check itself can use it: the
/// probe reads configuration, which runs nothing the repository names.
fn neutralized_command(root: &Path) -> Command {
    let mut command = tooling::command("git");
    command.arg("--no-pager");
    for entry in [
        "core.fsmonitor=false".to_string(),
        format!("core.hooksPath={INERT_PATH}"),
        "core.pager=".to_string(),
        format!("core.sshCommand={DISABLED_SSH_COMMAND}"),
        "diff.external=".to_string(),
    ] {
        command.arg("-c").arg(entry);
    }
    command
        .arg("-C")
        .arg(root)
        // The repository is the one `root` names. A location carried in the
        // environment — a development shell that exported `GIT_DIR`, say —
        // would otherwise point every project root at one repository, and the
        // shape checks above would have validated a different one.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        // Reading state must not write to the user's repository: an index
        // refresh is a mutation the user did not ask for.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C");
    command
}

/// Whether the repository's own content makes an invocation unsafe.
///
/// Two sources are checked, both of them repository content:
///
/// - `$GIT_DIR/info/attributes` outranks every other attribute source, so a
///   non-empty one can select a driver no command line replaces;
/// - the repository's own configuration files can define a driver program,
///   which no command line can name in advance.
fn repository_refusal(root: &Path, git_dir: &Path) -> Option<GitRefusal> {
    let common = match common_git_directory(git_dir) {
        Ok(common) => common,
        Err(refusal) => return Some(refusal),
    };
    if let Some(refusal) = info_attributes_refusal(git_dir, &common) {
        return Some(refusal);
    }
    repository_program_refusal(root, git_dir, &common)
}

/// The directory that holds the repository's shared configuration and
/// attributes.
///
/// `$GIT_DIR/commondir` moves both elsewhere, and a directory-shaped checkout
/// can carry the file exactly as a linked worktree does. A probe that only knew
/// the git directory would read files Git never opens, and report a
/// configuration as safe while Git read another: the pointer is resolved here,
/// under the same cap as every other metadata read, and a pointer that cannot be
/// read refuses the repository instead of being ignored.
fn common_git_directory(git_dir: &Path) -> Result<PathBuf, GitRefusal> {
    let pointer = git_dir.join("commondir");
    match read_capped(&pointer) {
        CappedRead::Absent => Ok(git_dir.to_path_buf()),
        CappedRead::Unreadable => Err(GitRefusal::RepositoryConfigurationUnreadable(
            mask_paths_in_text(&format!("{} could not be read safely", pointer.display())),
        )),
        CappedRead::Read(contents) => recorded_path(git_dir, &contents).ok_or_else(|| {
            GitRefusal::RepositoryConfigurationUnreadable(mask_paths_in_text(&format!(
                "{} names no directory",
                pointer.display()
            )))
        }),
        CappedRead::OverCap => Err(GitRefusal::RepositoryConfigurationUnreadable(
            mask_paths_in_text(&format!(
                "{} is larger than {MAX_GIT_METADATA_BYTES} bytes and was not read",
                pointer.display()
            )),
        )),
    }
}

/// The refusal for a usable attribute file in any git directory the repository's
/// attribute lookup reaches.
///
/// A linked worktree shares its repository's `info/attributes`: the worktree
/// keeps its own `HEAD` in `git_dir` but reads the common directory. Checking
/// both keeps the worktree and the checkout that owns it in the same state.
fn info_attributes_refusal(git_dir: &Path, common: &Path) -> Option<GitRefusal> {
    let mut directories = vec![git_dir];
    if common != git_dir {
        directories.push(common);
    }
    for directory in directories {
        let path = directory.join("info").join("attributes");
        match read_capped(&path) {
            CappedRead::Absent => {}
            CappedRead::Unreadable => {
                return Some(GitRefusal::InfoAttributes(mask_paths_in_text(&format!(
                    "{} exists but could not be read safely, so it cannot be checked for program-selecting attributes",
                    path.display()
                ))));
            }
            CappedRead::Read(contents) if contents.trim().is_empty() => {}
            CappedRead::Read(_) => {
                // The reason travels to the interface as well as the log, so a
                // path in it is masked the same way a log line is: no absolute
                // location leaves this module.
                return Some(GitRefusal::InfoAttributes(mask_paths_in_text(&format!(
                    "{} exists and can select a program for git to run, which Zenith does not run",
                    path.display()
                ))));
            }
            CappedRead::OverCap => {
                return Some(GitRefusal::InfoAttributes(mask_paths_in_text(&format!(
                    "{} is larger than {MAX_GIT_METADATA_BYTES} bytes and cannot be inspected",
                    path.display()
                ))));
            }
        }
    }
    None
}

/// The refusal for a driver program the repository's own configuration defines.
///
/// The shared configuration file lives in the common directory — which is the
/// git directory itself for the ordinary checkout and the submodule, and the
/// directory `commondir` names for a linked worktree or a directory-shaped
/// checkout — and a worktree may add a worktree-scoped file beside its own
/// `HEAD`. Both are repository content, and both are read through Git's own
/// parser (`--file` plus `--includes`), because a hand-written parser would be a
/// second answer to what Git actually reads: values can be quoted, escaped, and
/// split across lines, and an `include` directive pulls in another file.
fn repository_program_refusal(root: &Path, git_dir: &Path, common: &Path) -> Option<GitRefusal> {
    let mut files = vec![common.join("config")];
    let worktree_config = git_dir.join("config.worktree");
    if worktree_config != files[0] {
        files.push(worktree_config);
    }
    for file in files {
        if !file.is_file() {
            continue;
        }
        let file_display = mask_paths_in_text(&file.display().to_string());
        match config_names_a_program(root, &file) {
            Ok(Some(key)) => {
                return Some(GitRefusal::RepositorySuppliesProgram {
                    key: bounded_key(&key),
                    file: file_display,
                })
            }
            Ok(None) => {}
            Err(reason) => {
                return Some(GitRefusal::RepositoryConfigurationUnreadable(format!(
                    "{file_display}: {reason}"
                )))
            }
        }
    }
    None
}

/// The first driver definition `file` supplies, through Git's own parser.
///
/// Exit code 1 is Git's answer for "no key matched" and is not a failure; any
/// other non-zero exit leaves the answer unknown, which the caller refuses.
fn config_names_a_program(root: &Path, file: &Path) -> Result<Option<String>, String> {
    let mut command = neutralized_command(root);
    command.args([
        "config",
        "--file",
        &file.to_string_lossy(),
        "--includes",
        "-z",
        "--get-regexp",
        REPOSITORY_PROGRAM_KEYS,
    ]);
    let output = zenith_platform::subprocess::run_with_timeout(command, CONFIG_PROBE_TIMEOUT)
        .map_err(|error| error.to_string())?;
    match output.status.code() {
        Some(0) => {}
        Some(1) => return Ok(None),
        _ => {
            return Err(match output.status.code() {
                Some(code) => format!("git config exited with {code}"),
                None => "git config was terminated by a signal".to_string(),
            })
        }
    }
    if output.stdout.len() >= MAX_PROGRAM_PROBE_BYTES {
        return Err(format!(
            "the configuration listing is larger than {MAX_PROGRAM_PROBE_BYTES} bytes"
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // `-z` separates entries as `key\nvalue`; only the key is reported, and the
    // listing cannot be truncated here because its size was just bounded.
    let key = text
        .split('\0')
        .find(|entry| !entry.is_empty())
        .and_then(|entry| entry.split('\n').next())
        .unwrap_or_default()
        .to_string();
    Ok(Some(key))
}

/// A repository-supplied key, bounded before it is quoted in a message.
fn bounded_key(key: &str) -> String {
    let masked = mask_paths_in_text(key);
    if masked.len() <= MAX_KEY_BYTES {
        return masked;
    }
    format!("{}…", &masked[..masked.floor_char_boundary(MAX_KEY_BYTES)])
}
