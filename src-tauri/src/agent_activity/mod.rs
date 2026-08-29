pub mod adapters;
pub mod correlation;
pub mod events;
pub mod hooks;
pub mod notifications;
pub mod projects;
pub mod store;
pub mod termination;

use crate::models::{
    AgentActivitySnapshot, AgentActivityStatus, AgentEvidence, AgentSession, SnapshotQuality,
};
use adapters::{adapter_for_executable, health_with_integrations};
use projects::{opaque_id, resolve_project};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

pub const SNAPSHOT_TTL_SECONDS: u64 = 10;

#[derive(Debug, Clone)]
pub struct AgentActivityRegistry {
    pub snapshot: AgentActivitySnapshot,
    pub project_roots: HashMap<String, PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ProcessRecord {
    pub pid: u32,
    pub uid: Option<u32>,
    pub started_at: u64,
    pub executable: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}

static GLOBAL_STORE: OnceLock<Arc<Mutex<store::AgentActivityStore>>> = OnceLock::new();

pub fn global_store() -> Arc<Mutex<store::AgentActivityStore>> {
    GLOBAL_STORE
        .get_or_init(|| Arc::new(Mutex::new(store::AgentActivityStore::new())))
        .clone()
}

pub fn collect() -> AgentActivitySnapshot {
    collect_registry().snapshot
}

pub fn has_active_verified_session() -> bool {
    let snapshot = collect();
    snapshot
        .projects
        .iter()
        .flat_map(|p| &p.sessions)
        .chain(&snapshot.unassigned_sessions)
        .any(|s| {
            matches!(
                s.status,
                AgentActivityStatus::Active
                    | AgentActivityStatus::Working
                    | AgentActivityStatus::Starting
                    | AgentActivityStatus::WaitingForUser
                    | AgentActivityStatus::Idle
            )
        })
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

    let store = global_store();
    let mut store_guard = store.lock().unwrap();
    registry_from_records(records, current_uid, observed_at, &mut store_guard)
}

pub fn registry_from_records(
    records: Vec<ProcessRecord>,
    current_uid: u32,
    observed_at: u64,
    store: &mut store::AgentActivityStore,
) -> AgentActivityRegistry {
    let mut discovered_projects = HashMap::new();
    let mut project_roots = HashMap::new();
    let mut observed_ids = HashSet::new();
    let mut partial_errors = Vec::new();
    let mut session_inputs = Vec::new();
    let mut current_observed_session_ids = HashSet::new();

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

        let session_identity = PathBuf::from(format!(
            "{}:{}:{}",
            adapter.id, record.pid, record.started_at
        ));
        let session_id = opaque_id("session", &session_identity);
        current_observed_session_ids.insert(session_id.clone());

        // Create stop lease
        let lease_id = store.stop_leases.create_lease(
            &session_id,
            record.pid,
            record.started_at,
            executable.to_path_buf(),
            record.cwd.clone(),
            current_uid,
            observed_at,
        );

        let elapsed = observed_at.saturating_sub(record.started_at);

        // Check if there is an active vendor event matching this tool
        let matching_event = store
            .active_events
            .values()
            .find(|e| e.tool_id == adapter.id);

        let (status, evidence, attention_reason, detail) = if let Some(event) = matching_event {
            let status = match event.lifecycle {
                crate::models::AgentLifecycleEvent::SessionStart => AgentActivityStatus::Starting,
                crate::models::AgentLifecycleEvent::Working => AgentActivityStatus::Working,
                crate::models::AgentLifecycleEvent::WaitingForUser => {
                    AgentActivityStatus::WaitingForUser
                }
                crate::models::AgentLifecycleEvent::Idle => AgentActivityStatus::Idle,
                crate::models::AgentLifecycleEvent::TurnComplete => {
                    AgentActivityStatus::WaitingForUser
                }
                crate::models::AgentLifecycleEvent::SessionEnd => AgentActivityStatus::Exited,
            };
            (
                status,
                AgentEvidence::VendorEvent,
                event.attention_reason,
                "Vendor event confirmed".to_string(),
            )
        } else {
            // Heuristic inactivity check: if no activity observed for > 15 minutes (or high threshold)
            let status = if elapsed > 900 && record.cpu_percent < 0.1 {
                AgentActivityStatus::PossiblyInactive
            } else {
                AgentActivityStatus::Working
            };
            let detail = if status == AgentActivityStatus::PossiblyInactive {
                format!("No activity observed for {} minutes", elapsed / 60)
            } else {
                "Process observed · detailed status unavailable".to_string()
            };
            (status, AgentEvidence::ProcessObserved, None, detail)
        };

        // Resolve candidate project for this cwd
        let canonical_cwd = record.cwd.as_deref().and_then(resolve_project);
        if let Some((root, identity)) = canonical_cwd {
            project_roots.insert(identity.id.clone(), root.clone());
            discovered_projects.insert(identity.id.clone(), (root, identity));
        }

        let session = AgentSession {
            id: session_id,
            tool_id: adapter.id.to_string(),
            tool_name: adapter.display_name.to_string(),
            status,
            attention_reason,
            evidence,
            observed_at,
            started_at: record.started_at,
            elapsed_seconds: elapsed,
            cpu_percent: record.cpu_percent,
            memory_bytes: record.memory_bytes,
            project_id: None,
            worktree_id: None,
            detail,
            can_stop: true,
            stop_lease_id: Some(lease_id),
        };

        session_inputs.push((record.cwd, session));
    }

    // Correlate sessions, listeners, and artifact sizes
    let (mut projects, unassigned_sessions) = correlation::correlate(
        discovered_projects,
        session_inputs,
        &[],
        &HashMap::new(),
        observed_at,
    );

    // Retain exited sessions if previously observed sessions disappeared
    store.prune_expired(observed_at);
    let exited = store.get_exited_sessions(observed_at);
    for exited_session in exited {
        if !current_observed_session_ids.contains(&exited_session.id) {
            if let Some(pid) = &exited_session.project_id {
                if let Some(p) = projects.iter_mut().find(|proj| &proj.identity.id == pid) {
                    p.sessions.push(exited_session);
                }
            }
        }
    }

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

    let adapters = health_with_integrations(&observed_ids, &HashSet::new());

    let snapshot = AgentActivitySnapshot {
        observed_at,
        quality,
        projects,
        unassigned_sessions,
        adapters,
        partial_errors,
    };

    store.last_successful_snapshot = Some(snapshot.clone());

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
        let mut store = store::AgentActivityStore::new();
        let registry = registry_from_records(
            vec![
                record("/tmp/codex-helper", Some(501), 10, Some(temp.path().into())),
                record("/usr/bin/codex", Some(502), 10, Some(temp.path().into())),
                record("/usr/bin/claude", Some(501), 0, Some(temp.path().into())),
            ],
            501,
            100,
            &mut store,
        );
        assert!(registry.snapshot.projects.is_empty());
        assert!(registry.snapshot.unassigned_sessions.is_empty());
    }

    #[test]
    fn groups_exact_processes_by_canonical_project_without_exposing_pid() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("project");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

        let mut store = store::AgentActivityStore::new();
        let registry = registry_from_records(
            vec![
                record("/usr/bin/codex", Some(501), 10, Some(repo.join("src"))),
                record("/usr/bin/claude", Some(501), 11, Some(repo.clone())),
            ],
            501,
            100,
            &mut store,
        );
        assert_eq!(registry.snapshot.projects.len(), 1);
        assert_eq!(registry.snapshot.projects[0].sessions.len(), 2);
        assert!(!registry.snapshot.projects[0].sessions[0]
            .id
            .contains("4242"));
        assert_eq!(
            registry.snapshot.projects[0].sessions[0].evidence,
            AgentEvidence::ProcessObserved
        );
        assert!(matches!(
            registry.snapshot.projects[0].sessions[0].status,
            AgentActivityStatus::Working | AgentActivityStatus::Active
        ));
        assert!(registry.snapshot.projects[0].sessions[0].can_stop);
        assert!(registry.snapshot.projects[0].sessions[0]
            .stop_lease_id
            .is_some());
    }
}
