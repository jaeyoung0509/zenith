//! System-wide CPU observation.
//!
//! `sysinfo` derives a CPU share by differencing two reads of the platform's
//! own counters, so a single read measures nothing and two reads too close
//! together measure a window the library refuses to difference. This module
//! owns that state machine: it holds its own [`System`], primed once, and every
//! answer it gives is either a share two real reads produced, or an explicit
//! statement of why there is none.
//!
//! The sampler deliberately does not refresh the process table. Memory
//! observation owns that refresh (one full pass per observation), and a second
//! sampler refreshing the same table would double the cost of both.

use std::panic::AssertUnwindSafe;
use std::sync::Mutex;
use std::time::Instant;

use sysinfo::{CpuRefreshKind, System};

use crate::metrics::{ipc_millis, unix_millis};
use crate::models::{CpuMetrics, CpuSampleState};

/// Age after which an existing reading must be reported `stale`.
///
/// This is the foreground poll budget: the dashboard samples the system at
/// roughly this interval while it is visible, so a reading older than it no
/// longer describes the machine now. A hidden window does not poll at all, so
/// the first observation after an activation is allowed to be a fresh pair of
/// readings rather than a value from before the window was hidden.
pub const CPU_STALE_AFTER_MS: u64 = 3_000;

/// The reason a machine with no logical CPU cannot be normalized.
const NO_LOGICAL_CPUS_REASON: &str =
    "The operating system reported no logical CPUs, so a system-wide busy share cannot be normalized.";

/// The reason a recovered sampler does not trust its own probe.
const POISONED_LOCK_REASON: &str =
    "A previous observation panicked while holding the CPU sampler state.";

/// The CPU counters one platform read touches.
///
/// The native implementation is `sysinfo`; tests inject a scripted one so a
/// refused or panicking platform read is a case in a test rather than an
/// assumption about the host.
trait CpuCounters: Send {
    /// Records the baseline the next difference is computed against.
    fn prime(&mut self);
    /// Reads the counters again, computing the difference since the baseline.
    fn refresh(&mut self);
    /// Logical CPU count as last read, or 0 when the platform reported none.
    fn logical_cores(&self) -> usize;
    /// The busy share across all logical cores.
    ///
    /// `System::global_cpu_usage()` averages every logical core rather than
    /// summing them, so this is the machine's busy share normalized 0-100
    /// regardless of how many cores it has.
    fn busy_percent(&self) -> f32;
}

impl CpuCounters for System {
    fn prime(&mut self) {
        // `System::new()` loads nothing, and a usage refresh does not create
        // the CPU list, so the list is read once here with usage enabled.
        // Re-reading the list later would reset the baseline the next
        // difference needs.
        self.refresh_cpu_list(CpuRefreshKind::nothing().with_cpu_usage());
    }

    fn refresh(&mut self) {
        self.refresh_cpu_usage();
    }

    fn logical_cores(&self) -> usize {
        self.cpus().len()
    }

    fn busy_percent(&self) -> f32 {
        self.global_cpu_usage()
    }
}

/// The sampler's own state: the counters and the last measurement they produced.
struct CpuState {
    counters: Box<dyn CpuCounters>,
    /// When the counters were last read, on the monotonic clock the platform
    /// itself uses between two differing reads.
    last_refresh: Option<Instant>,
    last_percent: Option<f32>,
    last_window_ms: Option<u64>,
    last_sampled_at: Option<u64>,
    /// Why the last probe failed, kept so a `Failed` report can explain itself.
    last_error: Option<String>,
}

impl CpuState {
    /// The reply for one state, carrying whatever reading the sampler holds.
    ///
    /// A held reading is never relabelled as a new measurement: it keeps the
    /// interval and the instant that produced it, and only a state that says
    /// the probe could not produce a value is allowed to accompany it.
    fn metrics(&self, state: CpuSampleState, cores: u32, reason: Option<String>) -> CpuMetrics {
        CpuMetrics {
            state,
            usage_percent: self.last_percent,
            sample_interval_ms: self.last_window_ms,
            sampled_at: self.last_sampled_at,
            stale_after_ms: CPU_STALE_AFTER_MS,
            cores,
            reason,
        }
    }

    /// The `Failed` report: the previous measurement stays, and the reason is
    /// what the probe died with.
    fn failed_metrics(&self) -> CpuMetrics {
        let reason = self
            .last_error
            .clone()
            .unwrap_or_else(|| "The CPU probe failed without stating a reason.".to_string());
        self.metrics(
            CpuSampleState::Failed,
            self.counters.logical_cores() as u32,
            Some(reason),
        )
    }

    /// Whether the held reading is older than the poll budget on the wall
    /// clock.
    ///
    /// The age is measured against the instant the reading was stamped with,
    /// not against the monotonic clock: `Instant` does not advance while the
    /// machine sleeps, so a laptop that was suspended for an hour comes back
    /// with a monotonic age of seconds and a reading taken before the sleep.
    fn is_stale_at(&self, now_unix_ms: u64) -> bool {
        self.last_sampled_at
            .is_some_and(|sampled_at| now_unix_ms.saturating_sub(sampled_at) > CPU_STALE_AFTER_MS)
    }

    /// One platform read, with a panicking probe turned into the text it died
    /// with instead of escaping the observation.
    fn probe(&mut self, prime: bool) -> Result<(), String> {
        let counters = &mut self.counters;
        match std::panic::catch_unwind(AssertUnwindSafe(|| {
            if prime {
                counters.prime();
            } else {
                counters.refresh();
            }
        })) {
            Ok(()) => Ok(()),
            Err(payload) => Err(panic_reason(payload.as_ref())),
        }
    }
}

/// The text a panicking probe died with, routed through the same redacting
/// sink as a joined worker so a failure cannot leak what the log would hide.
fn panic_reason(payload: &(dyn std::any::Any + Send)) -> String {
    let message = if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "the CPU probe panicked with a non-text payload".to_string()
    };
    crate::blocking::join_failure("The CPU probe panicked", message)
}

/// Observes the host's system-wide CPU share.
pub struct CpuSampler {
    state: Mutex<CpuState>,
}

impl Default for CpuSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuSampler {
    /// A sampler over this machine's own counters.
    pub fn new() -> Self {
        Self::with_counters(Box::new(System::new()))
    }

    fn with_counters(counters: Box<dyn CpuCounters>) -> Self {
        Self {
            state: Mutex::new(CpuState {
                counters,
                last_refresh: None,
                last_percent: None,
                last_window_ms: None,
                last_sampled_at: None,
                last_error: None,
            }),
        }
    }

    /// One observation of the system-wide CPU share.
    pub fn observe(&self) -> CpuMetrics {
        self.observe_at(Instant::now(), unix_millis())
    }

    fn observe_at(&self, now: Instant, now_unix_ms: u64) -> CpuMetrics {
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                // The observation is disposable, so the state comes back
                // rather than being dropped: the last real reading survives,
                // and the report says the probe is not trustworthy. Clearing
                // the poison is not an option — a worker that died inside this
                // state may have left it half-written.
                let mut recovered = poisoned.into_inner();
                recovered.last_error = Some(POISONED_LOCK_REASON.to_string());
                return recovered.failed_metrics();
            }
        };

        if state.last_refresh.is_none() {
            // Priming: the platform needs one read to difference the next one
            // against, and that first read measures nothing.
            if let Err(reason) = state.probe(true) {
                state.last_error = Some(reason);
                return state.failed_metrics();
            }
            let cores = state.counters.logical_cores();
            if cores == 0 {
                return state.metrics(
                    CpuSampleState::Unavailable,
                    0,
                    Some(NO_LOGICAL_CPUS_REASON.to_string()),
                );
            }
            state.last_refresh = Some(now);
            state.last_error = None;
            return state.metrics(CpuSampleState::Warmup, cores as u32, None);
        }

        let elapsed = now.saturating_duration_since(state.last_refresh.unwrap_or(now));
        let cores = state.counters.logical_cores() as u32;

        if elapsed < sysinfo::MINIMUM_CPU_UPDATE_INTERVAL {
            // A read this close would not difference anything: the platform
            // keeps the earlier counters, so reporting them against this
            // window would invent the interval as much as the value. The
            // measurement a real pair produced is repeated instead — still
            // `Fresh` while its age is inside the poll budget, `Stale` once
            // the wall clock says it is older.
            return match state.last_percent {
                Some(_) if state.is_stale_at(now_unix_ms) => {
                    state.metrics(CpuSampleState::Stale, cores, None)
                }
                Some(_) => state.metrics(CpuSampleState::Fresh, cores, None),
                None => state.metrics(CpuSampleState::Warmup, cores, None),
            };
        }

        if let Err(reason) = state.probe(false) {
            state.last_error = Some(reason);
            return state.failed_metrics();
        }
        let cores = state.counters.logical_cores();
        if cores == 0 {
            return state.metrics(
                CpuSampleState::Unavailable,
                0,
                Some(NO_LOGICAL_CPUS_REASON.to_string()),
            );
        }

        let percent = state.counters.busy_percent().clamp(0.0, 100.0);
        state.last_percent = Some(percent);
        state.last_window_ms = Some(ipc_millis(elapsed));
        state.last_sampled_at = Some(now_unix_ms);
        state.last_refresh = Some(now);
        state.last_error = None;
        state.metrics(CpuSampleState::Fresh, cores as u32, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::MemorySampler;
    use std::time::Duration;

    /// A probe driven by a script instead of by the host.
    ///
    /// Every step either reports `(logical cores, busy percent)` or panics the
    /// way a broken platform read would, so a refused or failed observation is
    /// exercised on any machine.
    struct ScriptedCounters {
        steps: Vec<Result<(usize, f32), &'static str>>,
        cursor: usize,
        last: (usize, f32),
    }

    impl ScriptedCounters {
        fn new(steps: Vec<Result<(usize, f32), &'static str>>) -> Self {
            Self {
                steps,
                cursor: 0,
                last: (0, 0.0),
            }
        }

        fn step(&mut self) {
            let step = self
                .steps
                .get(self.cursor)
                .copied()
                .unwrap_or(Err("the scripted probe ran out of steps"));
            self.cursor += 1;
            match step {
                Ok(counters) => self.last = counters,
                Err(message) => panic!("{message}"),
            }
        }
    }

    impl CpuCounters for ScriptedCounters {
        fn prime(&mut self) {
            self.step();
        }

        fn refresh(&mut self) {
            self.step();
        }

        fn logical_cores(&self) -> usize {
            self.last.0
        }

        fn busy_percent(&self) -> f32 {
            self.last.1
        }
    }

    fn scripted(steps: Vec<Result<(usize, f32), &'static str>>) -> CpuSampler {
        CpuSampler::with_counters(Box::new(ScriptedCounters::new(steps)))
    }

    /// An interval the platform accepts as a difference, standing in for the
    /// poll that follows one.
    fn outside_the_minimum_interval() -> Duration {
        sysinfo::MINIMUM_CPU_UPDATE_INTERVAL + Duration::from_millis(100)
    }

    fn minimum_interval_ms() -> u64 {
        u64::try_from(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.as_millis()).unwrap_or(0)
    }

    #[test]
    fn a_primed_sampler_warms_up_without_a_percentage() {
        let sampler = CpuSampler::new();

        let first = sampler.observe();

        assert_eq!(first.state, CpuSampleState::Warmup);
        assert_eq!(
            first.usage_percent, None,
            "one read of the counters measures no window, so it produces no share"
        );
        assert_eq!(first.sample_interval_ms, None);
        assert_eq!(first.sampled_at, None);
        assert_eq!(first.stale_after_ms, CPU_STALE_AFTER_MS);
        assert!(first.cores > 0, "the host reports logical CPUs");
        assert_eq!(first.reason, None);
    }

    #[test]
    fn a_second_reading_outside_the_minimum_interval_reports_a_measured_share() {
        let sampler = CpuSampler::new();
        sampler.observe();

        std::thread::sleep(outside_the_minimum_interval());
        let measured = sampler.observe();

        assert_eq!(measured.state, CpuSampleState::Fresh);
        let percent = measured
            .usage_percent
            .expect("a pair of readings apart must produce a share");
        assert!(
            (0.0..=100.0).contains(&percent),
            "the share is normalized across all logical cores: {percent}"
        );
        let interval = measured
            .sample_interval_ms
            .expect("the window the pair measured is reported with the share");
        assert!(
            interval >= minimum_interval_ms(),
            "the reported window must be the one the platform differenced: {interval}"
        );
        assert!(measured.sampled_at.is_some());
        assert_eq!(measured.reason, None);
    }

    #[test]
    fn an_observation_inside_the_minimum_interval_repeats_the_measured_reading() {
        let sampler = scripted(vec![Ok((8, 0.0)), Ok((8, 42.5)), Ok((8, 99.0))]);
        let base = Instant::now();
        let first_ms = 1_700_000_000_000u64;

        assert_eq!(
            sampler.observe_at(base, first_ms).state,
            CpuSampleState::Warmup
        );
        let measured = sampler.observe_at(base + outside_the_minimum_interval(), first_ms + 300);
        assert_eq!(measured.usage_percent, Some(42.5));

        // The next request arrives inside the platform's minimum interval.
        let repeated = sampler.observe_at(
            base + outside_the_minimum_interval() + Duration::from_millis(10),
            first_ms + 310,
        );

        assert_eq!(repeated.state, CpuSampleState::Fresh);
        assert_eq!(
            repeated.usage_percent,
            Some(42.5),
            "the share must be the one a real pair produced, never the next scripted read"
        );
        assert_eq!(repeated.sample_interval_ms, measured.sample_interval_ms);
        assert_eq!(
            repeated.sampled_at, measured.sampled_at,
            "a repeated reading keeps the instant it was taken"
        );
    }

    #[test]
    fn a_reading_past_the_poll_budget_is_reported_stale_and_kept() {
        let sampler = scripted(vec![Ok((8, 0.0)), Ok((8, 42.5))]);
        let base = Instant::now();
        let first_ms = 1_700_000_000_000u64;

        sampler.observe_at(base, first_ms);
        let measured = sampler.observe_at(base + outside_the_minimum_interval(), first_ms + 300);
        assert_eq!(measured.state, CpuSampleState::Fresh);

        // The machine slept: the monotonic clock the platform needs between
        // two differing reads has barely moved, while the reading's own age on
        // the wall clock is past the budget.
        let stale = sampler.observe_at(
            base + outside_the_minimum_interval() + Duration::from_millis(10),
            first_ms + 300 + CPU_STALE_AFTER_MS + 1,
        );

        assert_eq!(stale.state, CpuSampleState::Stale);
        assert_eq!(
            stale.usage_percent,
            Some(42.5),
            "a stale reading is reported with its value, not discarded"
        );
        assert_eq!(stale.sampled_at, measured.sampled_at);
        assert_eq!(stale.reason, None);
    }

    #[test]
    fn a_platform_without_logical_cpus_reports_unavailable() {
        let sampler = scripted(vec![Ok((0, 0.0))]);

        let metrics = sampler.observe();

        assert_eq!(metrics.state, CpuSampleState::Unavailable);
        assert_eq!(metrics.cores, 0);
        assert_eq!(
            metrics.usage_percent, None,
            "no share can be normalized without a logical CPU to divide by"
        );
        assert!(
            metrics
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("no logical CPUs")),
            "the refusal must say what the platform reported: {:?}",
            metrics.reason
        );
    }

    #[test]
    fn a_panicking_probe_fails_and_keeps_the_previous_reading() {
        let sampler = scripted(vec![
            Ok((8, 0.0)),
            Ok((8, 37.0)),
            Err("the platform refused the counter read"),
        ]);
        let base = Instant::now();
        let first_ms = 1_700_000_000_000u64;

        sampler.observe_at(base, first_ms);
        let measured = sampler.observe_at(base + outside_the_minimum_interval(), first_ms + 300);
        assert_eq!(measured.usage_percent, Some(37.0));

        let failed = sampler.observe_at(
            base + outside_the_minimum_interval().saturating_mul(2),
            first_ms + 600,
        );

        assert_eq!(failed.state, CpuSampleState::Failed);
        assert!(
            failed
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("the platform refused the counter read")),
            "the failure must carry what the probe died with: {:?}",
            failed.reason
        );
        assert_eq!(
            failed.usage_percent,
            Some(37.0),
            "a failed probe keeps the last measurement rather than reporting none"
        );
        assert_eq!(failed.sampled_at, measured.sampled_at);
    }

    #[test]
    fn observing_the_cpu_never_refreshes_the_process_table() {
        let memory = MemorySampler::new();
        memory.observe();
        assert_eq!(memory.process_refresh_count(), 1);

        let cpu = CpuSampler::new();
        cpu.observe();
        std::thread::sleep(outside_the_minimum_interval());
        cpu.observe();

        assert_eq!(
            memory.process_refresh_count(),
            1,
            "the CPU sampler owns its own counters and must not re-read the memory sampler's process table"
        );
    }
}
