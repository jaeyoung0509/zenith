use crate::models::ListenerProtocol;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(30);
pub const DEFAULT_STORE_CAPACITY: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenerLease {
    pub id: String,
    pub pid: u32,
    pub port: u16,
    pub protocol: ListenerProtocol,
    pub bind_address: String,
    pub uid: u32,
    pub started_at: Option<u64>,
    pub exe_path: Option<PathBuf>,
    pub server_name: String,
    pub created_at: Instant,
    pub can_release: bool,
}

impl ListenerLease {
    pub fn is_expired(&self, now: Instant, ttl: Duration) -> bool {
        now.saturating_duration_since(self.created_at) >= ttl
    }
}

pub struct DevelopmentPortStore {
    capacity: usize,
    ttl: Duration,
    leases: HashMap<String, ListenerLease>,
    order: VecDeque<String>,
}

impl Default for DevelopmentPortStore {
    fn default() -> Self {
        Self::new(DEFAULT_STORE_CAPACITY, DEFAULT_LEASE_TTL)
    }
}

#[derive(Debug, Clone)]
pub struct CreateLeaseParams {
    pub pid: u32,
    pub port: u16,
    pub protocol: ListenerProtocol,
    pub bind_address: String,
    pub uid: u32,
    pub started_at: Option<u64>,
    pub exe_path: Option<PathBuf>,
    pub server_name: String,
    pub can_release: bool,
    pub now: Instant,
}

impl DevelopmentPortStore {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            capacity: capacity.max(1),
            ttl,
            leases: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Generates a new random lease ID and inserts a lease into the store.
    pub fn create_lease(&mut self, params: CreateLeaseParams) -> String {
        self.prune_stale(params.now);

        let id = Uuid::new_v4().to_string();
        let lease = ListenerLease {
            id: id.clone(),
            pid: params.pid,
            port: params.port,
            protocol: params.protocol,
            bind_address: params.bind_address,
            uid: params.uid,
            started_at: params.started_at,
            exe_path: params.exe_path,
            server_name: params.server_name,
            created_at: params.now,
            can_release: params.can_release,
        };

        // Evict oldest if capacity exceeded
        while self.leases.len() >= self.capacity {
            if let Some(oldest_id) = self.order.pop_front() {
                self.leases.remove(&oldest_id);
            } else {
                break;
            }
        }

        self.leases.insert(id.clone(), lease);
        self.order.push_back(id.clone());
        id
    }

    /// One-shot consumption: takes and removes the lease if present and not expired.
    pub fn take_lease(&mut self, id: &str, now: Instant) -> Option<ListenerLease> {
        self.prune_stale(now);
        let lease = self.leases.remove(id)?;
        self.order.retain(|item_id| item_id != id);

        if lease.is_expired(now, self.ttl) {
            return None;
        }

        Some(lease)
    }

    /// Checks if a lease exists and is valid without consuming it (read-only peek).
    pub fn peek_lease(&self, id: &str, now: Instant) -> Option<&ListenerLease> {
        let lease = self.leases.get(id)?;
        if lease.is_expired(now, self.ttl) {
            return None;
        }
        Some(lease)
    }

    /// Removes expired leases from the store.
    pub fn prune_stale(&mut self, now: Instant) {
        let ttl = self.ttl;
        let mut expired_ids = Vec::new();
        for (id, lease) in &self.leases {
            if lease.is_expired(now, ttl) {
                expired_ids.push(id.clone());
            }
        }
        for id in expired_ids {
            self.leases.remove(&id);
            self.order.retain(|item_id| item_id != &id);
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

    #[test]
    fn ttl_valid_before_expiry_invalid_at_and_after_expiry() {
        let mut store = DevelopmentPortStore::new(10, Duration::from_secs(30));
        let t0 = Instant::now();

        let id = store.create_lease(CreateLeaseParams {
            pid: 1234,
            port: 5173,
            protocol: ListenerProtocol::Tcp,
            bind_address: "127.0.0.1".to_string(),
            uid: 501,
            started_at: Some(1000),
            exe_path: None,
            server_name: "Vite".to_string(),
            can_release: true,
            now: t0,
        });

        // Valid just before expiry (29.999s)
        let t_before = t0 + Duration::from_millis(29_999);
        assert!(store.peek_lease(&id, t_before).is_some());

        // Invalid exactly at expiry (30.000s)
        let t_exact = t0 + Duration::from_secs(30);
        assert!(store.peek_lease(&id, t_exact).is_none());

        // Invalid after expiry (31s)
        let t_after = t0 + Duration::from_secs(31);
        assert!(store.take_lease(&id, t_after).is_none());
    }

    #[test]
    fn store_cap_and_oldest_entry_eviction() {
        let mut store = DevelopmentPortStore::new(3, Duration::from_secs(60));
        let now = Instant::now();

        let id1 = store.create_lease(CreateLeaseParams {
            pid: 1,
            port: 5001,
            protocol: ListenerProtocol::Tcp,
            bind_address: "127.0.0.1".to_string(),
            uid: 501,
            started_at: None,
            exe_path: None,
            server_name: "Server1".to_string(),
            can_release: true,
            now,
        });
        let id2 = store.create_lease(CreateLeaseParams {
            pid: 2,
            port: 5002,
            protocol: ListenerProtocol::Tcp,
            bind_address: "127.0.0.1".to_string(),
            uid: 501,
            started_at: None,
            exe_path: None,
            server_name: "Server2".to_string(),
            can_release: true,
            now,
        });
        let id3 = store.create_lease(CreateLeaseParams {
            pid: 3,
            port: 5003,
            protocol: ListenerProtocol::Tcp,
            bind_address: "127.0.0.1".to_string(),
            uid: 501,
            started_at: None,
            exe_path: None,
            server_name: "Server3".to_string(),
            can_release: true,
            now,
        });

        assert_eq!(store.len(), 3);
        assert!(store.peek_lease(&id1, now).is_some());

        // Inserting 4th item should evict id1
        let id4 = store.create_lease(CreateLeaseParams {
            pid: 4,
            port: 5004,
            protocol: ListenerProtocol::Tcp,
            bind_address: "127.0.0.1".to_string(),
            uid: 501,
            started_at: None,
            exe_path: None,
            server_name: "Server4".to_string(),
            can_release: true,
            now,
        });

        assert_eq!(store.len(), 3);
        assert!(store.peek_lease(&id1, now).is_none());
        assert!(store.peek_lease(&id2, now).is_some());
        assert!(store.peek_lease(&id3, now).is_some());
        assert!(store.peek_lease(&id4, now).is_some());
    }

    #[test]
    fn one_shot_lease_consumption() {
        let mut store = DevelopmentPortStore::new(10, Duration::from_secs(60));
        let now = Instant::now();

        let id = store.create_lease(CreateLeaseParams {
            pid: 1234,
            port: 5173,
            protocol: ListenerProtocol::Tcp,
            bind_address: "127.0.0.1".to_string(),
            uid: 501,
            started_at: None,
            exe_path: None,
            server_name: "Vite".to_string(),
            can_release: true,
            now,
        });

        // First take succeeds
        let lease = store.take_lease(&id, now);
        assert!(lease.is_some());
        assert_eq!(lease.unwrap().port, 5173);

        // Second take fails (one-shot consumption)
        assert!(store.take_lease(&id, now).is_none());
        assert!(store.peek_lease(&id, now).is_none());
    }
}
