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
