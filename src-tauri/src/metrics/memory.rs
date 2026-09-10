use crate::collection::{Admission, Inflight, SingleFlight};
use crate::metrics::memory_termination::{
    CreateMemoryLeaseParams, MemoryLeaseMember, MemoryTerminationStore,
};
use crate::models::{
    MemoryMetrics, MemoryPressure, MemoryTerminationMode, MemoryTerminationOutcome,
    MemoryTerminationResult, ProcessMemory,
};
use crate::process_owner::ProcessOwner;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use sysinfo::{ProcessesToUpdate, Signal, System};

/// Internal snapshot of memory metrics and verified termination candidates.
/// Captured in a single OS observation pass and not exposed directly over IPC.
#[derive(Clone, Debug)]
pub struct MemoryObservation {
    pub metrics: MemoryMetrics,
    pub termination_candidates: HashMap<String, Vec<MemoryLeaseMember>>,
    pub captured_at: Instant,
}

impl MemoryObservation {
    /// Mints fresh, verified one-shot termination leases into the given store
    /// and returns an enriched copy of the metrics.
    pub fn mint_metrics_with_leases(&self, store: &Mutex<MemoryTerminationStore>) -> MemoryMetrics {
        let mut metrics = self.metrics.clone();
        MemoryInspector::attach_termination_leases(
            &mut metrics,
            store,
            &self.termination_candidates,
        );
        metrics
    }
}

pub struct MemorySampler {
    system: Mutex<Option<System>>,
    compressed_cache: Mutex<Option<(Instant, u64)>>,
    process_refresh_counter: AtomicU64,
    cached_observation: Mutex<Option<Arc<MemoryObservation>>>,
    singleflight: SingleFlight<Arc<MemoryObservation>, ()>,
}

/// Owns observation collection independently from the IPC request that admitted it.
/// A window closing or cancelling its request must not strand compatible waiters or
/// leave the single-flight entry occupied until its TTL expires.
fn supervise_memory_observation(
    sampler: Arc<MemorySampler>,
    entry: Arc<Inflight<Arc<MemoryObservation>, ()>>,
    collect: impl FnOnce() -> MemoryObservation + Send + 'static,
) {
    tauri::async_runtime::spawn(async move {
        let result = tauri::async_runtime::spawn_blocking(move || Arc::new(collect()))
            .await
            .map_err(|error| format!("Memory observation worker panicked: {error}"));

        if let Ok(observation) = &result {
            *sampler
                .cached_observation
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(observation.clone());
        }

        sampler.singleflight.complete(&entry, result).await;
    });
}

impl Default for MemorySampler {
    fn default() -> Self {
        Self::new()
    }
}

impl MemorySampler {
    pub fn new() -> Self {
        Self {
            system: Mutex::new(None),
            compressed_cache: Mutex::new(None),
            process_refresh_counter: AtomicU64::new(0),
            cached_observation: Mutex::new(None),
            singleflight: SingleFlight::default(),
        }
    }

    /// Number of full OS process table refreshes performed by this sampler.
    pub fn process_refresh_count(&self) -> u64 {
        self.process_refresh_counter.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(crate) fn is_initialized(&self) -> bool {
        self.system
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    /// Captures current system memory metrics and top resource-consuming developer processes.
    pub fn sample(&self) -> MemoryMetrics {
        self.observe().metrics
    }

    /// Single-pass observation: performs exactly one full process table refresh to collect
    /// both aggregate memory groups and verified termination candidate identities.
    pub fn observe(&self) -> MemoryObservation {
        let (
            total_bytes,
            used_bytes,
            available_bytes,
            free_bytes,
            swap_total_bytes,
            swap_used_bytes,
            top_processes,
            termination_candidates,
        ) = {
            let mut guard = self
                .system
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let sys = guard.get_or_insert_with(System::new_all);
            sys.refresh_memory();

            let total_bytes = sys.total_memory();
            let used_bytes = sys.used_memory();
            let free_bytes = sys.free_memory();
            let available_bytes = sys.available_memory();
            let swap_total_bytes = sys.total_swap();
            let swap_used_bytes = sys.used_swap();

            // Refresh processes exactly once for this observation pass
            sys.refresh_processes(ProcessesToUpdate::All, true);
            self.process_refresh_counter.fetch_add(1, Ordering::SeqCst);

            let own_pid = std::process::id();
            let own_uid = sys
                .process(sysinfo::Pid::from_u32(own_pid))
                .and_then(|process| process.effective_user_id().or_else(|| process.user_id()));
            let current_owner = ProcessOwner::current();

            let mut process_groups: HashMap<String, (u64, usize, u32, Vec<u32>, bool)> =
                HashMap::new();
            let mut termination_candidates: HashMap<String, Vec<MemoryLeaseMember>> =
                HashMap::new();

            for (pid, process) in sys.processes() {
                let pid_u32 = pid.as_u32();
                let raw_name = process.name().to_string_lossy();
                let norm_name = MemoryInspector::normalize_process_name(&raw_name, process.exe());
                let mem = process.memory();
                let can_terminate =
                    MemoryInspector::can_terminate_process(&norm_name, process.exe());

                let entry = process_groups
                    .entry(norm_name.clone())
                    .or_insert_with(|| (0, 0, pid_u32, Vec::new(), false));
                entry.0 += mem;
                entry.1 += 1;
                entry.3.push(pid_u32);
                entry.4 |= can_terminate;

                // Collect candidate members in this same pass
                if pid_u32 <= 1 || pid_u32 == own_pid {
                    continue;
                }
                if !can_terminate {
                    continue;
                }
                if crate::process_protection::is_protected_process(
                    &norm_name,
                    Some(&raw_name),
                    process.exe(),
                ) {
                    continue;
                }
                let Some(owner) = ProcessOwner::verified(
                    process.effective_user_id().or_else(|| process.user_id()),
                    own_uid,
                ) else {
                    continue;
                };
                if owner != current_owner {
                    continue;
                }
                if process.start_time() == 0 {
                    continue;
                }

                termination_candidates
                    .entry(norm_name.clone())
                    .or_default()
                    .push(MemoryLeaseMember {
                        pid: pid_u32,
                        owner,
                        start_time: process.start_time(),
                        exe: process.exe().map(PathBuf::from),
                        group: norm_name,
                    });
            }

            for members in termination_candidates.values_mut() {
                members.sort_by_key(|member| member.pid);
            }

            let mut top_processes: Vec<ProcessMemory> = process_groups
                .into_iter()
                .map(
                    |(name, (memory_bytes, process_count, _first_pid, mut pids, can_terminate))| {
                        pids.sort_unstable();
                        let representative_pid = pids.first().copied().unwrap_or(0);
                        ProcessMemory {
                            pid: representative_pid,
                            pids,
                            can_terminate,
                            name,
                            memory_bytes,
                            process_count,
                            termination_lease_id: None,
                        }
                    },
                )
                .collect();

            top_processes.sort_by_key(|process| std::cmp::Reverse(process.memory_bytes));
            top_processes.truncate(15);

            (
                total_bytes,
                used_bytes,
                available_bytes,
                free_bytes,
                swap_total_bytes,
                swap_used_bytes,
                top_processes,
                termination_candidates,
            )
        };

        // System lock is released before calculating compressed memory
        let compressed_bytes = self.compressed_memory();

        // Calculate memory pressure
        let used_ratio = if total_bytes > 0 {
            used_bytes as f64 / total_bytes as f64
        } else {
            0.0
        };

        let pressure = if used_ratio > 0.88
            || (swap_total_bytes > 0 && (swap_used_bytes as f64 / swap_total_bytes as f64) > 0.6)
        {
            MemoryPressure::Critical
        } else if used_ratio > 0.75 || swap_used_bytes > 1024 * 1024 * 1024 {
            MemoryPressure::Warning
        } else {
            MemoryPressure::Normal
        };

        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        MemoryObservation {
            metrics: MemoryMetrics {
                total_bytes,
                used_bytes,
                available_bytes,
                free_bytes,
                compressed_bytes,
                swap_used_bytes,
                swap_total_bytes,
                pressure,
                top_processes,
                timestamp,
            },
            termination_candidates,
            captured_at: Instant::now(),
        }
    }

    /// Retrieves an observation, reusing a cached observation if within `max_age`
    /// or coalescing concurrent requests across windows with SingleFlight.
    pub async fn get_observation(
        self: &Arc<Self>,
        max_age: Duration,
    ) -> Result<Arc<MemoryObservation>, String> {
        if let Some(observation) = self.cached_observation(max_age) {
            return Ok(observation);
        }

        match self
            .singleflight
            .admit("memory_observation".to_string(), 0)
            .await
        {
            Admission::Own(entry) => {
                // Another collection may have populated the cache after this
                // request checked it but before it acquired admission.
                if let Some(observation) = self.cached_observation(max_age) {
                    self.singleflight
                        .complete(&entry, Ok(observation.clone()))
                        .await;
                    return Ok(observation);
                }
                let worker_sampler = self.clone();
                supervise_memory_observation(self.clone(), entry.clone(), move || {
                    worker_sampler.observe()
                });
                self.singleflight.wait(&entry).await
            }
            Admission::Wait(entry) => self.singleflight.wait(&entry).await,
        }
    }

    fn cached_observation(&self, max_age: Duration) -> Option<Arc<MemoryObservation>> {
        self.cached_observation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .filter(|observation| observation.captured_at.elapsed() < max_age)
            .cloned()
    }

    fn compressed_memory(&self) -> u64 {
        const CACHE_TTL: Duration = Duration::from_secs(8);
        let now = Instant::now();

        {
            let cache = self
                .compressed_cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some((cached_at, value)) = *cache {
                if now.duration_since(cached_at) < CACHE_TTL {
                    return value;
                }
            }
        }

        // Run subprocess outside the mutex
        let value = MemoryInspector::get_compressed_memory_macos().unwrap_or(0);
        *self
            .compressed_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((now, value));
        value
    }
}

/// Abstraction over the OS process table so termination can be tested
/// deterministically without signaling real processes.
pub trait MemoryTerminationSystem: Send + Sync {
    fn current_owner(&self) -> ProcessOwner;
    fn own_pid(&self) -> u32;
    fn group_members(&self, group: &str) -> Vec<MemoryLeaseMember>;
    fn lookup(&self, pid: u32) -> Option<MemoryLeaseMember>;
    fn signal(&self, pid: u32, mode: MemoryTerminationMode) -> Result<(), String>;
    fn sleep(&self, duration: Duration);
}

pub struct RealMemorySystem {
    sys: Mutex<Option<System>>,
}

impl Default for RealMemorySystem {
    fn default() -> Self {
        Self::new()
    }
}

impl RealMemorySystem {
    pub fn new() -> Self {
        Self {
            sys: Mutex::new(None),
        }
    }

    fn with_system<T>(&self, f: impl FnOnce(&mut System) -> T) -> T {
        let mut guard = self
            .sys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let sys = guard.get_or_insert_with(System::new);
        f(sys)
    }
}

impl MemoryTerminationSystem for RealMemorySystem {
    fn current_owner(&self) -> ProcessOwner {
        ProcessOwner::current()
    }

    fn own_pid(&self) -> u32 {
        std::process::id()
    }

    fn group_members(&self, group: &str) -> Vec<MemoryLeaseMember> {
        self.with_system(|sys| {
            sys.refresh_processes(ProcessesToUpdate::All, true);
            let own_pid = std::process::id();
            let own_uid = sys
                .process(sysinfo::Pid::from_u32(own_pid))
                .and_then(|process| process.effective_user_id().or_else(|| process.user_id()));
            let current_owner = ProcessOwner::current();
            let mut members = Vec::new();
            for (pid, process) in sys.processes() {
                let pid_u32 = pid.as_u32();
                if pid_u32 <= 1 || pid_u32 == own_pid {
                    continue;
                }
                let raw_name = process.name().to_string_lossy();
                let norm_name = MemoryInspector::normalize_process_name(&raw_name, process.exe());
                if norm_name != group {
                    continue;
                }
                if !MemoryInspector::can_terminate_process(&norm_name, process.exe()) {
                    continue;
                }
                if crate::process_protection::is_protected_process(
                    &norm_name,
                    Some(&raw_name),
                    process.exe(),
                ) {
                    continue;
                }
                let Some(owner) = ProcessOwner::verified(
                    process.effective_user_id().or_else(|| process.user_id()),
                    own_uid,
                ) else {
                    continue;
                };
                if owner != current_owner {
                    continue;
                }
                if process.start_time() == 0 {
                    continue;
                }
                members.push(MemoryLeaseMember {
                    pid: pid_u32,
                    owner,
                    start_time: process.start_time(),
                    exe: process.exe().map(PathBuf::from),
                    group: norm_name,
                });
            }
            members.sort_by_key(|member| member.pid);
            members
        })
    }

    fn lookup(&self, pid: u32) -> Option<MemoryLeaseMember> {
        self.with_system(|sys| {
            let sys_pid = sysinfo::Pid::from_u32(pid);
            let own_pid = sysinfo::Pid::from_u32(std::process::id());
            sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[sys_pid, own_pid]),
                true,
                sysinfo::ProcessRefreshKind::everything(),
            );
            let own_uid = sys
                .process(own_pid)
                .and_then(|process| process.effective_user_id().or_else(|| process.user_id()))
                .cloned();
            let own_uid_ref = own_uid.as_ref();
            let process = sys.process(sys_pid)?;
            let raw_name = process.name().to_string_lossy();
            let group = MemoryInspector::normalize_process_name(&raw_name, process.exe());
            let owner = ProcessOwner::verified(
                process.effective_user_id().or_else(|| process.user_id()),
                own_uid_ref,
            )?;
            Some(MemoryLeaseMember {
                pid,
                owner,
                start_time: process.start_time(),
                exe: process.exe().map(PathBuf::from),
                group,
            })
        })
    }

    fn signal(&self, pid: u32, mode: MemoryTerminationMode) -> Result<(), String> {
        if pid <= 1 || pid == std::process::id() {
            return Err("Cannot terminate system or Zenith process.".to_string());
        }
        match mode {
            MemoryTerminationMode::Graceful => {
                #[cfg(unix)]
                {
                    let delivered = self.with_system(|sys| {
                        let sys_pid = sysinfo::Pid::from_u32(pid);
                        sys.refresh_processes_specifics(
                            ProcessesToUpdate::Some(&[sys_pid]),
                            true,
                            sysinfo::ProcessRefreshKind::everything(),
                        );
                        sys.process(sys_pid)
                            .and_then(|process| process.kill_with(Signal::Term))
                            .unwrap_or(false)
                    });
                    if delivered {
                        Ok(())
                    } else {
                        Err(
                            "The operating system did not allow Zenith to terminate the process"
                                .to_string(),
                        )
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = pid;
                    Err("Graceful termination is unavailable on Windows. Use force termination after an unsuccessful graceful attempt on a supported platform.".to_string())
                }
            }
            MemoryTerminationMode::Force => {
                let delivered = self.with_system(|sys| {
                    let sys_pid = sysinfo::Pid::from_u32(pid);
                    sys.refresh_processes_specifics(
                        ProcessesToUpdate::Some(&[sys_pid]),
                        true,
                        sysinfo::ProcessRefreshKind::everything(),
                    );
                    sys.process(sys_pid)
                        .and_then(|process| process.kill_with(Signal::Kill))
                        .unwrap_or(false)
                });
                if delivered {
                    Ok(())
                } else {
                    Err(
                        "The operating system did not allow Zenith to terminate the process"
                            .to_string(),
                    )
                }
            }
        }
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

pub struct MemoryInspector;

impl MemoryInspector {
    /// Captures current system memory metrics and top resource-consuming developer processes.
    pub fn get_metrics() -> MemoryMetrics {
        MemorySampler::new().sample()
    }

    /// Attaches short-lived backend-owned termination leases to terminable groups
    /// from pre-collected candidate identities (single-pass observation).
    pub fn attach_termination_leases(
        metrics: &mut MemoryMetrics,
        store: &Mutex<MemoryTerminationStore>,
        candidates: &HashMap<String, Vec<MemoryLeaseMember>>,
    ) {
        let now = Instant::now();
        let mut guard = store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for process in &mut metrics.top_processes {
            if !process.can_terminate {
                process.termination_lease_id = None;
                continue;
            }
            let Some(members) = candidates.get(&process.name) else {
                process.termination_lease_id = None;
                continue;
            };
            if members.is_empty() {
                process.termination_lease_id = None;
                continue;
            }
            let lease_id = guard.create_lease(CreateMemoryLeaseParams {
                group: process.name.clone(),
                members: members.clone(),
                can_terminate: true,
                // Windows has no safe generic graceful adapter. Its UI presents
                // the only supported action explicitly as Force Quit, while the
                // backend still requires this short-lived verified lease.
                force_authorized: cfg!(target_os = "windows"),
                now,
            });
            process.termination_lease_id = Some(lease_id);
        }
    }

    /// Attaches short-lived backend-owned termination leases by querying a termination system (e.g. for testing).
    pub fn attach_termination_leases_with_system(
        metrics: &mut MemoryMetrics,
        store: &Mutex<MemoryTerminationStore>,
        system: &dyn MemoryTerminationSystem,
    ) {
        let mut candidates = HashMap::new();
        for process in &metrics.top_processes {
            if process.can_terminate {
                let members = system.group_members(&process.name);
                if !members.is_empty() {
                    candidates.insert(process.name.clone(), members);
                }
            }
        }
        Self::attach_termination_leases(metrics, store, &candidates);
    }

    /// Consumes a backend-owned lease and terminates its members after a fresh
    /// snapshot revalidates PID, owner, start time, executable, eligibility,
    /// and protected status. Sends no signal when any member drifts.
    pub fn execute_termination(
        lease_id: &str,
        mode: MemoryTerminationMode,
        store: &Mutex<MemoryTerminationStore>,
        system: &dyn MemoryTerminationSystem,
    ) -> Result<MemoryTerminationResult, String> {
        // Windows never labels TerminateProcess as graceful.
        #[cfg(target_os = "windows")]
        if mode == MemoryTerminationMode::Graceful {
            return Err(
                "Graceful termination is unavailable on Windows for this process type.".to_string(),
            );
        }
        #[cfg(not(unix))]
        #[cfg(not(target_os = "windows"))]
        if mode == MemoryTerminationMode::Graceful {
            return Err("Graceful termination is only supported on Unix systems.".to_string());
        }

        let now = Instant::now();
        let lease = {
            let mut guard = store
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard
                .take_lease(lease_id, now)
                .ok_or_else(|| "Termination snapshot expired; refresh and try again.".to_string())?
        };

        if !lease.can_terminate {
            return Err("This process group is protected and cannot be terminated.".to_string());
        }
        if mode == MemoryTerminationMode::Force && !lease.force_authorized {
            return Err("Force termination requires a fresh failed graceful attempt.".to_string());
        }
        if lease.members.is_empty() {
            return Ok(MemoryTerminationResult {
                terminated_count: 0,
                outcome: MemoryTerminationOutcome::Released,
                fresh_lease_id: None,
            });
        }

        // Fresh revalidation of every member before signaling.
        let current_owner = system.current_owner();
        let own_pid = system.own_pid();
        for member in &lease.members {
            if member.pid <= 1 || member.pid == own_pid {
                return Err("Cannot terminate system or Zenith process.".to_string());
            }
            let Some(current) = system.lookup(member.pid) else {
                // Process already exited; treat as released only when every
                // member is gone (checked after the loop).
                continue;
            };
            if current.owner != member.owner || current.owner != current_owner {
                return Ok(MemoryTerminationResult {
                    terminated_count: 0,
                    outcome: MemoryTerminationOutcome::OwnershipChanged,
                    fresh_lease_id: None,
                });
            }
            if current.start_time != member.start_time {
                return Ok(MemoryTerminationResult {
                    terminated_count: 0,
                    outcome: MemoryTerminationOutcome::OwnershipChanged,
                    fresh_lease_id: None,
                });
            }
            if current.exe != member.exe {
                return Ok(MemoryTerminationResult {
                    terminated_count: 0,
                    outcome: MemoryTerminationOutcome::OwnershipChanged,
                    fresh_lease_id: None,
                });
            }
            if current.group != member.group || current.group != lease.group {
                return Ok(MemoryTerminationResult {
                    terminated_count: 0,
                    outcome: MemoryTerminationOutcome::OwnershipChanged,
                    fresh_lease_id: None,
                });
            }
            if !Self::can_terminate_process(&current.group, current.exe.as_deref()) {
                return Err("This process group is protected and cannot be terminated.".to_string());
            }
            if crate::process_protection::is_protected_process(
                &current.group,
                None,
                current.exe.as_deref(),
            ) {
                return Err("This process group is protected and cannot be terminated.".to_string());
            }
        }

        // Filter to members that still exist; missing members already exited.
        let mut live = Vec::new();
        for member in &lease.members {
            let Some(current) = system.lookup(member.pid) else {
                continue;
            };
            if current.owner != member.owner
                || current.owner != current_owner
                || current.start_time != member.start_time
                || current.exe != member.exe
                || current.group != member.group
                || current.group != lease.group
            {
                return Ok(MemoryTerminationResult {
                    terminated_count: 0,
                    outcome: MemoryTerminationOutcome::OwnershipChanged,
                    fresh_lease_id: None,
                });
            }
            live.push(member);
        }
        if live.is_empty() {
            return Ok(MemoryTerminationResult {
                terminated_count: 0,
                outcome: MemoryTerminationOutcome::Released,
                fresh_lease_id: None,
            });
        }

        let mut signaled = 0usize;
        for member in &live {
            // Revalidate each member again immediately before its signal. Group
            // validation cannot be atomic, so never rely on the earlier pass
            // after another member has taken time to terminate.
            let Some(current) = system.lookup(member.pid) else {
                continue;
            };
            if current.owner != member.owner
                || current.owner != current_owner
                || current.start_time != member.start_time
                || current.exe != member.exe
                || current.group != member.group
                || current.group != lease.group
            {
                return Ok(MemoryTerminationResult {
                    terminated_count: signaled,
                    outcome: MemoryTerminationOutcome::OwnershipChanged,
                    fresh_lease_id: None,
                });
            }
            match system.signal(member.pid, mode) {
                Ok(()) => signaled += 1,
                Err(error) => {
                    if mode == MemoryTerminationMode::Graceful {
                        break;
                    }
                    return Err(error);
                }
            }
        }
        if signaled == 0 && mode == MemoryTerminationMode::Force {
            return Err(
                "The operating system did not allow Zenith to terminate the process group"
                    .to_string(),
            );
        }

        // Poll briefly so a graceful attempt that made no progress can mint a
        // fresh force-authorized lease with the same verified identities.
        let poll_interval = Duration::from_millis(100);
        for _ in 0..15 {
            system.sleep(poll_interval);
            let remaining: Vec<&MemoryLeaseMember> = lease
                .members
                .iter()
                .filter(|member| {
                    system.lookup(member.pid).is_some_and(|current| {
                        current.owner == member.owner
                            && current.start_time == member.start_time
                            && current.exe == member.exe
                            && current.group == member.group
                    })
                })
                .collect();
            if remaining.is_empty() {
                // Either every member exited or at least one identity drifted.
                // Distinguish genuine exits from PID reuse by checking whether
                // any PID still resolves to a different identity.
                let reused = lease
                    .members
                    .iter()
                    .any(|member| system.lookup(member.pid).is_some());
                if reused {
                    return Ok(MemoryTerminationResult {
                        terminated_count: signaled,
                        outcome: MemoryTerminationOutcome::OwnershipChanged,
                        fresh_lease_id: None,
                    });
                }
                return Ok(MemoryTerminationResult {
                    terminated_count: signaled,
                    outcome: MemoryTerminationOutcome::Released,
                    fresh_lease_id: None,
                });
            }
        }

        // Same verified members are still listening after graceful signaling.
        let mut still_verified = Vec::new();
        let mut identity_drifted = false;
        for member in &lease.members {
            let Some(current) = system.lookup(member.pid) else {
                continue;
            };
            if current.owner == member.owner
                && current.start_time == member.start_time
                && current.exe == member.exe
                && current.group == member.group
            {
                still_verified.push(member.clone());
            } else {
                identity_drifted = true;
            }
        }
        if identity_drifted {
            return Ok(MemoryTerminationResult {
                terminated_count: signaled,
                outcome: MemoryTerminationOutcome::OwnershipChanged,
                fresh_lease_id: None,
            });
        }
        if still_verified.is_empty() {
            return Ok(MemoryTerminationResult {
                terminated_count: signaled,
                outcome: MemoryTerminationOutcome::Released,
                fresh_lease_id: None,
            });
        }
        if mode == MemoryTerminationMode::Graceful {
            let mut guard = store
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let fresh_now = Instant::now();
            let fresh_id = guard.create_lease(CreateMemoryLeaseParams {
                group: lease.group.clone(),
                members: still_verified,
                can_terminate: true,
                force_authorized: true,
                now: fresh_now,
            });
            return Ok(MemoryTerminationResult {
                terminated_count: signaled,
                outcome: MemoryTerminationOutcome::StillListening,
                fresh_lease_id: Some(fresh_id),
            });
        }

        // A force signal was delivered but at least one verified member is
        // still running. Do not mint another force-authorized lease.
        Ok(MemoryTerminationResult {
            terminated_count: signaled,
            outcome: MemoryTerminationOutcome::StillListening,
            fresh_lease_id: None,
        })
    }

    pub(crate) fn normalize_process_name(raw: &str, executable: Option<&Path>) -> String {
        if let Some(app_name) = Self::installed_app_name(executable) {
            return app_name;
        }
        let clean_raw = raw
            .strip_suffix(".exe")
            .or_else(|| raw.strip_suffix(".EXE"))
            .unwrap_or(raw);
        let lower = clean_raw.to_lowercase();
        if lower.contains("cursor") {
            "Cursor".to_string()
        } else if lower.contains("brave") {
            "Brave Browser".to_string()
        } else if lower.contains("chrome") {
            "Google Chrome".to_string()
        } else if lower.contains("docker") || lower.contains("com.docker") {
            "Docker Desktop".to_string()
        } else if lower.contains("claude") {
            "Claude".to_string()
        } else if lower.contains("chatgpt") {
            "ChatGPT".to_string()
        } else if lower.contains("anytype") {
            "Anytype".to_string()
        } else if lower.contains("xcode") || lower.contains("sourcekit") {
            "Xcode".to_string()
        } else if lower.contains("rust-analyzer") {
            "rust-analyzer".to_string()
        } else if lower == "code"
            || lower.starts_with("code helper")
            || lower.contains("visual studio code")
        {
            "VS Code".to_string()
        } else if lower.contains("node") {
            "Node.js".to_string()
        } else if lower.contains("python") {
            "Python".to_string()
        } else if lower.contains("ollama") {
            "Ollama Server".to_string()
        } else if lower.contains("safari") {
            "Safari".to_string()
        } else if lower.contains("antigravity") {
            "Antigravity".to_string()
        } else if lower == "agy" || lower.starts_with("agy ") {
            "agy".to_string()
        } else if lower.contains("kakaotalk") || lower.contains("kakao talk") {
            "KakaoTalk".to_string()
        } else if lower.contains("iterm")
            || lower.contains("terminal")
            || lower.contains("ghostty")
            || lower == "powershell"
            || lower == "cmd"
            || lower == "wt"
        {
            "Terminal".to_string()
        } else {
            clean_raw.to_string()
        }
    }

    fn installed_app_name(executable: Option<&Path>) -> Option<String> {
        let path = executable?.to_str()?;
        #[cfg(target_os = "macos")]
        {
            let is_user_application = path.starts_with("/Applications/")
                || (path.starts_with("/Users/") && path.contains("/Applications/"));
            if !is_user_application {
                return None;
            }
            let bundle_prefix = path.split_once(".app/")?.0;
            bundle_prefix.rsplit('/').next().map(str::to_string)
        }
        #[cfg(target_os = "windows")]
        {
            let normalized = path.replace('/', "\\");
            let is_user_application = normalized.contains("\\AppData\\Local\\Programs\\")
                || normalized.contains("\\AppData\\Roaming\\")
                || normalized.starts_with("C:\\Program Files\\")
                || normalized.starts_with("C:\\Program Files (x86)\\");
            if !is_user_application {
                return None;
            }
            let file_stem = executable?.file_stem()?.to_str()?;
            Some(file_stem.to_string())
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = path;
            None
        }
    }

    fn can_terminate_process(name: &str, executable: Option<&Path>) -> bool {
        if crate::process_protection::is_protected_process(name, None, executable) {
            return false;
        }
        // Legacy explicit deny-list entries that are not covered by the shared
        // module stay here until their owners migrate fully.
        let lower = name.to_lowercase();
        let legacy_protected = matches!(
            lower.as_str(),
            "powershell"
                | "powershell.exe"
                | "cmd"
                | "cmd.exe"
                | "wt"
                | "wt.exe"
                | "explorer"
                | "explorer.exe"
                | "svchost"
                | "svchost.exe"
                | "csrss"
                | "csrss.exe"
                | "services"
                | "services.exe"
                | "lsass"
                | "lsass.exe"
                | "taskmgr"
                | "taskmgr.exe"
                | "msmpeng"
                | "msmpeng.exe"
                | "system"
        );
        if legacy_protected {
            return false;
        }

        let explicitly_supported = matches!(
            name,
            "Google Chrome"
                | "Brave Browser"
                | "Cursor"
                | "Docker Desktop"
                | "Claude"
                | "Xcode"
                | "VS Code"
                | "Ollama Server"
                | "Safari"
                | "Antigravity"
                | "agy"
                | "KakaoTalk"
                | "ChatGPT"
                | "Anytype"
        );

        let installed_user_app = executable.and_then(Path::to_str).is_some_and(|path| {
            #[cfg(target_os = "macos")]
            {
                (path.starts_with("/Applications/")
                    || (path.starts_with("/Users/") && path.contains("/Applications/")))
                    && path.contains(".app/Contents/")
            }
            #[cfg(target_os = "windows")]
            {
                let p = path.replace('/', "\\");
                p.contains("\\AppData\\Local\\Programs\\")
                    || p.contains("\\AppData\\Roaming\\")
                    || p.starts_with("C:\\Program Files\\")
                    || p.starts_with("C:\\Program Files (x86)\\")
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            {
                let _ = path;
                false
            }
        });

        explicitly_supported || installed_user_app
    }

    #[cfg(target_os = "macos")]
    fn get_compressed_memory_macos() -> Option<u64> {
        use std::process::Command;
        let out = crate::tooling::run_with_timeout(
            Command::new("vm_stat"),
            std::time::Duration::from_secs(3),
        )
        .ok()?;
        if !out.status.success() {
            return None;
        }

        let text = String::from_utf8_lossy(&out.stdout);
        // Look for "Pages occupied by compressor: 123456."
        for line in text.lines() {
            if line.contains("Pages occupied by compressor:") {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() == 2 {
                    let num_str = parts[1].trim().trim_end_matches('.');
                    if let Ok(pages) = num_str.parse::<u64>() {
                        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
                        if page_size > 0 {
                            return Some(pages.saturating_mul(page_size as u64));
                        }
                    }
                }
            }
        }
        None
    }

    #[cfg(not(target_os = "macos"))]
    fn get_compressed_memory_macos() -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        supervise_memory_observation, MemoryInspector, MemoryObservation, MemorySampler,
        MemoryTerminationSystem, RealMemorySystem,
    };
    use crate::collection::Admission;
    use crate::metrics::memory_termination::{
        CreateMemoryLeaseParams, MemoryLeaseMember, MemoryTerminationStore,
    };
    use crate::models::{
        MemoryMetrics, MemoryPressure, MemoryTerminationMode, MemoryTerminationOutcome,
    };
    use crate::process_owner::ProcessOwner;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    fn observation_fixture() -> MemoryObservation {
        MemoryObservation {
            metrics: MemoryMetrics {
                total_bytes: 1,
                used_bytes: 0,
                available_bytes: 1,
                free_bytes: 1,
                compressed_bytes: 0,
                swap_used_bytes: 0,
                swap_total_bytes: 0,
                pressure: MemoryPressure::Normal,
                top_processes: Vec::new(),
                timestamp: 1,
            },
            termination_candidates: HashMap::new(),
            captured_at: Instant::now(),
        }
    }

    struct FakeMemorySystem {
        owner: ProcessOwner,
        own_pid: u32,
        members: Mutex<HashMap<u32, MemoryLeaseMember>>,
        signaled: Mutex<Vec<(u32, MemoryTerminationMode)>>,
        auto_exit_on_signal: std::sync::atomic::AtomicBool,
        surviving_pids: Mutex<HashSet<u32>>,
    }

    impl FakeMemorySystem {
        fn new() -> Self {
            Self {
                owner: ProcessOwner::Unix(501),
                own_pid: 1_000,
                members: Mutex::new(HashMap::new()),
                signaled: Mutex::new(Vec::new()),
                auto_exit_on_signal: std::sync::atomic::AtomicBool::new(true),
                surviving_pids: Mutex::new(HashSet::new()),
            }
        }

        fn add_member(&self, pid: u32, group: &str, start_time: u64, exe: &str) {
            self.members.lock().unwrap().insert(
                pid,
                MemoryLeaseMember {
                    pid,
                    owner: self.owner.clone(),
                    start_time,
                    exe: Some(PathBuf::from(exe)),
                    group: group.to_string(),
                },
            );
        }
    }

    impl MemoryTerminationSystem for FakeMemorySystem {
        fn current_owner(&self) -> ProcessOwner {
            self.owner.clone()
        }

        fn own_pid(&self) -> u32 {
            self.own_pid
        }

        fn group_members(&self, group: &str) -> Vec<MemoryLeaseMember> {
            let mut members: Vec<MemoryLeaseMember> = self
                .members
                .lock()
                .unwrap()
                .values()
                .filter(|member| member.group == group)
                .cloned()
                .collect();
            members.sort_by_key(|member| member.pid);
            members
        }

        fn lookup(&self, pid: u32) -> Option<MemoryLeaseMember> {
            self.members.lock().unwrap().get(&pid).cloned()
        }

        fn signal(&self, pid: u32, mode: MemoryTerminationMode) -> Result<(), String> {
            if pid <= 1 || pid == self.own_pid {
                return Err("Cannot terminate system or Zenith process.".to_string());
            }
            self.signaled.lock().unwrap().push((pid, mode));
            if self
                .auto_exit_on_signal
                .load(std::sync::atomic::Ordering::SeqCst)
                && !self.surviving_pids.lock().unwrap().contains(&pid)
            {
                self.members.lock().unwrap().remove(&pid);
            }
            Ok(())
        }

        fn sleep(&self, _duration: std::time::Duration) {}
    }

    fn lease_for(
        store: &Mutex<MemoryTerminationStore>,
        system: &FakeMemorySystem,
        group: &str,
    ) -> String {
        lease_for_with_force_authorization(store, system, group, cfg!(target_os = "windows"))
    }

    fn lease_for_with_force_authorization(
        store: &Mutex<MemoryTerminationStore>,
        system: &FakeMemorySystem,
        group: &str,
        force_authorized: bool,
    ) -> String {
        let members = system.group_members(group);
        assert!(!members.is_empty());
        store.lock().unwrap().create_lease(CreateMemoryLeaseParams {
            group: group.to_string(),
            members,
            can_terminate: true,
            force_authorized,
            now: Instant::now(),
        })
    }

    #[cfg(target_os = "windows")]
    fn initial_termination_mode() -> MemoryTerminationMode {
        MemoryTerminationMode::Force
    }

    #[cfg(not(target_os = "windows"))]
    fn initial_termination_mode() -> MemoryTerminationMode {
        MemoryTerminationMode::Graceful
    }

    #[test]
    fn browser_helpers_are_grouped_with_their_parent_app() {
        assert_eq!(
            MemoryInspector::normalize_process_name("Google Chrome Helper (Renderer)", None),
            "Google Chrome"
        );
        assert_eq!(
            MemoryInspector::normalize_process_name("Brave Browser Helper (GPU)", None),
            "Brave Browser"
        );
    }

    #[test]
    fn installed_app_helpers_are_grouped_by_bundle_name() {
        use std::path::Path;
        assert_eq!(
            MemoryInspector::normalize_process_name(
                "Anytype Helper (Renderer)",
                Some(Path::new(
                    "/Applications/Anytype.app/Contents/Frameworks/Anytype Helper.app/Contents/MacOS/Anytype Helper"
                ))
            ),
            "Anytype"
        );
    }

    #[test]
    fn user_apps_and_requested_agent_processes_can_be_terminated() {
        use std::path::Path;
        assert!(MemoryInspector::can_terminate_process("agy", None));
        assert!(MemoryInspector::can_terminate_process("Antigravity", None));
        assert!(MemoryInspector::can_terminate_process("ChatGPT", None));
        assert!(MemoryInspector::can_terminate_process("Claude", None));
        assert!(MemoryInspector::can_terminate_process("Anytype", None));
        #[cfg(target_os = "macos")]
        {
            assert!(MemoryInspector::can_terminate_process(
                "KakaoTalk",
                Some(Path::new(
                    "/Applications/KakaoTalk.app/Contents/MacOS/KakaoTalk"
                ))
            ));
            assert!(MemoryInspector::can_terminate_process(
                "Acme",
                Some(Path::new(
                    "/Users/test/Applications/Acme.app/Contents/MacOS/Acme"
                ))
            ));
            assert!(!MemoryInspector::can_terminate_process(
                "spotlightknowledged",
                Some(Path::new("/System/Library/Frameworks/spotlightknowledged"))
            ));
            assert!(!MemoryInspector::can_terminate_process(
                "Terminal",
                Some(Path::new(
                    "/Applications/Terminal.app/Contents/MacOS/Terminal"
                ))
            ));
        }

        #[cfg(target_os = "windows")]
        {
            assert!(MemoryInspector::can_terminate_process(
                "KakaoTalk",
                Some(Path::new(
                    "C:\\Program Files\\Kakao\\KakaoTalk\\KakaoTalk.exe"
                ))
            ));
            assert!(MemoryInspector::can_terminate_process(
                "Acme",
                Some(Path::new(
                    "C:\\Users\\test\\AppData\\Local\\Programs\\Acme\\Acme.exe"
                ))
            ));
            assert!(!MemoryInspector::can_terminate_process(
                "csrss",
                Some(Path::new("C:\\Windows\\System32\\csrss.exe"))
            ));
            assert!(!MemoryInspector::can_terminate_process(
                "Terminal",
                Some(Path::new("C:\\Program Files\\WindowsApps\\wt.exe"))
            ));
        }

        assert!(!MemoryInspector::can_terminate_process("Zenith", None));
    }

    #[test]
    fn shared_protection_blocks_terminals_shells_and_system() {
        assert!(!MemoryInspector::can_terminate_process("Terminal", None));
        assert!(!MemoryInspector::can_terminate_process("iTerm2", None));
        assert!(!MemoryInspector::can_terminate_process("Ghostty", None));
        assert!(!MemoryInspector::can_terminate_process("Zenith", None));
    }

    #[test]
    fn memory_sampler_initializes_system_lazily() {
        let sampler = MemorySampler::new();
        // Before sampling, the inner System must be None
        assert!(sampler.system.lock().expect("mutex poisoned").is_none());

        // After sampling, the inner System is populated
        let metrics = sampler.sample();
        assert!(sampler.system.lock().expect("mutex poisoned").is_some());
        assert!(metrics.total_bytes > 0);
    }

    #[test]
    fn stale_lease_is_rejected_without_signaling() {
        let system = FakeMemorySystem::new();
        system.add_member(
            200,
            "Cursor",
            1000,
            "/Applications/Cursor.app/Contents/MacOS/Cursor",
        );
        let store = Mutex::new(MemoryTerminationStore::default());
        let result = MemoryInspector::execute_termination(
            "missing-lease",
            initial_termination_mode(),
            &store,
            &system,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expired"));
        assert!(system.signaled.lock().unwrap().is_empty());
    }

    #[test]
    fn lease_consumption_is_one_shot() {
        let system = FakeMemorySystem::new();
        system.add_member(
            201,
            "Cursor",
            1000,
            "/Applications/Cursor.app/Contents/MacOS/Cursor",
        );
        let store = Mutex::new(MemoryTerminationStore::default());
        let lease = lease_for(&store, &system, "Cursor");
        let first = MemoryInspector::execute_termination(
            &lease,
            initial_termination_mode(),
            &store,
            &system,
        )
        .unwrap();
        assert_eq!(first.outcome, MemoryTerminationOutcome::Released);
        let second = MemoryInspector::execute_termination(
            &lease,
            initial_termination_mode(),
            &store,
            &system,
        );
        assert!(second.is_err());
    }

    #[test]
    fn pid_reuse_with_changed_start_time_sends_no_signal() {
        let system = FakeMemorySystem::new();
        system.add_member(
            202,
            "Cursor",
            1000,
            "/Applications/Cursor.app/Contents/MacOS/Cursor",
        );
        let store = Mutex::new(MemoryTerminationStore::default());
        let lease = lease_for(&store, &system, "Cursor");
        // Simulate PID reuse with a new start time.
        system.members.lock().unwrap().insert(
            202,
            MemoryLeaseMember {
                pid: 202,
                owner: ProcessOwner::Unix(501),
                start_time: 2000,
                exe: Some(PathBuf::from(
                    "/Applications/Cursor.app/Contents/MacOS/Cursor",
                )),
                group: "Cursor".to_string(),
            },
        );
        let result = MemoryInspector::execute_termination(
            &lease,
            initial_termination_mode(),
            &store,
            &system,
        )
        .unwrap();
        assert_eq!(result.outcome, MemoryTerminationOutcome::OwnershipChanged);
        assert!(system.signaled.lock().unwrap().is_empty());
    }

    #[test]
    fn owner_change_sends_no_signal() {
        let system = FakeMemorySystem::new();
        system.add_member(
            203,
            "Cursor",
            1000,
            "/Applications/Cursor.app/Contents/MacOS/Cursor",
        );
        let store = Mutex::new(MemoryTerminationStore::default());
        let lease = lease_for(&store, &system, "Cursor");
        system.members.lock().unwrap().insert(
            203,
            MemoryLeaseMember {
                pid: 203,
                owner: ProcessOwner::Unix(502),
                start_time: 1000,
                exe: Some(PathBuf::from(
                    "/Applications/Cursor.app/Contents/MacOS/Cursor",
                )),
                group: "Cursor".to_string(),
            },
        );
        let result = MemoryInspector::execute_termination(
            &lease,
            initial_termination_mode(),
            &store,
            &system,
        )
        .unwrap();
        assert_eq!(result.outcome, MemoryTerminationOutcome::OwnershipChanged);
        assert!(system.signaled.lock().unwrap().is_empty());
    }

    #[test]
    fn executable_change_sends_no_signal() {
        let system = FakeMemorySystem::new();
        system.add_member(
            204,
            "Cursor",
            1000,
            "/Applications/Cursor.app/Contents/MacOS/Cursor",
        );
        let store = Mutex::new(MemoryTerminationStore::default());
        let lease = lease_for(&store, &system, "Cursor");
        system.members.lock().unwrap().insert(
            204,
            MemoryLeaseMember {
                pid: 204,
                owner: ProcessOwner::Unix(501),
                start_time: 1000,
                exe: Some(PathBuf::from("/tmp/evil")),
                group: "Cursor".to_string(),
            },
        );
        let result = MemoryInspector::execute_termination(
            &lease,
            initial_termination_mode(),
            &store,
            &system,
        )
        .unwrap();
        assert_eq!(result.outcome, MemoryTerminationOutcome::OwnershipChanged);
        assert!(system.signaled.lock().unwrap().is_empty());
    }

    #[test]
    fn protected_targets_are_rejected() {
        assert!(!MemoryInspector::can_terminate_process("Terminal", None));
        assert!(!MemoryInspector::can_terminate_process("Zenith", None));
    }

    #[test]
    fn self_and_system_targets_are_rejected() {
        let system = FakeMemorySystem::new();
        let store = Mutex::new(MemoryTerminationStore::default());
        for pid in [0, 1, 1_000] {
            let lease = store.lock().unwrap().create_lease(CreateMemoryLeaseParams {
                group: "Cursor".to_string(),
                members: vec![MemoryLeaseMember {
                    pid,
                    owner: ProcessOwner::Unix(501),
                    start_time: 1000,
                    exe: Some(PathBuf::from(
                        "/Applications/Cursor.app/Contents/MacOS/Cursor",
                    )),
                    group: "Cursor".to_string(),
                }],
                can_terminate: true,
                force_authorized: cfg!(target_os = "windows"),
                now: Instant::now(),
            });
            let result = MemoryInspector::execute_termination(
                &lease,
                initial_termination_mode(),
                &store,
                &system,
            );
            assert!(result.is_err(), "pid {pid} must be rejected");
        }
        assert!(system.signaled.lock().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn graceful_failure_mints_force_authorized_lease() {
        let system = FakeMemorySystem::new();
        system
            .auto_exit_on_signal
            .store(false, std::sync::atomic::Ordering::SeqCst);
        system.add_member(
            205,
            "Cursor",
            1000,
            "/Applications/Cursor.app/Contents/MacOS/Cursor",
        );
        let store = Mutex::new(MemoryTerminationStore::default());
        let lease = lease_for_with_force_authorization(&store, &system, "Cursor", false);
        let graceful = MemoryInspector::execute_termination(
            &lease,
            MemoryTerminationMode::Graceful,
            &store,
            &system,
        )
        .unwrap();
        assert_eq!(graceful.outcome, MemoryTerminationOutcome::StillListening);
        let fresh = graceful.fresh_lease_id.expect("fresh lease");
        assert_ne!(fresh, lease);
        // Force without the fresh lease must fail.
        system.add_member(
            206,
            "Cursor",
            1001,
            "/Applications/Cursor.app/Contents/MacOS/Cursor",
        );
        let stale_force_lease = store.lock().unwrap().create_lease(CreateMemoryLeaseParams {
            group: "Cursor".to_string(),
            members: system.group_members("Cursor"),
            can_terminate: true,
            force_authorized: false,
            now: Instant::now(),
        });
        // Remove the extra member so the stale lease points at a consistent
        // snapshot but still lacks force authorization.
        system.members.lock().unwrap().remove(&206);
        let force_without_auth = MemoryInspector::execute_termination(
            &stale_force_lease,
            MemoryTerminationMode::Force,
            &store,
            &system,
        );
        assert!(force_without_auth.is_err());
        assert!(force_without_auth
            .unwrap_err()
            .contains("fresh failed graceful"));
        // The fresh force-authorized lease succeeds.
        system
            .auto_exit_on_signal
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let forced = MemoryInspector::execute_termination(
            &fresh,
            MemoryTerminationMode::Force,
            &store,
            &system,
        )
        .unwrap();
        assert_eq!(forced.outcome, MemoryTerminationOutcome::Released);
        let signaled = system.signaled.lock().unwrap();
        assert!(signaled
            .iter()
            .any(|(pid, mode)| *pid == 205 && *mode == MemoryTerminationMode::Graceful));
        assert!(signaled
            .iter()
            .any(|(pid, mode)| *pid == 205 && *mode == MemoryTerminationMode::Force));
    }

    #[cfg(unix)]
    #[test]
    fn partial_graceful_exit_authorizes_only_verified_survivors() {
        let system = FakeMemorySystem::new();
        system.add_member(
            209,
            "Cursor",
            1000,
            "/Applications/Cursor.app/Contents/MacOS/Cursor",
        );
        system.add_member(
            210,
            "Cursor",
            1001,
            "/Applications/Cursor.app/Contents/MacOS/Cursor",
        );
        system.surviving_pids.lock().unwrap().insert(210);
        let store = Mutex::new(MemoryTerminationStore::default());
        let lease = lease_for(&store, &system, "Cursor");

        let graceful = MemoryInspector::execute_termination(
            &lease,
            MemoryTerminationMode::Graceful,
            &store,
            &system,
        )
        .unwrap();

        assert_eq!(graceful.outcome, MemoryTerminationOutcome::StillListening);
        let fresh = graceful.fresh_lease_id.expect("force lease for survivor");
        let guard = store.lock().unwrap();
        let fresh_lease = guard.peek_lease(&fresh, Instant::now()).unwrap();
        assert!(fresh_lease.force_authorized);
        assert_eq!(fresh_lease.members.len(), 1);
        assert_eq!(fresh_lease.members[0].pid, 210);
    }

    #[test]
    fn force_requires_prior_graceful_attempt() {
        let system = FakeMemorySystem::new();
        system.add_member(
            207,
            "Cursor",
            1000,
            "/Applications/Cursor.app/Contents/MacOS/Cursor",
        );
        let store = Mutex::new(MemoryTerminationStore::default());
        let lease = lease_for_with_force_authorization(&store, &system, "Cursor", false);
        let result = MemoryInspector::execute_termination(
            &lease,
            MemoryTerminationMode::Force,
            &store,
            &system,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("fresh failed graceful"));
        assert!(system.signaled.lock().unwrap().is_empty());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_graceful_is_unavailable() {
        let system = FakeMemorySystem::new();
        system.add_member(208, "Cursor", 1000, "C:\\Program Files\\Cursor\\Cursor.exe");
        let store = Mutex::new(MemoryTerminationStore::default());
        let lease = lease_for(&store, &system, "Cursor");
        let result = MemoryInspector::execute_termination(
            &lease,
            MemoryTerminationMode::Graceful,
            &store,
            &system,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unavailable on Windows"));
        assert!(system.signaled.lock().unwrap().is_empty());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_explicit_force_uses_a_force_authorized_verified_lease() {
        let system = FakeMemorySystem::new();
        system.add_member(211, "Cursor", 1000, "C:\\Program Files\\Cursor\\Cursor.exe");
        let store = Mutex::new(MemoryTerminationStore::default());
        let lease = store.lock().unwrap().create_lease(CreateMemoryLeaseParams {
            group: "Cursor".to_string(),
            members: system.group_members("Cursor"),
            can_terminate: true,
            force_authorized: true,
            now: Instant::now(),
        });

        let result = MemoryInspector::execute_termination(
            &lease,
            MemoryTerminationMode::Force,
            &store,
            &system,
        )
        .unwrap();

        assert_eq!(result.outcome, MemoryTerminationOutcome::Released);
        assert_eq!(
            system.signaled.lock().unwrap().as_slice(),
            &[(211, MemoryTerminationMode::Force)]
        );
    }

    #[test]
    fn real_system_reports_platform_owner() {
        let system = RealMemorySystem::default();
        let owner = system.current_owner();
        #[cfg(unix)]
        assert!(matches!(owner, ProcessOwner::Unix(_)));
        #[cfg(not(unix))]
        assert!(matches!(owner, ProcessOwner::Windows(_)));
    }

    #[tokio::test]
    async fn single_observation_pass_refreshes_processes_once() {
        let sampler = MemorySampler::new();
        assert_eq!(sampler.process_refresh_count(), 0);

        let observation = sampler.observe();
        assert_eq!(sampler.process_refresh_count(), 1);
        assert!(observation.metrics.total_bytes > 0);

        for process in &observation.metrics.top_processes {
            if process.can_terminate {
                let candidates = observation.termination_candidates.get(&process.name);
                assert!(
                    candidates.is_some(),
                    "Terminable process {} must have pre-collected candidates",
                    process.name
                );
            }
        }
    }

    #[tokio::test]
    async fn concurrent_requests_share_observation_and_mint_separate_leases() {
        let sampler = Arc::new(MemorySampler::new());
        let store = Mutex::new(MemoryTerminationStore::new(64));

        let sampler1 = sampler.clone();
        let sampler2 = sampler.clone();

        let (obs1, obs2) = tokio::join!(
            sampler1.get_observation(Duration::from_millis(800)),
            sampler2.get_observation(Duration::from_millis(800)),
        );

        let obs1 = obs1.expect("obs1 failed");
        let obs2 = obs2.expect("obs2 failed");

        assert_eq!(sampler.process_refresh_count(), 1);

        let metrics1 = obs1.mint_metrics_with_leases(&store);
        let metrics2 = obs2.mint_metrics_with_leases(&store);

        for (p1, p2) in metrics1
            .top_processes
            .iter()
            .zip(metrics2.top_processes.iter())
        {
            if let (true, Some(id1), Some(id2)) = (
                p1.can_terminate,
                &p1.termination_lease_id,
                &p2.termination_lease_id,
            ) {
                assert_ne!(
                    id1, id2,
                    "Each response must receive a distinct one-shot lease ID"
                );

                let now = Instant::now();
                let lease1 = store.lock().unwrap().take_lease(id1, now);
                assert!(lease1.is_some(), "Lease 1 must be consumable");

                let guard = store.lock().unwrap();
                let lease2_peek = guard.peek_lease(id2, now);
                assert!(
                    lease2_peek.is_some(),
                    "Lease 2 must remain valid after consuming lease 1"
                );
            }
        }
    }

    #[tokio::test]
    async fn cancelled_observation_request_does_not_abandon_shared_collection() {
        let sampler = Arc::new(MemorySampler::new());
        let entry = match sampler
            .singleflight
            .admit("memory_observation".to_string(), 0)
            .await
        {
            Admission::Own(entry) => entry,
            Admission::Wait(_) => unreachable!(),
        };
        let (release, released) = std::sync::mpsc::channel();
        supervise_memory_observation(sampler.clone(), entry.clone(), move || {
            released.recv().unwrap();
            observation_fixture()
        });

        let admitting_request = {
            let sampler = sampler.clone();
            let entry = entry.clone();
            tokio::spawn(async move { sampler.singleflight.wait(&entry).await })
        };
        admitting_request.abort();
        let _ = admitting_request.await;

        release.send(()).unwrap();
        let observation =
            tokio::time::timeout(Duration::from_secs(1), sampler.singleflight.wait(&entry))
                .await
                .expect("detached observation supervisor did not complete")
                .expect("observation collection failed");
        assert_eq!(observation.metrics.total_bytes, 1);

        let retry = sampler
            .singleflight
            .admit("memory_observation".to_string(), 0)
            .await;
        let retry_entry = match retry {
            Admission::Own(entry) => entry,
            Admission::Wait(_) => panic!("completed observation remained in flight"),
        };
        sampler
            .singleflight
            .complete(&retry_entry, Ok(Arc::new(observation_fixture())))
            .await;
    }

    #[tokio::test]
    async fn panicking_observation_completes_waiters_and_allows_retry() {
        let sampler = Arc::new(MemorySampler::new());
        let entry = match sampler
            .singleflight
            .admit("memory_observation".to_string(), 0)
            .await
        {
            Admission::Own(entry) => entry,
            Admission::Wait(_) => unreachable!(),
        };
        supervise_memory_observation(sampler.clone(), entry.clone(), || panic!("fixture failure"));

        let result =
            tokio::time::timeout(Duration::from_secs(1), sampler.singleflight.wait(&entry))
                .await
                .expect("panicking observation stranded its waiters");
        assert!(result.is_err());

        let retry = sampler
            .singleflight
            .admit("memory_observation".to_string(), 0)
            .await;
        let retry_entry = match retry {
            Admission::Own(entry) => entry,
            Admission::Wait(_) => panic!("failed observation remained in flight"),
        };
        sampler
            .singleflight
            .complete(&retry_entry, Ok(Arc::new(observation_fixture())))
            .await;
    }
}
