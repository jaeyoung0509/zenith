//! Backend-owned leases for memory process-group termination.
//!
//! The frontend never supplies display names or PIDs for termination. It only
//! presents the opaque lease issued alongside `get_memory_metrics` and asks
//! the backend to consume that lease with an explicit graceful/force mode.

use crate::process_owner::ProcessOwner;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const LEASE_TTL: Duration = Duration::from_secs(30);
pub const STORE_CAPACITY: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryLeaseMember {
    pub pid: u32,
    pub owner: ProcessOwner,
    pub start_time: u64,
    pub exe: Option<PathBuf>,
    pub group: String,
}

#[derive(Debug, Clone)]
pub struct MemoryTerminationLease {
    pub id: String,
    pub group: String,
    pub members: Vec<MemoryLeaseMember>,
    pub created_at: Instant,
    pub can_terminate: bool,
    pub force_authorized: bool,
}

impl MemoryTerminationLease {
    pub fn is_expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.created_at) >= LEASE_TTL
    }
}

pub struct MemoryTerminationStore {
    capacity: usize,
    leases: HashMap<String, MemoryTerminationLease>,
    order: VecDeque<String>,
}

impl Default for MemoryTerminationStore {
    fn default() -> Self {
        Self::new(STORE_CAPACITY)
    }
}

pub struct CreateMemoryLeaseParams {
    pub group: String,
    pub members: Vec<MemoryLeaseMember>,
    pub can_terminate: bool,
    pub force_authorized: bool,
    pub now: Instant,
}

impl MemoryTerminationStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            leases: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub fn create_lease(&mut self, params: CreateMemoryLeaseParams) -> String {
        self.prune_stale(params.now);
        let id = uuid::Uuid::new_v4().to_string();
        let lease = MemoryTerminationLease {
            id: id.clone(),
            group: params.group,
            members: params.members,
            created_at: params.now,
            can_terminate: params.can_terminate,
            force_authorized: params.force_authorized,
        };
        while self.leases.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.leases.remove(&oldest);
            } else {
                break;
            }
        }
        self.leases.insert(id.clone(), lease);
        self.order.push_back(id.clone());
        id
    }

    /// One-shot consumption. Expired or missing leases return `None` and are
    /// removed so storage never grows without bound.
    pub fn take_lease(&mut self, id: &str, now: Instant) -> Option<MemoryTerminationLease> {
        self.prune_stale(now);
        let lease = self.leases.remove(id)?;
        self.order.retain(|item| item != id);
        if lease.is_expired(now) {
            return None;
        }
        Some(lease)
    }

    pub fn peek_lease(&self, id: &str, now: Instant) -> Option<&MemoryTerminationLease> {
        let lease = self.leases.get(id)?;
        if lease.is_expired(now) {
            return None;
        }
        Some(lease)
    }

    pub fn prune_stale(&mut self, now: Instant) {
        let expired: Vec<String> = self
            .leases
            .iter()
            .filter_map(|(id, lease)| {
                if lease.is_expired(now) {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();
        for id in expired {
            self.leases.remove(&id);
            self.order.retain(|item| item != &id);
        }
    }

    pub fn len(&self) -> usize {
        self.leases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.leases.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(pid: u32) -> MemoryLeaseMember {
        MemoryLeaseMember {
            pid,
            owner: ProcessOwner::Unix(501),
            start_time: 1000,
            exe: Some(PathBuf::from("/Applications/Cursor.app/Contents/MacOS/Cursor")),
            group: "Cursor".to_string(),
        }
    }

    #[test]
    fn lease_expires_at_ttl_boundary() {
        let mut store = MemoryTerminationStore::new(8);
        let t0 = Instant::now();
        let id = store.create_lease(CreateMemoryLeaseParams {
            group: "Cursor".to_string(),
            members: vec![member(100)],
            can_terminate: true,
            force_authorized: false,
            now: t0,
        });
        assert!(store
            .peek_lease(&id, t0 + Duration::from_millis(29_999))
            .is_some());
        assert!(store
            .peek_lease(&id, t0 + Duration::from_secs(30))
            .is_none());
        assert!(store.take_lease(&id, t0 + Duration::from_secs(31)).is_none());
    }

    #[test]
    fn lease_consumption_is_one_shot() {
        let mut store = MemoryTerminationStore::new(8);
        let now = Instant::now();
        let id = store.create_lease(CreateMemoryLeaseParams {
            group: "Cursor".to_string(),
            members: vec![member(100)],
            can_terminate: true,
            force_authorized: false,
            now,
        });
        assert!(store.take_lease(&id, now).is_some());
        assert!(store.take_lease(&id, now).is_none());
        assert!(store.peek_lease(&id, now).is_none());
    }

    #[test]
    fn store_enforces_hard_entry_cap() {
        let mut store = MemoryTerminationStore::new(2);
        let now = Instant::now();
        let first = store.create_lease(CreateMemoryLeaseParams {
            group: "A".to_string(),
            members: vec![member(1)],
            can_terminate: true,
            force_authorized: false,
            now,
        });
        let _second = store.create_lease(CreateMemoryLeaseParams {
            group: "B".to_string(),
            members: vec![member(2)],
            can_terminate: true,
            force_authorized: false,
            now,
        });
        let _third = store.create_lease(CreateMemoryLeaseParams {
            group: "C".to_string(),
            members: vec![member(3)],
            can_terminate: true,
            force_authorized: false,
            now,
        });
        assert_eq!(store.len(), 2);
        assert!(store.peek_lease(&first, now).is_none());
    }
}
