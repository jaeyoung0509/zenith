use crate::models::{MemoryMetrics, MemoryPressure, ProcessMemory};
use std::collections::HashMap;
use std::time::SystemTime;
use sysinfo::{ProcessesToUpdate, Signal, System};

pub struct MemoryInspector;

impl MemoryInspector {
    /// Captures current system memory metrics and top resource-consuming developer processes.
    pub fn get_metrics() -> MemoryMetrics {
        let mut sys = System::new_all();
        sys.refresh_memory();
        sys.refresh_processes(ProcessesToUpdate::All, true);

        let total_bytes = sys.total_memory();
        let used_bytes = sys.used_memory();
        let free_bytes = sys.free_memory();
        let available_bytes = sys.available_memory();
        let swap_total_bytes = sys.total_swap();
        let swap_used_bytes = sys.used_swap();

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

        // Estimate compressed memory on macOS (or fallback)
        let compressed_bytes = Self::get_compressed_memory_macos().unwrap_or(0);

        // Aggregate top processes
        let mut process_groups: HashMap<String, (u64, usize, u32)> = HashMap::new();

        for (pid, process) in sys.processes() {
            let raw_name = process.name().to_string_lossy();
            let norm_name = Self::normalize_process_name(&raw_name);
            let mem = process.memory();

            let entry = process_groups
                .entry(norm_name)
                .or_insert((0, 0, pid.as_u32()));
            entry.0 += mem;
            entry.1 += 1;
        }

        let mut top_processes: Vec<ProcessMemory> = process_groups
            .into_iter()
            .map(|(name, (memory_bytes, process_count, pid))| ProcessMemory {
                pid,
                can_terminate: Self::can_terminate_group(&name),
                name,
                memory_bytes,
                process_count,
            })
            .collect();

        top_processes.sort_by(|a, b| b.memory_bytes.cmp(&a.memory_bytes));
        top_processes.truncate(15);

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

    fn normalize_process_name(raw: &str) -> String {
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
        } else if lower.contains("iterm") || lower.contains("terminal") || lower.contains("ghostty")
        {
            "Terminal".to_string()
        } else {
            raw.to_string()
        }
    }

    fn can_terminate_group(name: &str) -> bool {
        matches!(
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
        )
    }

    /// Signals only an allowlisted user-application group resolved from a fresh
    /// process snapshot. Arbitrary PID termination is intentionally not exposed.
    pub fn terminate_group(name: &str, force: bool) -> Result<usize, String> {
        if !Self::can_terminate_group(name) {
            return Err(format!("{name} is protected from termination by Zenith"));
        }

        let mut system = System::new_all();
        system.refresh_processes(ProcessesToUpdate::All, true);
        let signal = if force { Signal::Kill } else { Signal::Term };
        let mut matched = 0usize;
        let mut signaled = 0usize;

        for process in system.processes().values() {
            let raw_name = process.name().to_string_lossy();
            if Self::normalize_process_name(&raw_name) != name {
                continue;
            }
            matched += 1;
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
        let out = Command::new("vm_stat").output().ok()?;
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
                        return Some(pages * 4096);
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
    use super::MemoryInspector;

    #[test]
    fn browser_helpers_are_grouped_with_their_parent_app() {
        assert_eq!(
            MemoryInspector::normalize_process_name("Google Chrome Helper (Renderer)"),
            "Google Chrome"
        );
        assert_eq!(
            MemoryInspector::normalize_process_name("Brave Browser Helper (GPU)"),
            "Brave Browser"
        );
    }

    #[test]
    fn only_known_user_apps_can_be_terminated() {
        assert!(MemoryInspector::can_terminate_group("Google Chrome"));
        assert!(MemoryInspector::can_terminate_group("Cursor"));
        assert!(!MemoryInspector::can_terminate_group("spotlightknowledged"));
        assert!(!MemoryInspector::can_terminate_group("Terminal"));
        assert!(!MemoryInspector::can_terminate_group("Zenith"));
    }
}
