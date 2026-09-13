//! Serializable projections, grouped by the workflow that produces them.

pub mod cleanup;
pub mod scan;
pub mod storage;

pub use cleanup::{
    CleanEvent, CleanFailureReason, CleanItemResult, CleanResult, CleanStatus, PlanPreview,
    PlanTargetPreview,
};
pub use scan::ScanEvent;
pub use storage::{
    AppRelatedItem, AppUninstallInspection, InstalledApp, InstalledAppInventory, LargeFileItem,
    LargeFileScanEvent, LargeFileScanRequest, LargeFileScanResult, TrashItemResult,
    TrashPlanPreview, TrashResult,
};
