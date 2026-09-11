use crate::platform::GracefulStopOutcome;
use crate::process_owner::ProcessOwner;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

pub const LEASE_TTL_SECS: u64 = 30;
pub const MAX_STOP_LEASES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopLease {
    pub lease_id: String,
    pub session_id: String,
    pub pid: u32,
    pub start_time: u64,
    pub executable: PathBuf,
    pub cwd: Option<PathBuf>,
    pub owner: ProcessOwner,
    pub expires_at: u64,
}

#[derive(Debug, Default)]
pub struct StopLeaseStore {
    leases: HashMap<String, StopLease>, // keyed by session_id
    order: VecDeque<String>,
}

impl StopLeaseStore {
    #[allow(clippy::too_many_arguments)]
    pub fn create_lease(
        &mut self,
        session_id: &str,
        pid: u32,
        start_time: u64,
        executable: PathBuf,
        cwd: Option<PathBuf>,
        owner: ProcessOwner,
        now: u64,
    ) -> String {
        let lease_id = format!("lease-{}", uuid::Uuid::new_v4());
        let lease = StopLease {
            lease_id: lease_id.clone(),
            session_id: session_id.to_string(),
            pid,
            start_time,
            executable,
            cwd,
            owner,
            expires_at: now + LEASE_TTL_SECS,
        };
        self.leases.retain(|_, l| l.expires_at > now);
        self.order.retain(|id| self.leases.contains_key(id));
        while self.leases.len() >= MAX_STOP_LEASES {
            if let Some(oldest) = self.order.pop_front() {
                self.leases.remove(&oldest);
            } else {
                break;
            }
        }
        self.order.push_back(session_id.to_string());
        self.leases.insert(session_id.to_string(), lease);
        lease_id
    }

    pub fn consume_lease(
        &mut self,
        session_id: &str,
        lease_id: &str,
        now: u64,
    ) -> Result<StopLease, String> {
        self.leases.retain(|_, l| l.expires_at > now);
        self.order.retain(|id| self.leases.contains_key(id));
        let lease = self.leases.get(session_id).ok_or_else(|| {
            "Stop lease expired or not found. Please refresh and try again.".to_string()
        })?;

        if lease.lease_id != lease_id {
            return Err("Invalid stop lease token.".to_string());
        }
        let lease = self
            .leases
            .remove(session_id)
            .ok_or_else(|| "Stop lease is no longer available.".to_string())?;
        self.order.retain(|id| id != session_id);
        Ok(lease)
    }
}

#[derive(Debug, Clone)]
pub struct ProcessCheckInfo {
    pub pid: u32,
    pub owner: ProcessOwner,
    pub start_time: u64,
    pub executable: Option<PathBuf>,
    pub cmd: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub parent_pid: Option<u32>,
    pub name: String,
}

pub trait TerminationSystem: Send + Sync {
    fn current_owner(&self) -> ProcessOwner;
    fn current_pid(&self) -> u32;
    fn get_process_info(&self, pid: u32) -> Option<ProcessCheckInfo>;
    fn is_terminal_or_protected(&self, info: &ProcessCheckInfo) -> bool;
    /// Requests a graceful stop without forcing. The caller owns the fallback
    /// so force termination only runs after a fresh identity verification.
    fn request_graceful_stop(&self, pid: u32) -> Result<GracefulStopOutcome, String>;
    /// Force termination, used only after a graceful request did not end the
    /// process and its identity was re-verified.
    fn force_terminate(&self, pid: u32) -> Result<(), String>;
}

pub struct RealTerminationSystem;

impl TerminationSystem for RealTerminationSystem {
    fn current_owner(&self) -> ProcessOwner {
        ProcessOwner::current()
    }

    fn current_pid(&self) -> u32 {
        std::process::id()
    }

    fn get_process_info(&self, pid: u32) -> Option<ProcessCheckInfo> {
        let mut sys = sysinfo::System::new();
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[
                sysinfo::Pid::from_u32(pid),
                sysinfo::Pid::from_u32(std::process::id()),
            ]),
            true,
            sysinfo::ProcessRefreshKind::everything(),
        );
        let process = sys.process(sysinfo::Pid::from_u32(pid))?;
        let own_uid = sys
            .process(sysinfo::Pid::from_u32(std::process::id()))
            .and_then(|process| process.effective_user_id().or_else(|| process.user_id()))
            .cloned();
        let owner = ProcessOwner::verified(
            process.effective_user_id().or_else(|| process.user_id()),
            own_uid.as_ref(),
        )?;
        Some(ProcessCheckInfo {
            pid,
            owner,
            start_time: process.start_time(),
            executable: process.exe().map(PathBuf::from),
            cmd: process
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().to_string())
                .collect(),
            cwd: process.cwd().map(PathBuf::from),
            parent_pid: process.parent().map(|p| p.as_u32()),
            name: process.name().to_string_lossy().to_string(),
        })
    }

    fn is_terminal_or_protected(&self, info: &ProcessCheckInfo) -> bool {
        if crate::process_protection::is_protected_process(
            &info.name,
            Some(&info.name),
            info.executable.as_deref(),
        ) {
            return true;
        }
        // Agent-specific executable guard stays local; the shared module owns
        // the terminal/shell/system deny-list so workflows cannot drift.
        const PROTECTED_NAMES: &[&str] =
            &["login", "launchd", "systemd", "zsh", "bash", "fish", "sh"];
        let name_lower = info.name.to_lowercase();
        if PROTECTED_NAMES
            .iter()
            .any(|p| name_lower.contains(&p.to_lowercase()))
        {
            return true;
        }
        if let Some(exe) = &info.executable {
            let exe_name = exe.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if PROTECTED_NAMES
                .iter()
                .any(|p| exe_name.eq_ignore_ascii_case(p))
            {
                return true;
            }
        }
        false
    }

    fn request_graceful_stop(&self, pid: u32) -> Result<GracefulStopOutcome, String> {
        // Never forces. `execute_graceful_stop` re-verifies the lease identity
        // before any force fallback, including when no graceful channel exists.
        crate::platform::request_graceful_stop(pid)
    }

    fn force_terminate(&self, pid: u32) -> Result<(), String> {
        #[cfg(any(unix, target_os = "windows"))]
        {
            crate::platform::terminate_process(pid, crate::platform::TerminationMode::Force)
        }
        #[cfg(not(any(unix, target_os = "windows")))]
        {
            let _ = pid;
            Err("Force stop is only supported on Unix or Windows systems.".to_string())
        }
    }
}

pub fn execute_graceful_stop(
    lease: &StopLease,
    system: &dyn TerminationSystem,
) -> Result<(), String> {
    // 1. Process must exist
    let info = system
        .get_process_info(lease.pid)
        .ok_or_else(|| "Process no longer exists.".to_string())?;

    // 2. Never signal system or self
    if info.pid <= 1 || info.pid == system.current_pid() {
        return Err("Cannot terminate system or Zenith process.".to_string());
    }

    // 3. Owner identity check compares real platform identities.
    if info.owner != system.current_owner() || info.owner != lease.owner {
        return Err("Process owner mismatch or process belongs to another user.".to_string());
    }

    // 4. Start time check (CRITICAL TOCTOU PID-reuse prevention!)
    if info.start_time != lease.start_time {
        return Err("Process identity drift detected: start time changed (PID reuse).".to_string());
    }

    // 5. Executable identity match
    let Some(current_exe) = &info.executable else {
        return Err("Cannot determine process executable path.".to_string());
    };
    if !crate::platform::NativePlatformPaths::paths_equal(current_exe, &lease.executable) {
        return Err("Process executable path drift detected.".to_string());
    }

    // 6. Check if adapter allows termination
    if crate::agent_activity::adapters::adapter_for_process(current_exe, &info.cmd).is_none() {
        return Err("Process is not an allowlisted agent CLI.".to_string());
    }

    // 7. Cwd check if present
    if lease.cwd != info.cwd {
        return Err("Process working directory is unavailable or has changed.".to_string());
    }

    // 8. Terminal and protected ancestry check
    if system.is_terminal_or_protected(&info) {
        return Err(
            "Target process or its parent is a protected terminal or system process.".to_string(),
        );
    }

    // 9. Deliver the stop request. On Unix this is SIGTERM and the caller's
    // contract is unchanged. On Windows a delivered WM_CLOSE/CTRL_BREAK can be
    // ignored, and a target without a window or console has no graceful channel
    // at all; both paths re-verify the lease identity immediately before any
    // force termination.
    let graceful = system.request_graceful_stop(lease.pid)?;

    #[cfg(target_os = "windows")]
    {
        if graceful == GracefulStopOutcome::Delivered {
            const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(150);
            const MAX_ATTEMPTS: usize = 10;
            for _ in 0..MAX_ATTEMPTS {
                std::thread::sleep(POLL_INTERVAL);
                let Some(info) = system.get_process_info(lease.pid) else {
                    return Ok(());
                };
                if !same_lease_identity(lease, &info, system) {
                    // A different process now owns this PID; the leased process
                    // is gone and the replacement must never be forced.
                    return Ok(());
                }
            }
        }

        // The process either ignored a delivered request or has no graceful
        // channel. Re-verify immediately before forcing so a recycled PID is
        // still rejected.
        let Some(info) = system.get_process_info(lease.pid) else {
            return Ok(());
        };
        if !same_lease_identity(lease, &info, system) {
            return Ok(());
        }
        system.force_terminate(lease.pid)?;
    }

    #[cfg(not(target_os = "windows"))]
    let _ = graceful;

    Ok(())
}

/// Identity checks shared by the initial validation and the pre-force recheck.
#[cfg(target_os = "windows")]
fn same_lease_identity(
    lease: &StopLease,
    info: &ProcessCheckInfo,
    system: &dyn TerminationSystem,
) -> bool {
    info.pid == lease.pid
        && info.owner == lease.owner
        && info.owner == system.current_owner()
        && info.start_time == lease.start_time
        && info.executable.as_deref().is_some_and(|executable| {
            crate::platform::NativePlatformPaths::paths_equal(executable, &lease.executable)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeSystem {
        current_owner: ProcessOwner,
        current_pid: u32,
        process: Option<ProcessCheckInfo>,
        graceful_outcome: GracefulStopOutcome,
        /// Per-call override sequence for `get_process_info`. Empty falls back
        /// to `process`, so tests can model a PID recycled between calls.
        responses: Mutex<VecDeque<Option<ProcessCheckInfo>>>,
        signaled: Mutex<Vec<u32>>,
        forced: Mutex<Vec<u32>>,
    }

    impl TerminationSystem for FakeSystem {
        fn current_owner(&self) -> ProcessOwner {
            self.current_owner.clone()
        }

        fn current_pid(&self) -> u32 {
            self.current_pid
        }

        fn get_process_info(&self, pid: u32) -> Option<ProcessCheckInfo> {
            if let Some(next) = self.responses.lock().unwrap().pop_front() {
                return next.filter(|p| p.pid == pid);
            }
            self.process.clone().filter(|p| p.pid == pid)
        }

        fn is_terminal_or_protected(&self, info: &ProcessCheckInfo) -> bool {
            info.name == "Terminal" || info.name == "zsh"
        }

        fn request_graceful_stop(&self, pid: u32) -> Result<GracefulStopOutcome, String> {
            self.signaled.lock().unwrap().push(pid);
            Ok(self.graceful_outcome)
        }

        fn force_terminate(&self, pid: u32) -> Result<(), String> {
            self.forced.lock().unwrap().push(pid);
            Ok(())
        }
    }

    fn test_lease(pid: u32, start_time: u64, exe: &str, cwd: Option<&str>) -> StopLease {
        StopLease {
            lease_id: "test-lease".into(),
            session_id: "test-session".into(),
            pid,
            start_time,
            executable: PathBuf::from(exe),
            cwd: cwd.map(PathBuf::from),
            owner: ProcessOwner::Unix(501),
            expires_at: 1000,
        }
    }

    #[cfg(unix)]
    #[test]
    fn succeeds_on_exact_matching_eligible_process() {
        let system = FakeSystem {
            current_owner: ProcessOwner::Unix(501),
            current_pid: 100,
            process: Some(ProcessCheckInfo {
                pid: 42,
                owner: ProcessOwner::Unix(501),
                start_time: 200,
                executable: Some(PathBuf::from("/usr/local/bin/claude")),
                cmd: Vec::new(),
                cwd: Some(PathBuf::from("/workspace/repo")),
                parent_pid: Some(10),
                name: "claude".into(),
            }),
            graceful_outcome: GracefulStopOutcome::Delivered,
            responses: Mutex::new(VecDeque::new()),
            signaled: Mutex::new(vec![]),
            forced: Mutex::new(vec![]),
        };

        let lease = test_lease(42, 200, "/usr/local/bin/claude", Some("/workspace/repo"));
        let res = execute_graceful_stop(&lease, &system);
        assert!(res.is_ok());
        assert_eq!(*system.signaled.lock().unwrap(), vec![42]);
    }

    #[test]
    fn rejects_pid_reuse_when_start_time_differs() {
        let system = FakeSystem {
            current_owner: ProcessOwner::Unix(501),
            current_pid: 100,
            process: Some(ProcessCheckInfo {
                pid: 42,
                owner: ProcessOwner::Unix(501),
                start_time: 250, // Different start time!
                executable: Some(PathBuf::from("/usr/local/bin/claude")),
                cmd: Vec::new(),
                cwd: Some(PathBuf::from("/workspace/repo")),
                parent_pid: Some(10),
                name: "claude".into(),
            }),
            graceful_outcome: GracefulStopOutcome::Delivered,
            responses: Mutex::new(VecDeque::new()),
            signaled: Mutex::new(vec![]),
            forced: Mutex::new(vec![]),
        };

        let lease = test_lease(42, 200, "/usr/local/bin/claude", Some("/workspace/repo"));
        let res = execute_graceful_stop(&lease, &system);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("PID reuse"));
        assert!(system.signaled.lock().unwrap().is_empty());
    }

    #[test]
    fn rejects_other_user_process() {
        let system = FakeSystem {
            current_owner: ProcessOwner::Unix(501),
            current_pid: 100,
            process: Some(ProcessCheckInfo {
                pid: 42,
                owner: ProcessOwner::Unix(502), // Other user
                start_time: 200,
                executable: Some(PathBuf::from("/usr/local/bin/claude")),
                cmd: Vec::new(),
                cwd: Some(PathBuf::from("/workspace/repo")),
                parent_pid: Some(10),
                name: "claude".into(),
            }),
            graceful_outcome: GracefulStopOutcome::Delivered,
            responses: Mutex::new(VecDeque::new()),
            signaled: Mutex::new(vec![]),
            forced: Mutex::new(vec![]),
        };

        let lease = test_lease(42, 200, "/usr/local/bin/claude", Some("/workspace/repo"));
        let res = execute_graceful_stop(&lease, &system);
        assert!(res.is_err());
        let message = res.unwrap_err();
        assert!(
            message.contains("owner mismatch") || message.contains("another user"),
            "unexpected error: {message}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn protects_terminal_ancestry() {
        let system = FakeSystem {
            current_owner: ProcessOwner::Unix(501),
            current_pid: 100,
            process: Some(ProcessCheckInfo {
                pid: 42,
                owner: ProcessOwner::Unix(501),
                start_time: 200,
                executable: Some(PathBuf::from("/usr/local/bin/claude")),
                cmd: Vec::new(),
                cwd: Some(PathBuf::from("/workspace/repo")),
                parent_pid: Some(10),
                name: "Terminal".into(), // Terminal name!
            }),
            graceful_outcome: GracefulStopOutcome::Delivered,
            responses: Mutex::new(VecDeque::new()),
            signaled: Mutex::new(vec![]),
            forced: Mutex::new(vec![]),
        };

        let lease = test_lease(42, 200, "/usr/local/bin/claude", Some("/workspace/repo"));
        let res = execute_graceful_stop(&lease, &system);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("protected"));
    }

    #[test]
    fn lease_store_expiry_and_one_shot_consumption() {
        let mut store = StopLeaseStore::default();
        let lease_id = store.create_lease(
            "session-1",
            42,
            100,
            PathBuf::from("/usr/local/bin/claude"),
            None,
            ProcessOwner::Unix(501),
            10,
        );

        // One-shot consumption
        let consumed = store.consume_lease("session-1", &lease_id, 15);
        assert!(consumed.is_ok());

        // Consumed already, second attempt fails
        let second = store.consume_lease("session-1", &lease_id, 16);
        assert!(second.is_err());

        // Expired lease fails
        let lease_id2 = store.create_lease(
            "session-2",
            43,
            100,
            PathBuf::from("/usr/local/bin/claude"),
            None,
            ProcessOwner::Unix(501),
            10,
        );
        let expired = store.consume_lease("session-2", &lease_id2, 10 + LEASE_TTL_SECS + 5);
        assert!(expired.is_err());
    }

    #[test]
    fn invalid_token_does_not_consume_the_valid_lease() {
        let mut store = StopLeaseStore::default();
        let lease_id = store.create_lease(
            "session-1",
            42,
            100,
            PathBuf::from("/usr/local/bin/claude"),
            None,
            ProcessOwner::Unix(501),
            10,
        );

        assert!(store.consume_lease("session-1", "wrong-token", 15).is_err());
        assert!(store.consume_lease("session-1", &lease_id, 16).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_when_a_leased_cwd_becomes_unavailable() {
        let system = FakeSystem {
            current_owner: ProcessOwner::Unix(501),
            current_pid: 100,
            process: Some(ProcessCheckInfo {
                pid: 42,
                owner: ProcessOwner::Unix(501),
                start_time: 200,
                executable: Some(PathBuf::from("/usr/local/bin/claude")),
                cmd: Vec::new(),
                cwd: None,
                parent_pid: Some(10),
                name: "claude".into(),
            }),
            graceful_outcome: GracefulStopOutcome::Delivered,
            responses: Mutex::new(VecDeque::new()),
            signaled: Mutex::new(vec![]),
            forced: Mutex::new(vec![]),
        };
        let lease = test_lease(42, 200, "/usr/local/bin/claude", Some("/workspace/repo"));

        let error = execute_graceful_stop(&lease, &system).unwrap_err();
        assert!(error.contains("working directory"));
        assert!(system.signaled.lock().unwrap().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn windows_graceful_timeout_forces_only_after_identity_recheck() {
        let system = FakeSystem {
            current_owner: ProcessOwner::Unix(501),
            current_pid: 100,
            process: Some(ProcessCheckInfo {
                pid: 42,
                owner: ProcessOwner::Unix(501),
                start_time: 200,
                executable: Some(PathBuf::from("/usr/local/bin/claude")),
                cmd: Vec::new(),
                cwd: Some(PathBuf::from("/workspace/repo")),
                parent_pid: Some(10),
                name: "claude".into(),
            }),
            graceful_outcome: GracefulStopOutcome::Delivered,
            responses: Mutex::new(VecDeque::new()),
            signaled: Mutex::new(vec![]),
            forced: Mutex::new(vec![]),
        };
        let lease = test_lease(42, 200, "/usr/local/bin/claude", Some("/workspace/repo"));

        // The process survives the whole bounded wait, so the same verified
        // identity is force-terminated.
        assert!(execute_graceful_stop(&lease, &system).is_ok());
        assert_eq!(*system.signaled.lock().unwrap(), vec![42]);
        assert_eq!(*system.forced.lock().unwrap(), vec![42]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_graceful_unavailable_never_forces_a_recycled_pid() {
        let info = ProcessCheckInfo {
            pid: 42,
            owner: ProcessOwner::Unix(501),
            start_time: 200,
            executable: Some(PathBuf::from("/usr/local/bin/claude")),
            cmd: Vec::new(),
            cwd: Some(PathBuf::from("/workspace/repo")),
            parent_pid: Some(10),
            name: "claude".into(),
        };
        let recycled = ProcessCheckInfo {
            start_time: 999,
            ..info.clone()
        };
        let system = FakeSystem {
            current_owner: ProcessOwner::Unix(501),
            current_pid: 100,
            process: Some(info.clone()),
            graceful_outcome: GracefulStopOutcome::Unavailable,
            // Initial validation sees the leased process; the pre-force
            // recheck sees a recycled PID.
            responses: Mutex::new(VecDeque::from([Some(info), Some(recycled)])),
            signaled: Mutex::new(vec![]),
            forced: Mutex::new(vec![]),
        };
        let lease = test_lease(42, 200, "/usr/local/bin/claude", Some("/workspace/repo"));

        assert!(execute_graceful_stop(&lease, &system).is_ok());
        assert_eq!(*system.signaled.lock().unwrap(), vec![42]);
        assert!(
            system.forced.lock().unwrap().is_empty(),
            "a recycled PID must never be force-terminated"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_graceful_unavailable_forces_once_after_identity_recheck() {
        let system = FakeSystem {
            current_owner: ProcessOwner::Unix(501),
            current_pid: 100,
            process: Some(ProcessCheckInfo {
                pid: 42,
                owner: ProcessOwner::Unix(501),
                start_time: 200,
                executable: Some(PathBuf::from("/usr/local/bin/claude")),
                cmd: Vec::new(),
                cwd: Some(PathBuf::from("/workspace/repo")),
                parent_pid: Some(10),
                name: "claude".into(),
            }),
            graceful_outcome: GracefulStopOutcome::Unavailable,
            responses: Mutex::new(VecDeque::new()),
            signaled: Mutex::new(vec![]),
            forced: Mutex::new(vec![]),
        };
        let lease = test_lease(42, 200, "/usr/local/bin/claude", Some("/workspace/repo"));

        assert!(execute_graceful_stop(&lease, &system).is_ok());
        assert_eq!(*system.forced.lock().unwrap(), vec![42]);
    }
}
