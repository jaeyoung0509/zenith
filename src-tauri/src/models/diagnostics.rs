use serde::{Deserialize, Serialize};

/// What the diagnostics panel shows about this machine.
///
/// The machine facts are first-class fields rather than prose in
/// `enabled_features`: a report from a machine the maintainers do not own is
/// only useful if the arch, the OS build, the webview runtime, the elevation
/// state, and the locale can be read without parsing a string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DiagnosticsSnapshot {
    pub app_version: String,
    pub os_version: String,
    /// OS build/revision when the platform exposes one.
    pub os_build: Option<String>,
    /// Process architecture: what this binary runs as.
    pub arch: String,
    /// Native architecture, `None` when the platform cannot report it.
    pub native_arch: Option<String>,
    /// True when this process runs under emulation.
    pub emulated: bool,
    pub webview_version: Option<String>,
    pub elevated: Option<bool>,
    pub locale: Option<String>,
    /// `None` while the log is writable; otherwise why it is not. A logger that
    /// disables itself silently removes the only evidence of what disabled it.
    pub log_failure: Option<String>,
    pub log_path: String,
    pub enabled_features: Vec<String>,
    pub recent_errors: Vec<String>,
    pub settings_corrupt_recovered: bool,
}
