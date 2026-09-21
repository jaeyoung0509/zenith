//! Scan progress, as the interface observes it.
//!
//! A scan reports progress while it runs and one [`ScanResult`] when it
//! finishes. The events carry the same items the result will, one at a time,
//! so a caller can render before the scan completes; they are a projection of
//! the scan, not a second source of truth for it.

use crate::domain::{Category, ScanItem, ScanResult};
use serde::{Deserialize, Serialize};

/// Whether the backend exhausted discovery or retained an in-memory checkpoint.
/// A continuation id is discovery authority only. It never contains a path and
/// never authorizes cleanup.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ScanDiscovery {
    #[default]
    Exhausted,
    Paused {
        continuation_id: String,
    },
    Stopped {
        reason: String,
    },
}

/// The latest backend-owned inventory and its discovery state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PublishedScan {
    pub result: ScanResult,
    #[serde(default)]
    pub discovery: ScanDiscovery,
}

/// The only facts a caller may submit to resume discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ResumeScanRequest {
    pub scan_id: String,
    pub continuation_id: String,
}

/// A scan's progress, streamed one event at a time.
///
/// `ItemFound` carries the measured item by value: the event exists to be
/// rendered once, and boxing it would add an allocation to every item a scan
/// discovers to save a `memcpy` on a value that is never stored in a
/// collection.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum ScanEvent {
    Started {
        scan_id: String,
    },
    CategoryStarted {
        category: Category,
    },
    /// One root of one signature is about to be read.
    ///
    /// Progress, not a result: a scan spends most of its time inside a single
    /// root, so naming the root is what lets the interface say where the scan
    /// is rather than only that it is running. No byte total is claimed here —
    /// the root's measurement arrives with the items it produced.
    RootStarted {
        category: Category,
        signature_id: String,
        name: String,
        /// The resolved root, as the walker reads it.
        root: String,
    },
    ItemFound {
        item: ScanItem,
    },
    CategoryFinished {
        category: Category,
        #[serde(with = "crate::ipc_numeric::u64")]
        #[specta(type = u64)]
        bytes: u64,
        item_count: usize,
    },
    Finished {
        result: ScanResult,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScanRequest {
    pub categories: Option<Vec<Category>>,
    pub excluded_signatures: Vec<String>,
    pub intensive_cleanup: bool,
}

pub trait ScanProgressSink: Send + Sync {
    fn emit(&self, event: ScanEvent);
}

impl<F> ScanProgressSink for F
where
    F: Fn(ScanEvent) + Send + Sync,
{
    fn emit(&self, event: ScanEvent) {
        self(event);
    }
}

pub trait CancellationProbe: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NeverCancelled;

impl CancellationProbe for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}
