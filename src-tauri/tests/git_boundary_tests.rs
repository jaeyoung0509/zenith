//! Zenith invokes `git` in directories the user did not nominate.
//!
//! The project root comes from an observed agent process working directory, so
//! a repository Zenith did not create can supply `.git/config` and
//! `.gitattributes` for the invocation. Several documented Git keys name a
//! program Git then runs: `core.fsmonitor` is consulted by `git status`
//! specifically, and `core.pager`, `core.hooksPath`, `diff.external`,
//! `core.sshCommand`, and the `filter.*` clean/smudge pair are the same class.
//!
//! These tests hold both halves of the boundary: every invocation is built by
//! one constructor (`tooling::git_command`), and the programs a repository
//! names in that configuration do not run.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The one module allowed to construct a `git` command.
const CHOKEPOINT: &str = "src/tooling.rs";

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
fn production_source(text: &str) -> &str {
    match text.find("#[cfg(test)]\nmod tests") {
        Some(index) => &text[..index],
        None => text,
    }
}

#[test]
fn every_git_invocation_goes_through_the_neutralizing_constructor() {
    let workspace = workspace_root();
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
        let relative = source
            .strip_prefix(&workspace)
            .unwrap_or(source)
            .to_string_lossy()
            .to_string();
        if relative.ends_with(CHOKEPOINT) {
            continue;
        }
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
        "every git invocation must go through `tooling::git_command`: {offenders:#?}"
    );

    // The constructor itself is the other half: it must still be the place the
    // neutralization lives, so a rename cannot silently empty this guard.
    let chokepoint = std::fs::read_to_string(workspace.join("src-tauri").join(CHOKEPOINT))
        .expect("the tooling module is readable");
    assert!(
        chokepoint.contains("pub fn git_command("),
        "the chokepoint constructor is missing from {CHOKEPOINT}"
    );
}

/// A repository that names programs in the configuration keys Git executes.
struct PoisonedRepository {
    _temp: tempfile::TempDir,
    root: PathBuf,
    /// Named by `core.fsmonitor`, which `git status` consults.
    fsmonitor_program: PathBuf,
    /// Named by `filter.probe.clean`, which reading working-tree content runs.
    filter_program: PathBuf,
    /// Named by `diff.external`, which a content diff would otherwise run.
    external_diff_program: PathBuf,
}

impl PoisonedRepository {
    fn create() -> Self {
        let temp = tempfile::tempdir().expect("a temporary directory is available");
        let root = temp.path().join("workspace");
        let missing = temp.path().join("no-such-program");
        std::fs::create_dir_all(&root).expect("the fixture repository is creatable");
        let fsmonitor_program = missing.join("fsmonitor-program");
        let filter_program = missing.join("filter-program");
        let external_diff_program = missing.join("external-diff-program");

        let git = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .expect("git is installed on this host");
            assert!(
                status.status.success(),
                "fixture `git {args:?}` failed: {}",
                String::from_utf8_lossy(&status.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "fixture@example.invalid"]);
        git(&["config", "user.name", "Fixture"]);
        std::fs::write(root.join("tracked.txt"), "baseline content\n").expect("fixture file");
        std::fs::write(root.join(".gitattributes"), "* filter=probe\n").expect("fixture file");
        git(&["add", "-A"]);
        git(&["commit", "-qm", "initial"]);

        // The poisoned configuration goes in after the commit: a clean filter
        // is also applied by `git add`, and the fixture needs a repository that
        // was created legitimately and configured afterwards.
        git(&[
            "config",
            "core.fsmonitor",
            &fsmonitor_program.to_string_lossy(),
        ]);
        git(&[
            "config",
            "filter.probe.clean",
            &filter_program.to_string_lossy(),
        ]);
        git(&["config", "filter.probe.required", "true"]);
        git(&[
            "config",
            "diff.external",
            &external_diff_program.to_string_lossy(),
        ]);
        std::fs::write(root.join("tracked.txt"), "modified content\n").expect("fixture file");

        Self {
            _temp: temp,
            root,
            fsmonitor_program,
            filter_program,
            external_diff_program,
        }
    }

    /// A `git` invocation that does not go through the neutralized constructor,
    /// used as the fixture's positive control.
    fn unguarded(&self, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()
            .expect("git is installed on this host")
    }

    fn guarded(&self, args: &[&str]) -> std::process::Output {
        let mut command = zenith_lib::tooling::git_command(&self.root);
        command.args(args);
        zenith_platform::subprocess::run_with_timeout(command, Duration::from_secs(10))
            .expect("the neutralized invocation completes")
    }
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn a_repository_cannot_name_a_program_that_git_status_runs() {
    let repository = PoisonedRepository::create();
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
fn a_repository_cannot_name_a_program_that_reading_working_tree_content_runs() {
    let repository = PoisonedRepository::create();
    let filter = repository.filter_program.to_string_lossy().to_string();
    let external_diff = repository
        .external_diff_program
        .to_string_lossy()
        .to_string();

    // Positive control: a clean filter is what Git runs to compare working-tree
    // content with the index, the fixture makes it required, and a content diff
    // without `--no-ext-diff` would run the configured external diff too.
    let unguarded = repository.unguarded(&["diff", "--no-color", "--", "tracked.txt"]);
    let unguarded_stderr = stderr_of(&unguarded);
    assert!(
        unguarded_stderr.contains(&filter),
        "the fixture is expected to make git reach the repository's clean filter, got: {unguarded_stderr}"
    );

    // The production shape: every content diff carries `--no-ext-diff`, so the
    // neutralized invocation still reports the change it exists to report.
    let guarded = repository.guarded(&["diff", "--no-ext-diff", "--no-color", "--", "tracked.txt"]);
    assert!(
        guarded.status.success(),
        "the neutralized diff must still succeed: {}",
        stderr_of(&guarded)
    );
    let guarded_stderr = stderr_of(&guarded);
    assert!(
        !guarded_stderr.contains(&filter) && !guarded_stderr.contains(&external_diff),
        "repository-supplied programs were reached despite neutralization: {guarded_stderr}"
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
