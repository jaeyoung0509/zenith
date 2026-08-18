use crate::models::{AwakeBehavior, AwakeRule, AwakeState, ZenithError};
use crate::power::PowerAssertion;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use sysinfo::{ProcessesToUpdate, System};

pub struct KeepAwakeManager {
    rules: Arc<Mutex<Vec<AwakeRule>>>,
    active_assertion: Arc<Mutex<Option<PowerAssertion>>>,
    manual_mode: Arc<Mutex<Option<(AwakeBehavior, Option<u64>)>>>, // (behavior, expires_at_timestamp)
    last_trigger_app: Arc<Mutex<Option<String>>>,
}

impl Default for KeepAwakeManager {
    fn default() -> Self {
        Self::new()
    }
}

impl KeepAwakeManager {
    pub fn new() -> Self {
        Self {
            rules: Arc::new(Mutex::new(Vec::new())),
            active_assertion: Arc::new(Mutex::new(None)),
            manual_mode: Arc::new(Mutex::new(None)),
            last_trigger_app: Arc::new(Mutex::new(None)),
        }
    }

    /// Sets the active rules to monitor.
    pub fn set_rules(&self, rules: Vec<AwakeRule>) {
        let mut r = self.rules.lock().unwrap();
        *r = rules;
        drop(r);
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

        self.evaluate();
        Ok(())
    }

    /// Disables manual Keep Awake mode.
    pub fn disable_manual(&self) {
        let mut manual = self.manual_mode.lock().unwrap();
        *manual = None;
        drop(manual);

        self.evaluate();
    }

    /// Gets current Keep Awake state.
    pub fn get_state(&self) -> AwakeState {
        let assertion = self.active_assertion.lock().unwrap();
        let is_active = assertion.is_some();
        let behavior = assertion.as_ref().map(|a| a.behavior);
        let trigger = self.last_trigger_app.lock().unwrap().clone();
        let manual = self.manual_mode.lock().unwrap();
        let manual_expires_at = manual.as_ref().and_then(|(_, exp)| *exp);
        let rules_count = self
            .rules
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.enabled)
            .count();

        AwakeState {
            is_active,
            behavior,
            trigger_source: if manual.is_some() {
                Some("Manual override".to_string())
            } else {
                trigger.as_ref().map(|app| format!("Triggered by {}", app))
            },
            active_process_name: trigger,
            manual_expires_at,
            active_rules_count: rules_count,
        }
    }

    /// Evaluates current active processes against rules or manual timers to acquire/release assertions.
    pub fn evaluate(&self) {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 1. Check manual mode
        let mut manual = self.manual_mode.lock().unwrap();
        if let Some((behavior, expires_at)) = *manual {
            if let Some(exp) = expires_at {
                if now >= exp {
                    // Expired
                    *manual = None;
                } else {
                    self.ensure_assertion(
                        behavior,
                        "Zenith Manual Keep Awake",
                        Some("Manual".to_string()),
                    );
                    return;
                }
            } else {
                self.ensure_assertion(
                    behavior,
                    "Zenith Manual Keep Awake",
                    Some("Manual".to_string()),
                );
                return;
            }
        }
        drop(manual);

        // 2. Check rules
        let rules = self.rules.lock().unwrap().clone();
        let enabled_rules: Vec<&AwakeRule> = rules.iter().filter(|r| r.enabled).collect();

        if enabled_rules.is_empty() {
            self.release_assertion();
            return;
        }

        let mut sys = System::new();
        sys.refresh_processes(ProcessesToUpdate::All, true);

        for rule in enabled_rules {
            let patterns: Vec<&str> = rule
                .executable_pattern
                .split('|')
                .map(|s| s.trim())
                .collect();
            for (_, proc) in sys.processes() {
                let name = proc.name().to_string_lossy();
                for pat in &patterns {
                    if name.to_lowercase().contains(&pat.to_lowercase()) {
                        self.ensure_assertion(
                            rule.behavior,
                            &format!("Zenith Keep Awake triggered by {}", rule.app_name),
                            Some(rule.app_name.clone()),
                        );
                        return;
                    }
                }
            }
        }

        // No matching app found
        self.release_assertion();
    }

    fn ensure_assertion(
        &self,
        behavior: AwakeBehavior,
        reason: &str,
        trigger_name: Option<String>,
    ) {
        let mut assertion = self.active_assertion.lock().unwrap();
        let mut last_trig = self.last_trigger_app.lock().unwrap();

        if let Some(ref current) = *assertion {
            if current.behavior == behavior {
                *last_trig = trigger_name;
                return;
            }
        }

        // Acquire new assertion
        if let Ok(new_assertion) = PowerAssertion::acquire(behavior, reason) {
            *assertion = Some(new_assertion);
            *last_trig = trigger_name;
        }
    }

    fn release_assertion(&self) {
        let mut assertion = self.active_assertion.lock().unwrap();
        let mut last_trig = self.last_trigger_app.lock().unwrap();
        *assertion = None;
        *last_trig = None;
    }
}
