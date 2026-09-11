//! Native platform capability provider selected at the composition boundary.

#[cfg(target_os = "windows")]
use super::SecurityPolicyState;
use super::{PlatformCapabilitiesProvider, RuntimeEnvironment};
use crate::models::PlatformCapabilities;
#[cfg(target_os = "windows")]
use crate::models::PlatformFeatureCapability;

#[derive(Debug, Clone, Copy, Default)]
pub struct NativePlatformCapabilities;

impl NativePlatformCapabilities {
    pub fn new() -> Self {
        Self
    }

    /// Builds the capability snapshot from the runtime environment probe so a
    /// feature that this machine cannot perform reports why, distinctly from a
    /// feature Zenith has not implemented.
    pub fn runtime_capabilities(environment: &RuntimeEnvironment) -> PlatformCapabilities {
        Self::runtime_capabilities_with_container_cli(
            environment,
            crate::docker::container_cli_detected(),
        )
    }

    #[cfg(target_os = "windows")]
    pub fn runtime_capabilities_with_container_cli(
        environment: &RuntimeEnvironment,
        container_cli_detected: bool,
    ) -> PlatformCapabilities {
        let mut capabilities = PlatformCapabilities::current();

        if environment.controlled_folder_access == SecurityPolicyState::Enabled {
            capabilities.large_files = PlatformFeatureCapability::read_only(
                "Controlled Folder Access is enabled, so Zenith cannot move reviewed files to the Recycle Bin. Allow Zenith under Windows Security > Virus & threat protection > Ransomware protection, or disable Controlled Folder Access.",
            );
        }
        if environment.application_control_policy == SecurityPolicyState::Enabled {
            capabilities.system_actions = PlatformFeatureCapability::read_only(
                "An application control policy (Smart App Control or WDAC) is enforced, so helper programs launched by Zenith may be blocked. Relax the policy or allow Zenith's helpers before relying on this feature.",
            );
        }
        if !container_cli_detected {
            capabilities.docker = PlatformFeatureCapability::unavailable(
                "No Docker-compatible CLI (docker or podman) was detected in PATH or known tool locations. Install Docker, Podman, or Rancher Desktop to enable container cleanup.",
            );
        }

        capabilities
    }

    #[cfg(not(target_os = "windows"))]
    pub fn runtime_capabilities_with_container_cli(
        _environment: &RuntimeEnvironment,
        _container_cli_detected: bool,
    ) -> PlatformCapabilities {
        PlatformCapabilities::current()
    }
}

impl PlatformCapabilitiesProvider for NativePlatformCapabilities {
    fn capabilities(&self) -> PlatformCapabilities {
        Self::runtime_capabilities(crate::platform::environment::current())
    }
}

#[cfg(test)]
mod tests {
    use super::NativePlatformCapabilities;
    use crate::platform::RuntimeEnvironment;

    #[test]
    fn probe_driven_capabilities_mark_unimplemented_windows_features_unavailable() {
        let environment = RuntimeEnvironment::probe(None);
        let capabilities = NativePlatformCapabilities::runtime_capabilities(&environment);

        if cfg!(target_os = "windows") {
            assert!(capabilities
                .app_uninstall
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("Zenith")));
            assert!(capabilities
                .intensive_cleanup
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("Zenith")));
        }
    }

    #[test]
    fn podman_only_environments_keep_container_capability_supported() {
        let environment = RuntimeEnvironment::probe(None);
        let with_podman =
            NativePlatformCapabilities::runtime_capabilities_with_container_cli(&environment, true);
        let without_cli = NativePlatformCapabilities::runtime_capabilities_with_container_cli(
            &environment,
            false,
        );

        if cfg!(target_os = "windows") {
            assert!(with_podman.docker.is_available());
            assert!(!without_cli.docker.is_available());
        } else {
            assert!(with_podman.docker.is_available());
            assert!(without_cli.docker.is_available());
        }
    }
}
