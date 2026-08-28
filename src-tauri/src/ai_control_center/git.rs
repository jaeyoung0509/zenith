use crate::models::GitChangeSummary;
use crate::tooling;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct GitBaselineStore {
    baselines: HashMap<String, GitBaseline>,
}

#[derive(Debug, Clone)]
struct GitBaseline {
    head: Option<String>,
    statuses: HashMap<String, String>,
    fingerprints: HashMap<String, String>,
    captured_at: u64,
}

impl GitBaselineStore {
    pub fn summaries(
        &mut self,
        roots: &HashMap<String, PathBuf>,
        now: u64,
    ) -> Vec<GitChangeSummary> {
        self.baselines.retain(|id, _| roots.contains_key(id));
        let mut summaries = Vec::new();
        for (project_id, root) in roots {
            let current = capture(root, now);
            let baseline = self
                .baselines
                .entry(project_id.clone())
                .or_insert_with(|| current.clone());
            summaries.push(compare(project_id, baseline, &current));
        }
        summaries.sort_by(|a, b| a.project_id.cmp(&b.project_id));
        summaries
    }

    pub fn paths_for_diff(
        &self,
        project_id: &str,
        root: &Path,
        now: u64,
    ) -> Result<Vec<String>, String> {
        let baseline = self
            .baselines
            .get(project_id)
            .ok_or_else(|| "Git baseline is stale or unavailable".to_string())?;
        Ok(changed_entries(baseline, &capture(root, now))
            .into_iter()
            .map(|(path, _)| path)
            .collect())
    }
}

fn capture(root: &Path, now: u64) -> GitBaseline {
    let head = run_git(root, &["rev-parse", "--verify", "HEAD"])
        .ok()
        .and_then(|value| value.lines().next().map(str::to_string));
    let statuses = run_git(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )
    .map(|value| parse_status(value.as_bytes()))
    .unwrap_or_default();
    let fingerprints = statuses
        .keys()
        .map(|path| (path.clone(), worktree_fingerprint(root, path)))
        .collect();
    GitBaseline {
        head,
        statuses,
        fingerprints,
        captured_at: now,
    }
}

fn compare(project_id: &str, baseline: &GitBaseline, current: &GitBaseline) -> GitChangeSummary {
    if baseline.head.is_none() && current.head.is_none() && current.statuses.is_empty() {
        return GitChangeSummary {
            project_id: project_id.into(),
            baseline_head: None,
            current_head: None,
            baseline_at: baseline.captured_at,
            added: 0,
            modified: 0,
            deleted: 0,
            renamed: 0,
            untracked: 0,
            changed_paths: vec![],
            available: false,
            status_message: "Git unavailable, not a repository, or HEAD has not been created."
                .into(),
        };
    }
    let changed = changed_entries(baseline, current);
    let count = |needle: char| {
        changed
            .iter()
            .filter(|(_, status)| status.contains(needle))
            .count() as u32
    };
    GitChangeSummary {
        project_id: project_id.into(),
        baseline_head: baseline.head.clone(),
        current_head: current.head.clone(),
        baseline_at: baseline.captured_at,
        added: count('A'),
        modified: count('M'),
        deleted: count('D'),
        renamed: count('R'),
        untracked: changed.iter().filter(|(_, status)| status == "??").count() as u32,
        changed_paths: changed.into_iter().map(|(path, _)| path).collect(),
        available: true,
        status_message:
            "Files changed since baseline; diff shows current Git working-tree changes.".into(),
    }
}

fn changed_entries(baseline: &GitBaseline, current: &GitBaseline) -> Vec<(String, String)> {
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
            return format!("{:x}", digest.finalize())[..16].to_string();
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

fn run_git(root: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = tooling::command("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C");
    let output = tooling::run_with_timeout(command, Duration::from_secs(3))
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("Git command unavailable".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end_matches('\0')
        .trim()
        .to_string())
}

fn run_diff_command(root: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = tooling::command("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C");
    let output = tooling::run_with_timeout(command, Duration::from_secs(3))
        .map_err(|error| error.to_string())?;
    if output.status.success() || output.status.code() == Some(1) {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err("Git diff unavailable".into())
    }
}

pub fn explicit_diff(root: &Path, paths: &[String]) -> Result<String, String> {
    if paths.is_empty() {
        return Ok(String::new());
    }
    const MAX: usize = 262_144;
    let mut combined_diff = String::new();

    // 1. Try git diff HEAD for tracked modifications
    let mut head_args = vec!["diff", "HEAD", "--no-ext-diff", "--no-color", "--"];
    for p in paths {
        head_args.push(p);
    }
    if let Ok(tracked_diff) = run_diff_command(root, &head_args) {
        combined_diff.push_str(&tracked_diff);
    } else {
        // Fallback for fresh repos before initial commit
        let mut empty_args = vec!["diff", "--no-ext-diff", "--no-color", "--"];
        for p in paths {
            empty_args.push(p);
        }
        if let Ok(staged_diff) = run_diff_command(root, &empty_args) {
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
                        root,
                        &["diff", "--no-index", "--no-color", "--", "/dev/null", p],
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
    format!("{:x}", digest.finalize())[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn git(root: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
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
        let roots = HashMap::from([("p".into(), temp.path().into())]);
        let mut store = GitBaselineStore::default();
        assert!(store.summaries(&roots, 10)[0].changed_paths.is_empty());
        std::fs::write(temp.path().join("after.txt"), "after").unwrap();
        let summary = store.summaries(&roots, 20).remove(0);
        assert_eq!(summary.changed_paths, vec!["after.txt"]);
        assert_eq!(summary.untracked, 1);
    }

    #[test]
    fn baseline_detects_new_content_when_porcelain_status_is_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init", "-q"]);
        std::fs::write(temp.path().join("same-status.txt"), "before").unwrap();
        let roots = HashMap::from([("p".into(), temp.path().into())]);
        let mut store = GitBaselineStore::default();
        assert!(store.summaries(&roots, 10)[0].changed_paths.is_empty());
        std::fs::write(temp.path().join("same-status.txt"), "after").unwrap();
        assert_eq!(
            store.summaries(&roots, 20)[0].changed_paths,
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
    fn explicit_diff_includes_untracked_files() {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init", "-q"]);
        git(
            temp.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        git(temp.path(), &["config", "user.name", "Test"]);
        std::fs::write(temp.path().join("tracked.txt"), "hello\n").unwrap();
        git(temp.path(), &["add", "tracked.txt"]);
        git(temp.path(), &["commit", "-qm", "initial"]);

        // Create an untracked file and modify tracked file
        std::fs::write(temp.path().join("untracked.rs"), "fn main() {}\n").unwrap();
        std::fs::write(temp.path().join("tracked.txt"), "hello world\n").unwrap();

        let diff = explicit_diff(temp.path(), &["tracked.txt".into(), "untracked.rs".into()])
            .expect("diff succeeds");

        assert!(diff.contains("tracked.txt"));
        assert!(diff.contains("untracked.rs"));
        assert!(diff.contains("fn main()"));
    }
}
