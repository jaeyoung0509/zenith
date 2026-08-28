pub mod adapters;
pub mod projects;

use crate::models::{
    AgentActivitySnapshot, AgentActivityStatus, AgentEvidence, AgentSession, ProjectContext,
    SnapshotQuality,
};
use adapters::{adapter_for_executable, health};
use projects::{opaque_id, resolve_project};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

pub const SNAPSHOT_TTL_SECONDS: u64 = 10;

#[derive(Debug, Clone)]
pub struct AgentActivityRegistry {
    pub snapshot: AgentActivitySnapshot,
    pub project_roots: HashMap<String, PathBuf>,
}

#[derive(Debug, Clone)]
struct ProcessRecord {
    pid: u32,
    uid: Option<u32>,
    started_at: u64,
    executable: Option<PathBuf>,
    cwd: Option<PathBuf>,
    cpu_percent: f32,
    memory_bytes: u64,
}

pub fn collect() -> AgentActivitySnapshot {
    collect_registry().snapshot
}

pub fn collect_registry() -> AgentActivityRegistry {
    let observed_at = now();
    let current_uid = current_user_uid();
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::everything(),
    );
    let records = system
        .processes()
        .iter()
        .map(|(pid, process)| ProcessRecord {
            pid: pid.as_u32(),
            uid: process
                .effective_user_id()
                .or_else(|| process.user_id())
                .and_then(|uid| uid.to_string().parse().ok()),
            started_at: process.start_time(),
            executable: process.exe().map(PathBuf::from),
            cwd: process.cwd().map(PathBuf::from),
            cpu_percent: process.cpu_usage(),
            memory_bytes: process.memory(),
        })
        .collect::<Vec<_>>();
    registry_from_records(records, current_uid, observed_at)
}

#[cfg(test)]
fn snapshot_from_records(
    records: Vec<ProcessRecord>,
    current_uid: u32,
    observed_at: u64,
) -> AgentActivitySnapshot {
    registry_from_records(records, current_uid, observed_at).snapshot
}

fn registry_from_records(
    records: Vec<ProcessRecord>,
    current_uid: u32,
    observed_at: u64,
) -> AgentActivityRegistry {
    let mut projects: HashMap<String, ProjectContext> = HashMap::new();
    let mut unassigned_sessions = Vec::new();
    let mut observed_ids = HashSet::new();
    let mut partial_errors = Vec::new();
    let mut project_roots = HashMap::new();

    for record in records {
        let Some(executable) = record.executable.as_deref() else {
            continue;
        };
        let Some(adapter) = adapter_for_executable(executable) else {
            continue;
        };
        if record.uid != Some(current_uid)
            || record.started_at == 0
            || record.pid <= 1
            || record.pid == std::process::id()
        {
            continue;
        }
        observed_ids.insert(adapter.id);

        let project = record.cwd.as_deref().and_then(resolve_project);
        let project_id = project.as_ref().map(|(_, identity)| identity.id.clone());
        let session_identity = PathBuf::from(format!(
            "{}:{}:{}",
            adapter.id, record.pid, record.started_at
        ));
        let session = AgentSession {
            id: opaque_id("session", &session_identity),
            tool_id: adapter.id.to_string(),
            tool_name: adapter.display_name.to_string(),
            status: AgentActivityStatus::Active,
            evidence: AgentEvidence::ProcessObserved,
            observed_at,
            started_at: record.started_at,
            elapsed_seconds: observed_at.saturating_sub(record.started_at),
            cpu_percent: record.cpu_percent,
            memory_bytes: record.memory_bytes,
            project_id: project_id.clone(),
            detail: "Process observed · detailed status unavailable".to_string(),
        };

        if let Some((root, identity)) = project {
            project_roots.insert(identity.id.clone(), root);
            let entry = projects
                .entry(identity.id.clone())
                .or_insert(ProjectContext {
                    identity,
                    sessions: Vec::new(),
                    last_seen_at: observed_at,
                });
            entry.sessions.push(session);
        } else {
            unassigned_sessions.push(session);
        }
    }

    for context in projects.values_mut() {
        context
            .sessions
            .sort_by(|a, b| a.tool_name.cmp(&b.tool_name));
    }
    let mut projects = projects.into_values().collect::<Vec<_>>();
    projects.sort_by(|a, b| {
        a.identity
            .display_name
            .cmp(&b.identity.display_name)
            .then_with(|| a.identity.id.cmp(&b.identity.id))
    });
    unassigned_sessions.sort_by(|a, b| a.tool_name.cmp(&b.tool_name));

    if projects.is_empty() && !unassigned_sessions.is_empty() {
        partial_errors.push(
            "Some agent processes did not expose an accessible working directory.".to_string(),
        );
    }
    let quality = if partial_errors.is_empty() {
        SnapshotQuality::Fresh
    } else {
        SnapshotQuality::Partial
    };
    let snapshot = AgentActivitySnapshot {
        observed_at,
        quality,
        projects,
        unassigned_sessions,
        adapters: health(&observed_ids),
        partial_errors,
    };
    AgentActivityRegistry {
        snapshot,
        project_roots,
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn current_user_uid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::geteuid()
    }
    #[cfg(not(unix))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(
        executable: &str,
        uid: Option<u32>,
        started_at: u64,
        cwd: Option<PathBuf>,
    ) -> ProcessRecord {
        ProcessRecord {
            pid: 4242,
            uid,
            started_at,
            executable: Some(PathBuf::from(executable)),
            cwd,
            cpu_percent: 2.5,
            memory_bytes: 1024,
        }
    }

    #[test]
    fn rejects_lookalikes_other_users_and_missing_identity() {
        let temp = tempfile::tempdir().unwrap();
        let snapshot = snapshot_from_records(
            vec![
                record("/tmp/codex-helper", Some(501), 10, Some(temp.path().into())),
                record("/usr/bin/codex", Some(502), 10, Some(temp.path().into())),
                record("/usr/bin/claude", Some(501), 0, Some(temp.path().into())),
            ],
            501,
            100,
        );
        assert!(snapshot.projects.is_empty());
        assert!(snapshot.unassigned_sessions.is_empty());
        assert!(snapshot
            .adapters
            .iter()
            .all(|adapter| adapter.evidence.is_none()));
    }

    #[test]
    fn groups_exact_processes_by_canonical_project_without_exposing_pid() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("project");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let snapshot = snapshot_from_records(
            vec![
                record("/usr/bin/codex", Some(501), 10, Some(repo.join("src"))),
                record("/usr/bin/claude", Some(501), 11, Some(repo.clone())),
            ],
            501,
            100,
        );
        assert_eq!(snapshot.projects.len(), 1);
        assert_eq!(snapshot.projects[0].sessions.len(), 2);
        assert!(!snapshot.projects[0].sessions[0].id.contains("4242"));
        assert_eq!(
            snapshot.projects[0].sessions[0].evidence,
            AgentEvidence::ProcessObserved
        );
        assert_eq!(
            snapshot.projects[0].sessions[0].status,
            AgentActivityStatus::Active
        );
    }
}
