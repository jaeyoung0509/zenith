//! Product semantics that would still make sense if Zenith had a CLI instead
//! of a desktop application.
//!
//! Nothing in this module knows about Tauri, a webview, a window, or a native
//! OS binding. It is checked by `scripts/check_core_boundaries.cjs`, which
//! fails the build when a dependency that would end that property appears in
//! `zenith-core`'s resolved graph.
//!
//! The module is organized by what a concept *is*, not by which screen shows
//! it:
//!
//! - [`scan`] — what was measured, and whether it may be cleaned
//! - [`cleanup`] — what may be deleted, and the authorization that says so
//! - [`storage`] — the vocabulary and thresholds of the storage workflows
//! - [`platform`] — what this platform can do, and what it calls things
//! - [`risk`] — how much consequence a cleanup carries
//! - [`identity`] — whether a path still names the file that was measured
//! - [`paths`] — path values whose invariants are enforced at construction
//! - [`observation`] — how far a reading may be trusted, and how long

pub mod category;
pub mod cleanup;
pub mod error;
pub mod identity;
pub mod observation;
pub mod paths;
pub mod platform;
pub mod risk;
pub mod scan;
pub mod storage;

pub use category::Category;
pub use cleanup::{
    CleanStrategy, CleanupMode, DeletePlan, DeleteTarget, EntryKind, StructuredStateKind,
};
pub use error::{ZenithError, ZenithResult};
pub use identity::{CleanupIdentity, FileIdentity, ModifiedStamp, ReviewedFileIdentity};
pub use observation::{is_within_window, ObservationQuality};
pub use paths::{AbsolutePath, CanonicalPath, PathViolation};
pub use platform::{
    CapabilityAccess, PlatformAccelerator, PlatformCapabilities, PlatformCapabilityError,
    PlatformContext, PlatformFeature, PlatformFeatureCapability, PlatformFeatureStatus,
    PlatformKind,
};
pub use risk::{RiskSummary, RiskTier};
pub use scan::{
    derive_cleanup_disposition, is_safety_blocked_reason, AgeObservation, CacheArtifactKind,
    CacheManagementMode, CacheMetadata, CacheSizeSemantics, CacheUsageConfidence, CategoryResult,
    CleanupDisposition, CleanupEligibility, CleanupOwnership, CleanupUnit, CleanupUnitKind,
    DispositionFacts, EligibilityBucket, EligibilityGate, EligibilitySummary, FileSize,
    OwnershipConfidence, ScanItem, ScanMetrics, ScanResult,
};
pub use storage::{
    AppInstallSource, AppRelatedConfidence, AppRelatedKind, LargeFileFilter, LargeFileKind,
};
