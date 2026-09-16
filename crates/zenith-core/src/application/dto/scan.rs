//! Scan progress, as the interface observes it.
//!
//! A scan reports progress while it runs and one [`ScanResult`] when it
//! finishes. The events carry the same items the result will, one at a time,
//! so a caller can render before the scan completes; they are a projection of
//! the scan, not a second source of truth for it.

use crate::domain::{Category, ScanItem, ScanResult};
use serde::{Deserialize, Serialize};

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
