//! Cleanup domain: what may be deleted, and the authority that says so.
//!
//! [`plan::DeletePlan`] is authorization state, not a frontend contract. It is
//! deliberately free of `serde` and `specta` derives: the only way a plan
//! reaches the interface is through the projections in
//! [`crate::application::dto::cleanup`], which carry item IDs and byte totals
//! and nothing a caller could replay into a mutation.

pub mod operation;
pub mod plan;
pub mod strategy;

pub use operation::{
    CleanupOperation, ContainerCleanup, FilesystemCleanup, FilesystemMutation, ProviderCleanup,
};
pub use plan::{DeletePlan, DeleteTarget};
pub use strategy::CleanStrategy;
