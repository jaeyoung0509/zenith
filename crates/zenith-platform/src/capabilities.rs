//! Native platform capability provider selected at the composition boundary.

use std::sync::Arc;

use super::{
    PlatformCapabilitiesProvider, PlatformEnvironment, RuntimeEnvironment, SecurityPolicyState,
};
use zenith_core::domain::platform::{
    PlatformCapabilities, PlatformFeatureCapability, PlatformFeatureStatus,
};

/// Whether a Docker-compatible CLI is installed in the stated environment.
///
/// Tool resolution — PATH plus the version-manager and package-manager install
/// roots — belongs to the tooling layer rather than to platform probing, so the
/// answer arrives as a probe instead of this crate reaching for it. Capability
/// reporting and the container adapter therefore answer the same question from
/// the same source, and a test can state the answer without a host.
pub type ContainerCliProbe = Arc<dyn Fn(&PlatformEnvironment) -> bool + Send + Sync>;

pub struct NativePlatformCapabilities {
    environment: Arc<PlatformEnvironment>,
    container_cli: ContainerCliProbe,
}

impl NativePlatformCapabilities {
    /// The provider is built at the composition boundary, which owns the
    /// environment description and the container CLI probe; nothing here reads
    /// the process environment.
    pub fn new(environment: Arc<PlatformEnvironment>, container_cli: ContainerCliProbe) -> Self {
        Self {
            environment,
            container_cli,
        }
    }

    /// The runtime snapshot, with the container CLI outcome stated explicitly.
    ///
    /// The downgrades below depend only on what the environment reports, not on
    /// the compiled target, so a `read_only` result produced by Controlled
    /// Folder Access or an application-control policy is reproducible in a test
    /// on any host. A feature the platform does not implement at all keeps its
    /// stronger "unsupported" reason instead of being reported as merely
    /// read-only or as missing a CLI.
    pub fn runtime_capabilities(
        environment: &RuntimeEnvironment,
        container_cli_detected: bool,
    ) -> PlatformCapabilities {
        let mut capabilities = PlatformCapabilities::current();

        if environment.controlled_folder_access == SecurityPolicyState::Enabled
            && capabilities.large_files.status == PlatformFeatureStatus::Available
        {
            capabilities.large_files = PlatformFeatureCapability::read_only(
                "Controlled Folder Access is enabled, so Zenith cannot move reviewed files to the Recycle Bin. Allow Zenith under Windows Security > Virus & threat protection > Ransomware protection, or disable Controlled Folder Access.",
            );
        }
        if environment.application_control_policy == SecurityPolicyState::Enabled
            && capabilities.system_actions.status == PlatformFeatureStatus::Available
        {
            capabilities.system_actions = PlatformFeatureCapability::read_only(
                "An application control policy (Smart App Control or WDAC) is enforced, so helper programs launched by Zenith may be blocked. Relax the policy or allow Zenith's helpers before relying on this feature.",
            );
        }
        if !container_cli_detected && capabilities.docker.status == PlatformFeatureStatus::Available
        {
            capabilities.docker = PlatformFeatureCapability::unavailable(
                "No Docker-compatible CLI (docker or podman) was detected in PATH or known tool locations. Install Docker, Podman, or Rancher Desktop to enable container cleanup.",
            );
        }

        capabilities
    }
}

impl PlatformCapabilitiesProvider for NativePlatformCapabilities {
    fn capabilities(&self) -> PlatformCapabilities {
        Self::runtime_capabilities(
            crate::environment::current(),
            (self.container_cli)(&self.environment),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::NativePlatformCapabilities;
    use crate::path_algebra::PathFlavor;
    use crate::{PlatformEnvironment, RuntimeEnvironment, SecurityPolicyState};
    use zenith_core::domain::platform::{
        CapabilityAccess, PlatformCapabilities, PlatformFeature, PlatformFeatureStatus,
    };

    fn simulated_platform() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::current())
    }

    #[test]
    fn the_provider_reports_container_support_from_the_injected_probe() {
        use crate::PlatformCapabilitiesProvider;

        // The probe is the single source of the container answer: a machine
        // that states no container CLI must not advertise container cleanup,
        // and the reason must say what is missing.
        let environment = Arc::new(simulated_platform());
        let without = NativePlatformCapabilities::new(environment.clone(), Arc::new(|_| false))
            .capabilities();
        assert_eq!(without.docker.status, PlatformFeatureStatus::Unavailable);
        assert!(
            without
                .docker
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("No Docker-compatible CLI")),
            "reason: {:?}",
            without.docker.reason
        );

        // A probe that finds a CLI enables it: the adapter has something to run.
        let with = NativePlatformCapabilities::new(environment, Arc::new(|_| true)).capabilities();
        assert!(
            with.docker.is_available(),
            "a detected container CLI must enable container cleanup"
        );
        assert!(with.docker.reason.is_none());
    }

    #[test]
    fn a_feature_without_an_implementing_adapter_is_unavailable_and_keeps_its_reason() {
        let environment = RuntimeEnvironment::probe(None);
        let base = NativePlatformCapabilities::runtime_capabilities(&environment, true);
        let without_cli = NativePlatformCapabilities::runtime_capabilities(&environment, false);

        // Unimplemented Windows features are a property of the snapshot itself,
        // not of this machine's probe. Intensive cleanup used to be one of them;
        // it now has Windows signatures and a Windows adapter behind the same
        // capability, so it is asserted as available where that is the case.
        let windows = PlatformCapabilities::windows();
        assert_eq!(
            windows.feature(PlatformFeature::IntensiveCleanup).status,
            PlatformFeatureStatus::Available
        );
        for feature in [
            PlatformFeature::InstalledApps,
            PlatformFeature::AppUninstall,
        ] {
            assert_eq!(
                windows.feature(feature).status,
                PlatformFeatureStatus::Unavailable
            );
            assert!(windows.feature(feature).reason.is_some());
        }

        // A container CLI that is not present leaves no adapter to run.
        assert!(!without_cli.docker.is_available());
        if base.docker.is_available() {
            let reason = without_cli
                .docker
                .reason
                .as_deref()
                .expect("a missing CLI must explain itself");
            assert!(
                reason.contains("No Docker-compatible CLI"),
                "unexpected reason: {reason}"
            );
        } else {
            // The platform does not implement containers at all; that stronger
            // reason must survive rather than be replaced by a CLI hint.
            assert_eq!(without_cli.docker.status, base.docker.status);
        }
    }

    #[test]
    fn a_runtime_policy_downgrade_reports_read_only_with_its_reason() {
        let mut environment = RuntimeEnvironment::probe(None);
        environment.controlled_folder_access = SecurityPolicyState::Enabled;
        environment.application_control_policy = SecurityPolicyState::Enabled;

        let restricted = NativePlatformCapabilities::runtime_capabilities(&environment, true);
        let baseline = NativePlatformCapabilities::runtime_capabilities(
            &RuntimeEnvironment::probe(None),
            true,
        );

        assert_eq!(
            restricted.large_files.status,
            PlatformFeatureStatus::ReadOnly
        );
        let reason = restricted
            .large_files
            .reason
            .as_deref()
            .expect("a downgraded feature must explain itself");
        assert!(
            reason.contains("Controlled Folder Access"),
            "unexpected reason: {reason}"
        );
        // Inspection still works; mutation is refused with the same reason.
        assert!(restricted
            .require(PlatformFeature::LargeFiles, CapabilityAccess::Inspect)
            .is_ok());
        let error = restricted
            .require(PlatformFeature::LargeFiles, CapabilityAccess::Mutate)
            .expect_err("a read-only feature must refuse mutation");
        assert!(error.to_string().contains("read-only"));

        assert_eq!(
            restricted.system_actions.status,
            PlatformFeatureStatus::ReadOnly
        );
        assert!(restricted
            .system_actions
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("application control policy")));

        // The policy downgrades only the affected features.
        assert_eq!(restricted.cleanup, baseline.cleanup);
        assert_eq!(restricted.local_models, baseline.local_models);
    }
}
