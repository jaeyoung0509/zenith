//! Cleanup domain: what may be deleted, and the authority that says so.
//!
//! [`plan::DeletePlan`] is authorization state, not a frontend contract. It is
//! deliberately free of `serde` and `specta` derives: the only way a plan
//! reaches the interface is through the projections in
//! [`crate::application::dto::cleanup`], which carry item IDs and byte totals
//! and nothing a caller could replay into a mutation.

pub mod operation;
pub mod owner;
pub mod plan;
pub mod provider;
pub mod strategy;
pub mod structured;

pub use operation::{
    CleanupOperation, ContainerCleanup, FilesystemCleanup, FilesystemMutation,
    LifecycleProviderCleanup, ProviderCleanup,
};
pub use owner::{
    OwnerProviderAuthorization, OwnerProviderExecution, OwnerProviderRefusal,
    OwnerProviderSelection, OwnerProviderUnit, OwnerStoreObservation, OwnerUnitMeasurement,
    OwnerUnitMeasurer, OwnerUnitObservation, OwnerUnitOutcome, OwnerUnitRefusal, OwnerUnitState,
    RunningProcessProbe,
};
pub use plan::{
    CleanFailureReason, CleanupMode, DeletePlan, DeleteTarget, PlanItemRefusal,
    RunningProcessPolicy,
};
pub use provider::{ProviderOutcome, ProviderProbe, ProviderStatus};
pub use strategy::CleanStrategy;
pub use structured::{classify_structured_state, EntryKind, PathFacts, StructuredStateKind};
