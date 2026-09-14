//! Zenith Platform Adapters
//!
//! Native OS integrations for macOS and Windows behind narrow domain ports.

pub mod trash;

pub use trash::{MockTrashBackend, NativeTrashBackend, TrashBackend};
