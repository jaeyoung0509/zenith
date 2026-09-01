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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum PlatformKind {
    Macos,
    Windows,
    Linux,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PlatformCapabilities {
    pub platform: PlatformKind,
    pub system_actions: PlatformFeatureCapability,
    pub cleanup: PlatformFeatureCapability,
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
    pub fn macos() -> Self {
        Self {
            platform: PlatformKind::Macos,
            system_actions: PlatformFeatureCapability::available(),
            cleanup: PlatformFeatureCapability::available(),
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

    /// Returns the honest baseline before a platform adapter is implemented.
    ///
    /// Windows can still build and launch the shell at this stage, but native
    /// operations are deliberately unavailable until their dedicated issues
    /// land. Portable memory/disk inspection is read-only and therefore safe
    /// to expose once the corresponding command is exercised on Windows.
    pub fn windows_baseline() -> Self {
        let unavailable = || {
            PlatformFeatureCapability::unavailable(
                "Windows adapter is not implemented in this build.",
            )
        };

        Self {
            platform: PlatformKind::Windows,
            system_actions: unavailable(),
            cleanup: unavailable(),
            large_files: unavailable(),
            developer_artifacts: unavailable(),
            installed_apps: unavailable(),
            app_uninstall: unavailable(),
            memory_metrics: PlatformFeatureCapability::read_only(
                "Memory metrics are available; process actions are not yet ported.",
            ),
            process_termination: unavailable(),
            development_ports: unavailable(),
            keep_awake: unavailable(),
            local_models: unavailable(),
            docker: PlatformFeatureCapability::read_only(
                "Docker inspection is not yet ported to Windows.",
            ),
            ai_integrations: unavailable(),
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
            Self::windows_baseline()
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
    fn windows_baseline_is_explicitly_limited() {
        let capabilities = PlatformCapabilities::windows_baseline();

        assert_eq!(capabilities.platform, PlatformKind::Windows);
        assert_eq!(
            capabilities.cleanup.status,
            PlatformFeatureStatus::Unavailable
        );
        assert_eq!(
            capabilities.memory_metrics.status,
            PlatformFeatureStatus::ReadOnly
        );
        assert!(!capabilities.process_termination.is_available());
    }

    #[test]
    fn capability_serialization_omits_missing_reason_only_when_available() {
        let json = serde_json::to_value(PlatformCapabilities::macos()).unwrap();
        assert_eq!(json["system_actions"]["status"], "available");
        assert!(json["system_actions"].get("reason").is_none());

        let windows = serde_json::to_value(PlatformCapabilities::windows_baseline()).unwrap();
        assert_eq!(windows["cleanup"]["status"], "unavailable");
        assert!(windows["cleanup"].get("reason").is_some());
    }

    #[test]
    fn default_uses_the_compiled_platform_contract() {
        assert_eq!(
            PlatformCapabilities::default().platform,
            PlatformCapabilities::current().platform
        );
    }
}
