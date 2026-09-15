//! Health records for the long-lived background loops the desktop shell drives.
//!
//! Two workers run for the life of the process: the Keep Awake evaluation loop
//! and the AI Control advisory tick. A dead iteration must be observable rather
//! than silent — the invariant #166 established for scanning and cleanup,
//! applied to the runtime loops. Each loop owns one [`BackgroundLoopHealth`]
//! record: a panicked iteration is captured instead of ending the thread, its
//! payload is reported through the same redacting sink as
//! [`crate::blocking::join_failure`], and the next interval retries.
//!
//! The record never claims a newer evaluation than the last one that actually
//! completed, so the interface cannot present a stale evaluation as current.
//!
//! # Poison policy
//!
//! A lock inside these loops guards disposable observation or derived state,
//! so it recovers from poisoning the way `operation_gate` does: the next
//! idempotent pass rebuilds whatever a panicked pass left half-mutated.
//! Fail-closed handling stays with transactional/authorization state (the
//! settings authority and the bounded plan store), which is not driven by
//! these loops and keeps its own stated reasons.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Whether a long-lived background loop's last iteration succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundLoopStatus {
    /// The loop is running and its stored evaluation is the last completed one.
    #[default]
    Healthy,
    /// The last iteration panicked; the loop still runs and retries on the
    /// next interval, but the stored evaluation is the last *successful* one.
    Degraded,
}

/// Observable health of one long-lived background loop.
///
/// Timestamps are Unix seconds; `None` means the loop has not reached that
/// event yet, which is itself observable (a loop that has never completed
/// cannot back a "current" evaluation either).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, specta::Type)]
pub struct BackgroundLoopHealth {
    pub status: BackgroundLoopStatus,
    #[serde(default, with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub last_completed_at: Option<u64>,
    #[serde(default, with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub failed_at: Option<u64>,
    #[serde(default)]
    pub reason: Option<String>,
}

impl BackgroundLoopHealth {
    /// Records a completed iteration: the failure marker is cleared because
    /// the stored evaluation is current again.
    fn record_completed(&mut self, now: u64) {
        self.status = BackgroundLoopStatus::Healthy;
        self.last_completed_at = Some(now);
        self.failed_at = None;
        self.reason = None;
    }

    /// Records a panicked iteration, keeping the last successful pass so the
    /// age of the stored evaluation stays visible.
    fn record_failure(&mut self, now: u64, reason: String) {
        self.status = BackgroundLoopStatus::Degraded;
        self.failed_at = Some(now);
        self.reason = Some(reason);
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Runs one iteration of a background loop, capturing a panic instead of
/// letting it end the thread.
///
/// On success the health record is marked current. On panic the payload goes
/// to the diagnostics sink (which redacts credentials and masks paths) and the
/// sanitized form is stored in the record, so the failure is observable and
/// the loop keeps its next interval. `None` means the iteration produced
/// nothing; the caller simply waits for the next one.
pub(crate) fn run_iteration<T>(
    context: &'static str,
    health: &Arc<Mutex<BackgroundLoopHealth>>,
    work: impl FnOnce() -> T,
) -> Option<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
        Ok(value) => {
            health
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .record_completed(unix_timestamp());
            Some(value)
        }
        Err(panic) => {
            let payload = panic_message(panic.as_ref());
            // The log keeps the raw payload; what is stored and surfaced is
            // sanitized exactly like a joined worker's failure.
            let sanitized = crate::blocking::join_failure(context, payload);
            health
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .record_failure(unix_timestamp(), sanitized);
            None
        }
    }
}

/// The panic payload as text: `String` and `&str` payloads keep their message,
/// anything else is named instead of guessed at.
fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "the iteration panicked with a non-text payload".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> Arc<Mutex<BackgroundLoopHealth>> {
        Arc::new(Mutex::new(BackgroundLoopHealth::default()))
    }

    fn snapshot(health: &Arc<Mutex<BackgroundLoopHealth>>) -> BackgroundLoopHealth {
        health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    #[test]
    fn a_completed_iteration_marks_the_record_current() {
        let health = record();

        let value = run_iteration("Test loop panicked", &health, || 7);

        assert_eq!(value, Some(7));
        let record = snapshot(&health);
        assert_eq!(record.status, BackgroundLoopStatus::Healthy);
        assert!(record.last_completed_at.is_some());
        assert_eq!(record.reason, None);
    }

    #[test]
    fn a_panicking_iteration_is_recorded_sanitized_and_the_caller_continues() {
        let health = record();
        // Mark a prior success so the record can keep the age of the stored
        // evaluation visible after the failure.
        run_iteration("Test loop panicked", &health, || ());

        let value = run_iteration("Test loop panicked", &health, || {
            panic!("cache index was truncated at /Users/apple/secret");
        });
        assert_eq!(value, None, "the caller must get a result, not a panic");

        let record = snapshot(&health);
        assert_eq!(record.status, BackgroundLoopStatus::Degraded);
        assert!(
            record.last_completed_at.is_some(),
            "the last successful pass stays visible"
        );
        assert!(
            record.failed_at.unwrap_or_default() >= record.last_completed_at.unwrap_or_default(),
            "the failure is at least as recent as the last completion"
        );
        let reason = record.reason.expect("the failure reason is stored");
        assert!(
            reason.contains("cache index was truncated"),
            "the panic payload must survive sanitization: {reason}"
        );
        assert!(
            !reason.contains("/Users/apple/secret"),
            "paths must be masked before a failure crosses IPC: {reason}"
        );
        assert!(
            reason.starts_with("Test loop panicked: "),
            "the caller's context must stay attached: {reason}"
        );
    }

    #[test]
    fn a_recovered_iteration_clears_the_degradation() {
        let health = record();

        run_iteration("Test loop panicked", &health, || panic!("first pass dies"));
        run_iteration("Test loop panicked", &health, || ());

        let record = snapshot(&health);
        assert_eq!(record.status, BackgroundLoopStatus::Healthy);
        assert_eq!(record.reason, None);
        assert_eq!(record.failed_at, None);
    }

    #[test]
    fn a_poisoned_health_record_does_not_break_the_loop() {
        let health = record();
        // Poison the record the way a panicked holder would, then prove the
        // loop keeps working through the recovered lock.
        std::thread::scope(|scope| {
            let holder = health.clone();
            scope
                .spawn(move || {
                    let _guard = holder.lock().unwrap();
                    panic!("the holder dies with the lock");
                })
                .join()
                .unwrap_err();
        });

        let value = run_iteration("Test loop panicked", &health, || 3);
        assert_eq!(value, Some(3));
        assert_eq!(snapshot(&health).status, BackgroundLoopStatus::Healthy);
    }
}
