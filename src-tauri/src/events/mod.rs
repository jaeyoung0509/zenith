//! Tauri transport adapters for the events the application services emit.
//!
//! An application service states what happened and stops there: cleanup and
//! scan progress arrive as `zenith-core` sinks, reviewed-storage and provider
//! progress as the sinks in [`crate::services::progress`]. The adapters here
//! are the only code that knows the transport is a Tauri `Channel` or the
//! notification plugin, which is what keeps the framework out of every service
//! and lets a service be exercised without a Tauri runtime.

pub mod ai;
pub mod cleanup;
pub mod notifications;
pub mod scan;
pub mod storage;
