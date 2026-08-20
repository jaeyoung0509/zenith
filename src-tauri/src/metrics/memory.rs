use crate::models::{MemoryMetrics, MemoryPressure, ProcessMemory};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};
use sysinfo::{ProcessesToUpdate, Signal, System};

pub struct MemorySampler {
    system: Mutex<Option<System>>,
    compressed_cache: Mutex<Option<(Instant, u64)>>,
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
        }
    }

    /// Captures current system memory metrics and top resource-consuming developer processes.
    pub fn sample(&self) -> MemoryMetrics {
        let (
            total_bytes,
            used_bytes,
            available_bytes,
            free_bytes,
            swap_total_bytes,
            swap_used_bytes,
            top_processes,
        ) = {
            let mut guard = self.system.lock().unwrap();
            let sys = guard.get_or_insert_with(System::new_all);
            sys.refresh_memory();

            let total_bytes = sys.total_memory();
            let used_bytes = sys.used_memory();
            let free_bytes = sys.free_memory();
            let available_bytes = sys.available_memory();
            let swap_total_bytes = sys.total_swap();
            let swap_used_bytes = sys.used_swap();

            // Refresh processes
            sys.refresh_processes(ProcessesToUpdate::All, true);

            // Aggregate top processes
            let mut process_groups: HashMap<String, (u64, usize, u32, Vec<u32>, bool)> =
                HashMap::new();

            for (pid, process) in sys.processes() {
                let raw_name = process.name().to_string_lossy();
                let norm_name = MemoryInspector::normalize_process_name(&raw_name, process.exe());
                let mem = process.memory();
                let can_terminate =
                    MemoryInspector::can_terminate_process(&norm_name, process.exe());

                let entry = process_groups
                    .entry(norm_name)
                    .or_insert_with(|| (0, 0, pid.as_u32(), Vec::new(), false));
                entry.0 += mem;
                entry.1 += 1;
                entry.3.push(pid.as_u32());
                entry.4 |= can_terminate;
            }

            let mut top_processes: Vec<ProcessMemory> = process_groups
                .into_iter()
                .map(
                    |(name, (memory_bytes, process_count, pid, mut pids, can_terminate))| {
                        pids.sort_unstable();
                        ProcessMemory {
                            pid,
                            pids,
                            can_terminate,
                            name,
                            memory_bytes,
                            process_count,
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

        MemoryMetrics {
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
        }
    }

    fn compressed_memory(&self) -> u64 {
        const CACHE_TTL: Duration = Duration::from_secs(8);
        let now = Instant::now();

        {
            let cache = self.compressed_cache.lock().unwrap();
            if let Some((cached_at, value)) = *cache {
                if now.duration_since(cached_at) < CACHE_TTL {
                    return value;
                }
            }
        }

        // Run subprocess outside the mutex
        let value = MemoryInspector::get_compressed_memory_macos().unwrap_or(0);
        *self.compressed_cache.lock().unwrap() = Some((now, value));
        value
    }
}

pub struct MemoryInspector;

impl MemoryInspector {
    /// Captures current system memory metrics and top resource-consuming developer processes.
    pub fn get_metrics() -> MemoryMetrics {
        MemorySampler::new().sample()
    }

    fn normalize_process_name(raw: &str, executable: Option<&Path>) -> String {
        if let Some(app_name) = Self::installed_app_name(executable) {
            return app_name;
        }
        let lower = raw.to_lowercase();
        if lower.contains("cursor") {
            "Cursor".to_string()
        } else if lower.contains("brave browser") {
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
        } else if lower.contains("iterm") || lower.contains("terminal") || lower.contains("ghostty")
        {
            "Terminal".to_string()
        } else {
            raw.to_string()
        }
    }

    fn installed_app_name(executable: Option<&Path>) -> Option<String> {
        let path = executable?.to_str()?;
        let is_user_application = path.starts_with("/Applications/")
            || (path.starts_with("/Users/") && path.contains("/Applications/"));
        if !is_user_application {
            return None;
        }
        let bundle_prefix = path.split_once(".app/")?.0;
        bundle_prefix.rsplit('/').next().map(str::to_string)
    }

    fn can_terminate_process(name: &str, executable: Option<&Path>) -> bool {
        if matches!(name, "Zenith" | "Terminal") {
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
            (path.starts_with("/Applications/")
                || (path.starts_with("/Users/") && path.contains("/Applications/")))
                && path.contains(".app/Contents/")
        });
        explicitly_supported || installed_user_app
    }

    /// Signals only an allowlisted user-application group resolved from a fresh
    /// process snapshot. Arbitrary PID termination is intentionally not exposed.
    pub fn terminate_group(name: &str, force: bool) -> Result<usize, String> {
        let mut system = System::new_all();
        system.refresh_processes(ProcessesToUpdate::All, true);
        let signal = if force { Signal::Kill } else { Signal::Term };
        let mut matched = 0usize;
        let mut signaled = 0usize;

        for process in system.processes().values() {
            let raw_name = process.name().to_string_lossy();
            if Self::normalize_process_name(&raw_name, process.exe()) != name {
                continue;
            }
            matched += 1;
            if !Self::can_terminate_process(name, process.exe()) {
                continue;
            }
            if process.kill_with(signal).unwrap_or(false) {
                signaled += 1;
            }
        }

        if matched == 0 {
            return Err(format!("{name} is no longer running"));
        }
        if signaled == 0 {
            return Err(format!("macOS did not allow Zenith to terminate {name}"));
        }
        Ok(signaled)
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
    use super::{MemoryInspector, MemorySampler};

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
        assert!(!MemoryInspector::can_terminate_process("Zenith", None));
    }

    #[test]
    fn memory_sampler_initializes_system_lazily() {
        let sampler = MemorySampler::new();
        // Before sampling, the inner System must be None
        assert!(sampler.system.lock().unwrap().is_none());

        // After sampling, the inner System is populated
        let metrics = sampler.sample();
        assert!(sampler.system.lock().unwrap().is_some());
        assert!(metrics.total_bytes > 0);
    }
}
