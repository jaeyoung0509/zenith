//! Zenith's platform layer: every native OS integration behind a narrow port.
//!
//! The desktop crate depends on this crate; this crate never depends on the
//! desktop crate or on Tauri. That direction is the point of the layer: a
//! module that owns WebView IPC, tray or window lifecycle, or desktop
//! composition lives in `zenith-desktop`, and a module that reads a Windows
//! registry value, resolves a known folder, terminates a process, reveals a
//! path in Finder, or moves a file to the Trash lives here.
//!
//! Two rules keep the layer honest:
//!
//! * Product semantics stay in `zenith-core`. This crate asks for the
//!   capability snapshot and the platform vocabulary from there and adds the
//!   native probing that produces them.
//! * Windows behavior is described, not host-dependent. Where a rule can be
//!   expressed as a pure function of a stated [`path_algebra::PathFlavor`], it
//!   is, so a macOS runner asserts the same Windows rule a Windows runner does
//!   instead of only compiling it.

pub mod capabilities;
pub mod description;
pub mod environment;
pub mod file_ops;
pub mod path_algebra;
pub mod paths;
pub mod process;
pub mod subprocess;
pub mod system_actions;
pub mod trash;

pub use capabilities::NativePlatformCapabilities;
pub use description::{
    EnvironmentFixture, EnvironmentShape, KnownFolder, PlatformEnvironment, ProfileShape,
    ToolResolution, VolumeIdentity,
};
pub use environment::{RuntimeEnvironment, SecurityPolicyState};
pub use path_algebra::PathFlavor;
pub use paths::{NativePlatformPaths, PlatformPathsProvider};
pub use process::{request_graceful_stop, terminate_process, GracefulStopOutcome, TerminationMode};
pub use subprocess::{run_with_timeout, run_with_timeout_async, set_error_sink, SubprocessError};
pub use system_actions::{NativeSystemActions, SystemActionProvider};
pub use trash::{MockTrashBackend, NativeTrashBackend, ReviewedTrashEntry, TrashBackend};

use zenith_core::domain::platform::PlatformCapabilities;

/// Narrow provider boundary for platform capability discovery.
///
/// More specific providers (paths, system actions, process lifecycle, and
/// filesystem safety) are separate traits, so no caller receives one object
/// that can do everything to a machine.
pub trait PlatformCapabilitiesProvider: Send + Sync {
    fn capabilities(&self) -> PlatformCapabilities;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{NativePlatformCapabilities, PathFlavor, PlatformCapabilitiesProvider};
    use zenith_core::domain::platform::{
        PlatformCapabilities, PlatformFeatureStatus, PlatformKind,
    };

    struct MockCapabilitiesProvider(PlatformCapabilities);

    impl PlatformCapabilitiesProvider for MockCapabilitiesProvider {
        fn capabilities(&self) -> PlatformCapabilities {
            self.0.clone()
        }
    }

    #[test]
    fn native_provider_reports_the_compiled_platform() {
        let capabilities = NativePlatformCapabilities::new(
            Arc::new(super::PlatformEnvironment::simulated(PathFlavor::current())),
            Arc::new(|_| true),
        )
        .capabilities();

        #[cfg(target_os = "macos")]
        assert_eq!(capabilities.platform, PlatformKind::Macos);
        #[cfg(target_os = "windows")]
        assert_eq!(capabilities.platform, PlatformKind::Windows);
        #[cfg(target_os = "linux")]
        assert_eq!(capabilities.platform, PlatformKind::Linux);
    }

    #[test]
    fn capability_provider_can_be_injected_with_a_deterministic_mock() {
        let provider = MockCapabilitiesProvider(PlatformCapabilities::windows());
        let capabilities = provider.capabilities();

        assert_eq!(capabilities.platform, PlatformKind::Windows);
        assert_eq!(
            capabilities.app_uninstall.status,
            PlatformFeatureStatus::Unavailable
        );
    }
}
