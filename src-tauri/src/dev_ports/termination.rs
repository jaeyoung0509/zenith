use super::classifier::{classify_listener, ProcessClassificationInput};
use super::discovery::{parse_lsof_output, RawListenerRecord};
use super::store::{CreateLeaseParams, DevelopmentPortStore};
use crate::models::{
    DevelopmentListener, ReleaseDevelopmentListenerResult, ReleaseMode, ReleaseOutcome,
};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSnapshot {
    pub pid: u32,
    pub uid: Option<u32>,
    pub start_time: u64,
    pub raw_command: String,
    pub process_name: String,
    pub exe_path: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub argv: Vec<String>,
}

/// Abstract system trait to enable 100% deterministic unit testing without executing real commands or killing processes.
pub trait DevPortSystem: Send + Sync {
    fn current_uid(&self) -> u32;
    fn own_pid(&self) -> u32;
    fn discover_listeners(&self) -> Result<Vec<RawListenerRecord>, String>;
    fn get_process_info(&self, pid: u32) -> Option<ProcessSnapshot>;
    fn send_signal(&self, pid: u32, signal: i32) -> Result<(), String>;
    fn sleep(&self, duration: Duration);
    fn now(&self) -> Instant;
}

pub struct RealDevPortSystem {
    sys: Mutex<Option<sysinfo::System>>,
}

impl Default for RealDevPortSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl RealDevPortSystem {
    pub fn new() -> Self {
        Self {
            sys: Mutex::new(None),
        }
    }
}

impl DevPortSystem for RealDevPortSystem {
    fn current_uid(&self) -> u32 {
        #[cfg(unix)]
        {
            unsafe { libc::getuid() }
        }
        #[cfg(not(unix))]
        {
            1000
        }
    }

    fn own_pid(&self) -> u32 {
        std::process::id()
    }

    fn discover_listeners(&self) -> Result<Vec<RawListenerRecord>, String> {
        let mut cmd = std::process::Command::new("/usr/sbin/lsof");
        cmd.args(["-nP", "-a", "-iTCP", "-sTCP:LISTEN", "-F0pcuLn"]);

        let output =
            crate::tooling::run_with_timeout(cmd, Duration::from_secs(2)).map_err(|e| {
                crate::diagnostics::log_error(
                    "dev_ports",
                    "Listener inspection timed out or failed",
                );
                format!("Listener inspection timed out or failed: {e}")
            })?;

        if !output.status.success() && output.stdout.is_empty() {
            return Ok(Vec::new());
        }

        Ok(parse_lsof_output(&output.stdout))
    }

    fn get_process_info(&self, pid: u32) -> Option<ProcessSnapshot> {
        let mut guard = self.sys.lock().expect("sysinfo lock poisoned");
        let sys = guard.get_or_insert_with(sysinfo::System::new_all);

        let sys_pid = sysinfo::Pid::from_u32(pid);
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        let process = sys.process(sys_pid)?;

        let raw_command = process.name().to_string_lossy().to_string();
        let process_name = process.name().to_string_lossy().to_string();
        let exe_path = process.exe().map(|p| p.to_path_buf());
        let cwd = process.cwd().map(|p| p.to_path_buf());
        let argv: Vec<String> = process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        let start_time = process.start_time();
        let uid = process
            .effective_user_id()
            .or_else(|| process.user_id())
            .and_then(|u| u.to_string().parse::<u32>().ok());

        Some(ProcessSnapshot {
            pid,
            uid,
            start_time,
            raw_command,
            process_name,
            exe_path,
            cwd,
            argv,
        })
    }

    fn send_signal(&self, pid: u32, signal: i32) -> Result<(), String> {
        if pid <= 1 || pid == std::process::id() {
            return Err("Cannot signal system or Zenith process".to_string());
        }

        #[cfg(unix)]
        {
            let res = unsafe { libc::kill(pid as i32, signal) };
            if res != 0 {
                let errno = std::io::Error::last_os_error();
                if errno.raw_os_error() == Some(libc::ESRCH) {
                    return Ok(()); // Process already exited
                }
                crate::diagnostics::log_error(
                    "dev_ports",
                    "Zenith could not request termination from macOS",
                );
                return Err("Zenith could not request termination from macOS.".to_string());
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = signal;
            Err("Termination is only supported on Unix systems".to_string())
        }
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Lists all current TCP listeners, classifies each process, and stores short-lived leases.
pub fn list_listeners(
    store: &Mutex<DevelopmentPortStore>,
    system: &dyn DevPortSystem,
) -> Result<Vec<DevelopmentListener>, String> {
    let raw_records = system.discover_listeners()?;
    let now = system.now();
    let current_uid = system.current_uid();
    let own_pid = system.own_pid();

    let mut listeners = Vec::new();
    let mut store_guard = store.lock().expect("store lock poisoned");
    store_guard.prune_stale(now);

    for record in raw_records {
        let proc_info = system.get_process_info(record.pid);

        let (
            server_name,
            project_name,
            working_directory,
            can_release,
            blocked_reason,
            started_at,
            exe_path,
            uid,
        ) = if let Some(ref proc) = proc_info {
            let classification = classify_listener(&ProcessClassificationInput {
                pid: record.pid,
                uid: proc.uid.or(record.uid),
                current_user_uid: current_uid,
                zenith_pid: own_pid,
                port: record.port,
                raw_command: &proc.raw_command,
                process_name: &proc.process_name,
                exe_path: proc.exe_path.as_deref(),
                cwd: proc.cwd.as_deref(),
                argv: &proc.argv,
                started_at: Some(proc.start_time),
            });
            (
                classification.server_name,
                classification.project_name,
                classification.working_directory,
                classification.can_release,
                classification.blocked_reason,
                Some(proc.start_time),
                proc.exe_path.clone(),
                proc.uid.or(record.uid).unwrap_or(current_uid),
            )
        } else {
            let classification = classify_listener(&ProcessClassificationInput {
                pid: record.pid,
                uid: record.uid,
                current_user_uid: current_uid,
                zenith_pid: own_pid,
                port: record.port,
                raw_command: &record.command,
                process_name: &record.command,
                exe_path: None,
                cwd: None,
                argv: &[],
                started_at: None,
            });
            (
                classification.server_name,
                classification.project_name,
                classification.working_directory,
                classification.can_release,
                classification.blocked_reason,
                None,
                None,
                record.uid.unwrap_or(current_uid),
            )
        };

        let lease_id = store_guard.create_lease(CreateLeaseParams {
            pid: record.pid,
            port: record.port,
            protocol: record.protocol,
            bind_address: record.bind_address.clone(),
            uid,
            started_at,
            exe_path,
            server_name: server_name.clone(),
            can_release,
            now,
        });

        listeners.push(DevelopmentListener {
            id: lease_id,
            port: record.port,
            protocol: record.protocol,
            bind_address: record.bind_address,
            exposure: record.exposure,
            pid: record.pid,
            server_name,
            project_name,
            working_directory,
            started_at,
            can_release,
            blocked_reason,
        });
    }

    // Sort order: releasable first, then ascending port
    listeners.sort_by(|a, b| {
        b.can_release
            .cmp(&a.can_release)
            .then_with(|| a.port.cmp(&b.port))
    });

    Ok(listeners)
}

/// Safely terminates a development listener after revalidating process ownership and identity.
pub fn release_listener(
    store: &Mutex<DevelopmentPortStore>,
    system: &dyn DevPortSystem,
    lease_id: &str,
    mode: ReleaseMode,
) -> Result<ReleaseDevelopmentListenerResult, String> {
    let now = system.now();

    // 1. One-shot consumption: take lease from store
    let lease = {
        let mut store_guard = store.lock().expect("store lock poisoned");
        store_guard
            .take_lease(lease_id, now)
            .ok_or_else(|| "Listener snapshot expired; refresh and try again.".to_string())?
    };

    // 2. Eligibility check
    if !lease.can_release {
        return Err("This listener is protected and cannot be released.".to_string());
    }

    // 3. TOCTOU revalidation: verify current listeners from system
    let current_listeners = system.discover_listeners()?;
    let current_listener = current_listeners
        .iter()
        .find(|l| l.port == lease.port && l.protocol == lease.protocol);

    let Some(found_listener) = current_listener else {
        // Port is already free!
        return Ok(ReleaseDevelopmentListenerResult {
            port: lease.port,
            outcome: ReleaseOutcome::Released,
            listener: None,
        });
    };

    if found_listener.pid != lease.pid {
        // Port ownership changed to another PID
        return Ok(ReleaseDevelopmentListenerResult {
            port: lease.port,
            outcome: ReleaseOutcome::OwnershipChanged,
            listener: None,
        });
    }

    // 4. Inspect current process identity
    let current_proc = system.get_process_info(lease.pid).ok_or_else(|| {
        // Process no longer exists
        "Process exited before signaling".to_string()
    });

    let proc_info = match current_proc {
        Ok(info) => info,
        Err(_) => {
            return Ok(ReleaseDevelopmentListenerResult {
                port: lease.port,
                outcome: ReleaseOutcome::Released,
                listener: None,
            });
        }
    };

    // Verify UID, start time, and executable path
    if proc_info.uid.unwrap_or(0) != lease.uid
        || proc_info.start_time != lease.started_at.unwrap_or(0)
        || proc_info.exe_path != lease.exe_path
    {
        return Ok(ReleaseDevelopmentListenerResult {
            port: lease.port,
            outcome: ReleaseOutcome::OwnershipChanged,
            listener: None,
        });
    }

    // Re-run classifier on current process info
    let current_uid = system.current_uid();
    let own_pid = system.own_pid();
    let reclassification = classify_listener(&ProcessClassificationInput {
        pid: lease.pid,
        uid: proc_info.uid,
        current_user_uid: current_uid,
        zenith_pid: own_pid,
        port: lease.port,
        raw_command: &proc_info.raw_command,
        process_name: &proc_info.process_name,
        exe_path: proc_info.exe_path.as_deref(),
        cwd: proc_info.cwd.as_deref(),
        argv: &proc_info.argv,
        started_at: Some(proc_info.start_time),
    });

    if !reclassification.can_release {
        return Err("This listener is protected and cannot be released.".to_string());
    }

    // 5. Send Signal
    let sig = match mode {
        ReleaseMode::Graceful => libc::SIGTERM,
        ReleaseMode::Force => libc::SIGKILL,
    };

    system.send_signal(lease.pid, sig)?;

    // 6. Polling grace period (up to 1.5s, 15 x 100ms)
    let poll_interval = Duration::from_millis(100);
    let max_attempts = 15;

    for _ in 0..max_attempts {
        system.sleep(poll_interval);

        let listeners = system.discover_listeners()?;
        let still_open = listeners
            .iter()
            .any(|l| l.port == lease.port && l.protocol == lease.protocol);

        if !still_open {
            return Ok(ReleaseDevelopmentListenerResult {
                port: lease.port,
                outcome: ReleaseOutcome::Released,
                listener: None,
            });
        }
    }

    // 7. Post-grace inspection
    let post_listeners = system.discover_listeners()?;
    let post_listener = post_listeners
        .iter()
        .find(|l| l.port == lease.port && l.protocol == lease.protocol);

    let Some(found_post) = post_listener else {
        return Ok(ReleaseDevelopmentListenerResult {
            port: lease.port,
            outcome: ReleaseOutcome::Released,
            listener: None,
        });
    };

    if found_post.pid == lease.pid {
        if let Some(post_proc) = system.get_process_info(lease.pid) {
            if post_proc.start_time == lease.started_at.unwrap_or(0)
                && post_proc.exe_path == lease.exe_path
                && post_proc.uid.unwrap_or(0) == lease.uid
            {
                // Same process remains listening! Create a fresh lease for possible Force action.
                let mut store_guard = store.lock().expect("store lock poisoned");
                let fresh_now = system.now();
                let new_id = store_guard.create_lease(CreateLeaseParams {
                    pid: lease.pid,
                    port: lease.port,
                    protocol: lease.protocol,
                    bind_address: lease.bind_address.clone(),
                    uid: lease.uid,
                    started_at: lease.started_at,
                    exe_path: lease.exe_path.clone(),
                    server_name: lease.server_name.clone(),
                    can_release: true,
                    now: fresh_now,
                });

                let updated_listener = DevelopmentListener {
                    id: new_id,
                    port: lease.port,
                    protocol: lease.protocol,
                    bind_address: lease.bind_address,
                    exposure: found_post.exposure,
                    pid: lease.pid,
                    server_name: lease.server_name,
                    project_name: reclassification.project_name,
                    working_directory: reclassification.working_directory,
                    started_at: lease.started_at,
                    can_release: true,
                    blocked_reason: None,
                };

                return Ok(ReleaseDevelopmentListenerResult {
                    port: lease.port,
                    outcome: ReleaseOutcome::StillListening,
                    listener: Some(updated_listener),
                });
            }
        }
    }

    // If a different PID or respawned process owns the port
    Ok(ReleaseDevelopmentListenerResult {
        port: lease.port,
        outcome: ReleaseOutcome::OwnershipChanged,
        listener: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ListenerExposure, ListenerProtocol};
    use std::sync::atomic::{AtomicBool, Ordering};

    struct FakeDevPortSystem {
        uid: u32,
        pid: u32,
        listeners: Mutex<Vec<RawListenerRecord>>,
        processes: Mutex<HashMap<u32, ProcessSnapshot>>,
        signaled_pids: Mutex<Vec<(u32, i32)>>,
        auto_exit_on_signal: AtomicBool,
    }

    use std::collections::HashMap;

    impl FakeDevPortSystem {
        fn new() -> Self {
            Self {
                uid: 501,
                pid: 1000,
                listeners: Mutex::new(Vec::new()),
                processes: Mutex::new(HashMap::new()),
                signaled_pids: Mutex::new(Vec::new()),
                auto_exit_on_signal: AtomicBool::new(true),
            }
        }

        fn add_listener(
            &self,
            pid: u32,
            port: u16,
            command: &str,
            bind_address: &str,
            exposure: ListenerExposure,
        ) {
            let mut guard = self.listeners.lock().unwrap();
            guard.push(RawListenerRecord {
                pid,
                command: command.to_string(),
                uid: Some(self.uid),
                port,
                bind_address: bind_address.to_string(),
                exposure,
                protocol: ListenerProtocol::Tcp,
            });
        }

        fn add_process(&self, snapshot: ProcessSnapshot) {
            let mut guard = self.processes.lock().unwrap();
            guard.insert(snapshot.pid, snapshot);
        }
    }

    impl DevPortSystem for FakeDevPortSystem {
        fn current_uid(&self) -> u32 {
            self.uid
        }

        fn own_pid(&self) -> u32 {
            self.pid
        }

        fn discover_listeners(&self) -> Result<Vec<RawListenerRecord>, String> {
            Ok(self.listeners.lock().unwrap().clone())
        }

        fn get_process_info(&self, pid: u32) -> Option<ProcessSnapshot> {
            self.processes.lock().unwrap().get(&pid).cloned()
        }

        fn send_signal(&self, pid: u32, signal: i32) -> Result<(), String> {
            self.signaled_pids.lock().unwrap().push((pid, signal));
            if self.auto_exit_on_signal.load(Ordering::SeqCst) {
                // Remove from listeners and processes
                self.listeners.lock().unwrap().retain(|l| l.pid != pid);
                self.processes.lock().unwrap().remove(&pid);
            }
            Ok(())
        }

        fn sleep(&self, _duration: Duration) {}

        fn now(&self) -> Instant {
            Instant::now()
        }
    }

    #[test]
    fn list_and_release_vite_graceful_success() {
        let fake = FakeDevPortSystem::new();
        fake.add_listener(32892, 5173, "node", "127.0.0.1", ListenerExposure::Loopback);
        fake.add_process(ProcessSnapshot {
            pid: 32892,
            uid: Some(501),
            start_time: 1700000000,
            raw_command: "node".to_string(),
            process_name: "node".to_string(),
            exe_path: Some(PathBuf::from("/opt/homebrew/bin/node")),
            cwd: Some(PathBuf::from("/Users/apple/Myproject/clean1")),
            argv: vec![
                "node".to_string(),
                "/Users/apple/Myproject/clean1/node_modules/vite/bin/vite.js".to_string(),
            ],
        });

        let store = Mutex::new(DevelopmentPortStore::default());
        let list = list_listeners(&store, &fake).unwrap();

        assert_eq!(list.len(), 1);
        let listener = &list[0];
        assert_eq!(listener.port, 5173);
        assert_eq!(listener.server_name, "Vite");
        assert!(listener.can_release);

        let result = release_listener(&store, &fake, &listener.id, ReleaseMode::Graceful).unwrap();

        assert_eq!(result.outcome, ReleaseOutcome::Released);
        assert_eq!(result.port, 5173);

        // Verify SIGTERM was sent to the exact PID
        let signals = fake.signaled_pids.lock().unwrap();
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0], (32892, libc::SIGTERM));
    }

    #[test]
    fn release_process_that_ignores_sigterm_returns_still_listening_with_fresh_lease() {
        let fake = FakeDevPortSystem::new();
        fake.auto_exit_on_signal.store(false, Ordering::SeqCst);

        fake.add_listener(40000, 3000, "node", "127.0.0.1", ListenerExposure::Loopback);
        fake.add_process(ProcessSnapshot {
            pid: 40000,
            uid: Some(501),
            start_time: 1700000000,
            raw_command: "node".to_string(),
            process_name: "node".to_string(),
            exe_path: Some(PathBuf::from("/opt/homebrew/bin/node")),
            cwd: Some(PathBuf::from("/Users/apple/app")),
            argv: vec![
                "node".to_string(),
                "/Users/apple/app/node_modules/.bin/next".to_string(),
                "dev".to_string(),
            ],
        });

        let store = Mutex::new(DevelopmentPortStore::default());
        let list = list_listeners(&store, &fake).unwrap();
        let listener = &list[0];

        // Graceful release attempt -> process ignores SIGTERM
        let res_graceful =
            release_listener(&store, &fake, &listener.id, ReleaseMode::Graceful).unwrap();

        assert_eq!(res_graceful.outcome, ReleaseOutcome::StillListening);
        assert!(res_graceful.listener.is_some());
        let fresh_listener = res_graceful.listener.unwrap();
        assert_ne!(fresh_listener.id, listener.id); // Fresh lease!

        // Now perform Force release on the fresh lease
        fake.auto_exit_on_signal.store(true, Ordering::SeqCst);
        let res_force =
            release_listener(&store, &fake, &fresh_listener.id, ReleaseMode::Force).unwrap();

        assert_eq!(res_force.outcome, ReleaseOutcome::Released);

        let signals = fake.signaled_pids.lock().unwrap();
        assert_eq!(signals.len(), 2);
        assert_eq!(signals[0], (40000, libc::SIGTERM));
        assert_eq!(signals[1], (40000, libc::SIGKILL));
    }

    #[test]
    fn pid_reuse_with_changed_start_time_is_rejected_as_ownership_changed() {
        let fake = FakeDevPortSystem::new();
        fake.add_listener(32892, 5173, "node", "127.0.0.1", ListenerExposure::Loopback);
        fake.add_process(ProcessSnapshot {
            pid: 32892,
            uid: Some(501),
            start_time: 1700000000,
            raw_command: "node".to_string(),
            process_name: "node".to_string(),
            exe_path: Some(PathBuf::from("/opt/homebrew/bin/node")),
            cwd: Some(PathBuf::from("/Users/apple/app")),
            argv: vec!["node".to_string(), "vite.js".to_string()],
        });

        let store = Mutex::new(DevelopmentPortStore::default());
        let list = list_listeners(&store, &fake).unwrap();
        let listener = &list[0];

        // Simulate PID reuse: PID 32892 was recycled by OS and now has start_time 1700009999
        fake.add_process(ProcessSnapshot {
            pid: 32892,
            uid: Some(501),
            start_time: 1700009999,
            raw_command: "node".to_string(),
            process_name: "node".to_string(),
            exe_path: Some(PathBuf::from("/opt/homebrew/bin/node")),
            cwd: Some(PathBuf::from("/Users/apple/app")),
            argv: vec!["node".to_string(), "vite.js".to_string()],
        });

        let result = release_listener(&store, &fake, &listener.id, ReleaseMode::Graceful).unwrap();

        assert_eq!(result.outcome, ReleaseOutcome::OwnershipChanged);
        assert!(fake.signaled_pids.lock().unwrap().is_empty());
    }

    #[test]
    fn port_handoff_to_different_pid_is_rejected_as_ownership_changed() {
        let fake = FakeDevPortSystem::new();
        fake.add_listener(11111, 5173, "node", "127.0.0.1", ListenerExposure::Loopback);
        fake.add_process(ProcessSnapshot {
            pid: 11111,
            uid: Some(501),
            start_time: 1700000000,
            raw_command: "node".to_string(),
            process_name: "node".to_string(),
            exe_path: Some(PathBuf::from("/opt/homebrew/bin/node")),
            cwd: None,
            argv: vec!["node".to_string(), "vite.js".to_string()],
        });

        let store = Mutex::new(DevelopmentPortStore::default());
        let list = list_listeners(&store, &fake).unwrap();
        let listener = &list[0];

        // Port handoff: PID 22222 is now listening on 5173
        fake.listeners.lock().unwrap().clear();
        fake.add_listener(
            22222,
            5173,
            "other",
            "127.0.0.1",
            ListenerExposure::Loopback,
        );

        let result = release_listener(&store, &fake, &listener.id, ReleaseMode::Graceful).unwrap();

        assert_eq!(result.outcome, ReleaseOutcome::OwnershipChanged);
        assert!(fake.signaled_pids.lock().unwrap().is_empty());
    }

    #[test]
    fn protected_listener_cannot_be_released() {
        let fake = FakeDevPortSystem::new();
        fake.add_listener(
            5432,
            5432,
            "postgres",
            "127.0.0.1",
            ListenerExposure::Loopback,
        );
        fake.add_process(ProcessSnapshot {
            pid: 5432,
            uid: Some(501),
            start_time: 1700000000,
            raw_command: "postgres".to_string(),
            process_name: "postgres".to_string(),
            exe_path: Some(PathBuf::from("/opt/homebrew/bin/postgres")),
            cwd: None,
            argv: vec![],
        });

        let store = Mutex::new(DevelopmentPortStore::default());
        let list = list_listeners(&store, &fake).unwrap();
        let listener = &list[0];

        assert!(!listener.can_release);

        let err = release_listener(&store, &fake, &listener.id, ReleaseMode::Graceful).unwrap_err();
        assert!(err.contains("protected"));
        assert!(fake.signaled_pids.lock().unwrap().is_empty());
    }
}
