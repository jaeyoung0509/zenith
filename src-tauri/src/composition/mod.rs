//! The composition root.
//!
//! This is the only layer allowed to name every concrete implementation and
//! decide what the process's object graph is: which environment the backend is
//! described by, which catalog the scan uses, which native adapter answers a
//! port, and which shared handle two services deliberately hold together.
//!
//! Nothing below this layer constructs a `PlatformEnvironment`, reads a
//! concrete adapter's dependencies from the process environment, or binds a
//! port to an implementation. A handler receives the bounded services the graph
//! produced; a service receives only the handles it needs.

mod desktop;

pub use desktop::{desktop_state, desktop_state_with_catalog};
