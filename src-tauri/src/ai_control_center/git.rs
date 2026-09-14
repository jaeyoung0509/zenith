use crate::git::{GitDirectory, GitInspection, GitRefusal};
use crate::models::GitChangeSummary;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zenith_platform::PlatformEnvironment;

#[derive(Debug, Clone, Default)]
pub struct GitBaselineStore {
    baselines: HashMap<String, GitObservation>,
    generation: u64,
}

/// One observation of a project's Git state.
///
/// The two states are exclusive, so an unreadable observation carries the
/// reason it could not be read instead of an empty "readable" one: a repository
/// whose invocation was refused, or whose `git` failed, must not be presented
/// as an unchanged tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GitObservation {
    Read {
        captured_at: u64,
        state: ReadState,
    },
    Unreadable {
        captured_at: u64,
        reason: GitUnavailable,
    },
}

/// Why a project's Git state could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GitUnavailable {
    /// The root is not inside a repository: no `.git`, or one that resolves to
    /// nothing.
    NoRepository,
    /// Zenith refuses to run `git` there, and the reason says why.
    Refused(GitRefusal),
    /// `git` ran but did not answer: a failure, not an empty state.
    CommandFailed(String),
}

impl GitUnavailable {
    /// The reason as a user-facing sentence.
    fn message(&self) -> String {
        match self {
            GitUnavailable::NoRepository => "Git unavailable: not a repository".into(),
            GitUnavailable::Refused(refusal) => refusal.message(),
            GitUnavailable::CommandFailed(reason) => {
                format!("Git could not be read in this repository: {reason}")
            }
        }
    }
}

impl GitObservation {
    fn read(captured_at: u64, state: ReadState) -> Self {
        GitObservation::Read { captured_at, state }
    }

    fn unreadable(captured_at: u64, reason: GitUnavailable) -> Self {
        GitObservation::Unreadable {
            captured_at,
            reason,
        }
    }

    fn captured_at(&self) -> u64 {
        match self {
            GitObservation::Read { captured_at, .. }
            | GitObservation::Unreadable { captured_at, .. } => *captured_at,
        }
    }

    /// The state an invocation can compare against, when this observation could
    /// be read.
    fn read_state(&self) -> Option<&ReadState> {
        match self {
            GitObservation::Read { state, .. } => Some(state),
            GitObservation::Unreadable { .. } => None,
        }
    }

    fn head(&self) -> Option<&str> {
        self.read_state().and_then(|state| state.head.as_deref())
    }
}

/// The values one capture reads out of a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadState {
    /// `HEAD`, or `None` in a repository whose branch has no commits yet.
    head: Option<String>,
    statuses: HashMap<String, String>,
    fingerprints: HashMap<String, String>,
    /// Whether the working-tree listing reached Zenith's capture cap, so the
    /// untracked paths are missing from this state.
    untracked_dropped: bool,
}

impl GitBaselineStore {
    pub fn summaries(
        &mut self,
        roots: &HashMap<String, PathBuf>,
        environment: &PlatformEnvironment,
        now: u64,
    ) -> Vec<GitChangeSummary> {
        let generation = self.generation;
        let snapshot = self.snapshot_baselines();
        let (summaries, collected) = Self::collect_summaries(&snapshot, roots, environment, now);
        // Generation cannot have changed under `&mut self`; the commit always
        // applies here. The split admission path below uses the checked form.
        let _ = self.commit_collected(generation, roots, collected);
        summaries
    }

    /// Snapshot the smallest immutable inputs needed for collection. Callers
    /// hold the shared Control Center lock only for this clone, then run
    /// Git/filesystem work outside the lock.
    pub(crate) fn snapshot_baselines(&self) -> HashMap<String, GitObservation> {
        self.baselines.clone()
    }

    pub(crate) fn baseline_generation(&self) -> u64 {
        self.generation
    }

    /// Run Git commands and filesystem fingerprint work. Pure: touches no
    /// shared state. Returns summaries plus the updated baseline map.
    ///
    /// The baseline of a project is the first observation that could be read.
    /// A project observed while refusable, failed, or outside a repository has
    /// no baseline to compare against, and the first readable observation after
    /// that becomes one: comparing against an observation that never happened
    /// would report every change since then as a change since the baseline.
    pub(crate) fn collect_summaries(
        baselines: &HashMap<String, GitObservation>,
        roots: &HashMap<String, PathBuf>,
        environment: &PlatformEnvironment,
        now: u64,
    ) -> (Vec<GitChangeSummary>, HashMap<String, GitObservation>) {
        let mut next = baselines.clone();
        // Prune baselines for projects that no longer exist in this observation
        // without touching live shared state.
        next.retain(|id, _| roots.contains_key(id));
        let mut summaries = Vec::new();
        for (project_id, root) in roots {
            // One inspection per observation: the identity, the status read, and
            // the committed diff all run against the same decision.
            let inspection = GitInspection::open(root, environment);
            let current = capture(&inspection, now);
            let baseline = next
                .entry(project_id.clone())
                .or_insert_with(|| current.clone());
            let reestablished = matches!(baseline, GitObservation::Unreadable { .. })
                && matches!(current, GitObservation::Read { .. });
            if reestablished {
                *baseline = current.clone();
            }
            summaries.push(compare(
                project_id,
                &inspection,
                baseline,
                &current,
                reestablished,
            ));
        }
        summaries.sort_by(|a, b| a.project_id.cmp(&b.project_id));
        // `next` already holds first-read baselines: existing entries keep their
        // original capture, new ids store their own first capture.
        (summaries, next)
    }

    /// Reacquire the lock and merge. Validates the generation and project
    /// identity: superseded observations are discarded without restoring
    /// removed projects or overwriting newer baselines.
    pub(crate) fn commit_collected(
        &mut self,
        snapshot_generation: u64,
        observed_roots: &HashMap<String, PathBuf>,
        collected: HashMap<String, GitObservation>,
    ) -> bool {
        if self.generation != snapshot_generation {
            return false;
        }
        // Never restore projects removed from live state by a newer commit
        // (generation check above covers the concurrent case); only merge ids
        // present in this observation and prune to it.
        self.baselines
            .retain(|id, _| observed_roots.contains_key(id));
        for (id, baseline) in collected {
            if observed_roots.contains_key(&id) {
                self.baselines.insert(id, baseline);
            }
        }
        self.generation = self.generation.wrapping_add(1);
        true
    }

    /// Clone one baseline under a short lock. The caller runs the filesystem
    /// capture and diff computation outside the shared lock, then uses the
    /// pure [`Self::diff_context_with_baseline`] helper.
    pub(crate) fn baseline_snapshot(&self, project_id: &str) -> Option<GitObservation> {
        self.baselines.get(project_id).cloned()
    }

    /// Pure diff-context computation from an already-snapshotted baseline.
    /// Runs outside any shared-state lock.
    ///
    /// `Err` carries the reason the context cannot be produced — an unreadable
    /// baseline, or a capture that just failed — because an empty diff would
    /// present unread state as a measurement.
    pub(crate) fn diff_context_with_baseline(
        baseline: &GitObservation,
        root: &Path,
        environment: &PlatformEnvironment,
        now: u64,
    ) -> Result<GitDiffContext, String> {
        let inspection = GitInspection::open(root, environment);
        let current = capture(&inspection, now);
        if let GitObservation::Unreadable { reason, .. } = &current {
            return Err(reason.message());
        }
        let GitObservation::Read { .. } = baseline else {
            return Err(match baseline {
                GitObservation::Unreadable { reason, .. } => reason.message(),
                GitObservation::Read { .. } => unreachable!("matched above"),
            });
        };
        let (changed, committed_failure) = all_changed_entries(&inspection, baseline, &current);
        let caveat = current
            .read_state()
            .and_then(|state| observation_caveat(state, committed_failure));
        Ok(GitDiffContext {
            baseline_head: baseline.head().map(str::to_string),
            paths: changed.into_iter().map(|(path, _)| path).collect(),
            caveat,
        })
    }
}

/// The inputs of one diff request: what to compare, and what the comparison
/// cannot show.
pub(crate) struct GitDiffContext {
    pub baseline_head: Option<String>,
    pub paths: Vec<String>,
    /// What could not be read when the paths were collected, when something
    /// could not: a diff that omitted it silently would look complete.
    pub caveat: Option<String>,
}

/// Reads one observation of `root` through an already-open inspection.
fn capture(inspection: &GitInspection, now: u64) -> GitObservation {
    if matches!(inspection.directory(), GitDirectory::Absent) {
        return GitObservation::unreadable(now, GitUnavailable::NoRepository);
    }
    if let Some(refusal) = inspection.refusal() {
        return GitObservation::unreadable(now, GitUnavailable::Refused(refusal.clone()));
    }
    match read_state(inspection) {
        Ok(state) => GitObservation::read(now, state),
        Err(reason) => GitObservation::unreadable(now, GitUnavailable::CommandFailed(reason)),
    }
}

/// The HEAD and working-tree state of a readable repository.
///
/// Every step distinguishes "Git answered" from "Git failed": a repository
/// whose `git status` times out is an unreadable repository, not a clean one.
fn read_state(inspection: &GitInspection) -> Result<ReadState, String> {
    let head = read_head(inspection)?;
    let (statuses, untracked_dropped) = read_status(inspection)?;
    let fingerprints = statuses
        .keys()
        .map(|path| (path.clone(), worktree_fingerprint(inspection.root(), path)))
        .collect();
    Ok(ReadState {
        head,
        statuses,
        fingerprints,
        untracked_dropped,
    })
}

/// `HEAD` of the repository, or `None` before its first commit.
fn read_head(inspection: &GitInspection) -> Result<Option<String>, String> {
    let output = run_git(inspection, &["rev-parse", "--verify", "--quiet", "HEAD"])?;
    match output.code {
        Some(0) => Ok(output.stdout.lines().next().map(str::to_string)),
        // `--quiet` answers "no such revision" with exit 1: the repository is
        // usable and its branch simply has no commits yet.
        Some(1) => Ok(None),
        _ => Err(format!(
            "git rev-parse HEAD exited with {}",
            describe_exit(output.code)
        )),
    }
}

/// The working-tree status Git reports for the repository, and whether the
/// untracked listing had to be dropped to read it at all.
///
/// Reaching the capture cap is a limit of this read, not a repository that
/// cannot be read: a checkout with tens of thousands of untracked paths (a fresh
/// project without `.gitignore`) would otherwise never be readable. The tracked
/// state is read without the untracked listing instead, and the observation
/// carries what was dropped so the summary can say so. A tracked listing that
/// reaches the cap as well leaves the state unknown, and that is refused.
fn read_status(inspection: &GitInspection) -> Result<(HashMap<String, String>, bool), String> {
    let output = run_git(
        inspection,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if output.code != Some(0) {
        return Err(format!(
            "git status exited with {}",
            describe_exit(output.code)
        ));
    }
    if !truncated(&output) {
        return Ok((parse_status(output.stdout.as_bytes()), false));
    }
    let output = run_git(
        inspection,
        &["status", "--porcelain=v1", "-z", "--untracked-files=no"],
    )?;
    if output.code != Some(0) {
        return Err(format!(
            "git status exited with {}",
            describe_exit(output.code)
        ));
    }
    if truncated(&output) {
        return Err(format!(
            "git status answered with more than {} bytes even without the untracked listing, which cannot be read as a complete status",
            zenith_platform::subprocess::MAX_CAPTURE_BYTES
        ));
    }
    Ok((parse_status(output.stdout.as_bytes()), true))
}

/// What could not be read about one observation, when something could not.
///
/// A caveat is reported instead of being subtracted: a summary that counted only
/// what it could see would present an incomplete read as a complete one.
fn observation_caveat(current: &ReadState, committed_failure: Option<String>) -> Option<String> {
    let mut notes = Vec::new();
    if let Some(reason) = committed_failure {
        notes.push(format!(
            "the committed comparison could not be read ({reason})"
        ));
    }
    if current.untracked_dropped {
        notes.push(format!(
            "untracked paths were not listed (the working-tree listing exceeded {} bytes)",
            zenith_platform::subprocess::MAX_CAPTURE_BYTES
        ));
    }
    (!notes.is_empty()).then(|| notes.join("; "))
}

/// Whether an answer reached the capture cap, which means it may have been cut
/// short. A partial listing must not be read as a complete one.
fn truncated(output: &GitOutput) -> bool {
    output.stdout.len() >= zenith_platform::subprocess::MAX_CAPTURE_BYTES
}

fn describe_exit(code: Option<i32>) -> String {
    match code {
        Some(code) => code.to_string(),
        None => "a signal".to_string(),
    }
}

/// One `git` invocation against a repository that was already inspected.
struct GitOutput {
    stdout: String,
    code: Option<i32>,
}

fn run_git(inspection: &GitInspection, args: &[&str]) -> Result<GitOutput, String> {
    let mut command = inspection.command().map_err(|refusal| refusal.message())?;
    command.args(args);
    let output = zenith_platform::subprocess::run_with_timeout(command, COMMAND_TIMEOUT)
        .map_err(|error| error.to_string())?;
    Ok(GitOutput {
        // NUL delimiters and path whitespace are preserved for the
        // porcelain/name-status parsers; scalar callers normalize their own
        // output.
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        code: output.status.code(),
    })
}

/// Bound on one invocation. The window is per command, not per capture: the
/// caller may run several against the same repository.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

fn compare(
    project_id: &str,
    inspection: &GitInspection,
    baseline: &GitObservation,
    current: &GitObservation,
    reestablished: bool,
) -> GitChangeSummary {
    if let GitObservation::Unreadable { reason, .. } = current {
        return unreadable_summary(project_id, baseline.captured_at(), reason.message());
    }
    if reestablished {
        // The baseline was just established by this observation: there is no
        // earlier state to count changes against.
        return GitChangeSummary {
            project_id: project_id.into(),
            baseline_head: current.head().map(str::to_string),
            current_head: current.head().map(str::to_string),
            baseline_at: current.captured_at(),
            added: 0,
            modified: 0,
            deleted: 0,
            renamed: 0,
            untracked: 0,
            changed_paths: vec![],
            available: true,
            status_message:
                "Baseline established by this observation; Git state was not readable before it."
                    .into(),
        };
    }
    if let GitObservation::Unreadable { reason, .. } = baseline {
        return unreadable_summary(project_id, baseline.captured_at(), reason.message());
    }
    let (changed, committed_failure) = all_changed_entries(inspection, baseline, current);
    let caveat = current
        .read_state()
        .and_then(|state| observation_caveat(state, committed_failure));
    let count = |needle: char| {
        changed
            .iter()
            .filter(|(_, status)| status.contains(needle))
            .count() as u32
    };
    GitChangeSummary {
        project_id: project_id.into(),
        baseline_head: baseline.head().map(str::to_string),
        current_head: current.head().map(str::to_string),
        baseline_at: baseline.captured_at(),
        added: count('A'),
        modified: count('M'),
        deleted: count('D'),
        renamed: count('R'),
        untracked: changed.iter().filter(|(_, status)| status == "??").count() as u32,
        changed_paths: changed.into_iter().map(|(path, _)| path).collect(),
        available: true,
        // What could not be read is reported instead of quietly subtracting the
        // changes it could not see.
        status_message: match caveat {
            None => UNCHANGED_MESSAGE.into(),
            Some(caveat) => format!("{UNCHANGED_MESSAGE} Incomplete: {caveat}."),
        },
    }
}

const UNCHANGED_MESSAGE: &str = "Files changed since baseline across commits and the working tree.";

fn unreadable_summary(project_id: &str, baseline_at: u64, message: String) -> GitChangeSummary {
    GitChangeSummary {
        project_id: project_id.into(),
        baseline_head: None,
        current_head: None,
        baseline_at,
        added: 0,
        modified: 0,
        deleted: 0,
        renamed: 0,
        untracked: 0,
        changed_paths: vec![],
        available: false,
        status_message: message,
    }
}

/// The changed entries of one observation, plus the reason a commit-to-commit
/// comparison could not be added when it failed.
fn all_changed_entries(
    inspection: &GitInspection,
    baseline: &GitObservation,
    current: &GitObservation,
) -> (Vec<(String, String)>, Option<String>) {
    let (Some(baseline), Some(current)) = (baseline.read_state(), current.read_state()) else {
        return (Vec::new(), None);
    };
    let mut changed = changed_entries(baseline, current)
        .into_iter()
        .collect::<HashMap<_, _>>();
    let committed = committed_entries(inspection, baseline, current);
    if let Ok(entries) = &committed {
        for (path, status) in entries {
            changed
                .entry(path.clone())
                .and_modify(|existing| {
                    if existing == "resolved" {
                        *existing = status.clone();
                    }
                })
                .or_insert_with(|| status.clone());
        }
    }
    let mut changed = changed.into_iter().collect::<Vec<_>>();
    changed.sort_by(|a, b| a.0.cmp(&b.0));
    changed.truncate(256);
    (changed, committed.err())
}

fn committed_entries(
    inspection: &GitInspection,
    baseline: &ReadState,
    current: &ReadState,
) -> Result<Vec<(String, String)>, String> {
    let (Some(base), Some(head)) = (baseline.head.as_deref(), current.head.as_deref()) else {
        return Ok(Vec::new());
    };
    if base == head {
        return Ok(Vec::new());
    }
    let output = run_git(
        inspection,
        &["diff", "--name-status", "-z", base, head, "--"],
    )?;
    if output.code != Some(0) {
        return Err(format!(
            "git diff {base} {head} exited with {}",
            describe_exit(output.code)
        ));
    }
    if truncated(&output) {
        return Err(format!(
            "git diff {base} {head} answered with more than {} bytes, which cannot be read as a complete listing",
            zenith_platform::subprocess::MAX_CAPTURE_BYTES
        ));
    }
    Ok(parse_name_status(output.stdout.as_bytes()))
}

fn parse_name_status(bytes: &[u8]) -> Vec<(String, String)> {
    let tokens = bytes
        .split(|byte| *byte == 0)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let mut entries = Vec::new();
    let mut index = 0;
    while index + 1 < tokens.len() {
        let status = String::from_utf8_lossy(tokens[index]).to_string();
        index += 1;
        let first_path = String::from_utf8_lossy(tokens[index]).to_string();
        index += 1;
        let path = if (status.starts_with('R') || status.starts_with('C')) && index < tokens.len() {
            let destination = String::from_utf8_lossy(tokens[index]).to_string();
            index += 1;
            destination
        } else {
            first_path
        };
        entries.push((path, status.chars().next().unwrap_or('M').to_string()));
    }
    entries
}

fn changed_entries(baseline: &ReadState, current: &ReadState) -> Vec<(String, String)> {
    let mut changed = current
        .statuses
        .iter()
        .filter(|(path, status)| {
            baseline.statuses.get(*path) != Some(*status)
                || baseline.fingerprints.get(*path) != current.fingerprints.get(*path)
        })
        .map(|(path, status)| (path.clone(), status.clone()))
        .collect::<Vec<_>>();
    for path in baseline
        .statuses
        .keys()
        .filter(|path| !current.statuses.contains_key(*path))
    {
        changed.push((path.clone(), "resolved".into()));
    }
    changed.sort_by(|a, b| a.0.cmp(&b.0));
    changed.truncate(256);
    changed
}

fn worktree_fingerprint(root: &Path, relative: &str) -> String {
    let path = root.join(relative);
    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
        return "missing".into();
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return format!("non-file:{}", metadata.len());
    }
    const MAX_HASH_BYTES: u64 = 4 * 1024 * 1024;
    if metadata.len() <= MAX_HASH_BYTES {
        if let Ok(bytes) = std::fs::read(&path) {
            let mut digest = Sha256::new();
            digest.update(&bytes);
            return crate::hash::hex(&digest.finalize())[..16].to_string();
        }
    }
    format!("len:{}", metadata.len())
}

fn parse_status(bytes: &[u8]) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let mut index = 0;
    let tokens = bytes
        .split(|byte| *byte == 0)
        .filter(|slice| !slice.is_empty())
        .collect::<Vec<_>>();
    while index < tokens.len() {
        let entry = String::from_utf8_lossy(tokens[index]).to_string();
        if entry.len() < 4 {
            index += 1;
            continue;
        }
        let status = entry[..2].to_string();
        let path = entry[3..].to_string();
        if status.contains('R') || status.contains('C') {
            index += 1;
        }
        result.insert(path, status);
        index += 1;
    }
    result
}

fn run_diff_command(inspection: &GitInspection, args: &[&str]) -> Result<String, String> {
    let mut command = inspection.command().map_err(|refusal| refusal.message())?;
    command.args(args);
    let output = zenith_platform::subprocess::run_with_timeout(command, COMMAND_TIMEOUT)
        .map_err(|error| error.to_string())?;
    if output.status.success() || output.status.code() == Some(1) {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err("Git diff unavailable".into())
    }
}

/// The explicit diff between the baseline and the working tree.
///
/// A repository Zenith refuses is not an unchanged one: reporting an empty diff
/// would present unread state as a measurement.
pub fn explicit_diff(
    root: &Path,
    environment: &PlatformEnvironment,
    baseline_head: Option<&str>,
    paths: &[String],
) -> Result<String, String> {
    let inspection = GitInspection::open(root, environment);
    inspection.command().map_err(|refusal| refusal.message())?;
    if paths.is_empty() {
        return Ok(String::new());
    }
    const MAX: usize = 262_144;
    let mut combined_diff = String::new();

    // 1. Try git diff HEAD for tracked modifications
    let mut head_args = vec!["diff"];
    head_args.push(baseline_head.unwrap_or("HEAD"));
    head_args.extend(["--no-ext-diff", "--no-textconv", "--no-color", "--"]);
    for p in paths {
        head_args.push(p);
    }
    if let Ok(tracked_diff) = run_diff_command(&inspection, &head_args) {
        combined_diff.push_str(&tracked_diff);
    } else {
        // Fallback for fresh repos before initial commit
        let mut empty_args = vec!["diff", "--no-ext-diff", "--no-textconv", "--no-color", "--"];
        for p in paths {
            empty_args.push(p);
        }
        if let Ok(staged_diff) = run_diff_command(&inspection, &empty_args) {
            combined_diff.push_str(&staged_diff);
        }
    }

    // 2. Include untracked files using diff --no-index /dev/null <path>
    for p in paths {
        let full_path = root.join(p);
        if !full_path.starts_with(root) {
            continue;
        }
        if let Ok(metadata) = std::fs::symlink_metadata(&full_path) {
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() <= MAX as u64
            {
                let marker = format!("b/{}", p);
                if !combined_diff.contains(&marker) {
                    if let Ok(untracked_diff) = run_diff_command(
                        &inspection,
                        &[
                            "diff",
                            "--no-index",
                            "--no-ext-diff",
                            "--no-textconv",
                            "--no-color",
                            "--",
                            "/dev/null",
                            p,
                        ],
                    ) {
                        if !combined_diff.is_empty() && !combined_diff.ends_with('\n') {
                            combined_diff.push('\n');
                        }
                        combined_diff.push_str(&untracked_diff);
                    }
                }
            }
        }
    }

    if combined_diff.len() > MAX {
        Ok(format!(
            "{}\n\n[Diff truncated by Zenith at 256 KiB]",
            &combined_diff[..combined_diff.floor_char_boundary(MAX)]
        ))
    } else {
        Ok(combined_diff)
    }
}

pub fn fingerprint_path(path: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(path);
    crate::hash::hex(&digest.finalize())[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenith_platform::PlatformEnvironment;

    fn environment() -> PlatformEnvironment {
        PlatformEnvironment::native()
    }

    fn git(root: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "fixture `git {args:?}` failed");
    }

    fn roots(root: &Path) -> HashMap<String, PathBuf> {
        HashMap::from([("p".to_string(), root.to_path_buf())])
    }

    /// A repository with one commit, and `HEAD` on a branch.
    fn committed_repository(root: &Path) {
        git(root, &["init", "-q", "-b", "main"]);
        git(root, &["config", "user.email", "fixture@example.invalid"]);
        git(root, &["config", "user.name", "Fixture"]);
        std::fs::write(root.join("tracked.txt"), "baseline\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "initial"]);
    }

    /// A program-naming definition of the kind a repository supplies for its own
    /// filter driver.
    fn define_filter_program(root: &Path) {
        git(root, &["config", "filter.probe.clean", "no-such-program"]);
    }

    #[test]
    fn baseline_excludes_preexisting_and_reports_subsequent_changes() {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init", "-q"]);
        git(
            temp.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        git(temp.path(), &["config", "user.name", "Test"]);
        std::fs::write(temp.path().join("old.txt"), "old").unwrap();
        git(temp.path(), &["add", "old.txt"]);
        git(temp.path(), &["commit", "-qm", "initial"]);
        std::fs::write(temp.path().join("pre.txt"), "pre").unwrap();
        let roots = roots(temp.path());
        let mut store = GitBaselineStore::default();
        assert!(store.summaries(&roots, &environment(), 10)[0]
            .changed_paths
            .is_empty());
        std::fs::write(temp.path().join("after.txt"), "after").unwrap();
        let summary = store.summaries(&roots, &environment(), 20).remove(0);
        assert_eq!(summary.changed_paths, vec!["after.txt"]);
        assert_eq!(summary.untracked, 1);
    }

    #[test]
    fn baseline_detects_new_content_when_porcelain_status_is_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init", "-q"]);
        std::fs::write(temp.path().join("same-status.txt"), "before").unwrap();
        let roots = roots(temp.path());
        let mut store = GitBaselineStore::default();
        assert!(store.summaries(&roots, &environment(), 10)[0]
            .changed_paths
            .is_empty());
        std::fs::write(temp.path().join("same-status.txt"), "after").unwrap();
        assert_eq!(
            store.summaries(&roots, &environment(), 20)[0].changed_paths,
            vec!["same-status.txt"]
        );
    }

    #[test]
    fn parser_handles_rename_records_without_exposing_old_record_as_status() {
        let parsed = parse_status(b"R  new.txt\0old.txt\0?? loose.txt\0");
        assert_eq!(parsed.get("new.txt").map(String::as_str), Some("R "));
        assert_eq!(parsed.get("loose.txt").map(String::as_str), Some("??"));
        assert!(!parsed.contains_key("old.txt"));
    }

    #[test]
    fn name_status_parser_uses_the_rename_destination() {
        let parsed = parse_name_status(b"R100\0old.txt\0new.txt\0M\0changed.txt\0");
        assert_eq!(
            parsed,
            vec![
                ("new.txt".into(), "R".into()),
                ("changed.txt".into(), "M".into())
            ]
        );
    }

    #[test]
    fn explicit_diff_includes_untracked_files() {
        let temp = tempfile::tempdir().unwrap();
        committed_repository(temp.path());
        std::fs::write(temp.path().join("untracked.rs"), "fn main() {}\n").unwrap();
        std::fs::write(temp.path().join("tracked.txt"), "baseline changed\n").unwrap();

        let diff = explicit_diff(
            temp.path(),
            &environment(),
            None,
            &["tracked.txt".into(), "untracked.rs".into()],
        )
        .expect("diff succeeds");

        assert!(diff.contains("tracked.txt"));
        assert!(diff.contains("untracked.rs"));
        assert!(diff.contains("fn main()"));
    }

    #[test]
    fn baseline_reports_and_diffs_changes_committed_after_capture() {
        let temp = tempfile::tempdir().unwrap();
        committed_repository(temp.path());
        let roots = roots(temp.path());
        let mut store = GitBaselineStore::default();
        assert!(store.summaries(&roots, &environment(), 10)[0]
            .changed_paths
            .is_empty());

        std::fs::write(temp.path().join("tracked.txt"), "after\n").unwrap();
        git(temp.path(), &["add", "tracked.txt"]);
        git(temp.path(), &["commit", "-qm", "agent change"]);

        let summary = store.summaries(&roots, &environment(), 20).remove(0);
        assert_eq!(summary.changed_paths, vec!["tracked.txt"]);
        assert_eq!(summary.modified, 1);
        let baseline = store
            .baseline_snapshot("p")
            .expect("the baseline is stored");
        let context = GitBaselineStore::diff_context_with_baseline(
            &baseline,
            temp.path(),
            &environment(),
            20,
        )
        .unwrap();
        assert!(context.caveat.is_none(), "{:?}", context.caveat);
        let diff = explicit_diff(
            temp.path(),
            &environment(),
            context.baseline_head.as_deref(),
            &context.paths,
        )
        .unwrap();
        assert!(diff.contains("-baseline"));
        assert!(diff.contains("+after"));
    }

    #[test]
    fn split_collection_does_not_hold_the_shared_lock() {
        use std::sync::{mpsc, Arc, Barrier, Mutex};
        use std::time::Duration;

        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init", "-q"]);
        let roots: HashMap<String, PathBuf> =
            HashMap::from([("p".into(), temp.path().to_path_buf())]);
        let store = Arc::new(Mutex::new(GitBaselineStore::default()));

        // Snapshot under a short critical section, then collect outside.
        let (snapshot, generation) = {
            let guard = store.lock().unwrap();
            (guard.snapshot_baselines(), guard.baseline_generation())
        };
        let barrier = Arc::new(Barrier::new(2));
        let (done_tx, done_rx) = mpsc::channel();
        let barrier_collector = barrier.clone();
        let roots_collector = roots.clone();
        let collector = std::thread::spawn(move || {
            barrier_collector.wait();
            // Deliberately blocked Git/filesystem work runs with no lock held.
            std::thread::sleep(Duration::from_millis(150));
            let (summaries, collected) = GitBaselineStore::collect_summaries(
                &snapshot,
                &roots_collector,
                &environment(),
                99,
            );
            done_tx.send((summaries, collected)).unwrap();
        });

        barrier.wait();
        // The shared lock must be acquirable while collection is blocked.
        let lock_acquired = store.lock().map(|_| ()).map_err(|_| ()).is_ok();
        assert!(lock_acquired, "collector must not hold the shared lock");
        // Baseline semantics are preserved through the split path.
        assert!(store.lock().unwrap().snapshot_baselines().is_empty());

        let (summaries, collected) = done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        collector.join().unwrap();
        assert!(summaries[0].changed_paths.is_empty());
        assert!(store
            .lock()
            .unwrap()
            .commit_collected(generation, &roots, collected));
        assert_eq!(store.lock().unwrap().baseline_generation(), generation + 1);
    }

    #[test]
    fn split_commit_rejects_outdated_results() {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init", "-q"]);
        let roots: HashMap<String, PathBuf> =
            HashMap::from([("p".into(), temp.path().to_path_buf())]);
        let mut store = GitBaselineStore::default();
        let generation = store.baseline_generation();
        let snapshot = store.snapshot_baselines();
        let (_, stale_collected) =
            GitBaselineStore::collect_summaries(&snapshot, &roots, &environment(), 10);
        // A newer commit supersedes the stale observation.
        let (_, fresh_collected) =
            GitBaselineStore::collect_summaries(&snapshot, &roots, &environment(), 11);
        assert!(store.commit_collected(generation, &roots, fresh_collected));
        assert!(
            !store.commit_collected(generation, &roots, stale_collected),
            "outdated results must not overwrite newer baselines"
        );
        // Committing an empty observation prunes without restoring removed
        // projects.
        let gen = store.baseline_generation();
        let empty_snapshot = store.snapshot_baselines();
        let empty: HashMap<String, PathBuf> = HashMap::new();
        let (_, empty_collected) =
            GitBaselineStore::collect_summaries(&empty_snapshot, &empty, &environment(), 12);
        assert!(store.commit_collected(gen, &empty, empty_collected));
        assert!(store.snapshot_baselines().is_empty());
    }

    #[test]
    fn a_repository_that_defines_a_filter_program_is_reported_unavailable() {
        let temp = tempfile::tempdir().unwrap();
        committed_repository(temp.path());
        define_filter_program(temp.path());
        let roots = roots(temp.path());
        let mut store = GitBaselineStore::default();

        let summary = store.summaries(&roots, &environment(), 10).remove(0);
        assert!(
            !summary.available,
            "a repository Zenith refuses must not be reported as available: {summary:?}"
        );
        assert!(
            summary.status_message.contains("filter.probe.clean"),
            "the reason must name the definition: {}",
            summary.status_message
        );
        assert!(summary.changed_paths.is_empty());
    }

    #[test]
    fn the_first_readable_observation_after_a_refusal_becomes_the_baseline() {
        let temp = tempfile::tempdir().unwrap();
        committed_repository(temp.path());
        define_filter_program(temp.path());
        let roots = roots(temp.path());
        let mut store = GitBaselineStore::default();

        // 1. Refused: there is no baseline yet.
        assert!(!store.summaries(&roots, &environment(), 10)[0].available);

        // 2. The repository becomes readable; the baseline is established by
        //    this observation, so nothing is counted against a state that was
        //    never read.
        git(temp.path(), &["config", "--unset", "filter.probe.clean"]);
        let recovered = store.summaries(&roots, &environment(), 20).remove(0);
        assert!(recovered.available, "{recovered:?}");
        assert!(recovered.changed_paths.is_empty(), "{recovered:?}");
        assert!(
            recovered.baseline_head.is_some(),
            "a readable repository must be read with its HEAD: {recovered:?}"
        );

        // 3. A change committed between two observations is still reported: it
        //    is invisible in `git status`, so the baseline HEAD has to be real.
        std::fs::write(temp.path().join("tracked.txt"), "committed change\n").unwrap();
        git(temp.path(), &["add", "tracked.txt"]);
        git(temp.path(), &["commit", "-qm", "agent change"]);
        let later = store.summaries(&roots, &environment(), 30).remove(0);
        assert!(later.available, "{later:?}");
        assert_eq!(later.changed_paths, vec!["tracked.txt"]);
    }

    #[test]
    fn a_failed_status_is_reported_instead_of_an_unchanged_tree() {
        let temp = tempfile::tempdir().unwrap();
        committed_repository(temp.path());
        // A damaged index fails `git status` while `HEAD` still answers, which
        // is the state a swallowed error used to present as a clean tree.
        std::fs::write(temp.path().join(".git/index"), b"not an index").unwrap();
        let roots = roots(temp.path());
        let mut store = GitBaselineStore::default();

        let summary = store.summaries(&roots, &environment(), 10).remove(0);
        assert!(
            !summary.available,
            "a failed invocation must not be reported as an unchanged tree: {summary:?}"
        );
        assert!(
            summary.status_message.contains("could not be read"),
            "the reason must say the state could not be read: {}",
            summary.status_message
        );
    }

    #[test]
    fn a_clean_checkout_under_line_ending_configuration_reports_no_changes() {
        for (label, attributes, autocrlf) in [
            ("core.autocrlf=true", None, Some("true")),
            (
                "text eol=crlf attribute",
                Some("*.txt text eol=crlf\n"),
                None,
            ),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            git(root, &["init", "-q", "-b", "main"]);
            git(root, &["config", "user.email", "fixture@example.invalid"]);
            git(root, &["config", "user.name", "Fixture"]);
            if let Some(autocrlf) = autocrlf {
                git(root, &["config", "core.autocrlf", autocrlf]);
            }
            if let Some(attributes) = attributes {
                std::fs::write(root.join(".gitattributes"), attributes).unwrap();
            }
            std::fs::write(root.join("a.txt"), "line1\nline2\n").unwrap();
            git(root, &["add", "-A"]);
            git(root, &["commit", "-qm", "initial"]);
            // Let Git write the working tree the way this configuration calls
            // for, then touch the file so Git compares content again.
            std::fs::remove_file(root.join("a.txt")).unwrap();
            git(root, &["checkout", "-f", "--", "a.txt"]);
            let checked_out = std::fs::read(root.join("a.txt")).unwrap();
            std::fs::write(root.join("a.txt"), &checked_out).unwrap();
            if autocrlf.is_some() {
                assert!(
                    checked_out.windows(2).any(|pair| pair == b"\r\n"),
                    "the fixture must check out CRLF under {label}"
                );
            }

            let roots = roots(root);
            let mut store = GitBaselineStore::default();
            assert!(store.summaries(&roots, &environment(), 10)[0]
                .changed_paths
                .is_empty());
            // A rewrite that keeps the bytes identical is what an editor save
            // does; the checkout is still clean, for the user and for Zenith.
            let user = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["status", "--porcelain=v1", "-uno"])
                .output()
                .unwrap();
            assert!(
                user.stdout.is_empty(),
                "the fixture must be clean for the user's own git ({label})"
            );
            let summary = store.summaries(&roots, &environment(), 20).remove(0);
            assert!(
                summary.changed_paths.is_empty(),
                "a clean {label} checkout must report no changes: {summary:?}"
            );
        }
    }
}
