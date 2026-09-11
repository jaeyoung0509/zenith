use crate::models::PlatformCapabilities;

pub mod capabilities;
pub mod environment;
pub mod paths;
pub mod process;
pub mod system_actions;

pub use capabilities::NativePlatformCapabilities;
pub use environment::{RuntimeEnvironment, SecurityPolicyState};
pub use paths::{NativePlatformPaths, PlatformPathsProvider};
pub use process::{terminate_process, TerminationMode};
pub use system_actions::{NativeSystemActions, SystemActionProvider};

/// Narrow provider boundary for platform capability discovery.
///
/// More specific providers (paths, system actions, process lifecycle, and
/// filesystem safety) are introduced by their owning domain issues. Keeping
/// this provider separate from those services prevents a god-object platform
/// abstraction and gives tests a deterministic injection point now.
pub trait PlatformCapabilitiesProvider: Send + Sync {
    fn capabilities(&self) -> PlatformCapabilities;
}

#[cfg(test)]
mod tests {
    use super::{NativePlatformCapabilities, PlatformCapabilitiesProvider};
    use crate::models::{PlatformCapabilities, PlatformKind};

    struct MockCapabilitiesProvider(PlatformCapabilities);

    impl PlatformCapabilitiesProvider for MockCapabilitiesProvider {
        fn capabilities(&self) -> PlatformCapabilities {
            self.0.clone()
        }
    }

    #[test]
    fn native_provider_reports_the_compiled_platform() {
        let capabilities = NativePlatformCapabilities::new().capabilities();

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
            crate::models::PlatformFeatureStatus::Unavailable
        );
    }
}
