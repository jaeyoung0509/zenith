use crate::ai_control_center::{notifications, resources};
use crate::models::Recommendation;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};
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
    wake_signal: Arc<(Mutex<bool>, Condvar)>,
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
            wake_signal: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    pub fn notify_wake(&self) {
        let (lock, cvar) = &*self.wake_signal;
        let mut wake = lock.lock().unwrap_or_else(|p| p.into_inner());
        *wake = true;
        cvar.notify_all();
    }

    pub fn are_advisories_enabled(&self) -> bool {
        let preferences = self
            .settings
            .lock()
            .map(|s| s.ai_control.clone())
            .unwrap_or_default();
        background_advisories_enabled(&preferences.autopilot)
    }

    pub fn wait_next_tick(&self, timeout: Duration) {
        let (lock, cvar) = &*self.wake_signal;
        let mut wake = lock.lock().unwrap_or_else(|p| p.into_inner());
        if !*wake {
            let (guard, _) = cvar
                .wait_timeout(wake, timeout)
                .unwrap_or_else(|p| p.into_inner());
            wake = guard;
        }
        *wake = false;
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

        // Passive observations are built on explicit main-window refreshes. When every
        // native advisory is disabled there is no background policy work to perform, so
        // avoid a full process snapshot, memory sample, and `lsof` invocation every five
        // seconds while Zenith is otherwise idle.
        if !background_advisories_enabled(&preferences.autopilot) {
            return Vec::new();
        }

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

        // 2. Sample only the signals required by the enabled advisories. Listener
        // discovery shells out to `lsof`, so battery/memory-only policies must not pay
        // that cost on every runtime tick.
        let memory = preferences
            .autopilot
            .notify_on_memory_pressure
            .then(|| self.memory_sampler.sample());
        let awake_state = self.awake_manager.get_state();
        let listeners = if preferences.autopilot.notify_on_session_completion {
            crate::dev_ports::list_listeners(
                &self.dev_port_store,
                &crate::dev_ports::RealDevPortSystem::default(),
            )
            .unwrap_or_default()
        } else {
            Vec::new()
        };

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
            memory.map(|sample| sample.pressure),
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

fn background_advisories_enabled(preferences: &crate::models::AutopilotPreferences) -> bool {
    preferences.notify_on_battery
        || preferences.notify_on_memory_pressure
        || preferences.notify_on_session_completion
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_sampling_is_disabled_until_an_advisory_is_enabled() {
        let disabled = crate::models::AutopilotPreferences::default();
        assert!(!background_advisories_enabled(&disabled));

        for enabled in [
            crate::models::AutopilotPreferences {
                notify_on_battery: true,
                ..Default::default()
            },
            crate::models::AutopilotPreferences {
                notify_on_memory_pressure: true,
                ..Default::default()
            },
            crate::models::AutopilotPreferences {
                notify_on_session_completion: true,
                ..Default::default()
            },
        ] {
            assert!(background_advisories_enabled(&enabled));
        }
    }

    #[test]
    fn notify_wake_unblocks_waiting_runtime_immediately() {
        let runtime = Arc::new(AiControlRuntime::new(
            Arc::new(crate::metrics::MemorySampler::new()),
            Arc::new(Mutex::new(crate::dev_ports::DevelopmentPortStore::default())),
            Arc::new(Mutex::new(None)),
            Arc::new(Mutex::new(
                crate::ai_control_center::state::AiControlCenterState::default(),
            )),
            Arc::new(crate::power::KeepAwakeManager::new()),
            Arc::new(Mutex::new(crate::models::ZenithSettings::default())),
        ));

        let runtime_bg = runtime.clone();
        let start = std::time::Instant::now();
        let handle = std::thread::spawn(move || {
            runtime_bg.wait_next_tick(Duration::from_secs(10));
        });

        std::thread::sleep(Duration::from_millis(50));
        runtime.notify_wake();
        handle.join().unwrap();
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
