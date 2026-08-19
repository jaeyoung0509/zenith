use crate::models::{
    AwakeBehavior, AwakeRule, AwakeRuleEvaluation, AwakeRuleStatus, AwakeState, PowerCondition,
    PowerSourceType, ZenithError,
};
use crate::power::{
    NativeAssertionProvider, PowerAssertion, PowerAssertionProvider, PowerSourceProvider,
    SystemPowerSource,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};
use sysinfo::{ProcessesToUpdate, System};

pub struct KeepAwakeManager {
    rules: Arc<Mutex<Vec<AwakeRule>>>,
    active_assertion: Arc<Mutex<Option<PowerAssertion>>>,
    manual_mode: Arc<Mutex<ManualMode>>,
    last_trigger_app: Arc<Mutex<Option<String>>>,
    last_active_rule_id: Arc<Mutex<Option<String>>>,
    last_error: Arc<Mutex<Option<String>>>,
    rule_evaluations: Arc<Mutex<Vec<AwakeRuleEvaluation>>>,
    power_source_type: Arc<Mutex<PowerSourceType>>,
    power_source: Arc<dyn PowerSourceProvider>,
    assertion_provider: Arc<dyn PowerAssertionProvider>,
    wake_generation: AtomicU64,
    wake_signal: (Mutex<()>, Condvar),
}

type ManualMode = Option<(AwakeBehavior, Option<u64>)>;

impl Default for KeepAwakeManager {
    fn default() -> Self {
        Self::new()
    }
}

impl KeepAwakeManager {
    pub fn new() -> Self {
        Self::with_providers(
            Arc::new(SystemPowerSource::new()),
            Arc::new(NativeAssertionProvider::new()),
        )
    }

    pub fn with_providers(
        power_source: Arc<dyn PowerSourceProvider>,
        assertion_provider: Arc<dyn PowerAssertionProvider>,
    ) -> Self {
        Self {
            rules: Arc::new(Mutex::new(Vec::new())),
            active_assertion: Arc::new(Mutex::new(None)),
            manual_mode: Arc::new(Mutex::new(None)),
            last_trigger_app: Arc::new(Mutex::new(None)),
            last_active_rule_id: Arc::new(Mutex::new(None)),
            last_error: Arc::new(Mutex::new(None)),
            rule_evaluations: Arc::new(Mutex::new(Vec::new())),
            power_source_type: Arc::new(Mutex::new(PowerSourceType::Unknown)),
            power_source,
            assertion_provider,
            wake_generation: AtomicU64::new(0),
            wake_signal: (Mutex::new(()), Condvar::new()),
        }
    }

    /// Sets the active rules to monitor.
    pub fn set_rules(&self, rules: Vec<AwakeRule>) {
        let mut r = self.rules.lock().unwrap();
        *r = rules;
        drop(r);
        self.notify_watcher();
        self.evaluate();
    }

    /// Sets manual Keep Awake duration (in seconds, or None for indefinite until turned off).
    pub fn set_manual(
        &self,
        duration_secs: Option<u64>,
        behavior: AwakeBehavior,
    ) -> Result<(), ZenithError> {
        let mut manual = self.manual_mode.lock().unwrap();
        let expires_at = duration_secs.map(|s| {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                + s
        });

        *manual = Some((behavior, expires_at));
        drop(manual);

        self.notify_watcher();
        let res = self.ensure_assertion(
            behavior,
            "Zenith Manual Keep Awake",
            Some("Manual".to_string()),
            None,
        );
        self.evaluate();
        res
    }

    /// Disables manual Keep Awake mode.
    pub fn disable_manual(&self) {
        let mut manual = self.manual_mode.lock().unwrap();
        *manual = None;
        drop(manual);

        self.notify_watcher();
        self.evaluate();
    }

    /// Gets current Keep Awake state.
    pub fn get_state(&self) -> AwakeState {
        let assertion = self.active_assertion.lock().unwrap();
        let is_active = assertion.is_some();
        let behavior = assertion.as_ref().map(|a| a.behavior);
        let trigger = self.last_trigger_app.lock().unwrap().clone();
        let active_rule_id = self.last_active_rule_id.lock().unwrap().clone();
        let manual = self.manual_mode.lock().unwrap();
        let manual_expires_at = manual.as_ref().and_then(|(_, exp)| *exp);
        let rules = self.rules.lock().unwrap();
        let rules_count = rules.iter().filter(|r| r.enabled).count();
        let power_source = *self.power_source_type.lock().unwrap();
        let last_error = self.last_error.lock().unwrap().clone();
        let rule_evaluations = self.rule_evaluations.lock().unwrap().clone();

        AwakeState {
            is_active,
            behavior,
            trigger_source: if manual.is_some() {
                Some("Manual override".to_string())
            } else {
                trigger.as_ref().map(|app| format!("Triggered by {}", app))
            },
            active_process_name: trigger,
            active_rule_id,
            manual_expires_at,
            active_rules_count: rules_count,
            power_source,
            last_error,
            rule_evaluations,
        }
    }

    /// Evaluates current active processes against rules or manual timers to acquire/release assertions.
    pub fn evaluate(&self) {
        let power_source = self.power_source.current_power_source();
        *self.power_source_type.lock().unwrap() = power_source;

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 1. Check manual mode expiration
        let manual_snapshot = {
            let mut manual = self.manual_mode.lock().unwrap();
            match *manual {
                Some((_, Some(expires_at))) if now >= expires_at => {
                    *manual = None;
                    None
                }
                value => value,
            }
        };

        // 2. Evaluate rules (single process snapshot)
        let rules = self.rules.lock().unwrap().clone();
        let mut evaluations = Vec::new();
        let mut first_eligible_rule: Option<AwakeRule> = None;

        if rules.iter().any(|r| r.enabled) {
            let mut sys = System::new();
            sys.refresh_processes(ProcessesToUpdate::All, true);

            for rule in &rules {
                if !rule.enabled {
                    evaluations.push(AwakeRuleEvaluation {
                        rule_id: rule.id.clone(),
                        status: AwakeRuleStatus::Disabled,
                        is_process_running: false,
                        is_power_eligible: match rule.power_condition {
                            PowerCondition::Always => true,
                            PowerCondition::AcPowerOnly => power_source.is_ac(),
                        },
                    });
                    continue;
                }

                let patterns: Vec<&str> = rule
                    .executable_pattern
                    .split('|')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();

                let is_running = sys.processes().values().any(|proc| {
                    let name = proc.name().to_string_lossy();
                    patterns
                        .iter()
                        .any(|pat| name.to_lowercase().contains(&pat.to_lowercase()))
                });

                let is_power_eligible = match rule.power_condition {
                    PowerCondition::Always => true,
                    PowerCondition::AcPowerOnly => power_source.is_ac(),
                };

                let status = if !is_running {
                    AwakeRuleStatus::WaitingProcess
                } else if !is_power_eligible {
                    AwakeRuleStatus::WaitingPower
                } else {
                    if first_eligible_rule.is_none() {
                        first_eligible_rule = Some(rule.clone());
                    }
                    AwakeRuleStatus::Active
                };

                evaluations.push(AwakeRuleEvaluation {
                    rule_id: rule.id.clone(),
                    status,
                    is_process_running: is_running,
                    is_power_eligible,
                });
            }
        } else {
            for rule in &rules {
                evaluations.push(AwakeRuleEvaluation {
                    rule_id: rule.id.clone(),
                    status: AwakeRuleStatus::Disabled,
                    is_process_running: false,
                    is_power_eligible: match rule.power_condition {
                        PowerCondition::Always => true,
                        PowerCondition::AcPowerOnly => power_source.is_ac(),
                    },
                });
            }
        }

        *self.rule_evaluations.lock().unwrap() = evaluations;

        // 3. Manual mode overrides process rules
        if let Some((behavior, _)) = manual_snapshot {
            let _ = self.ensure_assertion(
                behavior,
                "Zenith Manual Keep Awake",
                Some("Manual".to_string()),
                None,
            );
            return;
        }

        // 4. Apply eligible process rule, or release assertion
        if let Some(rule) = first_eligible_rule {
            let _ = self.ensure_assertion(
                rule.behavior,
                &format!("Zenith Keep Awake triggered by {}", rule.app_name),
                Some(rule.app_name),
                Some(rule.id),
            );
        } else {
            self.release_assertion();
        }
    }

    pub fn wait_for_next_evaluation(&self) {
        let observed = self.wake_generation.load(Ordering::Acquire);
        let has_work = self.manual_mode.lock().unwrap().is_some()
            || self.rules.lock().unwrap().iter().any(|rule| rule.enabled);
        let guard = self.wake_signal.0.lock().unwrap();
        if self.wake_generation.load(Ordering::Acquire) != observed {
            return;
        }
        if has_work {
            let _ = self
                .wake_signal
                .1
                .wait_timeout_while(guard, Duration::from_secs(5), |_| {
                    self.wake_generation.load(Ordering::Acquire) == observed
                });
        } else {
            drop(
                self.wake_signal
                    .1
                    .wait_while(guard, |_| {
                        self.wake_generation.load(Ordering::Acquire) == observed
                    })
                    .unwrap(),
            );
        }
    }

    fn notify_watcher(&self) {
        self.wake_generation.fetch_add(1, Ordering::Release);
        self.wake_signal.1.notify_one();
    }

    fn ensure_assertion(
        &self,
        behavior: AwakeBehavior,
        reason: &str,
        trigger_name: Option<String>,
        rule_id: Option<String>,
    ) -> Result<(), ZenithError> {
        let mut assertion = self.active_assertion.lock().unwrap();
        let mut last_trig = self.last_trigger_app.lock().unwrap();
        let mut last_rule = self.last_active_rule_id.lock().unwrap();
        let mut last_err = self.last_error.lock().unwrap();

        if let Some(ref current) = *assertion {
            if current.behavior == behavior {
                *last_trig = trigger_name;
                *last_rule = rule_id;
                *last_err = None;
                return Ok(());
            }
        }

        match self.assertion_provider.acquire(behavior, reason) {
            Ok(new_assertion) => {
                *assertion = Some(new_assertion);
                *last_trig = trigger_name;
                *last_rule = rule_id;
                *last_err = None;
                Ok(())
            }
            Err(err) => {
                *assertion = None;
                *last_trig = None;
                *last_rule = None;
                let msg = err.to_string();
                *last_err = Some(msg);
                Err(err)
            }
        }
    }

    fn release_assertion(&self) {
        let mut assertion = self.active_assertion.lock().unwrap();
        let mut last_trig = self.last_trigger_app.lock().unwrap();
        let mut last_rule = self.last_active_rule_id.lock().unwrap();
        *assertion = None;
        *last_trig = None;
        *last_rule = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::MockPowerSource;
    use std::sync::atomic::AtomicBool;

    struct TestAssertionProvider {
        should_fail: AtomicBool,
    }

    impl TestAssertionProvider {
        fn new(should_fail: bool) -> Self {
            Self {
                should_fail: AtomicBool::new(should_fail),
            }
        }

        fn set_should_fail(&self, fail: bool) {
            self.should_fail.store(fail, Ordering::SeqCst);
        }
    }

    impl PowerAssertionProvider for TestAssertionProvider {
        fn acquire(
            &self,
            behavior: AwakeBehavior,
            reason: &str,
        ) -> Result<PowerAssertion, ZenithError> {
            if self.should_fail.load(Ordering::SeqCst) {
                Err(ZenithError::Io("Mock assertion acquisition failed".into()))
            } else {
                PowerAssertion::acquire(behavior, reason)
            }
        }
    }

    #[test]
    fn power_condition_eligibility_and_rule_status() {
        let power_mock = Arc::new(MockPowerSource::new(PowerSourceType::Battery));
        let assertion_mock = Arc::new(TestAssertionProvider::new(false));
        let manager = KeepAwakeManager::with_providers(power_mock.clone(), assertion_mock);

        let rule_ac_only = AwakeRule {
            id: "rule.test_ac".to_string(),
            app_name: "NonExistentApp123".to_string(),
            executable_pattern: "non_existent_process_xyz".to_string(),
            behavior: AwakeBehavior::PreventSystemSleep,
            power_condition: PowerCondition::AcPowerOnly,
            enabled: true,
        };

        manager.set_rules(vec![rule_ac_only]);
        let state = manager.get_state();

        assert!(!state.is_active);
        assert_eq!(state.rule_evaluations.len(), 1);
        assert_eq!(
            state.rule_evaluations[0].status,
            AwakeRuleStatus::WaitingProcess
        );
        assert!(!state.rule_evaluations[0].is_power_eligible);
    }

    #[test]
    fn native_failure_sets_last_error_and_subsequent_success_clears_it() {
        let power_mock = Arc::new(MockPowerSource::new(PowerSourceType::Ac));
        let assertion_mock = Arc::new(TestAssertionProvider::new(true)); // Initially fails
        let manager = KeepAwakeManager::with_providers(power_mock, assertion_mock.clone());

        // Attempt manual keep awake with failing assertion provider
        let res = manager.set_manual(Some(1800), AwakeBehavior::PreventSystemSleep);
        assert!(res.is_err());

        let state = manager.get_state();
        assert!(!state.is_active);
        assert!(state.last_error.is_some());
        assert!(state
            .last_error
            .as_ref()
            .unwrap()
            .contains("Mock assertion acquisition failed"));

        // Now fix provider and retry
        assertion_mock.set_should_fail(false);
        let res2 = manager.set_manual(Some(1800), AwakeBehavior::PreventSystemSleep);
        assert!(res2.is_ok());

        let state2 = manager.get_state();
        assert!(state2.is_active);
        assert!(state2.last_error.is_none());
    }

    #[test]
    fn manual_override_and_expiration_lifecycle() {
        let power_mock = Arc::new(MockPowerSource::new(PowerSourceType::Ac));
        let assertion_mock = Arc::new(TestAssertionProvider::new(false));
        let manager = KeepAwakeManager::with_providers(power_mock, assertion_mock);

        // Manual indefinite
        manager
            .set_manual(None, AwakeBehavior::KeepDisplayAwake)
            .unwrap();
        let state = manager.get_state();
        assert!(state.is_active);
        assert_eq!(state.behavior, Some(AwakeBehavior::KeepDisplayAwake));
        assert_eq!(state.trigger_source, Some("Manual override".to_string()));

        // Disable manual
        manager.disable_manual();
        let state2 = manager.get_state();
        assert!(!state2.is_active);
        assert!(state2.behavior.is_none());
    }
}
