//! Observation quality: how much a reading may be trusted.
//!
//! Every measurement Zenith presents carries one of these, because a number
//! that came from a partial scan and a number that came from a complete one
//! are not the same claim. The value is a domain fact rather than a
//! presentation detail: the safety planner refuses to clean an `Unavailable`
//! observation, quick clean skips `Partial` ones, and storage workflows report
//! it alongside the bytes they counted.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ObservationQuality {
    /// The measurement completed and describes every entry it covers.
    #[default]
    Fresh,
    /// The measurement completed, but the underlying source lags the live
    /// state it describes.
    Stale,
    /// Part of the tree could not be measured. The total is a lower bound.
    Partial,
    /// Nothing could be measured. The absence of a number is the reading.
    Unavailable,
}

impl ObservationQuality {
    /// Whether a reading of this quality describes everything it covers.
    pub fn is_complete(&self) -> bool {
        matches!(self, ObservationQuality::Fresh)
    }
}

/// Whether something taken at `created_at` is still inside its window at `now`.
///
/// One rule, used by every time-bounded reading and authority in Zenith: a
/// scan result, a reviewed inventory, a cleanup plan, and a Trash plan all ask
/// this question, and they must answer it the same way.
///
/// The window is half-open, so an item exactly `window_seconds` old is stale.
/// A clock that moved backwards — an NTP correction, a manual change, a VM
/// snapshot restore — makes the subtraction fail, and that counts as stale
/// rather than as age zero. Stores for destructive authorization additionally
/// enforce elapsed monotonic time, because wall-clock timestamps alone cannot
/// prevent authority from reviving after a clock rollback and recovery.
pub fn is_within_window(created_at: u64, now: u64, window_seconds: u64) -> bool {
    now.checked_sub(created_at)
        .is_some_and(|age| age < window_seconds)
}

#[cfg(test)]
mod tests {
    use super::is_within_window;

    #[test]
    fn the_window_is_half_open() {
        assert!(is_within_window(1_000, 1_299, 300));
        assert!(!is_within_window(1_000, 1_300, 300));
        assert!(!is_within_window(1_000, 1_301, 300));
        // A zero-length window admits nothing, not even the same instant.
        assert!(!is_within_window(1_000, 1_000, 0));
    }

    #[test]
    fn a_clock_that_moved_backwards_reads_as_stale() {
        // The reading was taken at 12:00 and the wall clock now says 11:05.
        assert!(!is_within_window(12_000, 11_300, 300));
        // Even one second backwards is not "fresh for one second longer".
        assert!(!is_within_window(12_000, 11_999, 300));
    }
}
