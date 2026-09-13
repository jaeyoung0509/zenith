//! The contract between the Zenith domain and whoever is presenting it.
//!
//! Types here exist to cross a boundary. They are serializable by design, and
//! they are assembled from domain values rather than being the domain values:
//! a projection can be dropped, logged, or sent to a webview without carrying
//! the authority to change anything on disk.

pub mod dto;

pub use dto::*;
