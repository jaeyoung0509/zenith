use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

use crate::models::DeletePlan;

/// Backend-owned bounded store for one-shot cleanup plans.
///
/// Encapsulates plan capacity, TTL expiration, oldest-first eviction,
/// stale-plan rejection, and one-shot consumption so IPC commands do not
/// coordinate plan lifecycle state directly.
pub struct PlanStore {
    plans: Mutex<HashMap<Uuid, DeletePlan>>,
    ttl_seconds: u64,
    capacity: usize,
}

impl PlanStore {
    pub const DEFAULT_TTL_SECONDS: u64 = 300;
    pub const DEFAULT_CAPACITY: usize = 64;

    pub fn new() -> Self {
        Self::with_ttl_and_capacity(Self::DEFAULT_TTL_SECONDS, Self::DEFAULT_CAPACITY)
    }

    pub fn with_ttl_and_capacity(ttl_seconds: u64, capacity: usize) -> Self {
        Self {
            plans: Mutex::new(HashMap::new()),
            ttl_seconds,
            capacity,
        }
    }

    pub fn ttl_seconds(&self) -> u64 {
        self.ttl_seconds
    }

    /// Inserts a newly generated plan, enforcing TTL purging and bounded capacity.
    pub fn insert(&self, plan: DeletePlan, now: u64) -> Result<(), String> {
        let mut plans = self
            .plans
            .lock()
            .map_err(|_| "Plan store lock poisoned".to_string())?;

        // Purge expired plans first
        plans.retain(|_, stored| now.saturating_sub(stored.created_at) < self.ttl_seconds);

        // If at capacity, evict the oldest plan by created_at
        if plans.len() >= self.capacity {
            if let Some(oldest_id) = plans
                .iter()
                .min_by_key(|(_, stored)| stored.created_at)
                .map(|(id, _)| *id)
            {
                plans.remove(&oldest_id);
            }
        }

        plans.insert(plan.id, plan);
        Ok(())
    }

    /// Takes a valid plan for one-shot execution.
    ///
    /// The plan is removed on retrieval so it cannot be replayed. Fails if the
    /// plan does not exist or has expired according to `ttl_seconds`.
    pub fn take_valid(&self, plan_id: Uuid, now: u64) -> Result<DeletePlan, String> {
        let mut plans = self
            .plans
            .lock()
            .map_err(|_| "Plan store lock poisoned".to_string())?;

        let plan = plans
            .remove(&plan_id)
            .ok_or_else(|| "Delete plan not found or already used".to_string())?;

        if now.saturating_sub(plan.created_at) >= self.ttl_seconds {
            return Err("Delete plan expired. Scan again before cleaning.".to_string());
        }

        Ok(plan)
    }

    /// Removes all expired plans.
    pub fn purge_expired(&self, now: u64) {
        if let Ok(mut plans) = self.plans.lock() {
            plans.retain(|_, stored| now.saturating_sub(stored.created_at) < self.ttl_seconds);
        }
    }

    /// Returns the number of currently retained plans (including any not-yet-purged expired ones).
    pub fn len(&self) -> usize {
        self.plans.lock().map(|p| p.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for PlanStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RiskSummary;

    fn make_test_plan(id: Uuid, created_at: u64) -> DeletePlan {
        DeletePlan {
            id,
            scan_id: "scan_1".to_string(),
            targets: vec![],
            expected_reclaim_bytes: 100,
            risk: RiskSummary::default(),
            created_at,
        }
    }

    #[test]
    fn insert_and_take_valid_removes_plan_once() {
        let store = PlanStore::new();
        let plan_id = Uuid::new_v4();
        let plan = make_test_plan(plan_id, 1000);

        assert!(store.insert(plan, 1000).is_ok());
        assert_eq!(store.len(), 1);

        // Valid take
        let taken = store.take_valid(plan_id, 1050).unwrap();
        assert_eq!(taken.id, plan_id);

        // Replay attempt fails
        assert!(store.take_valid(plan_id, 1050).is_err());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn expired_plan_cannot_be_taken() {
        let store = PlanStore::with_ttl_and_capacity(300, 64);
        let plan_id = Uuid::new_v4();
        let plan = make_test_plan(plan_id, 1000);

        store.insert(plan, 1000).unwrap();

        // Expired take (1000 + 300 = 1300)
        let err = store.take_valid(plan_id, 1301).unwrap_err();
        assert!(err.contains("expired"));
    }

    #[test]
    fn capacity_evicts_oldest_plan() {
        let store = PlanStore::with_ttl_and_capacity(300, 2);
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        store.insert(make_test_plan(id1, 100), 100).unwrap();
        store.insert(make_test_plan(id2, 200), 200).unwrap();
        assert_eq!(store.len(), 2);

        // Inserting 3rd evicts oldest (id1 with created_at 100)
        store.insert(make_test_plan(id3, 300), 300).unwrap();
        assert_eq!(store.len(), 2);

        assert!(store.take_valid(id1, 300).is_err());
        assert!(store.take_valid(id2, 300).is_ok());
        assert!(store.take_valid(id3, 300).is_ok());
    }
}
