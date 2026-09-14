use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

use crate::models::DeletePlan;

use zenith_core::domain::is_within_window;

/// A plan a bounded store can expire, evict, and consume exactly once.
///
/// The store owns the lifecycle; a plan type states only its identity and when
/// it was created, so a cleanup plan and a reviewed-storage plan share one
/// implementation instead of each re-implementing TTL, capacity, and one-shot
/// removal beside its own commands.
pub trait OneShotPlan {
    fn plan_id(&self) -> Uuid;
    fn created_at(&self) -> u64;
}

/// The lifecycle a store enforces, and the wording a user reads when it fires.
///
/// The messages live here rather than at each call site: a plan that expired is
/// a user-visible event, and the text has to name the workflow the user was in.
pub struct PlanLifecycle {
    ttl_seconds: u64,
    capacity: usize,
    not_found: &'static str,
    expired: &'static str,
}

impl PlanLifecycle {
    /// Cleanup plans: five minutes and sixty-four plans, as the cleanup flow
    /// has always bounded them.
    pub fn cleanup() -> Self {
        Self {
            ttl_seconds: 300,
            capacity: 64,
            not_found: "Delete plan not found or already used",
            expired: "Delete plan expired. Scan again before cleaning.",
        }
    }

    /// Reviewed Trash plans: the same window and bound as cleanup plans, so one
    /// reviewed workflow cannot outlive another.
    pub fn trash() -> Self {
        Self {
            ttl_seconds: 300,
            capacity: 64,
            not_found: "Trash plan not found or already used",
            expired: "Trash plan expired. Review the items again.",
        }
    }
}

/// Backend-owned bounded store for one-shot plans.
///
/// Encapsulates plan capacity, TTL expiration, oldest-first eviction,
/// stale-plan rejection, and one-shot consumption so IPC commands do not
/// coordinate plan lifecycle state directly. The underlying map is private:
/// there is no accessor that hands it out, so a caller cannot bypass the
/// lifecycle by editing it.
pub struct PlanStore<P: OneShotPlan> {
    plans: Mutex<HashMap<Uuid, P>>,
    lifecycle: PlanLifecycle,
}

impl<P: OneShotPlan> PlanStore<P> {
    pub fn new(lifecycle: PlanLifecycle) -> Self {
        Self {
            plans: Mutex::new(HashMap::new()),
            lifecycle,
        }
    }

    pub fn ttl_seconds(&self) -> u64 {
        self.lifecycle.ttl_seconds
    }

    /// Inserts a newly generated plan, enforcing TTL purging and bounded capacity.
    pub fn insert(&self, plan: P, now: u64) -> Result<(), String> {
        let mut plans = self
            .plans
            .lock()
            .map_err(|_| "Plan store lock poisoned".to_string())?;

        // Purge expired plans first
        plans.retain(|_, stored| {
            now.saturating_sub(stored.created_at()) < self.lifecycle.ttl_seconds
        });

        // If at capacity, evict the oldest plan by created_at
        if plans.len() >= self.lifecycle.capacity {
            if let Some(oldest_id) = plans
                .iter()
                .min_by_key(|(_, stored)| stored.created_at())
                .map(|(id, _)| *id)
            {
                plans.remove(&oldest_id);
            }
        }

        plans.insert(plan.plan_id(), plan);
        Ok(())
    }

    /// Takes a valid plan for one-shot execution.
    ///
    /// The plan is removed on retrieval so it cannot be replayed, whether the
    /// execution then succeeds or fails. Fails if the plan does not exist or has
    /// expired according to the lifecycle.
    pub fn take_valid(&self, plan_id: Uuid, now: u64) -> Result<P, String> {
        let mut plans = self
            .plans
            .lock()
            .map_err(|_| "Plan store lock poisoned".to_string())?;

        let plan = plans
            .remove(&plan_id)
            .ok_or_else(|| self.lifecycle.not_found.to_string())?;

        if !is_within_window(plan.created_at(), now, self.lifecycle.ttl_seconds) {
            return Err(self.lifecycle.expired.to_string());
        }

        Ok(plan)
    }

    /// Returns the number of currently retained plans (including any not-yet-purged expired ones).
    #[cfg(test)]
    fn len(&self) -> usize {
        self.plans.lock().map(|p| p.len()).unwrap_or(0)
    }
}

impl OneShotPlan for DeletePlan {
    fn plan_id(&self) -> Uuid {
        self.id
    }

    fn created_at(&self) -> u64 {
        self.created_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RiskSummary;

    fn make_test_plan(id: Uuid, created_at: u64) -> DeletePlan {
        DeletePlan {
            id,
            scan_id: "scan".to_string(),
            targets: Vec::new(),
            expected_reclaim_bytes: 0,
            risk: RiskSummary::default(),
            created_at,
        }
    }

    fn cleanup_store() -> PlanStore<DeletePlan> {
        PlanStore::new(PlanLifecycle::cleanup())
    }

    #[test]
    fn a_taken_plan_cannot_be_replayed() {
        let store = cleanup_store();
        let plan_id = Uuid::new_v4();
        store.insert(make_test_plan(plan_id, 1_000), 1_000).unwrap();

        assert!(store.take_valid(plan_id, 1_050).is_ok());
        let second = store.take_valid(plan_id, 1_051);
        assert!(second.is_err(), "a consumed plan must not be replayable");
        assert!(second.unwrap_err().contains("not found"));
    }

    #[test]
    fn an_expired_plan_is_refused_and_removed() {
        let store = cleanup_store();
        let plan_id = Uuid::new_v4();
        store.insert(make_test_plan(plan_id, 1_000), 1_000).unwrap();

        // 300 seconds is the TTL boundary: at the boundary the plan is stale.
        let expired = store.take_valid(plan_id, 1_300);
        assert!(expired.is_err(), "a plan at its TTL boundary is expired");
        assert!(expired.unwrap_err().contains("expired"));
        assert_eq!(store.len(), 0, "a refused plan is still consumed");
    }

    #[test]
    fn a_clock_that_moved_backwards_does_not_extend_a_plan() {
        let store = cleanup_store();
        let plan_id = Uuid::new_v4();
        store.insert(make_test_plan(plan_id, 1_000), 1_000).unwrap();

        // The plan was created at 1000 and the wall clock now says 999: the
        // deletion authority it carries must not survive the correction.
        let refused = store.take_valid(plan_id, 999);
        assert!(
            refused.is_err(),
            "a plan read through a rolled-back clock must be refused"
        );
        assert!(refused.unwrap_err().contains("expired"));
    }

    #[test]
    fn capacity_evicts_the_oldest_plan() {
        let store = PlanStore::new(PlanLifecycle {
            ttl_seconds: 300,
            capacity: 2,
            not_found: "not found",
            expired: "expired",
        });
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let third = Uuid::new_v4();

        store.insert(make_test_plan(first, 1_000), 1_000).unwrap();
        store.insert(make_test_plan(second, 1_001), 1_001).unwrap();
        store.insert(make_test_plan(third, 1_002), 1_002).unwrap();

        assert_eq!(store.len(), 2);
        assert!(
            store.take_valid(first, 1_003).is_err(),
            "the oldest plan is the one evicted"
        );
        assert!(store.take_valid(second, 1_003).is_ok());
        assert!(store.take_valid(third, 1_003).is_ok());
    }
}
