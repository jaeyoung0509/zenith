use serde::{Deserialize, Serialize};

/// Describes whether a platform-sensitive feature can be used safely.
/// `ReadOnly` is intentionally distinct from `Available`: a platform can
/// expose inspection while withholding the destructive or mutating action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum PlatformFeatureStatus {
    Available,
    ReadOnly,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PlatformFeatureCapability {
    pub status: PlatformFeatureStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl PlatformFeatureCapability {
    pub fn available() -> Self {
        Self {
            status: PlatformFeatureStatus::Available,
            reason: None,
        }
    }

    pub fn read_only(reason: impl Into<String>) -> Self {
        Self {
            status: PlatformFeatureStatus::ReadOnly,
            reason: Some(reason.into()),
        }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            status: PlatformFeatureStatus::Unavailable,
            reason: Some(reason.into()),
        }
    }

    pub fn is_available(&self) -> bool {
        matches!(self.status, PlatformFeatureStatus::Available)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum PlatformKind {
    Macos,
    Windows,
    Linux,
    Other,
}

impl PlatformKind {
    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::Macos
        }
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(target_os = "linux")]
        {
            Self::Linux
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Self::Other
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum PlatformFeature {
    SystemActions,
    Cleanup,
    IntensiveCleanup,
    LargeFiles,
    DeveloperArtifacts,
    InstalledApps,
    AppUninstall,
    MemoryMetrics,
    ProcessTermination,
    DevelopmentPorts,
    KeepAwake,
    LocalModels,
    Docker,
    AiIntegrations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityAccess {
    Inspect,
    Mutate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformCapabilityError {
    ReadOnly(PlatformFeature),
    Unavailable {
        feature: PlatformFeature,
        reason: Option<String>,
    },
}

impl std::fmt::Display for PlatformCapabilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadOnly(feature) => {
                write!(
                    f,
                    "Feature {feature:?} is read-only on this platform; mutation is not permitted."
                )
            }
            Self::Unavailable { feature, reason } => {
                if let Some(r) = reason {
                    write!(
                        f,
                        "Feature {feature:?} is unavailable on this platform: {r}"
                    )
                } else {
                    write!(f, "Feature {feature:?} is unavailable on this platform.")
                }
            }
        }
    }
}

impl std::error::Error for PlatformCapabilityError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PlatformCapabilities {
    pub platform: PlatformKind,
    pub system_actions: PlatformFeatureCapability,
    pub cleanup: PlatformFeatureCapability,
    pub intensive_cleanup: PlatformFeatureCapability,
    pub large_files: PlatformFeatureCapability,
    pub developer_artifacts: PlatformFeatureCapability,
    pub installed_apps: PlatformFeatureCapability,
    pub app_uninstall: PlatformFeatureCapability,
    pub memory_metrics: PlatformFeatureCapability,
    pub process_termination: PlatformFeatureCapability,
    pub development_ports: PlatformFeatureCapability,
    pub keep_awake: PlatformFeatureCapability,
    pub local_models: PlatformFeatureCapability,
    pub docker: PlatformFeatureCapability,
    pub ai_integrations: PlatformFeatureCapability,
}

impl PlatformCapabilities {
    pub fn feature(&self, feature: PlatformFeature) -> &PlatformFeatureCapability {
        match feature {
            PlatformFeature::SystemActions => &self.system_actions,
            PlatformFeature::Cleanup => &self.cleanup,
            PlatformFeature::IntensiveCleanup => &self.intensive_cleanup,
            PlatformFeature::LargeFiles => &self.large_files,
            PlatformFeature::DeveloperArtifacts => &self.developer_artifacts,
            PlatformFeature::InstalledApps => &self.installed_apps,
            PlatformFeature::AppUninstall => &self.app_uninstall,
            PlatformFeature::MemoryMetrics => &self.memory_metrics,
            PlatformFeature::ProcessTermination => &self.process_termination,
            PlatformFeature::DevelopmentPorts => &self.development_ports,
            PlatformFeature::KeepAwake => &self.keep_awake,
            PlatformFeature::LocalModels => &self.local_models,
            PlatformFeature::Docker => &self.docker,
            PlatformFeature::AiIntegrations => &self.ai_integrations,
        }
    }

    pub fn require(
        &self,
        feature: PlatformFeature,
        access: CapabilityAccess,
    ) -> Result<(), PlatformCapabilityError> {
        let capability = self.feature(feature);
        match (capability.status, access) {
            (PlatformFeatureStatus::Available, _) => Ok(()),
            (PlatformFeatureStatus::ReadOnly, CapabilityAccess::Inspect) => Ok(()),
            (PlatformFeatureStatus::ReadOnly, CapabilityAccess::Mutate) => {
                Err(PlatformCapabilityError::ReadOnly(feature))
            }
            (PlatformFeatureStatus::Unavailable, _) => Err(PlatformCapabilityError::Unavailable {
                feature,
                reason: capability.reason.clone(),
            }),
        }
    }
    pub fn macos() -> Self {
        Self {
            platform: PlatformKind::Macos,
            system_actions: PlatformFeatureCapability::available(),
            cleanup: PlatformFeatureCapability::available(),
            intensive_cleanup: PlatformFeatureCapability::available(),
            large_files: PlatformFeatureCapability::available(),
            developer_artifacts: PlatformFeatureCapability::available(),
            installed_apps: PlatformFeatureCapability::available(),
            app_uninstall: PlatformFeatureCapability::available(),
            memory_metrics: PlatformFeatureCapability::available(),
            process_termination: PlatformFeatureCapability::available(),
            development_ports: PlatformFeatureCapability::available(),
            keep_awake: PlatformFeatureCapability::available(),
            local_models: PlatformFeatureCapability::available(),
            docker: PlatformFeatureCapability::available(),
            ai_integrations: PlatformFeatureCapability::available(),
        }
    }

    /// Returns the capability snapshot for the Windows platform.
    pub fn windows() -> Self {
        Self {
            platform: PlatformKind::Windows,
            system_actions: PlatformFeatureCapability::available(),
            cleanup: PlatformFeatureCapability::available(),
            intensive_cleanup: PlatformFeatureCapability::unavailable(
                "Zenith does not implement intensive cleanup on Windows yet; no Windows-specific intensive signatures are defined.",
            ),
            large_files: PlatformFeatureCapability::available(),
            developer_artifacts: PlatformFeatureCapability::available(),
            installed_apps: PlatformFeatureCapability::unavailable(
                "Zenith does not implement Windows application inventory yet; registry uninstall keys exist but are not read.",
            ),
            app_uninstall: PlatformFeatureCapability::unavailable(
                "Zenith does not implement Windows application uninstallation yet; the registry UninstallString exists but is not executed.",
            ),
            memory_metrics: PlatformFeatureCapability::available(),
            process_termination: PlatformFeatureCapability::available(),
            development_ports: PlatformFeatureCapability::available(),
            keep_awake: PlatformFeatureCapability::available(),
            local_models: PlatformFeatureCapability::available(),
            docker: PlatformFeatureCapability::available(),
            ai_integrations: PlatformFeatureCapability::available(),
        }
    }

    pub fn unsupported(kind: PlatformKind) -> Self {
        let unavailable = || {
            PlatformFeatureCapability::unavailable(
                "This platform is not supported by the desktop application.",
            )
        };

        Self {
            platform: kind,
            system_actions: unavailable(),
            cleanup: unavailable(),
            intensive_cleanup: unavailable(),
            large_files: unavailable(),
            developer_artifacts: unavailable(),
            installed_apps: unavailable(),
            app_uninstall: unavailable(),
            memory_metrics: unavailable(),
            process_termination: unavailable(),
            development_ports: unavailable(),
            keep_awake: unavailable(),
            local_models: unavailable(),
            docker: unavailable(),
            ai_integrations: unavailable(),
        }
    }

    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::macos()
        }

        #[cfg(target_os = "windows")]
        {
            Self::windows()
        }

        #[cfg(target_os = "linux")]
        {
            Self::unsupported(PlatformKind::Linux)
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Self::unsupported(PlatformKind::Other)
        }
    }
}

impl Default for PlatformCapabilities {
    fn default() -> Self {
        Self::current()
    }
}

#[cfg(test)]
mod tests {
    use super::{PlatformCapabilities, PlatformFeatureStatus, PlatformKind};

    #[test]
    fn windows_capabilities_are_honest() {
        let capabilities = PlatformCapabilities::windows();

        assert_eq!(capabilities.platform, PlatformKind::Windows);
        assert_eq!(
            capabilities.cleanup.status,
            PlatformFeatureStatus::Available
        );
        assert_eq!(
            capabilities.installed_apps.status,
            PlatformFeatureStatus::Unavailable
        );
        assert_eq!(
            capabilities.app_uninstall.status,
            PlatformFeatureStatus::Unavailable
        );
        assert_eq!(
            capabilities.intensive_cleanup.status,
            PlatformFeatureStatus::Unavailable
        );
    }

    #[test]
    fn capability_serialization_omits_missing_reason_only_when_available() {
        let json = serde_json::to_value(PlatformCapabilities::macos()).unwrap();
        assert_eq!(json["system_actions"]["status"], "available");
        assert!(json["system_actions"].get("reason").is_none());

        let windows = serde_json::to_value(PlatformCapabilities::windows()).unwrap();
        assert_eq!(windows["app_uninstall"]["status"], "unavailable");
        assert!(windows["app_uninstall"].get("reason").is_some());
        assert_eq!(windows["installed_apps"]["status"], "unavailable");
        assert!(windows["installed_apps"].get("reason").is_some());
        assert_eq!(windows["intensive_cleanup"]["status"], "unavailable");
        assert!(windows["intensive_cleanup"].get("reason").is_some());
    }

    #[test]
    fn default_uses_the_compiled_platform_contract() {
        assert_eq!(
            PlatformCapabilities::default().platform,
            PlatformCapabilities::current().platform
        );
    }

    #[test]
    fn require_enforces_platform_feature_and_access() {
        use super::{
            CapabilityAccess, PlatformCapabilityError, PlatformFeature, PlatformFeatureCapability,
        };

        let windows = PlatformCapabilities::windows();

        // Available feature permits both Inspect and Mutate
        assert!(windows
            .require(PlatformFeature::Cleanup, CapabilityAccess::Inspect)
            .is_ok());
        assert!(windows
            .require(PlatformFeature::Cleanup, CapabilityAccess::Mutate)
            .is_ok());

        // Unavailable feature rejects both Inspect and Mutate
        assert!(windows
            .require(PlatformFeature::InstalledApps, CapabilityAccess::Inspect)
            .is_err());
        assert!(windows
            .require(PlatformFeature::InstalledApps, CapabilityAccess::Mutate)
            .is_err());
        assert!(windows
            .require(PlatformFeature::IntensiveCleanup, CapabilityAccess::Inspect)
            .is_err());

        // ReadOnly capability test
        let mut readonly_caps = PlatformCapabilities::macos();
        readonly_caps.cleanup = PlatformFeatureCapability::read_only("Disk is read-only.");
        assert!(readonly_caps
            .require(PlatformFeature::Cleanup, CapabilityAccess::Inspect)
            .is_ok());
        let err = readonly_caps
            .require(PlatformFeature::Cleanup, CapabilityAccess::Mutate)
            .unwrap_err();
        assert_eq!(
            err,
            PlatformCapabilityError::ReadOnly(PlatformFeature::Cleanup)
        );
    }
}
