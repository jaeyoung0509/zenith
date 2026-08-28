use crate::ai_control_center::{notifications, resources};
use crate::models::Recommendation;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tauri::AppHandle;

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub struct AiControlRuntime {
    memory_sampler: Arc<crate::metrics::MemorySampler>,
    dev_port_store: Arc<Mutex<crate::dev_ports::DevelopmentPortStore>>,
    agent_activity_cache: Arc<Mutex<Option<crate::agent_activity::AgentActivityRegistry>>>,
    ai_control_state: Arc<Mutex<crate::ai_control_center::state::AiControlCenterState>>,
    awake_manager: Arc<crate::power::KeepAwakeManager>,
    settings: Arc<Mutex<crate::models::ZenithSettings>>,
}

impl AiControlRuntime {
    pub fn new(
        memory_sampler: Arc<crate::metrics::MemorySampler>,
        dev_port_store: Arc<Mutex<crate::dev_ports::DevelopmentPortStore>>,
        agent_activity_cache: Arc<Mutex<Option<crate::agent_activity::AgentActivityRegistry>>>,
        ai_control_state: Arc<Mutex<crate::ai_control_center::state::AiControlCenterState>>,
        awake_manager: Arc<crate::power::KeepAwakeManager>,
        settings: Arc<Mutex<crate::models::ZenithSettings>>,
    ) -> Self {
        Self {
            memory_sampler,
            dev_port_store,
            agent_activity_cache,
            ai_control_state,
            awake_manager,
            settings,
        }
    }

    /// Evaluates local background signals: active agent activity, dev ports, memory pressure,
    /// power source transitions, and autopilot advisory notifications.
    /// Does NOT perform external provider calls, full filesystem scans, or Git queries.
    pub fn tick(&self, app_handle: Option<&AppHandle>) -> Vec<Recommendation> {
        let now = unix_timestamp();
        let preferences = {
            self.settings
                .lock()
                .map(|s| s.ai_control.clone())
                .unwrap_or_default()
        };

        // 1. Collect or check local agent activity snapshot
        let activity = {
            let cached = self
                .agent_activity_cache
                .lock()
                .ok()
                .and_then(|guard| guard.clone());
            if let Some(val) = cached.filter(|val| {
                now.saturating_sub(val.snapshot.observed_at)
                    < crate::agent_activity::SNAPSHOT_TTL_SECONDS
            }) {
                val
            } else {
                let fresh = crate::agent_activity::collect_registry();
                if let Ok(mut guard) = self.agent_activity_cache.lock() {
                    *guard = Some(fresh.clone());
                }
                fresh
            }
        };

        // 2. Sample memory pressure and power source
        let memory = self.memory_sampler.sample();
        let awake_state = self.awake_manager.get_state();
        let listeners = crate::dev_ports::list_listeners(
            &self.dev_port_store,
            &crate::dev_ports::RealDevPortSystem::default(),
        )
        .unwrap_or_default();

        let resources = resources::attribute(
            &activity.snapshot,
            &activity.project_roots,
            &listeners,
            awake_state.power_source,
            preferences.autopilot.keep_awake_ac_only,
        );

        // 3. Evaluate policy
        let mut control = match self.ai_control_state.lock() {
            Ok(guard) => guard,
            Err(_) => return Vec::new(),
        };

        let new_items = control.policy.evaluate(
            &resources,
            Some(memory.pressure),
            awake_state.power_source,
            &preferences.autopilot,
            now,
        );

        if !new_items.is_empty() {
            if let Some(app) = app_handle {
                let _ = notifications::emit_advisories(app, &new_items);
            }
            control.recommendations.extend(new_items.clone());
            control
                .recommendations
                .sort_by_key(|item| std::cmp::Reverse(item.created_at));
            control.recommendations.truncate(64);
        }

        new_items
    }
}
