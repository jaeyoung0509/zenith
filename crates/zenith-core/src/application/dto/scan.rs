//! Scan progress, as the interface observes it.
//!
//! A scan reports progress while it runs and one [`ScanResult`] when it
//! finishes. The events carry the same items the result will, one at a time,
//! so a caller can render before the scan completes; they are a projection of
//! the scan, not a second source of truth for it.

use crate::domain::{Category, ScanItem, ScanResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type")]
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
