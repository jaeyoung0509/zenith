//! Zenith's product semantics, independent of the desktop framework.
//!
//! This crate answers one question: *would this code still make sense if
//! Zenith had a CLI instead of a Tauri window?* Scanning, cleanup safety,
//! storage policy, capability description, and the DTOs the interface is
//! allowed to see all say yes, so they live here. Webview IPC, tray and window
//! lifecycle, capability grants, and desktop composition all say no, so they
//! stay in `zenith-desktop`.
//!
//! The boundary is enforced rather than documented. `zenith-core` must not
//! resolve `tauri`, `tauri-build`, `tauri-plugin-*`, `windows-sys`,
//! `security-framework`, or `rfd` — directly or transitively — and
//! `scripts/check_core_boundaries.cjs` reads `cargo metadata` to prove it. A
//! desktop framework that reached back into the domain would make every
//! non-desktop consumer of this crate impossible, which is the whole reason
//! the split exists.
//!
//! ```text
//! zenith-core
//!     ↑
//!     ├──────── zenith-platform   (next architecture issue)
//!     │
//! zenith-desktop / src-tauri
//! ```
//!
//! [`domain`] holds the semantics; [`application`] holds the serializable
//! projections of them. [`ipc_numeric`] is the one shared wire rule the
//! projections depend on: a `u64` that crosses the boundary is checked against
//! `Number.MAX_SAFE_INTEGER` on both sides of the call.

pub mod application;
pub mod domain;
pub mod ipc_numeric;
