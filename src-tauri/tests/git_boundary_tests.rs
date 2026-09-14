//! Zenith invokes `git` in directories the user did not nominate.
//!
//! The project root comes from an observed agent process working directory, so
//! a repository Zenith did not create can supply `.git/config` and
//! `.gitattributes` for the invocation. Several documented Git keys name a
//! program Git then runs: `core.fsmonitor` is consulted by `git status`
//! specifically, and `core.pager`, `core.hooksPath`, `diff.external`,
//! `core.sshCommand`, and the `filter.*` clean/smudge pair are the same class.
//!
//! These tests hold both halves of the boundary:
//!
//! - every invocation is built by one constructor (`git::git_command`), the
//!   fixed program-naming keys are replaced on the command line, and a
//!   repository that defines a driver program of its own is refused at
//!   inspection and again at command construction;
//! - nothing else is neutralized: the machine's configuration and the
//!   repository's attributes decide what a checkout looks like, so a clean
//!   checkout reads as clean here exactly as it does in the user's own shell.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use zenith_platform::PlatformEnvironment;

fn environment() -> PlatformEnvironment {
    PlatformEnvironment::native()
}

/// The one module allowed to construct a `git` command.
const CHOKEPOINT: &str = "src-tauri/src/git/command.rs";

/// Constructor spellings that would bypass the neutralized configuration.
const BYPASSES: [&str; 3] = [
    "command(\"git\")",
    "command_with(\"git\"",
    "Command::new(\"git\")",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate lives in the workspace root")
        .to_path_buf()
}

fn rust_sources(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

/// The production half of a source file: everything before its `#[cfg(test)]`
/// module, so a fixture helper in a test may build its own `git` command.
///
/// Line endings differ per checkout, so the marker is matched after normalizing
/// them: a scan that quietly matched nothing on a Windows checkout would report
/// coverage it does not have.
fn production_source(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n");
    match normalized.find("#[cfg(test)]\nmod tests") {
        Some(index) => normalized[..index].to_string(),
        None => normalized,
    }
}

#[test]
fn every_git_invocation_goes_through_the_neutralizing_constructor() {
    let workspace = workspace_root();
    let chokepoint = workspace.join(CHOKEPOINT);
    let mut sources = Vec::new();
    for directory in [workspace.join("src-tauri/src"), workspace.join("crates")] {
        rust_sources(&directory, &mut sources);
    }
    assert!(
        sources.len() > 50,
        "the scan is expected to read the Rust tree, found {} files",
        sources.len()
    );

    let mut offenders = Vec::new();
    for source in &sources {
        if is_chokepoint(source, &workspace) {
            continue;
        }
        let relative = source.strip_prefix(&workspace).unwrap_or(source).display();
        let text = std::fs::read_to_string(source).expect("a Rust source is readable");
        let production = production_source(&text);
        for bypass in BYPASSES {
            if production.contains(bypass) {
                offenders.push(format!("{relative} builds a git command with `{bypass}`"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "every git invocation must go through `git::git_command`: {offenders:#?}"
    );

    // The constructor itself is the other half: it must still be the place the
    // neutralization lives, so a rename cannot silently empty this guard.
    let module = std::fs::read_to_string(&chokepoint).expect("the git module is readable");
    assert!(
        module.contains("pub fn git_command("),
        "the chokepoint constructor is missing from {CHOKEPOINT}"
    );
}

/// Whether a scanned file is the module allowed to build a `git` command.
///
/// Compared as paths, not as a string suffix: on Windows the same file can be
/// spelled with either separator, and a suffix comparison that assumes one of
/// them stops excluding the chokepoint without failing anything.
fn is_chokepoint(source: &Path, workspace: &Path) -> bool {
    source == workspace.join(CHOKEPOINT)
}

#[test]
fn the_scan_reads_a_windows_shaped_checkout_the_same_way() {
    // Line endings: a Windows checkout has CRLF, and the test-module marker must
    // still be found, or a fixture helper's own `git` command is reported as a
    // production violation.
    let crlf = "#[cfg(test)]\r\nmod tests {\r\n    Command::new(\"git\");\r\n}\r\n";
    assert!(
        !production_source(crlf).contains("Command::new"),
        "a test module must not be read as production code"
    );

    // Separators: the constant spells the module with `/`, and on Windows that
    // joins into a mixed-separator path for the same file the walker reports.
    let workspace = workspace_root();
    assert!(is_chokepoint(&workspace.join(CHOKEPOINT), &workspace));
    assert!(
        is_chokepoint(
            &workspace
                .join("src-tauri")
                .join("src")
                .join("git")
                .join("command.rs"),
            &workspace
        ),
        "the chokepoint must be recognized however its path was built"
    );
    assert!(!is_chokepoint(
        &workspace
            .join("src-tauri")
            .join("src")
            .join("git")
            .join("repository.rs"),
        &workspace
    ));
}

/// The configuration the constructor carries, read off a real command.
#[test]
fn the_constructor_replaces_the_fixed_program_keys_and_keeps_the_users_own_configuration() {
    let command = zenith_lib::git::git_command(Path::new("/workspace/project"), &environment())
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

    // Every key whose fixed name lets a repository point Git at a program.
    for expected in [
        "core.fsmonitor=false".to_string(),
        "core.pager=".to_string(),
        "diff.external=".to_string(),
    ] {
        assert!(
            config.contains(&expected),
            "`git` is invoked without `{expected}`; got {config:?}"
        );
    }
    assert!(
        config
            .iter()
            .any(|entry| entry.starts_with("core.hooksPath=")),
        "`core.hooksPath` must be pinned inert: {config:?}"
    );
    assert!(
        config
            .iter()
            .any(|entry| entry.starts_with("core.sshCommand=")),
        "`core.sshCommand` must be pinned inert: {config:?}"
    );
    assert!(
        args.contains(&"--no-pager".to_string()),
        "a repository `core.pager` must not be reachable: {args:?}"
    );
    // The project directory is the one the caller hands over, not the
    // process's own working directory.
    assert!(args
        .windows(2)
        .any(|pair| pair == ["-C", "/workspace/project"]));

    // The machine's and the user's own configuration stay in force: they decide
    // what a checkout looks like (`core.autocrlf` on Windows is one of them),
    // and a `git status` that cannot see them reports a clean tree as modified.
    for removed in [
        "GIT_CONFIG_NOSYSTEM",
        "GIT_ATTR_NOSYSTEM",
        "core.attributesFile",
        "attr.tree",
    ] {
        let in_config = config.iter().any(|entry| entry.starts_with(removed));
        let in_environment = command
            .get_envs()
            .any(|(key, _)| key.to_string_lossy() == removed);
        assert!(
            !in_config && !in_environment,
            "`{removed}` must not be neutralized: {args:?}"
        );
    }
    let environment = command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().to_string(),
                value.map(|value| value.to_string_lossy().to_string()),
            )
        })
        .collect::<Vec<_>>();
    assert!(environment.contains(&("GIT_OPTIONAL_LOCKS".into(), Some("0".into()))));
    // The repository is the one the root names, not one an inherited
    // environment points at.
    for removed in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ] {
        assert!(
            environment.contains(&(removed.into(), None)),
            "`{removed}` must not reach the invocation: {environment:?}"
        );
    }
}

/// A repository that writes its own configuration for the fixed program keys,
/// which the command line replaces.
struct FixedKeyRepository {
    _temp: tempfile::TempDir,
    root: PathBuf,
    /// Named by `core.fsmonitor`, which `git status` consults.
    fsmonitor_program: PathBuf,
    /// Named by `diff.external`, which a content diff would otherwise run.
    external_diff_program: PathBuf,
}

/// A repository that defines a driver program of its own, which no command line
/// can name in advance.
struct DriverRepository {
    _temp: tempfile::TempDir,
    root: PathBuf,
    /// Named by `filter.probe.clean`, which reading working-tree content runs.
    filter_program: PathBuf,
    /// Named by `diff.probe.textconv`, which rendering a patch runs.
    textconv_program: PathBuf,
}

/// Which attribute source a refused fixture uses.
#[derive(PartialEq, Eq, Clone, Copy)]
enum AttributeSource {
    /// `.gitattributes`, read from the working tree.
    Tracked,
    /// `$GIT_DIR/info/attributes`, which outranks every other source and has no
    /// command-line replacement.
    Info,
}

/// A `git` command run directly by a fixture, outside the neutralized
/// constructor. Used to create repositories and, as a positive control, to show
/// a program is reachable when nothing neutralizes it.
fn fixture_git(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git is installed on this host")
}

fn fixture_git_or_panic(root: &Path, args: &[&str]) {
    let output = fixture_git(root, args);
    assert!(
        output.status.success(),
        "fixture `git {args:?}` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Commits a one-file repository and configures it afterwards, so the fixture's
/// programs are not reached while the repository is being built.
fn commit_baseline(root: &Path) {
    fixture_git_or_panic(root, &["init", "-q"]);
    fixture_git_or_panic(root, &["config", "user.email", "fixture@example.invalid"]);
    fixture_git_or_panic(root, &["config", "user.name", "Fixture"]);
    std::fs::write(root.join("tracked.txt"), "baseline content\n").expect("fixture file");
    fixture_git_or_panic(root, &["add", "-A"]);
    fixture_git_or_panic(root, &["commit", "-qm", "initial"]);
}

impl FixedKeyRepository {
    fn create() -> Self {
        let temp = tempfile::tempdir().expect("a temporary directory is available");
        let root = temp.path().join("workspace");
        let missing = temp.path().join("no-such-program");
        std::fs::create_dir_all(&root).expect("the fixture repository is creatable");
        let fsmonitor_program = missing.join("fsmonitor-program");
        let external_diff_program = missing.join("external-diff-program");
        commit_baseline(&root);

        // `core.pager` is pinned the same way, but a pager is only consulted
        // when the invocation has a terminal, which a captured command never
        // has; the pin itself is asserted against the constructor.
        for (key, program) in [
            ("core.fsmonitor", &fsmonitor_program),
            ("diff.external", &external_diff_program),
        ] {
            fixture_git_or_panic(&root, &["config", key, &program.to_string_lossy()]);
        }
        std::fs::write(root.join("tracked.txt"), "modified content\n").expect("fixture file");

        Self {
            _temp: temp,
            root,
            fsmonitor_program,
            external_diff_program,
        }
    }

    fn unguarded(&self, args: &[&str]) -> std::process::Output {
        fixture_git(&self.root, args)
    }

    fn guarded(&self, args: &[&str]) -> std::process::Output {
        let mut command = zenith_lib::git::git_command(&self.root, &environment())
            .expect("the fixture repository is not refused");
        command.args(args);
        zenith_platform::subprocess::run_with_timeout(command, Duration::from_secs(10))
            .expect("the neutralized invocation completes")
    }
}

impl DriverRepository {
    /// `Tracked` selects the driver through `.gitattributes`; `Info` through
    /// `$GIT_DIR/info/attributes`, which outranks every source.
    fn create(source: AttributeSource) -> Self {
        let temp = tempfile::tempdir().expect("a temporary directory is available");
        let root = temp.path().join("workspace");
        let missing = temp.path().join("no-such-program");
        std::fs::create_dir_all(&root).expect("the fixture repository is creatable");
        let filter_program = missing.join("filter-program");
        let textconv_program = missing.join("textconv-program");

        if source == AttributeSource::Tracked {
            std::fs::write(root.join(".gitattributes"), "* filter=probe\n").expect("fixture file");
        }
        commit_baseline(&root);
        if source == AttributeSource::Info {
            // Written after the commit: the fixture needs a repository that was
            // created legitimately and configured afterwards.
            std::fs::create_dir_all(root.join(".git/info")).expect("fixture directory");
            std::fs::write(root.join(".git/info/attributes"), "* filter=probe\n")
                .expect("fixture file");
        }

        fixture_git_or_panic(&root, &["config", "filter.probe.required", "true"]);
        fixture_git_or_panic(
            &root,
            &[
                "config",
                "diff.probe.textconv",
                &textconv_program.to_string_lossy(),
            ],
        );
        std::fs::write(root.join("tracked.txt"), "modified content\n").expect("fixture file");

        // The filter program is configured last: `git add` applies a clean
        // filter, and the fixture is built before the poison is in place.
        fixture_git_or_panic(
            &root,
            &[
                "config",
                "filter.probe.clean",
                &filter_program.to_string_lossy(),
            ],
        );

        Self {
            _temp: temp,
            root,
            filter_program,
            textconv_program,
        }
    }

    fn unguarded(&self, args: &[&str]) -> std::process::Output {
        fixture_git(&self.root, args)
    }

    fn refusal(&self) -> zenith_lib::git::GitRefusal {
        zenith_lib::git::git_command(&self.root, &environment())
            .expect_err("a repository that defines a driver program is refused")
    }
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn a_repository_cannot_name_a_program_that_git_status_runs() {
    let repository = FixedKeyRepository::create();
    let program = repository.fsmonitor_program.to_string_lossy().to_string();

    // Positive control: the fixture is live, so the assertion below is about
    // the neutralization rather than about a repository that never had a
    // program to run.
    let unguarded = repository.unguarded(&["status", "--porcelain=v1", "-z"]);
    assert!(
        stderr_of(&unguarded).contains(&program),
        "the fixture is expected to make git reach the repository's fsmonitor program, got: {}",
        stderr_of(&unguarded)
    );

    let guarded = repository.guarded(&["status", "--porcelain=v1", "-z"]);
    assert!(
        guarded.status.success(),
        "the neutralized status must still succeed: {}",
        stderr_of(&guarded)
    );
    assert!(
        !stderr_of(&guarded).contains(&program),
        "`core.fsmonitor` reached the repository's program despite neutralization: {}",
        stderr_of(&guarded)
    );
}

#[test]
fn a_repository_cannot_name_a_program_that_rendering_a_patch_runs() {
    let repository = FixedKeyRepository::create();
    let external_diff = repository
        .external_diff_program
        .to_string_lossy()
        .to_string();

    // Positive control: a content diff runs the repository's external program.
    let unguarded = repository.unguarded(&["diff", "--no-color", "--", "tracked.txt"]);
    assert!(
        stderr_of(&unguarded).contains(&external_diff),
        "the fixture is expected to make git reach the repository's external diff, got: {}",
        stderr_of(&unguarded)
    );

    // The production shape: every content diff carries `--no-ext-diff`, so the
    // neutralized invocation still reports the change it exists to report.
    let guarded = repository.guarded(&["diff", "--no-ext-diff", "--no-color", "--", "tracked.txt"]);
    assert!(
        guarded.status.success(),
        "the neutralized diff must still succeed: {}",
        stderr_of(&guarded)
    );
    assert!(
        !stderr_of(&guarded).contains(&external_diff),
        "`diff.external` reached the repository's program: {}",
        stderr_of(&guarded)
    );
    let diff = String::from_utf8_lossy(&guarded.stdout).to_string();
    assert!(
        diff.contains("modified content"),
        "the neutralized diff must still compare the working tree: {diff}"
    );

    // A content diff that forgets the flag fails closed: the neutralized
    // `diff.external` is empty, so Git attempts no repository-supplied program
    // and stops instead. The invariant is the same either way — the program the
    // repository named is never reached.
    let unflagged = repository.guarded(&["diff", "--no-color", "--", "tracked.txt"]);
    assert!(
        !stderr_of(&unflagged).contains(&external_diff),
        "`diff.external` reached the repository's program: {}",
        stderr_of(&unflagged)
    );
}

#[test]
fn a_repository_that_defines_a_filter_program_is_refused_rather_than_read() {
    let repository = DriverRepository::create(AttributeSource::Tracked);
    let program = repository.filter_program.to_string_lossy().to_string();

    // Positive control: reading working-tree content runs the repository's clean
    // filter, and the fixture makes it required.
    let unguarded = repository.unguarded(&["status", "--porcelain=v1", "-z"]);
    assert!(
        stderr_of(&unguarded).contains(&program),
        "the fixture is expected to make git reach the repository's clean filter, got: {}",
        stderr_of(&unguarded)
    );

    // A driver name is the repository's to invent, so no command line can
    // replace the definition: the repository is refused instead, and the reason
    // names the key it defines.
    let refusal = repository.refusal();
    let message = refusal.message();
    assert!(
        message.contains("filter.probe.clean"),
        "the refusal must name the definition it refused: {message}"
    );
    assert!(
        message.contains("not read"),
        "the refusal must state what was not done: {message}"
    );
    assert!(
        !message.contains(repository.root.to_str().unwrap()),
        "the refusal leaked an absolute path: {message}"
    );

    let diff_error = zenith_lib::ai_control_center::git::explicit_diff(
        &repository.root,
        &environment(),
        None,
        &[],
    )
    .expect_err("an empty path selection must not turn a refused read into an empty diff");
    assert!(
        diff_error.contains("filter.probe.clean"),
        "the explicit diff must preserve the refusal: {diff_error}"
    );
}

#[test]
fn an_inspection_is_revalidated_before_each_command_is_built() {
    let temp = tempfile::tempdir().expect("a temporary directory is available");
    let root = temp.path().join("workspace");
    std::fs::create_dir_all(&root).expect("the fixture repository is creatable");
    std::fs::write(root.join(".gitattributes"), "* filter=late\n").expect("fixture file");
    commit_baseline(&root);

    let environment = environment();
    let inspection = zenith_lib::git::GitInspection::open(&root, &environment);
    assert!(inspection.refusal().is_none());

    fixture_git_or_panic(&root, &["config", "filter.late.clean", "no-such-program"]);
    let message = inspection
        .command()
        .expect_err("a driver added after inspection must still be refused")
        .message();
    assert!(
        message.contains("filter.late.clean"),
        "the command-time refusal must name the new definition: {message}"
    );
}

#[test]
fn a_driver_defined_in_an_included_configuration_file_is_refused_too() {
    let repository = DriverRepository::create(AttributeSource::Tracked);
    let included = repository.root.join(".git").join("extra-config");
    std::fs::write(
        &included,
        "[filter \"hidden\"]\n\tclean = no-such-program\n",
    )
    .expect("fixture file");
    let mut config = std::fs::read_to_string(repository.root.join(".git/config")).unwrap();
    config.push_str("[include]\n\tpath = extra-config\n");
    std::fs::write(repository.root.join(".git/config"), config).expect("fixture file");
    // The fixture's own definitions are removed so the included file is what
    // refuses the repository.
    fixture_git_or_panic(
        &repository.root,
        &["config", "--unset", "filter.probe.clean"],
    );
    fixture_git_or_panic(
        &repository.root,
        &["config", "--unset", "diff.probe.textconv"],
    );

    let message = repository.refusal().message();
    assert!(
        message.contains("filter.hidden.clean"),
        "the refusal must follow Git's include directives: {message}"
    );
}

/// A project whose `.git` directory records a common directory, which is how a
/// linked worktree and a directory-shaped checkout keep their configuration and
/// attributes somewhere else. Git reads them from the recorded directory, so a
/// probe that only knew `.git` would read files Git never opens.
fn common_directory_project(configure: impl FnOnce(&Path)) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("a temporary directory is available");
    let root = temp.path().join("workspace");
    let common = temp.path().join("common");
    std::fs::create_dir_all(root.join(".git")).expect("the fixture git directory is creatable");
    std::fs::create_dir_all(&common).expect("the fixture common directory is creatable");
    // Relative, as Git records it: resolved against the git directory.
    std::fs::write(root.join(".git/commondir"), "../../common\n").expect("fixture pointer");
    configure(&common);
    (temp, root)
}

#[test]
fn a_common_directory_that_holds_the_configuration_is_probed_through_the_pointer() {
    let (_temp, root) = common_directory_project(|common| {
        std::fs::write(
            common.join("config"),
            "[filter \"probe\"]\n\tclean = no-such-program\n",
        )
        .expect("fixture file");
    });

    let message = zenith_lib::git::git_command(&root, &environment())
        .expect_err("a driver definition behind `commondir` must refuse the repository")
        .message();
    assert!(
        message.contains("filter.probe.clean"),
        "the refusal must name the definition Git would read: {message}"
    );
}

#[test]
fn a_common_directory_that_holds_info_attributes_is_refused() {
    let (_temp, root) = common_directory_project(|common| {
        std::fs::create_dir_all(common.join("info")).expect("fixture directory");
        std::fs::write(common.join("info/attributes"), "* filter=probe\n").expect("fixture file");
    });

    let message = zenith_lib::git::git_command(&root, &environment())
        .expect_err("an attribute source behind `commondir` must refuse the repository")
        .message();
    assert!(
        message.contains("attributes"),
        "the refusal must name what it refused to read: {message}"
    );
}

#[test]
fn a_repository_that_carries_info_attributes_is_refused_rather_than_read() {
    let repository = DriverRepository::create(AttributeSource::Info);
    let program = repository.filter_program.to_string_lossy().to_string();

    // Positive control: this source outranks the tracked one, so an unguarded
    // invocation reaches the repository's program.
    let unguarded = repository.unguarded(&["status", "--porcelain=v1", "-z"]);
    assert!(
        stderr_of(&unguarded).contains(&program),
        "the fixture is expected to make git reach the repository's clean filter: {}",
        stderr_of(&unguarded)
    );

    // The attribute file is checked before the configuration, and its refusal is
    // reported instead of the state being read.
    let refusal = repository.refusal();
    let message = refusal.message();
    assert!(
        message.contains("attributes"),
        "the refusal must name the file it refused to read: {message}"
    );
    // The reason crosses IPC, so it carries a masked location rather than the
    // absolute path the file actually has.
    assert!(
        !message.contains(&repository.root.to_string_lossy().to_string()),
        "the refusal leaked an absolute path: {message}"
    );
    assert_eq!(
        zenith_lib::git::git_command(&repository.root, &environment())
            .expect_err("the constructor must refuse the repository"),
        refusal
    );
}

#[test]
fn undecodable_info_attributes_are_refused_instead_of_treated_as_absent() {
    let temp = tempfile::tempdir().expect("a temporary directory is available");
    let root = temp.path().join("workspace");
    std::fs::create_dir_all(&root).expect("the fixture repository is creatable");
    commit_baseline(&root);
    std::fs::create_dir_all(root.join(".git/info")).expect("fixture directory");
    std::fs::write(
        root.join(".git/info/attributes"),
        b"tracked.txt filter=probe\n\xff",
    )
    .expect("fixture file");

    let message = zenith_lib::git::git_command(&root, &environment())
        .expect_err("an attribute file that cannot be fully inspected must be refused")
        .message();
    assert!(
        message.contains("could not be read safely"),
        "the refusal must distinguish unreadable metadata from absence: {message}"
    );
}

#[test]
fn a_repository_cannot_name_a_textconv_program_through_info_attributes() {
    let repository = DriverRepository::create(AttributeSource::Info);
    let sentinel = repository.textconv_program.to_string_lossy().to_string();
    std::fs::write(
        repository.root.join(".git/info/attributes"),
        "tracked.txt diff=probe\n",
    )
    .expect("fixture file");
    // Positive control: `--no-ext-diff` does not cover textconv, which Git gates
    // with its own switch, so an unguarded content diff reaches the program.
    let unguarded = repository.unguarded(&["diff", "--no-ext-diff", "--no-color"]);
    assert!(
        stderr_of(&unguarded).contains(&sentinel),
        "the fixture is expected to make git reach the repository's textconv program: {}",
        stderr_of(&unguarded)
    );

    // The refusal happens before the invocation, so the diff never runs and the
    // program is never reached.
    assert!(repository.refusal().message().contains("attributes"));
}

/// Which line ending a checkout materializes.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum WorktreeEol {
    Lf,
    Crlf,
}

/// One repository shape whose working tree Git materialized itself, so the bytes
/// on disk are exactly what this configuration and these attributes call for.
struct Checkout {
    _temp: tempfile::TempDir,
    root: PathBuf,
    eol: WorktreeEol,
}

impl Checkout {
    fn create(attributes: Option<&str>, autocrlf: Option<&str>, linked_worktree: bool) -> Self {
        let temp = tempfile::tempdir().expect("a temporary directory is available");
        let repository = temp.path().join("repository");
        std::fs::create_dir_all(&repository).expect("the fixture repository is creatable");
        fixture_git_or_panic(&repository, &["init", "-q", "-b", "main"]);
        fixture_git_or_panic(
            &repository,
            &["config", "user.email", "fixture@example.invalid"],
        );
        fixture_git_or_panic(&repository, &["config", "user.name", "Fixture"]);
        if let Some(autocrlf) = autocrlf {
            fixture_git_or_panic(&repository, &["config", "core.autocrlf", autocrlf]);
        }
        if let Some(attributes) = attributes {
            std::fs::write(repository.join(".gitattributes"), attributes).expect("fixture file");
        }
        std::fs::write(repository.join("a.txt"), "line1\nline2\n").expect("fixture file");
        fixture_git_or_panic(&repository, &["add", "-A"]);
        fixture_git_or_panic(&repository, &["commit", "-qm", "initial"]);

        let root = if linked_worktree {
            let worktree = temp.path().join("linked");
            fixture_git_or_panic(
                &repository,
                &["worktree", "add", "-q", &worktree.to_string_lossy()],
            );
            worktree
        } else {
            repository
        };
        // Let Git write the working tree from the index under this
        // configuration, which is what a checkout does. `.git` is skipped: in a
        // linked worktree it is the pointer file that names the repository.
        for entry in std::fs::read_dir(&root).expect("the checkout is readable") {
            let path = entry.expect("a checkout entry").path();
            if path.is_file() && path.file_name().is_some_and(|name| name != ".git") {
                std::fs::remove_file(&path).expect("a tracked file is removable");
            }
        }
        fixture_git_or_panic(&root, &["checkout", "-f", "--", "."]);

        // A tool that rewrites a file without changing it (an editor save, a
        // formatter, a checkout script) makes Git compare content again. That is
        // the state in which a checkout whose normalization Zenith cannot see
        // reads as modified, so the fixture touches the file the same way.
        let touched = std::fs::read(root.join("a.txt")).expect("the checked-out file");
        std::fs::write(root.join("a.txt"), &touched).expect("the file is rewritable");

        let bytes = std::fs::read(root.join("a.txt")).expect("the checked-out file");
        let eol = if bytes.windows(2).any(|pair| pair == b"\r\n") {
            WorktreeEol::Crlf
        } else {
            assert!(
                bytes.contains(&b'\n'),
                "the fixture is expected to check out a text file"
            );
            WorktreeEol::Lf
        };
        Self {
            _temp: temp,
            root,
            eol,
        }
    }

    /// The working tree Git produced is the one this case claims.
    fn assert_expected_bytes(&self, expected: WorktreeEol) {
        assert_eq!(
            self.eol, expected,
            "the fixture checkout did not produce {expected:?} bytes"
        );
    }

    fn user_status(&self) -> String {
        String::from_utf8_lossy(
            &fixture_git(&self.root, &["status", "--porcelain=v1", "-uno"]).stdout,
        )
        .to_string()
    }

    fn zenith_status(&self) -> String {
        let mut command = zenith_lib::git::git_command(&self.root, &environment())
            .expect("a repository with a clean configuration is not refused");
        command.args(["status", "--porcelain=v1", "-z", "--untracked-files=all"]);
        let output =
            zenith_platform::subprocess::run_with_timeout(command, Duration::from_secs(10))
                .expect("the neutralized invocation completes");
        assert!(
            output.status.success(),
            "the neutralized status must succeed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }
}

/// A clean checkout has to read as clean, for the user's own `git status` and
/// for Zenith's.
///
/// The attribute-driven cases are the ones a neutralized attribute source broke:
/// with `attr.tree` pinned to the empty tree, Git compared the raw bytes of a
/// `text`/`eol` file with the index and reported a modified file that the user's
/// own `git status` calls clean. `core.autocrlf` is the same class of machine
/// configuration, and the Windows checkout that failed CI had it set from the
/// system configuration.
#[test]
fn a_clean_checkout_reads_clean_for_the_user_and_for_zenith() {
    let cases: [(&str, Option<&str>, Option<&str>, WorktreeEol); 4] = [
        ("core.autocrlf=true", None, Some("true"), WorktreeEol::Crlf),
        ("core.autocrlf=false", None, Some("false"), WorktreeEol::Lf),
        (
            "text eol=crlf attribute",
            Some("*.txt text eol=crlf\n"),
            None,
            WorktreeEol::Crlf,
        ),
        (
            "text eol=lf attribute",
            Some("*.txt text eol=lf\n"),
            None,
            WorktreeEol::Lf,
        ),
    ];
    for (label, attributes, autocrlf, expected) in cases {
        for linked_worktree in [false, true] {
            let kind = if linked_worktree {
                "linked worktree"
            } else {
                "checkout"
            };
            let checkout = Checkout::create(attributes, autocrlf, linked_worktree);
            checkout.assert_expected_bytes(expected);
            let user = checkout.user_status();
            assert!(
                user.is_empty(),
                "the fixture must be clean for the user's own git ({label}, {kind}): {user:?}"
            );
            let zenith = checkout.zenith_status();
            assert!(
                zenith.is_empty(),
                "Zenith must report the clean {label} {kind} as clean, but read {zenith:?}"
            );
        }
    }
}
