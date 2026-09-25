pub mod cpu;
pub mod disk;
pub mod memory;
pub mod memory_termination;

pub use cpu::{CpuSampler, CPU_STALE_AFTER_MS};
pub use disk::DiskMetricsCollector;
pub use memory::{MemoryInspector, MemoryObservation, MemorySampler};
pub use memory_termination::MemoryTerminationStore;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The instant an observation carries, in Unix milliseconds.
///
/// Every metrics observation is stamped from this one function so two surfaces
/// reporting the same machine cannot disagree about what "now" was.
pub(crate) fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(ipc_millis)
        .unwrap_or(0)
}

/// A duration in milliseconds, clamped to the largest integer the IPC boundary
/// can carry without losing precision in JavaScript.
pub(crate) fn ipc_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis())
        .unwrap_or(u64::MAX)
        .min(crate::ipc_numeric::MAX_SAFE_INTEGER)
}
