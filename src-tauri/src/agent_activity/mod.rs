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
pub const DEFAULT_INACTIVITY_THRESHOLD_SECONDS: u64 = 15 * 60;

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
    collect_registry_with_inactivity_threshold(DEFAULT_INACTIVITY_THRESHOLD_SECONDS)
}

pub fn collect_registry_with_inactivity_threshold(
    inactivity_threshold_seconds: u64,
) -> AgentActivityRegistry {
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
    registry_from_records_with_inactivity_threshold(
        records,
        current_uid,
        observed_at,
        &mut store_guard,
        inactivity_threshold_seconds,
    )
}

pub fn registry_from_records(
    records: Vec<ProcessRecord>,
    current_uid: u32,
    observed_at: u64,
    store: &mut store::AgentActivityStore,
) -> AgentActivityRegistry {
    registry_from_records_with_inactivity_threshold(
        records,
        current_uid,
        observed_at,
        store,
        DEFAULT_INACTIVITY_THRESHOLD_SECONDS,
    )
}

fn registry_from_records_with_inactivity_threshold(
    records: Vec<ProcessRecord>,
    current_uid: u32,
    observed_at: u64,
    store: &mut store::AgentActivityStore,
    inactivity_threshold_seconds: u64,
) -> AgentActivityRegistry {
    store.prune_active_events(observed_at);
    let previous_snapshot = store.last_successful_snapshot.clone();
    let mut discovered_projects = HashMap::new();
    let mut project_roots = HashMap::new();
    let mut observed_ids = HashSet::new();
    let mut partial_errors = Vec::new();
    let mut session_inputs = Vec::new();
    let mut current_observed_session_ids = HashSet::new();

    let eligible_records = records
        .into_iter()
        .filter_map(|record| {
            let executable = record.executable.as_deref()?;
            let adapter = adapter_for_executable(executable)?;
            if record.uid != Some(current_uid)
                || record.started_at == 0
                || record.pid <= 1
                || record.pid == std::process::id()
            {
                return None;
            }
            Some((record, adapter))
        })
        .collect::<Vec<_>>();

    // A vendor event is trustworthy only when it identifies exactly one observed
    // process. Tool-only matching would incorrectly copy one session's state to
    // every concurrent process from the same CLI.
    let mut matched_events = HashMap::new();
    for event in store.active_events.values() {
        let candidates = eligible_records
            .iter()
            .filter(|(record, adapter)| {
                if adapter.id != event.tool_id {
                    return false;
                }
                match event.cwd.as_deref() {
                    Some(event_cwd) => record.cwd.as_deref().is_some_and(|cwd| {
                        same_project_or_directory(cwd, std::path::Path::new(event_cwd))
                    }),
                    None => true,
                }
            })
            .collect::<Vec<_>>();
        if candidates.len() == 1 {
            let (record, adapter) = candidates[0];
            let session_id = session_id(adapter.id, record.pid, record.started_at);
            let replace = matched_events.get(&session_id).is_none_or(
                |current: &crate::models::IngestedAgentEvent| event.timestamp > current.timestamp,
            );
            if replace {
                matched_events.insert(session_id, event.clone());
            }
        }
    }

    for (record, adapter) in eligible_records {
        let Some(executable) = record.executable.as_deref() else {
            continue;
        };
        observed_ids.insert(adapter.id);

        let session_id = session_id(adapter.id, record.pid, record.started_at);
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

        let matching_event = matched_events.get(&session_id);

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
            let last_activity_at =
                store.observe_process_activity(&session_id, record.cpu_percent, observed_at);
            let inactive_for = observed_at.saturating_sub(last_activity_at);
            let status = if inactive_for >= inactivity_threshold_seconds {
                AgentActivityStatus::PossiblyInactive
            } else {
                AgentActivityStatus::Working
            };
            let detail = if status == AgentActivityStatus::PossiblyInactive {
                format!("No activity observed for {} minutes", inactive_for / 60)
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
    let (mut projects, mut unassigned_sessions) = correlation::correlate(
        discovered_projects,
        session_inputs,
        &[],
        &HashMap::new(),
        observed_at,
    );

    // Detect normal process exits from the previous snapshot. Explicit stop is
    // not considered an exit until the process actually disappears.
    if let Some(previous) = &previous_snapshot {
        for previous_session in previous
            .projects
            .iter()
            .flat_map(|project| &project.sessions)
            .chain(&previous.unassigned_sessions)
        {
            if previous_session.status != AgentActivityStatus::Exited
                && !current_observed_session_ids.contains(&previous_session.id)
            {
                store.record_exited_session(previous_session.clone(), observed_at);
            }
        }
    }
    store.retain_observed_processes(&current_observed_session_ids);

    // Retain exited sessions for 60 seconds, including projects that no longer
    // have a running process and sessions that were previously unassigned.
    store.prune_expired(observed_at);
    let exited = store.get_exited_sessions(observed_at);
    for exited_session in exited {
        if !current_observed_session_ids.contains(&exited_session.id) {
            if let Some(project_id) = &exited_session.project_id {
                if let Some(project) = projects
                    .iter_mut()
                    .find(|project| &project.identity.id == project_id)
                {
                    project.sessions.push(exited_session);
                    continue;
                }
                if let Some(previous_project) = previous_snapshot.as_ref().and_then(|snapshot| {
                    snapshot
                        .projects
                        .iter()
                        .find(|project| &project.identity.id == project_id)
                }) {
                    let mut retained_project = previous_project.clone();
                    retained_project.sessions = vec![exited_session];
                    projects.push(retained_project);
                    continue;
                }
            }
            unassigned_sessions.push(exited_session);
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

fn session_id(tool_id: &str, pid: u32, started_at: u64) -> String {
    opaque_id(
        "session",
        &PathBuf::from(format!("{tool_id}:{pid}:{started_at}")),
    )
}

fn same_project_or_directory(left: &std::path::Path, right: &std::path::Path) -> bool {
    let left = left.canonicalize().ok();
    let right = right.canonicalize().ok();
    match (left, right) {
        (Some(left), Some(right)) if left == right => true,
        (Some(left), Some(right)) => {
            let left_project = resolve_project(&left).map(|(root, _)| root);
            let right_project = resolve_project(&right).map(|(root, _)| root);
            left_project.is_some() && left_project == right_project
        }
        _ => false,
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

    fn record_with_pid(
        pid: u32,
        executable: &str,
        started_at: u64,
        cwd: Option<PathBuf>,
        cpu_percent: f32,
    ) -> ProcessRecord {
        ProcessRecord {
            pid,
            uid: Some(501),
            started_at,
            executable: Some(PathBuf::from(executable)),
            cwd,
            cpu_percent,
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

    #[test]
    fn vendor_event_is_assigned_only_to_the_unique_matching_process() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        for repo in [&first, &second] {
            std::fs::create_dir_all(repo.join(".git")).unwrap();
            std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        }
        let mut store = store::AgentActivityStore::new();
        store.record_event(crate::models::IngestedAgentEvent {
            tool_id: "claude".into(),
            vendor_session_id: "vendor-1".into(),
            cwd: Some(first.display().to_string()),
            lifecycle: crate::models::AgentLifecycleEvent::WaitingForUser,
            timestamp: 100,
            turn_id: Some("turn-1".into()),
            attention_reason: Some(crate::models::AttentionReason::Input),
        });

        let registry = registry_from_records(
            vec![
                record_with_pid(41, "/usr/bin/claude", 10, Some(first), 1.0),
                record_with_pid(42, "/usr/bin/claude", 11, Some(second), 1.0),
            ],
            501,
            100,
            &mut store,
        );
        let sessions = registry
            .snapshot
            .projects
            .iter()
            .flat_map(|project| &project.sessions)
            .collect::<Vec<_>>();
        assert_eq!(
            sessions
                .iter()
                .filter(|session| session.evidence == AgentEvidence::VendorEvent)
                .count(),
            1
        );
        assert_eq!(
            sessions
                .iter()
                .filter(|session| session.status == AgentActivityStatus::WaitingForUser)
                .count(),
            1
        );
    }

    #[test]
    fn process_lifetime_is_not_mistaken_for_observed_inactivity() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = store::AgentActivityStore::new();
        let first = registry_from_records(
            vec![record_with_pid(
                42,
                "/usr/bin/codex",
                10,
                Some(temp.path().into()),
                0.0,
            )],
            501,
            10_000,
            &mut store,
        );
        assert_eq!(
            first.snapshot.projects[0].sessions[0].status,
            AgentActivityStatus::Working
        );

        let second = registry_from_records(
            vec![record_with_pid(
                42,
                "/usr/bin/codex",
                10,
                Some(temp.path().into()),
                0.0,
            )],
            501,
            10_000 + DEFAULT_INACTIVITY_THRESHOLD_SECONDS,
            &mut store,
        );
        assert_eq!(
            second.snapshot.projects[0].sessions[0].status,
            AgentActivityStatus::PossiblyInactive
        );
    }

    #[test]
    fn retains_a_naturally_exited_session_with_its_project_context() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("project");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let mut store = store::AgentActivityStore::new();

        registry_from_records(
            vec![record_with_pid(42, "/usr/bin/codex", 10, Some(repo), 1.0)],
            501,
            100,
            &mut store,
        );
        let exited = registry_from_records(vec![], 501, 110, &mut store);
        assert_eq!(exited.snapshot.projects.len(), 1);
        assert_eq!(
            exited.snapshot.projects[0].sessions[0].status,
            AgentActivityStatus::Exited
        );

        let expired = registry_from_records(
            vec![],
            501,
            110 + store::EXITED_SESSION_RETENTION_SECS,
            &mut store,
        );
        assert!(expired.snapshot.projects.is_empty());
    }
}
