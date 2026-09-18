//! The desktop crate's model surface.
//!
//! Product semantics live in `zenith_core`. This module re-exports them
//! alongside the DTOs that only the desktop adapter produces, so a command or
//! service module keeps one import root (`crate::models::…`) and does not have
//! to know which side of the boundary a given concept landed on.
//!
//! What is here rather than in `zenith_core` is what does not survive the
//! question "would this still make sense if Zenith had a CLI instead of a
//! Tauri window?" — AI provider snapshots, Docker and metrics readings,
//! keep-awake rules, developer-port state, agent activity, developer
//! artifacts, diagnostics, and the persisted user settings document.
//!
//! The re-exports are listed by name, not globbed: a glob would silently
//! absorb a future core type into this namespace, and the compiler cannot warn
//! about a name that was never written down.

pub mod agent_activity;
pub mod ai_control_center;
pub mod ai_usage;
pub mod awake;
pub mod dev_ports;
pub mod developer_artifacts;
pub mod diagnostics;
pub mod docker;
pub mod local_model;
pub mod metrics;
pub mod settings;
pub mod signature;

pub use agent_activity::*;
pub use ai_control_center::*;
pub use ai_usage::*;
pub use awake::*;
pub use dev_ports::*;
pub use developer_artifacts::*;
pub use diagnostics::*;
pub use docker::*;
pub use local_model::*;
pub use metrics::*;
pub use settings::*;
pub use signature::*;
pub use zenith_platform::PlatformCapabilitiesProvider;

// ---------------------------------------------------------------------------
// Zenith domain semantics, re-exported from `zenith_core`.
// ---------------------------------------------------------------------------

pub use zenith_core::application::dto::cleanup::{
    CleanEvent, CleanFailureReason, CleanItemResult, CleanResult, CleanStatus, CleanupProgressSink,
    PlanPreview, PlanTargetPreview,
};
pub use zenith_core::application::dto::scan::{
    CancellationProbe, NeverCancelled, ScanEvent, ScanProgressSink, ScanRequest,
};
pub use zenith_core::application::dto::storage::{
    AppRelatedItem, AppUninstallInspection, InstalledApp, InstalledAppInventory, LargeFileItem,
    LargeFileScanEvent, LargeFileScanRequest, LargeFileScanResult, TrashItemResult,
    TrashPlanPreview, TrashResult,
};
pub use zenith_core::domain::category::Category;
pub use zenith_core::domain::cleanup::{
    classify_structured_state, CleanStrategy, CleanupMode, CleanupOperation, ContainerCleanup,
    DeletePlan, DeleteTarget, EntryKind, FilesystemCleanup, FilesystemMutation, PathFacts,
    ProviderCleanup, RunningProcessPolicy, StructuredStateKind,
};
pub use zenith_core::domain::error::{ZenithError, ZenithResult};
pub use zenith_core::domain::identity::{
    CleanupIdentity, FileIdentity, ModifiedStamp, ReviewedFileIdentity,
};
pub use zenith_core::domain::observation::ObservationQuality;
pub use zenith_core::domain::paths::{AbsolutePath, CanonicalPath, PathViolation};
pub use zenith_core::domain::platform::{
    CapabilityAccess, PlatformAccelerator, PlatformCapabilities, PlatformCapabilityError,
    PlatformContext, PlatformFeature, PlatformFeatureCapability, PlatformFeatureStatus,
    PlatformKind,
};
pub use zenith_core::domain::risk::{RiskSummary, RiskTier};
pub use zenith_core::domain::scan::{
    derive_cleanup_disposition, is_safety_blocked_reason, AgeObservation, CacheArtifactKind,
    CacheManagementMode, CacheMetadata, CacheSizeSemantics, CacheUsageConfidence, CategoryResult,
    CleanupDisposition, CleanupEligibility, CleanupOwnership, CleanupUnit, CleanupUnitIdentity,
    CleanupUnitKind, DispositionFacts, EligibilityBucket, EligibilityGate, EligibilitySummary,
    FileSize, OwnershipConfidence, PathIdentity, ScanItem, ScanResult, StaleEntryObservation,
};
pub use zenith_core::domain::storage::{
    AppInstallSource, AppRelatedConfidence, AppRelatedKind, LargeFileFilter, LargeFileKind,
};
