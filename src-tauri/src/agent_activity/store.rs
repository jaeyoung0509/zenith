use super::notifications::NotificationFilter;
use super::termination::StopLeaseStore;
use crate::models::{
    AgentActivitySnapshot, AgentActivityStatus, AgentLifecycleEvent, AgentSession,
    IngestedAgentEvent,
};
use std::collections::HashMap;

pub const EXITED_SESSION_RETENTION_SECS: u64 = 60;

#[derive(Debug, Default)]
pub struct AgentActivityStore {
    pub active_events: HashMap<(String, String), IngestedAgentEvent>, // (tool_id, vendor_session_id)
    pub exited_sessions: HashMap<String, (AgentSession, u64)>, // session_id -> (session, exited_at)
    pub stop_leases: StopLeaseStore,
    pub notification_filter: NotificationFilter,
    pub last_successful_snapshot: Option<AgentActivitySnapshot>,
}

impl AgentActivityStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_event(&mut self, event: IngestedAgentEvent) {
        let key = (event.tool_id.clone(), event.vendor_session_id.clone());
        if event.lifecycle == AgentLifecycleEvent::SessionEnd {
            self.active_events.remove(&key);
        } else {
            self.active_events.insert(key, event);
        }
    }

    pub fn record_exited_session(&mut self, mut session: AgentSession, now: u64) {
        session.status = AgentActivityStatus::Exited;
        session.detail = "Session exited.".to_string();
        session.can_stop = false;
        session.stop_lease_id = None;
        self.exited_sessions
            .insert(session.id.clone(), (session, now));
    }

    pub fn prune_expired(&mut self, now: u64) {
        self.exited_sessions.retain(|_, (_, exited_at)| {
            now.saturating_sub(*exited_at) < EXITED_SESSION_RETENTION_SECS
        });
    }

    pub fn get_exited_sessions(&self, now: u64) -> Vec<AgentSession> {
        self.exited_sessions
            .iter()
            .filter(|(_, (_, exited_at))| {
                now.saturating_sub(*exited_at) < EXITED_SESSION_RETENTION_SECS
            })
            .map(|(_, (session, _))| session.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AgentEvidence;

    #[test]
    fn retains_exited_sessions_until_ttl_expires() {
        let mut store = AgentActivityStore::new();
        let session = AgentSession {
            id: "s1".into(),
            tool_id: "antigravity".into(),
            tool_name: "Antigravity".into(),
            status: AgentActivityStatus::Working,
            attention_reason: None,
            evidence: AgentEvidence::ProcessObserved,
            observed_at: 100,
            started_at: 10,
            elapsed_seconds: 90,
            cpu_percent: 1.0,
            memory_bytes: 1024,
            project_id: Some("p1".into()),
            worktree_id: None,
            detail: "Working".into(),
            can_stop: true,
            stop_lease_id: None,
        };

        store.record_exited_session(session, 100);
        assert_eq!(store.get_exited_sessions(120).len(), 1);
        assert_eq!(
            store.get_exited_sessions(120)[0].status,
            AgentActivityStatus::Exited
        );

        // After TTL expires
        store.prune_expired(100 + EXITED_SESSION_RETENTION_SECS + 5);
        assert!(store
            .get_exited_sessions(100 + EXITED_SESSION_RETENTION_SECS + 5)
            .is_empty());
    }
}
