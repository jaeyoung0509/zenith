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
    control_center_mode: Arc<Mutex<Option<bool>>>,
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
            control_center_mode: Arc::new(Mutex::new(None)),
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
        let mut r = self.rules.lock().expect("rules poisoned");
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
        let expires_at = duration_secs.map(|s| {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                + s
        });

        // Attempt assertion acquisition first
        match self.ensure_assertion(
            behavior,
            "Zenith Manual Keep Awake",
            Some("Manual".to_string()),
            None,
        ) {
            Ok(()) => {
                let mut manual = self.manual_mode.lock().expect("manual_mode poisoned");
                *manual = Some((behavior, expires_at));
                drop(manual);
                self.notify_watcher();
                Ok(())
            }
            Err(err) => {
                let mut manual = self.manual_mode.lock().expect("manual_mode poisoned");
                *manual = None;
                drop(manual);
                self.notify_watcher();
                self.evaluate();
                Err(err)
            }
        }
    }

    /// Disables manual Keep Awake mode.
    pub fn disable_manual(&self) {
        let mut manual = self.manual_mode.lock().expect("manual_mode poisoned");
        *manual = None;
        drop(manual);

        self.notify_watcher();
        self.evaluate();
    }

    /// Applies the Control Center's backend-verified session policy. The caller
    /// supplies only the canonical snapshot result, never a PID or process name.
    pub fn set_control_center_session_awake(&self, active: bool, ac_only: bool) {
        *self
            .control_center_mode
            .lock()
            .expect("control_center_mode poisoned") = active.then_some(ac_only);
        self.notify_watcher();
        self.evaluate();
    }

    /// Gets current Keep Awake state.
    pub fn get_state(&self) -> AwakeState {
        let assertion = self
            .active_assertion
            .lock()
            .expect("active_assertion poisoned");
        let is_active = assertion.is_some();
        let behavior = assertion.as_ref().map(|a| a.behavior);
        let trigger = self
            .last_trigger_app
            .lock()
            .expect("last_trigger_app poisoned")
            .clone();
        let active_rule_id = self
            .last_active_rule_id
            .lock()
            .expect("last_active_rule_id poisoned")
            .clone();
        let manual = self.manual_mode.lock().expect("manual_mode poisoned");
        let manual_expires_at = if is_active {
            manual.as_ref().and_then(|(_, exp)| *exp)
        } else {
            None
        };
        let rules = self.rules.lock().expect("rules poisoned");
        let rules_count = rules.iter().filter(|r| r.enabled).count();
        let power_source = *self
            .power_source_type
            .lock()
            .expect("power_source_type poisoned");
        let last_error = self.last_error.lock().expect("last_error poisoned").clone();
        let rule_evaluations = self
            .rule_evaluations
            .lock()
            .expect("rule_evaluations poisoned")
            .clone();
        AwakeState {
            is_active,
            behavior,
            trigger_source: if manual.is_some() && is_active {
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
        *self
            .power_source_type
            .lock()
            .expect("power_source_type poisoned") = power_source;

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 1. Check manual mode expiration
        let manual_snapshot = {
            let mut manual = self.manual_mode.lock().expect("manual_mode poisoned");
            match *manual {
                Some((_, Some(expires_at))) if now >= expires_at => {
                    *manual = None;
                    None
                }
                value => value,
            }
        };

        // 2. Evaluate rules (single process snapshot pass)
        let rules = self.rules.lock().expect("rules poisoned").clone();
        let any_enabled = rules.iter().any(|r| r.enabled);

        let sys = if any_enabled {
            let mut s = System::new();
            s.refresh_processes(ProcessesToUpdate::All, true);
            Some(s)
        } else {
            None
        };

        let mut evaluations = Vec::with_capacity(rules.len());
        let mut first_eligible_rule: Option<AwakeRule> = None;

        for rule in &rules {
            let is_power_eligible = match rule.power_condition {
                PowerCondition::Always => true,
                PowerCondition::AcPowerOnly => power_source.is_ac(),
            };

            if !rule.enabled {
                evaluations.push(AwakeRuleEvaluation {
                    rule_id: rule.id.clone(),
                    status: AwakeRuleStatus::Disabled,
                    is_process_running: false,
                    is_power_eligible,
                });
                continue;
            }

            let patterns_lower: Vec<String> = rule
                .executable_pattern
                .split('|')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect();

            let requires_lower: Option<Vec<String>> = rule
                .requires_process_pattern
                .as_deref()
                .map(|pat| {
                    pat.split('|')
                        .map(|s| s.trim().to_lowercase())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                })
                .filter(|v| !v.is_empty());

            let is_running = sys.as_ref().is_some_and(|s| {
                let has_primary = s
                    .processes()
                    .values()
                    .any(|proc| Self::process_matches_patterns(proc, &patterns_lower));
                if !has_primary {
                    return false;
                }
                if let Some(req) = &requires_lower {
                    // Require at least one (different or same) process matching the secondary pattern
                    s.processes()
                        .values()
                        .any(|proc| Self::process_matches_patterns(proc, req))
                } else {
                    true
                }
            });

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
        *self
            .rule_evaluations
            .lock()
            .expect("rule_evaluations poisoned") = evaluations;

        // 3. Manual mode overrides process rules
        if let Some((behavior, _)) = manual_snapshot {
            if let Err(err) = self.ensure_assertion(
                behavior,
                "Zenith Manual Keep Awake",
                Some("Manual".to_string()),
                None,
            ) {
                let _ = err;
                let mut manual = self.manual_mode.lock().expect("manual_mode poisoned");
                *manual = None;
            }
            return;
        }

        // 4. Control Center automation is explicitly enabled only for a
        // backend-verified active session. Unknown power fails AC-only closed.
        let control_mode = *self
            .control_center_mode
            .lock()
            .expect("control_center_mode poisoned");
        if let Some(ac_only) = control_mode {
            if !ac_only || power_source.is_ac() {
                let _ = self.ensure_assertion(
                    AwakeBehavior::PreventSystemSleep,
                    "Zenith AI Control Center verified agent session",
                    Some("AI Control Center".to_string()),
                    Some("ai-control.verified-session".to_string()),
                );
            } else {
                self.release_assertion();
            }
            return;
        }

        // 5. Apply eligible process rule, or release assertion
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
        let has_work = self
            .manual_mode
            .lock()
            .expect("manual_mode poisoned")
            .is_some()
            || self
                .rules
                .lock()
                .expect("rules poisoned")
                .iter()
                .any(|rule| rule.enabled);
        let has_work = has_work
            || self
                .control_center_mode
                .lock()
                .expect("control_center_mode poisoned")
                .is_some();
        let guard = self.wake_signal.0.lock().expect("wake_signal poisoned");
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
        let mut assertion = self
            .active_assertion
            .lock()
            .expect("active_assertion poisoned");
        let mut last_trig = self
            .last_trigger_app
            .lock()
            .expect("last_trigger_app poisoned");
        let mut last_rule = self
            .last_active_rule_id
            .lock()
            .expect("last_active_rule_id poisoned");
        let mut last_err = self.last_error.lock().expect("last_error poisoned");

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
        let mut assertion = self
            .active_assertion
            .lock()
            .expect("active_assertion poisoned");
        let mut last_trig = self
            .last_trigger_app
            .lock()
            .expect("last_trigger_app poisoned");
        let mut last_rule = self
            .last_active_rule_id
            .lock()
            .expect("last_active_rule_id poisoned");
        *assertion = None;
        *last_trig = None;
        *last_rule = None;
    }

    fn process_matches_patterns(proc: &sysinfo::Process, lower_patterns: &[String]) -> bool {
        if lower_patterns.is_empty() {
            return false;
        }
        let name = proc.name().to_string_lossy().to_lowercase();
        let exe = proc.exe().map(|p| p.to_string_lossy().to_lowercase());
        let cmd = proc
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        Self::matches_strings(&name, exe.as_deref(), &cmd, lower_patterns)
    }

    fn matches_strings(
        name_lower: &str,
        exe_lower: Option<&str>,
        cmd_lower: &str,
        lower_patterns: &[String],
    ) -> bool {
        if lower_patterns.is_empty() {
            return false;
        }
        if lower_patterns.iter().any(|pat| name_lower.contains(pat)) {
            return true;
        }
        if let Some(exe) = exe_lower {
            if lower_patterns.iter().any(|pat| exe.contains(pat)) {
                return true;
            }
        }
        if !cmd_lower.is_empty() && lower_patterns.iter().any(|pat| cmd_lower.contains(pat)) {
            return true;
        }
        false
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
            requires_process_pattern: None,
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
        assert_eq!(state.manual_expires_at, None);
        assert_eq!(state.trigger_source, None);
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

    #[test]
    fn multi_rule_evaluation_and_priority() {
        let power_mock = Arc::new(MockPowerSource::new(PowerSourceType::Ac));
        let assertion_mock = Arc::new(TestAssertionProvider::new(false));
        let manager = KeepAwakeManager::with_providers(power_mock, assertion_mock);

        let rule1 = AwakeRule {
            id: "rule.1".to_string(),
            app_name: "App 1".to_string(),
            executable_pattern: "non_existent_111".to_string(),
            requires_process_pattern: None,
            behavior: AwakeBehavior::PreventSystemSleep,
            power_condition: PowerCondition::AcPowerOnly,
            enabled: true,
        };

        let rule2 = AwakeRule {
            id: "rule.2".to_string(),
            app_name: "App 2".to_string(),
            executable_pattern: "non_existent_222".to_string(),
            requires_process_pattern: None,
            behavior: AwakeBehavior::KeepDisplayAwake,
            power_condition: PowerCondition::Always,
            enabled: false,
        };

        manager.set_rules(vec![rule1, rule2]);
        let state = manager.get_state();

        assert_eq!(state.rule_evaluations.len(), 2);
        assert_eq!(
            state.rule_evaluations[0].status,
            AwakeRuleStatus::WaitingProcess
        );
        assert_eq!(state.rule_evaluations[1].status, AwakeRuleStatus::Disabled);
        assert_eq!(state.active_rules_count, 1);
        assert_eq!(state.active_rule_id, None);
    }

    #[test]
    fn get_state_is_pure_in_memory_read_without_provider_calls() {
        let power_mock = Arc::new(MockPowerSource::new(PowerSourceType::Ac));
        let assertion_mock = Arc::new(TestAssertionProvider::new(false));
        let manager = KeepAwakeManager::with_providers(power_mock.clone(), assertion_mock);

        // evaluate once
        manager.evaluate();
        let calls_after_eval = power_mock.query_count();
        assert!(calls_after_eval >= 1);

        // multiple get_state calls must NOT invoke the power source provider or re-scan processes
        for _ in 0..10 {
            let _ = manager.get_state();
        }

        assert_eq!(
            power_mock.query_count(),
            calls_after_eval,
            "get_state must be a pure in-memory read without invoking power source query or process enumeration"
        );
    }

    #[test]
    fn control_center_assertion_honors_power_and_releases_on_session_exit() {
        let power = Arc::new(MockPowerSource::new(PowerSourceType::Battery));
        let assertion = Arc::new(TestAssertionProvider::new(false));
        let manager = KeepAwakeManager::with_providers(power.clone(), assertion);
        manager.set_control_center_session_awake(true, true);
        assert!(
            !manager.get_state().is_active,
            "AC-only must fail closed on battery"
        );
        let plugged_in = Arc::new(MockPowerSource::new(PowerSourceType::Ac));
        let assertion = Arc::new(TestAssertionProvider::new(false));
        let plugged_in_manager = KeepAwakeManager::with_providers(plugged_in, assertion);
        plugged_in_manager.set_control_center_session_awake(true, true);
        assert!(plugged_in_manager.get_state().is_active);
        assert_eq!(
            plugged_in_manager.get_state().active_rule_id.as_deref(),
            Some("ai-control.verified-session")
        );
        plugged_in_manager.set_control_center_session_awake(false, true);
        assert!(
            !plugged_in_manager.get_state().is_active,
            "assertion must release when the verified session exits"
        );
    }

    #[test]
    fn control_center_ac_only_rejects_unknown_power() {
        let power = Arc::new(MockPowerSource::new(PowerSourceType::Unknown));
        let assertion = Arc::new(TestAssertionProvider::new(false));
        let manager = KeepAwakeManager::with_providers(power, assertion);
        manager.set_control_center_session_awake(true, true);
        assert!(!manager.get_state().is_active);
    }

    #[test]
    fn string_matcher_covers_name_exe_and_cmd_with_case_insensitivity() {
        // name matches
        assert!(KeepAwakeManager::matches_strings(
            "codex",
            None,
            "",
            &["codex".to_string()]
        ));
        assert!(KeepAwakeManager::matches_strings(
            "CoDeX Helper".to_lowercase().as_str(),
            None,
            "",
            &["codex".to_string()]
        ));
        // exe matches — warp stable case via exe path
        assert!(KeepAwakeManager::matches_strings(
            "stable",
            Some("/applications/warp.app/contents/macos/stable"),
            "",
            &["warp".to_string()]
        ));
        // cmd matches — bun/node launching opencode/omp
        assert!(KeepAwakeManager::matches_strings(
            "bun",
            Some("/opt/homebrew/bin/bun"),
            "bun run opencode serve --port 3000",
            &["opencode".to_string()]
        ));
        assert!(KeepAwakeManager::matches_strings(
            "node",
            Some("/usr/local/bin/node"),
            "node /Users/test/.opencode/bin/omp start",
            &["omp".to_string()]
        ));
        // negative
        assert!(!KeepAwakeManager::matches_strings(
            "finder",
            Some("/system/library/coreservices/finder.app/contents/macos/finder"),
            "finder",
            &["warp".to_string()]
        ));
        // empty patterns never match
        assert!(!KeepAwakeManager::matches_strings("codex", None, "", &[]));
    }

    #[test]
    fn manual_override_takes_precedence_over_process_rules() {
        let power_mock = Arc::new(MockPowerSource::new(PowerSourceType::Ac));
        let assertion_mock = Arc::new(TestAssertionProvider::new(false));
        let manager = KeepAwakeManager::with_providers(power_mock, assertion_mock);

        // First create an active rule that matches the current test binary
        let current_exe = std::env::current_exe()
            .ok()
            .and_then(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase().to_string())
            })
            .unwrap_or_else(|| "test".to_string());
        let active_rule = AwakeRule {
            id: "rule.active".to_string(),
            app_name: "ActiveApp".to_string(),
            executable_pattern: current_exe,
            requires_process_pattern: None,
            behavior: AwakeBehavior::PreventSystemSleep,
            power_condition: PowerCondition::Always,
            enabled: true,
        };
        manager.set_rules(vec![active_rule]);
        manager.evaluate();
        assert!(
            manager.get_state().is_active,
            "precondition: active rule should be Active before manual"
        );

        // Even with an active process rule, manual should take precedence
        manager
            .set_manual(Some(3600), AwakeBehavior::KeepDisplayAwake)
            .unwrap();
        let state = manager.get_state();
        assert!(state.is_active);
        assert_eq!(state.trigger_source, Some("Manual override".to_string()));

        // evaluate should keep manual active and not downgrade to process rule
        manager.evaluate();
        let state2 = manager.get_state();
        assert!(state2.is_active);
        assert_eq!(state2.trigger_source, Some("Manual override".to_string()));
        assert_eq!(state2.behavior, Some(AwakeBehavior::KeepDisplayAwake));
    }

    #[test]
    fn power_condition_matrix_ac_vs_battery() {
        let current_exe = std::env::current_exe()
            .ok()
            .and_then(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase().to_string())
            })
            .unwrap_or_else(|| "test".to_string());
        let assertion_mock = Arc::new(TestAssertionProvider::new(false));

        // AC + AcPowerOnly + matching process => Active
        let power_ac = Arc::new(MockPowerSource::new(PowerSourceType::Ac));
        let manager_ac = KeepAwakeManager::with_providers(power_ac, assertion_mock.clone());
        let rule_ac_only = AwakeRule {
            id: "rule.ac".to_string(),
            app_name: "Test".to_string(),
            executable_pattern: current_exe.clone(),
            requires_process_pattern: None,
            behavior: AwakeBehavior::PreventSystemSleep,
            power_condition: PowerCondition::AcPowerOnly,
            enabled: true,
        };
        manager_ac.set_rules(vec![rule_ac_only.clone()]);
        manager_ac.evaluate();
        let eval_ac = manager_ac.get_state().rule_evaluations[0].clone();
        assert!(eval_ac.is_process_running);
        assert!(eval_ac.is_power_eligible);
        assert_eq!(eval_ac.status, AwakeRuleStatus::Active);

        // Battery + AcPowerOnly + matching process => WaitingPower
        let power_battery = Arc::new(MockPowerSource::new(PowerSourceType::Battery));
        let manager_bat =
            KeepAwakeManager::with_providers(power_battery.clone(), assertion_mock.clone());
        manager_bat.set_rules(vec![rule_ac_only]);
        manager_bat.evaluate();
        let eval_bat = manager_bat.get_state().rule_evaluations[0].clone();
        assert!(eval_bat.is_process_running);
        assert!(!eval_bat.is_power_eligible);
        assert_eq!(eval_bat.status, AwakeRuleStatus::WaitingPower);

        // Battery + Always + matching process => Active (power condition ignored)
        let rule_always = AwakeRule {
            id: "rule.always".to_string(),
            app_name: "TestAlways".to_string(),
            executable_pattern: current_exe,
            requires_process_pattern: None,
            behavior: AwakeBehavior::PreventSystemSleep,
            power_condition: PowerCondition::Always,
            enabled: true,
        };
        let manager_bat_always = KeepAwakeManager::with_providers(power_battery, assertion_mock);
        manager_bat_always.set_rules(vec![rule_always]);
        manager_bat_always.evaluate();
        let eval_bat_always = manager_bat_always.get_state().rule_evaluations[0].clone();
        assert!(eval_bat_always.is_process_running);
        assert!(eval_bat_always.is_power_eligible);
        assert_eq!(eval_bat_always.status, AwakeRuleStatus::Active);
    }

    #[test]
    fn compound_requires_pattern_and_behavior() {
        // Use pure matcher to avoid depending on live processes for determinism
        let primary = vec!["warp".to_string()];
        let requires = vec!["codex".to_string()];
        // Only primary present — compound should be false
        let has_primary = KeepAwakeManager::matches_strings(
            "warp",
            Some("/Applications/Warp.app/Contents/MacOS/stable"),
            "warp",
            &primary,
        );
        let has_requires = KeepAwakeManager::matches_strings(
            "warp",
            Some("/Applications/Warp.app/Contents/MacOS/stable"),
            "warp",
            &requires,
        );
        assert!(has_primary);
        assert!(!has_requires);
        assert!(!(has_primary && has_requires));
        // Both present — different processes, simulated via two separate matches
        let has_primary = KeepAwakeManager::matches_strings(
            "warp",
            Some("/Applications/Warp.app/Contents/MacOS/stable"),
            "warp",
            &primary,
        );
        let has_requires = KeepAwakeManager::matches_strings(
            "codex",
            Some("/usr/local/bin/codex"),
            "codex --help",
            &requires,
        );
        assert!(has_primary && has_requires);

        // Exercise the actual evaluate path with a real matching process for compound:
        // Create a manager with a compound rule that requires "warp" AND "codex".
        // Since the test host has many processes, we use a pattern that matches the test binary itself
        // for both halves to guarantee Active without needing to know exact process names.
        let current_exe = std::env::current_exe()
            .ok()
            .and_then(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase().to_string())
            })
            .unwrap_or_else(|| "test".to_string());
        let power_mock = Arc::new(MockPowerSource::new(PowerSourceType::Ac));
        let assertion_mock = Arc::new(TestAssertionProvider::new(false));
        let manager = KeepAwakeManager::with_providers(power_mock, assertion_mock);
        let compound_rule = AwakeRule {
            id: "rule.compound".to_string(),
            app_name: "Warp+Codex".to_string(),
            executable_pattern: current_exe.clone(),
            requires_process_pattern: Some(current_exe),
            behavior: AwakeBehavior::PreventSystemSleep,
            power_condition: PowerCondition::Always,
            enabled: true,
        };
        manager.set_rules(vec![compound_rule]);
        manager.evaluate();
        let eval = manager.get_state().rule_evaluations[0].clone();
        // Should be Active because both patterns match the same running test process
        assert_eq!(eval.status, AwakeRuleStatus::Active);
        assert!(eval.is_process_running);
    }

    #[test]
    fn first_active_rule_has_priority() {
        let current_exe = std::env::current_exe()
            .ok()
            .and_then(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase().to_string())
            })
            .unwrap_or_else(|| "test".to_string());
        let power_mock = Arc::new(MockPowerSource::new(PowerSourceType::Ac));
        let assertion_mock = Arc::new(TestAssertionProvider::new(false));
        let manager = KeepAwakeManager::with_providers(power_mock, assertion_mock);
        let rule1 = AwakeRule {
            id: "rule.first".to_string(),
            app_name: "First".to_string(),
            executable_pattern: current_exe.clone(),
            requires_process_pattern: None,
            behavior: AwakeBehavior::PreventSystemSleep,
            power_condition: PowerCondition::Always,
            enabled: true,
        };
        let rule2 = AwakeRule {
            id: "rule.second".to_string(),
            app_name: "Second".to_string(),
            executable_pattern: current_exe,
            requires_process_pattern: None,
            behavior: AwakeBehavior::KeepDisplayAwake,
            power_condition: PowerCondition::Always,
            enabled: true,
        };
        manager.set_rules(vec![rule1.clone(), rule2]);
        manager.evaluate();
        let state = manager.get_state();
        assert_eq!(state.active_rule_id, Some("rule.first".to_string()));
        assert_eq!(state.rule_evaluations[0].status, AwakeRuleStatus::Active);
        assert_eq!(state.rule_evaluations[1].status, AwakeRuleStatus::Active);
    }
}
