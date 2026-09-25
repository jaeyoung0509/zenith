use serde::{Deserialize, Serialize};

use super::awake::PowerSourceType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
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

/// How Zenith relates to a process group it is displaying.
///
/// The grouping is an observation of the system process table. `ZenithChild`
/// is only ever reported when the same fresh snapshot shows this Zenith process
/// in every member's parent chain, so the view never implies Zenith started a
/// process it merely observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ProcessOwnership {
    #[default]
    Observed,
    ZenithChild,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProcessMemory {
    pub pid: u32,
    #[serde(default)]
    pub pids: Vec<u32>,
    pub name: String,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub memory_bytes: u64,
    pub process_count: usize,
    pub can_terminate: bool,
    #[serde(default)]
    pub termination_lease_id: Option<String>,
    /// Names of direct parents outside this process group, de-duplicated and
    /// sorted. Parents normalized into the same group are omitted so the UI
    /// reports the group's external source rather than an internal worker.
    #[serde(default)]
    pub parent_process_names: Vec<String>,
    #[serde(default)]
    pub ownership: ProcessOwnership,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTerminationMode {
    Graceful,
    Force,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTerminationOutcome {
    Released,
    StillListening,
    OwnershipChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct MemoryTerminationResult {
    pub terminated_count: usize,
    pub outcome: MemoryTerminationOutcome,
    pub fresh_lease_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct MemoryMetrics {
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub total_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub used_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub available_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub free_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub compressed_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub swap_used_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub swap_total_bytes: u64,
    pub pressure: MemoryPressure,
    pub top_processes: Vec<ProcessMemory>,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct DiskMetrics {
    pub mount_point: String,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub total_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub used_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub free_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub available_bytes: u64,
    pub percent_used: f64,
}

/// How current the CPU reading in a [`CpuMetrics`] observation is.
///
/// The variant is the answer, not a styling hint: a caller that renders
/// `usage_percent` must render the state beside it, because `Stale` and
/// `Failed` both carry a value that was measured earlier but no longer
/// describes the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CpuSampleState {
    /// Primed, but the platform has not been given the interval it needs
    /// between two readings yet, so no share has been measured.
    Warmup,
    /// Measured from a real pair of readings, or the last such measurement
    /// while it is still inside the poll budget.
    Fresh,
    /// A measurement exists but is older than `stale_after_ms`; it is kept
    /// so the age of the last real reading stays visible.
    Stale,
    /// The platform refused a value it cannot produce, such as a normalized
    /// share with no logical CPU to divide by.
    Unavailable,
    /// The probe failed. The previous measurement, when there is one, is kept
    /// so the failure is not confused with a machine that was never measured.
    Failed,
}

/// One system-wide CPU observation.
///
/// Every field is either measured or explicitly absent; a percentage is only
/// ever present when two real readings produced it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct CpuMetrics {
    pub state: CpuSampleState,
    /// System-wide busy percentage normalized 0-100 across all logical cores.
    /// `None` unless a real pair of readings produced it.
    pub usage_percent: Option<f32>,
    /// Wall-clock milliseconds between the two readings that produced
    /// `usage_percent`.
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub sample_interval_ms: Option<u64>,
    /// Unix milliseconds of the reading used for `usage_percent`.
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub sampled_at: Option<u64>,
    /// Age after which an existing reading must be reported `stale`.
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub stale_after_ms: u64,
    pub cores: u32,
    pub reason: Option<String>,
}

/// Whether the machine has a battery at all.
///
/// `Absent` and `Unavailable` are different facts: `Absent` is the platform
/// saying there is no battery, `Unavailable` is the platform not being able to
/// answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum BatteryPresence {
    Present,
    Absent,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum BatteryChargeState {
    Charging,
    Discharging,
    Full,
    PluggedInNotCharging,
    Unknown,
}

/// One battery observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct BatteryMetrics {
    pub presence: BatteryPresence,
    pub charge_state: BatteryChargeState,
    /// Charge percentage 0-100, or `None` when the platform does not report one.
    pub percent: Option<f32>,
    /// Only when the platform returns a meaningful estimate.
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub time_remaining_seconds: Option<u64>,
    /// Reuse the existing `PowerSourceType` tri-state from
    /// `src-tauri/src/models/awake.rs`.
    pub power_source: PowerSourceType,
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub sampled_at: Option<u64>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct DiskVolume {
    pub name: String,
    pub mount_point: String,
    pub file_system: String,
    pub disk_type: String,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub total_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub used_bytes: u64,
    #[serde(with = "crate::ipc_numeric::u64")]
    #[specta(type = u64)]
    pub available_bytes: u64,
    pub percent_used: f64,
    pub is_removable: bool,
    pub is_primary: bool,
}
