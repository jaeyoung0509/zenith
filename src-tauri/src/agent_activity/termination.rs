use std::collections::HashMap;
use std::path::PathBuf;

pub const LEASE_TTL_SECS: u64 = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopLease {
    pub lease_id: String,
    pub session_id: String,
    pub pid: u32,
    pub start_time: u64,
    pub executable: PathBuf,
    pub cwd: Option<PathBuf>,
    pub uid: u32,
    pub expires_at: u64,
}

#[derive(Debug, Default)]
pub struct StopLeaseStore {
    leases: HashMap<String, StopLease>, // keyed by session_id
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
        uid: u32,
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
            uid,
            expires_at: now + LEASE_TTL_SECS,
        };
        self.leases.retain(|_, l| l.expires_at > now);
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
        let lease = self.leases.remove(session_id).ok_or_else(|| {
            "Stop lease expired or not found. Please refresh and try again.".to_string()
        })?;

        if lease.lease_id != lease_id {
            return Err("Invalid stop lease token.".to_string());
        }
        if now > lease.expires_at {
            return Err("Stop lease has expired.".to_string());
        }
        Ok(lease)
    }
}

#[derive(Debug, Clone)]
pub struct ProcessCheckInfo {
    pub pid: u32,
    pub uid: u32,
    pub start_time: u64,
    pub executable: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub parent_pid: Option<u32>,
    pub name: String,
}

pub trait TerminationSystem: Send + Sync {
    fn current_uid(&self) -> u32;
    fn current_pid(&self) -> u32;
    fn get_process_info(&self, pid: u32) -> Option<ProcessCheckInfo>;
    fn is_terminal_or_protected(&self, info: &ProcessCheckInfo) -> bool;
    fn send_sigterm(&self, pid: u32) -> Result<(), String>;
}

pub struct RealTerminationSystem;

impl TerminationSystem for RealTerminationSystem {
    fn current_uid(&self) -> u32 {
        #[cfg(unix)]
        unsafe {
            libc::geteuid()
        }
        #[cfg(not(unix))]
        {
            0
        }
    }

    fn current_pid(&self) -> u32 {
        std::process::id()
    }

    fn get_process_info(&self, pid: u32) -> Option<ProcessCheckInfo> {
        let mut sys = sysinfo::System::new();
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
            true,
            sysinfo::ProcessRefreshKind::everything(),
        );
        let process = sys.process(sysinfo::Pid::from_u32(pid))?;
        let uid = process
            .effective_user_id()
            .or_else(|| process.user_id())
            .and_then(|u| u.to_string().parse().ok())?;
        Some(ProcessCheckInfo {
            pid,
            uid,
            start_time: process.start_time(),
            executable: process.exe().map(PathBuf::from),
            cwd: process.cwd().map(PathBuf::from),
            parent_pid: process.parent().map(|p| p.as_u32()),
            name: process.name().to_string_lossy().to_string(),
        })
    }

    fn is_terminal_or_protected(&self, info: &ProcessCheckInfo) -> bool {
        const PROTECTED_NAMES: &[&str] = &[
            "Terminal",
            "iTerm2",
            "ghostty",
            "alacritty",
            "kitty",
            "warp",
            "wezterm",
            "login",
            "launchd",
            "systemd",
            "zsh",
            "bash",
            "fish",
            "sh",
        ];
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

    fn send_sigterm(&self, pid: u32) -> Result<(), String> {
        #[cfg(unix)]
        unsafe {
            if libc::kill(pid as i32, libc::SIGTERM) == 0 {
                Ok(())
            } else {
                Err(format!(
                    "Failed to send SIGTERM: {}",
                    std::io::Error::last_os_error()
                ))
            }
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            Err("Graceful stop is only supported on Unix systems.".to_string())
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

    // 3. UID check
    if info.uid != system.current_uid() || info.uid != lease.uid {
        return Err("Process UID mismatch or process belongs to another user.".to_string());
    }

    // 4. Start time check (CRITICAL TOCTOU PID-reuse prevention!)
    if info.start_time != lease.start_time {
        return Err("Process identity drift detected: start time changed (PID reuse).".to_string());
    }

    // 5. Executable identity match
    let Some(current_exe) = &info.executable else {
        return Err("Cannot determine process executable path.".to_string());
    };
    if current_exe != &lease.executable {
        return Err("Process executable path drift detected.".to_string());
    }

    // 6. Check if adapter allows termination
    if crate::agent_activity::adapters::adapter_for_executable(current_exe).is_none() {
        return Err("Process is not an allowlisted agent CLI.".to_string());
    }

    // 7. Cwd check if present
    if let (Some(lease_cwd), Some(current_cwd)) = (&lease.cwd, &info.cwd) {
        if lease_cwd != current_cwd {
            return Err("Process working directory has changed.".to_string());
        }
    }

    // 8. Terminal and protected ancestry check
    if system.is_terminal_or_protected(&info) {
        return Err(
            "Target process or its parent is a protected terminal or system process.".to_string(),
        );
    }

    // 9. Send SIGTERM only (never SIGKILL, never process group)
    system.send_sigterm(lease.pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeSystem {
        current_uid: u32,
        current_pid: u32,
        process: Option<ProcessCheckInfo>,
        signaled: Mutex<Vec<u32>>,
    }

    impl TerminationSystem for FakeSystem {
        fn current_uid(&self) -> u32 {
            self.current_uid
        }

        fn current_pid(&self) -> u32 {
            self.current_pid
        }

        fn get_process_info(&self, pid: u32) -> Option<ProcessCheckInfo> {
            self.process.clone().filter(|p| p.pid == pid)
        }

        fn is_terminal_or_protected(&self, info: &ProcessCheckInfo) -> bool {
            info.name == "Terminal" || info.name == "zsh"
        }

        fn send_sigterm(&self, pid: u32) -> Result<(), String> {
            self.signaled.lock().unwrap().push(pid);
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
            uid: 501,
            expires_at: 1000,
        }
    }

    #[test]
    fn succeeds_on_exact_matching_eligible_process() {
        let system = FakeSystem {
            current_uid: 501,
            current_pid: 100,
            process: Some(ProcessCheckInfo {
                pid: 42,
                uid: 501,
                start_time: 200,
                executable: Some(PathBuf::from("/usr/local/bin/claude")),
                cwd: Some(PathBuf::from("/workspace/repo")),
                parent_pid: Some(10),
                name: "claude".into(),
            }),
            signaled: Mutex::new(vec![]),
        };

        let lease = test_lease(42, 200, "/usr/local/bin/claude", Some("/workspace/repo"));
        let res = execute_graceful_stop(&lease, &system);
        assert!(res.is_ok());
        assert_eq!(*system.signaled.lock().unwrap(), vec![42]);
    }

    #[test]
    fn rejects_pid_reuse_when_start_time_differs() {
        let system = FakeSystem {
            current_uid: 501,
            current_pid: 100,
            process: Some(ProcessCheckInfo {
                pid: 42,
                uid: 501,
                start_time: 250, // Different start time!
                executable: Some(PathBuf::from("/usr/local/bin/claude")),
                cwd: Some(PathBuf::from("/workspace/repo")),
                parent_pid: Some(10),
                name: "claude".into(),
            }),
            signaled: Mutex::new(vec![]),
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
            current_uid: 501,
            current_pid: 100,
            process: Some(ProcessCheckInfo {
                pid: 42,
                uid: 502, // Other user
                start_time: 200,
                executable: Some(PathBuf::from("/usr/local/bin/claude")),
                cwd: Some(PathBuf::from("/workspace/repo")),
                parent_pid: Some(10),
                name: "claude".into(),
            }),
            signaled: Mutex::new(vec![]),
        };

        let lease = test_lease(42, 200, "/usr/local/bin/claude", Some("/workspace/repo"));
        let res = execute_graceful_stop(&lease, &system);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("UID mismatch"));
    }

    #[test]
    fn protects_terminal_ancestry() {
        let system = FakeSystem {
            current_uid: 501,
            current_pid: 100,
            process: Some(ProcessCheckInfo {
                pid: 42,
                uid: 501,
                start_time: 200,
                executable: Some(PathBuf::from("/usr/local/bin/claude")),
                cwd: Some(PathBuf::from("/workspace/repo")),
                parent_pid: Some(10),
                name: "Terminal".into(), // Terminal name!
            }),
            signaled: Mutex::new(vec![]),
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
            501,
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
            501,
            10,
        );
        let expired = store.consume_lease("session-2", &lease_id2, 10 + LEASE_TTL_SECS + 5);
        assert!(expired.is_err());
    }
}
