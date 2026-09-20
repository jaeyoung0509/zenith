//! Whether a running process makes a planned cleanup unsafe.
//!
//! The catalog states which executables guard a signature (a compiler or
//! runtime that holds its own cache open); this module answers the other half
//! of the question by looking at the process table. Both halves are needed
//! together, which is why the check is evaluated at execution time against the
//! policy the plan carried, rather than by looking a signature up again: the
//! plan is what was authorized, and the guard must judge what was authorized.
//!
//! A refusal here is per target. It never falls back to a broader deletion and
//! never retries: a cache whose owner is running stays where it is until the
//! user closes the tool, which is the outcome the user asked for.

use sysinfo::{ProcessesToUpdate, System};
use zenith_core::domain::cleanup::{RunningProcessPolicy, RunningProcessProbe};

/// The process-table port an owner-scoped provider reads through.
///
/// The provider must decide whether a store's owner is idle, and the process
/// table belongs to this module — the one that already reads it for the
/// execution guard. `None` is the honest answer when the table cannot be read:
/// an owner provider must fail closed rather than read an unreadable table as
/// an idle owner.
pub struct SysinfoProcessProbe;

impl RunningProcessProbe for SysinfoProcessProbe {
    fn running(&self, guard: &RunningProcessPolicy) -> Option<Vec<String>> {
        if guard.is_empty() {
            return Some(Vec::new());
        }
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::All, true);
        if system.processes().is_empty() {
            // A process table with no entries is not a machine with no
            // processes; it is a table this process could not read.
            return None;
        }
        let mut matched: Vec<String> = system
            .processes()
            .values()
            .map(|process| process.name().to_string_lossy().into_owned())
            .filter(|name| guard.matches(name))
            .collect();
        matched.sort();
        matched.dedup();
        Some(matched)
    }
}

/// The executables from `policy` that are running right now.
///
/// Returns every match, not just the first, so the message can name what the
/// user has to close.
pub fn running_executables(policy: &RunningProcessPolicy) -> Vec<String> {
    if policy.is_empty() {
        return Vec::new();
    }

    let mut matched: Vec<String> = running_process_names()
        .into_iter()
        .filter(|name| policy.matches(name))
        .collect();
    matched.sort();
    matched.dedup();
    matched
}

/// Whether a running process matches the target's guard.
pub fn blocked_by_running_process(policy: &RunningProcessPolicy) -> bool {
    if policy.is_empty() {
        return false;
    }
    blocked_by(policy, &running_process_names())
}

/// The decision, given a process list. Split out so the rule is testable
/// without depending on which processes the test runner happens to see.
fn blocked_by(policy: &RunningProcessPolicy, running: &[String]) -> bool {
    policy.matches_any(running.iter().map(String::as_str))
}

/// Every running process name, in one pass over the process table.
fn running_process_names() -> Vec<String> {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system
        .processes()
        .values()
        .map(|process| process.name().to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_policy_never_blocks() {
        assert!(!blocked_by_running_process(&RunningProcessPolicy::none()));
        assert!(running_executables(&RunningProcessPolicy::none()).is_empty());
        assert!(!blocked_by(
            &RunningProcessPolicy::none(),
            &["cargo".to_string()]
        ));
    }

    #[test]
    fn the_rule_matches_a_running_executable_by_name() {
        let policy = RunningProcessPolicy::guarding(vec!["cargo".into(), "python3".into()]);
        let running = vec!["zsh".to_string(), "PYTHON3".to_string()];
        assert!(blocked_by(&policy, &running));
        assert!(!blocked_by(
            &policy,
            &["zsh".to_string(), "node".to_string()]
        ));
    }

    /// The probe reads the live process table: a guard naming something that
    /// cannot be running reports nothing rather than failing.
    #[test]
    fn the_live_probe_answers_without_a_false_positive() {
        let policy =
            RunningProcessPolicy::guarding(vec!["zenith-guard-fixture-never-running".into()]);
        assert!(!blocked_by_running_process(&policy));
        assert!(running_executables(&policy).is_empty());
    }
}
