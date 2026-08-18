use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryPressure {
    Normal,
    Warning,
    Critical,
}

impl MemoryPressure {
    pub fn display_name(&self) -> &'static str {
        match self {
            MemoryPressure::Normal => "Normal",
            MemoryPressure::Warning => "Warning",
            MemoryPressure::Critical => "Critical",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessMemory {
    pub pid: u32,
    pub name: String,
    pub memory_bytes: u64,
    pub process_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryMetrics {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub free_bytes: u64,
    pub compressed_bytes: u64,
    pub swap_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub pressure: MemoryPressure,
    pub top_processes: Vec<ProcessMemory>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiskMetrics {
    pub mount_point: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub available_bytes: u64,
    pub percent_used: f64,
}
