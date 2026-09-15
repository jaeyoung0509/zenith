use crate::ai_control_center::resources;
use crate::ai_snapshots::fetch_activity_registry;
use crate::collection::SingleFlight;
use crate::models::Recommendation;
use crate::runtime_metrics::RuntimeMetrics;
use crate::services::desktop_notifications::DesktopNotifications;
use crate::services::SettingsAuthority;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub struct AiControlRuntime {
    memory_sampler: Arc<crate::metrics::MemorySampler>,
    dev_port_store: Arc<Mutex<crate::dev_ports::DevelopmentPortStore>>,
    /// The machine this runtime describes. Listener classification compares
    /// process paths and masks working directories, so it needs the stated
    /// environment rather than the host's.
    environment: std::sync::Arc<zenith_platform::PlatformEnvironment>,
    agent_activity_cache: Arc<Mutex<Option<crate::agent_activity::AgentActivityRegistry>>>,
    activity_singleflight: Arc<SingleFlight<crate::agent_activity::AgentActivityRegistry, ()>>,
    activity_generation: Arc<AtomicU64>,
    runtime_metrics: Arc<RuntimeMetrics>,
    ai_control_state: Arc<Mutex<crate::ai_control_center::state::AiControlCenterState>>,
    awake_manager: Arc<crate::power::KeepAwakeManager>,
    settings: Arc<SettingsAuthority>,
    wake_signal: Arc<(Mutex<bool>, Condvar)>,
    /// Health of the advisory tick loop the desktop shell drives; written by
    /// the guarded iteration, read by the snapshot builders.
    tick_health: Arc<Mutex<crate::runtime_health::BackgroundLoopHealth>>,
}

impl AiControlRuntime {
    // Nine shared handles wire the background tick into the same
    // single-flight/cache/generation ownership as foreground commands.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        memory_sampler: Arc<crate::metrics::MemorySampler>,
        dev_port_store: Arc<Mutex<crate::dev_ports::DevelopmentPortStore>>,
        environment: std::sync::Arc<zenith_platform::PlatformEnvironment>,
        agent_activity_cache: Arc<Mutex<Option<crate::agent_activity::AgentActivityRegistry>>>,
        activity_singleflight: Arc<SingleFlight<crate::agent_activity::AgentActivityRegistry, ()>>,
        activity_generation: Arc<AtomicU64>,
        runtime_metrics: Arc<RuntimeMetrics>,
        ai_control_state: Arc<Mutex<crate::ai_control_center::state::AiControlCenterState>>,
        awake_manager: Arc<crate::power::KeepAwakeManager>,
        settings: Arc<SettingsAuthority>,
    ) -> Self {
        Self {
            memory_sampler,
            dev_port_store,
            environment,
            agent_activity_cache,
            activity_singleflight,
            activity_generation,
            runtime_metrics,
            ai_control_state,
            awake_manager,
            settings,
            wake_signal: Arc::new((Mutex::new(false), Condvar::new())),
            tick_health: Arc::new(Mutex::new(
                crate::runtime_health::BackgroundLoopHealth::default(),
            )),
        }
    }

    /// One guarded iteration of the advisory tick loop the desktop shell
    /// drives.
    ///
    /// A panicked tick is recorded instead of ending the thread: the health
    /// record degrades with the sanitized failure and the next interval
    /// retries, exactly like the Keep Awake evaluation loop.
    pub fn run_background_tick(&self, notifications: Option<&dyn DesktopNotifications>) {
        crate::runtime_health::run_iteration(
            "AI Control advisory tick panicked",
            &self.tick_health,
            || self.tick(notifications),
        );
    }

    /// The advisory tick loop's observable health.
    pub fn tick_health(&self) -> crate::runtime_health::BackgroundLoopHealth {
        self.tick_health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
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
            .snapshot()
            .map(|settings| settings.ai_control)
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
    pub fn tick(&self, notifications: Option<&dyn DesktopNotifications>) -> Vec<Recommendation> {
        let now = unix_timestamp();
        let preferences = self
            .settings
            .snapshot()
            .map(|settings| settings.ai_control)
            .unwrap_or_default();

        // Passive observations are built on explicit main-window refreshes. When every
        // native advisory is disabled there is no background policy work to perform, so
        // avoid a full process snapshot, memory sample, and `lsof` invocation every five
        // seconds while Zenith is otherwise idle.
        if !background_advisories_enabled(&preferences.autopilot) {
            return Vec::new();
        }

        // 1. Collect or check local agent activity snapshot through the shared
        // single-flight so foreground and background callers execute one
        // underlying collection. The inactivity threshold comes from settings
        // so both paths publish identically configured results. This runs on
        // a plain background OS thread (not the async executor), so blocking
        // on the shared async service is safe here and never nests inside an
        // executor callback.
        let inactivity_threshold_secs = self
            .settings
            .snapshot()
            .map(|settings| {
                u64::from(settings.agent_notifications.inactivity_threshold_minutes) * 60
            })
            .unwrap_or(crate::agent_activity::DEFAULT_INACTIVITY_THRESHOLD_SECONDS);
        let activity = {
            let fetched = tauri::async_runtime::block_on(fetch_activity_registry(
                &self.agent_activity_cache,
                &self.activity_singleflight,
                &self.activity_generation,
                &self.runtime_metrics,
                &self.environment,
                inactivity_threshold_secs,
                false,
            ));
            match fetched {
                Ok(registry) => registry,
                Err(_) => return Vec::new(),
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
            crate::dev_ports::list_listeners_with_context(
                &self.dev_port_store,
                &crate::dev_ports::RealDevPortSystem::new(self.environment.flavor()),
                self.environment.user_home().as_deref(),
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
        //
        // The control state is disposable observation state: a poisoned lock
        // is recovered like the other loop locks so one panic cannot stop the
        // tick for the rest of the process (a silent skip would be the same
        // dead-worker failure this runtime exists to prevent).
        // The produced items are stored under the lock and the notification
        // runs outside it, in that order. `policy.evaluate` consumes the
        // cooldown for the session the moment it returns an item, so an
        // external notification call made while the lock is held could panic
        // and lose a recommendation that a retry can never regenerate — the
        // cooldown is already spent. External code never runs under the lock
        // either: it cannot poison the shared state it does not hold.
        let new_items = {
            let mut control = self
                .ai_control_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let new_items = control.policy.evaluate(
                &resources,
                memory.map(|sample| sample.pressure),
                awake_state.power_source,
                &preferences.autopilot,
                now,
            );
            if !new_items.is_empty() {
                control.recommendations.extend(new_items.clone());
                control
                    .recommendations
                    .sort_by_key(|item| std::cmp::Reverse(item.created_at));
                control.recommendations.truncate(64);
            }
            new_items
        };

        if !new_items.is_empty() {
            if let Some(notifications) = notifications {
                let _ = notifications.emit_recommendations(&new_items);
            }
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
    use std::sync::{mpsc, Barrier};

    fn test_runtime(
        memory_sampler: Arc<crate::metrics::MemorySampler>,
        agent_activity_cache: Arc<Mutex<Option<crate::agent_activity::AgentActivityRegistry>>>,
    ) -> Arc<AiControlRuntime> {
        test_runtime_with(
            memory_sampler,
            agent_activity_cache,
            crate::models::ZenithSettings::default(),
            crate::power::KeepAwakeManager::new(),
            Arc::new(Mutex::new(
                crate::ai_control_center::state::AiControlCenterState::default(),
            )),
        )
    }

    /// The same harness with explicit settings and a stated power source, so
    /// an advisory path can be driven deterministically.
    fn test_runtime_with(
        memory_sampler: Arc<crate::metrics::MemorySampler>,
        agent_activity_cache: Arc<Mutex<Option<crate::agent_activity::AgentActivityRegistry>>>,
        settings: crate::models::ZenithSettings,
        awake: crate::power::KeepAwakeManager,
        control_state: Arc<Mutex<crate::ai_control_center::state::AiControlCenterState>>,
    ) -> Arc<AiControlRuntime> {
        Arc::new(AiControlRuntime::new(
            memory_sampler,
            Arc::new(Mutex::new(crate::dev_ports::DevelopmentPortStore::default())),
            Arc::new(zenith_platform::PlatformEnvironment::native()),
            agent_activity_cache,
            Arc::new(SingleFlight::new()),
            Arc::new(AtomicU64::new(1)),
            Arc::new(RuntimeMetrics::new()),
            control_state,
            Arc::new(awake),
            Arc::new(SettingsAuthority::new(settings)),
        ))
    }

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
    fn disabled_runtime_performs_zero_background_observations() {
        let memory_sampler = Arc::new(crate::metrics::MemorySampler::new());
        let agent_activity_cache = Arc::new(Mutex::new(None));
        let runtime = test_runtime(memory_sampler.clone(), agent_activity_cache.clone());

        assert!(runtime.tick(None).is_empty());
        assert!(!memory_sampler.is_initialized());
        assert!(agent_activity_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_none());
    }

    #[test]
    fn notify_wake_unblocks_waiting_runtime_immediately() {
        let runtime = test_runtime(
            Arc::new(crate::metrics::MemorySampler::new()),
            Arc::new(Mutex::new(None)),
        );
        let barrier = Arc::new(Barrier::new(2));
        let (done_tx, done_rx) = mpsc::channel();

        let runtime_bg = runtime.clone();
        let barrier_bg = barrier.clone();
        let handle = std::thread::spawn(move || {
            barrier_bg.wait();
            runtime_bg.wait_next_tick(Duration::from_secs(10));
            done_tx.send(()).unwrap();
        });

        barrier.wait();
        runtime.notify_wake();
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        handle.join().unwrap();
    }

    /// A notification sink whose delivery panics, to inject a deterministic
    /// worker failure into one advisory tick.
    struct PanickingSink;
    struct SilentSink;

    impl crate::services::desktop_notifications::DesktopNotifications for PanickingSink {
        fn request_permission(&self) -> Result<(), String> {
            Ok(())
        }

        fn emit_recommendations(
            &self,
            _recommendations: &[crate::models::Recommendation],
        ) -> Vec<String> {
            panic!("notification sink exploded");
        }

        fn emit_process_advisories(
            &self,
            _snapshot: &crate::models::AgentActivitySnapshot,
            _preferences: &crate::models::AgentNotificationPreferences,
            _filter: &mut crate::agent_activity::notifications::NotificationFilter,
        ) -> Vec<String> {
            Vec::new()
        }
    }

    impl crate::services::desktop_notifications::DesktopNotifications for SilentSink {
        fn request_permission(&self) -> Result<(), String> {
            Ok(())
        }

        fn emit_recommendations(
            &self,
            _recommendations: &[crate::models::Recommendation],
        ) -> Vec<String> {
            Vec::new()
        }

        fn emit_process_advisories(
            &self,
            _snapshot: &crate::models::AgentActivitySnapshot,
            _preferences: &crate::models::AgentNotificationPreferences,
            _filter: &mut crate::agent_activity::notifications::NotificationFilter,
        ) -> Vec<String> {
            Vec::new()
        }
    }

    /// A fresh registry with one attributed session, so the battery advisory
    /// has something to fire on: the cache stays within its TTL, so the tick
    /// reuses it instead of collecting.
    fn seeded_activity_cache(
        now: u64,
    ) -> Arc<Mutex<Option<crate::agent_activity::AgentActivityRegistry>>> {
        let session = crate::models::AgentSession {
            id: "opaque.session".into(),
            tool_id: "codex".into(),
            tool_name: "Codex".into(),
            status: crate::models::AgentActivityStatus::Active,
            attention_reason: None,
            evidence: crate::models::AgentEvidence::ProcessObserved,
            observed_at: now,
            started_at: now,
            elapsed_seconds: 30,
            cpu_percent: 1.0,
            memory_bytes: 0,
            project_id: None,
            worktree_id: None,
            detail: "Seeded for the runtime health test.".into(),
            can_stop: false,
            stop_lease_id: None,
        };
        let registry = crate::agent_activity::AgentActivityRegistry {
            snapshot: crate::models::AgentActivitySnapshot {
                observed_at: now,
                quality: crate::models::SnapshotQuality::Fresh,
                projects: Vec::new(),
                unassigned_sessions: vec![session],
                adapters: Vec::new(),
                partial_errors: Vec::new(),
            },
            project_roots: std::collections::HashMap::new(),
        };
        Arc::new(Mutex::new(Some(registry)))
    }

    fn battery_advisory_runtime(
        activity_cache: Arc<Mutex<Option<crate::agent_activity::AgentActivityRegistry>>>,
        control_state: Arc<Mutex<crate::ai_control_center::state::AiControlCenterState>>,
    ) -> Arc<AiControlRuntime> {
        let settings = crate::models::ZenithSettings {
            ai_control: crate::models::AiControlPreferences {
                autopilot: crate::models::AutopilotPreferences {
                    notify_on_battery: true,
                    ..crate::models::AutopilotPreferences::default()
                },
                ..crate::models::AiControlPreferences::default()
            },
            ..crate::models::ZenithSettings::default()
        };
        let awake = crate::power::KeepAwakeManager::with_providers(
            Arc::new(crate::power::MockPowerSource::new(
                crate::models::PowerSourceType::Battery,
            )),
            Arc::new(crate::power::NativeAssertionProvider::new()),
        );
        test_runtime_with(
            Arc::new(crate::metrics::MemorySampler::new()),
            activity_cache,
            settings,
            awake,
            control_state,
        )
    }

    #[test]
    fn a_panicked_advisory_tick_is_recorded_and_the_next_interval_recovers() {
        let now = unix_timestamp();
        let activity = seeded_activity_cache(now);
        let control_state = Arc::new(Mutex::new(
            crate::ai_control_center::state::AiControlCenterState::default(),
        ));
        let runtime = battery_advisory_runtime(activity, control_state.clone());

        assert_eq!(
            runtime.tick_health().status,
            crate::runtime_health::BackgroundLoopStatus::Healthy
        );

        // The tick panics while delivering the battery advisory; the failure
        // becomes observable instead of silently ending the worker.
        runtime.run_background_tick(Some(&PanickingSink));
        let degraded = runtime.tick_health();
        assert_eq!(
            degraded.status,
            crate::runtime_health::BackgroundLoopStatus::Degraded
        );
        // The panicked delivery must not lose the recommendation: it was
        // stored before the notification ran, and the cooldown is already
        // consumed, so no later tick can regenerate it.
        let state = control_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            !state.recommendations.is_empty(),
            "the recommendation must survive the panicked notification"
        );
        drop(state);
        let reason = degraded
            .last_failure_reason
            .expect("the failure is surfaced with a reason");
        assert!(
            reason.contains("notification sink exploded"),
            "the failure reason carries the panic payload: {reason}"
        );

        // The next interval retries and the record is current again; the
        // advisory still reached the loop (it is now stored in state).
        runtime.run_background_tick(Some(&SilentSink));
        let recovered = runtime.tick_health();
        assert_eq!(
            recovered.status,
            crate::runtime_health::BackgroundLoopStatus::Healthy
        );
        assert!(
            recovered.last_failure_reason.is_some(),
            "the recovered record still carries the last failure for the interface"
        );
        assert!(
            runtime.tick_health().last_completed_at.is_some(),
            "the completed tick is timestamped"
        );
    }

    #[test]
    fn a_poisoned_control_state_lock_does_not_stop_the_tick() {
        let now = unix_timestamp();
        let activity = seeded_activity_cache(now);
        let control_state = Arc::new(Mutex::new(
            crate::ai_control_center::state::AiControlCenterState::default(),
        ));
        let runtime = battery_advisory_runtime(activity, control_state.clone());

        // Poison the control state the way a panicked holder would.
        std::thread::scope(|scope| {
            let holder = control_state.clone();
            scope
                .spawn(move || {
                    let _guard = holder.lock().unwrap();
                    panic!("the holder dies with the lock");
                })
                .join()
                .unwrap_err();
        });

        // The tick still completes through the recovered lock: one poisoned
        // pass cannot stop the worker for the remaining life of the process.
        runtime.run_background_tick(Some(&SilentSink));
        assert_eq!(
            runtime.tick_health().status,
            crate::runtime_health::BackgroundLoopStatus::Healthy
        );
        let state = control_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            !state.recommendations.is_empty(),
            "the tick's advisory reached the recovered state"
        );
    }
}
