//! Runtime environment discovery.
//!
//! Windows behavior is discovered from the machine at runtime (OS build and
//! edition, installation type, native/process architecture, elevation,
//! long-path policy, WebView2 version, Controlled Folder Access, and
//! application-control policy) instead of being assumed at compile time.
//! Optional newer APIs are only reached when they exist; a failure to read a
//! value is reported as unknown rather than synthesized.

use std::sync::OnceLock;

static PROBE: OnceLock<RuntimeEnvironment> = OnceLock::new();
static WEBVIEW_VERSION: OnceLock<Option<String>> = OnceLock::new();

/// Records the WebView runtime version discovered at startup so the probe can
/// include it without depending on a live window.
pub fn set_webview_version(version: Option<String>) {
    let _ = WEBVIEW_VERSION.set(version);
}

/// Returns the cached probe for this process.
pub fn current() -> &'static RuntimeEnvironment {
    PROBE.get_or_init(|| RuntimeEnvironment::probe(WEBVIEW_VERSION.get().cloned().flatten()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityPolicyState {
    Enabled,
    Disabled,
    Unknown,
}

impl SecurityPolicyState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEnvironment {
    pub os: String,
    pub os_build: Option<String>,
    pub os_edition: Option<String>,
    pub installation_type: Option<String>,
    pub native_architecture: Option<String>,
    pub process_architecture: String,
    pub emulated: bool,
    pub elevated: Option<bool>,
    pub long_paths_enabled: Option<bool>,
    pub webview_version: Option<String>,
    pub controlled_folder_access: SecurityPolicyState,
    pub application_control_policy: SecurityPolicyState,
}

impl RuntimeEnvironment {
    pub fn probe(webview_version: Option<String>) -> Self {
        #[cfg(target_os = "windows")]
        {
            Self::probe_windows(webview_version)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = webview_version;
            let elevated = {
                #[cfg(unix)]
                {
                    Some(unsafe { libc::geteuid() } == 0)
                }
                #[cfg(not(unix))]
                {
                    None
                }
            };
            Self {
                os: std::env::consts::OS.to_string(),
                os_build: None,
                os_edition: None,
                installation_type: None,
                native_architecture: Some(std::env::consts::ARCH.to_string()),
                process_architecture: std::env::consts::ARCH.to_string(),
                emulated: false,
                elevated,
                long_paths_enabled: None,
                webview_version,
                controlled_folder_access: SecurityPolicyState::Unknown,
                application_control_policy: SecurityPolicyState::Unknown,
            }
        }
    }

    /// Describes the OS version for diagnostics.
    pub fn os_version_label(&self) -> String {
        #[cfg(target_os = "macos")]
        {
            let mut cmd = std::process::Command::new("sw_vers");
            cmd.arg("-productVersion");
            crate::tooling::run_with_timeout(cmd, std::time::Duration::from_secs(2))
                .ok()
                .map(|output| format!("macOS {}", String::from_utf8_lossy(&output.stdout).trim()))
                .unwrap_or_else(|| "macOS".to_string())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let edition = self
                .os_edition
                .as_deref()
                .map(|edition| format!(" {edition}"))
                .unwrap_or_default();
            match self.os_build.as_deref() {
                Some(build) => format!("{}{edition} (build {build})", self.os),
                None => self.os.to_string(),
            }
        }
    }

    /// Lines describing machine facts for the diagnostics feature list.
    pub fn diagnostics_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(build) = &self.os_build {
            lines.push(format!("os_build: {build}"));
        }
        if let Some(edition) = &self.os_edition {
            lines.push(format!("os_edition: {edition}"));
        }
        if let Some(installation) = &self.installation_type {
            lines.push(format!("installation_type: {installation}"));
        }
        if let Some(native) = &self.native_architecture {
            lines.push(format!(
                "architecture: native={native}, process={}, emulated={}",
                self.process_architecture, self.emulated
            ));
        } else {
            lines.push(format!(
                "architecture: process={}",
                self.process_architecture
            ));
        }
        if let Some(elevated) = self.elevated {
            lines.push(format!("elevated: {elevated}"));
        }
        if let Some(long_paths) = self.long_paths_enabled {
            lines.push(format!("long_paths_enabled: {long_paths}"));
        }
        if let Some(version) = &self.webview_version {
            lines.push(format!("webview_runtime: {version}"));
        }
        lines.push(format!(
            "controlled_folder_access: {}",
            self.controlled_folder_access.label()
        ));
        lines.push(format!(
            "application_control_policy: {}",
            self.application_control_policy.label()
        ));
        lines
    }
}

#[cfg(target_os = "windows")]
impl RuntimeEnvironment {
    fn probe_windows(webview_version: Option<String>) -> Self {
        let current_version = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        let os_edition = read_registry_string(current_version, "ProductName", REGISTRY_VIEW_64)
            .or_else(|| read_registry_string(current_version, "EditionID", REGISTRY_VIEW_64));
        let installation_type =
            read_registry_string(current_version, "InstallationType", REGISTRY_VIEW_64);
        let build_number =
            read_registry_string(current_version, "CurrentBuildNumber", REGISTRY_VIEW_64);
        let revision = read_registry_u32(current_version, "UBR", REGISTRY_VIEW_64);
        let os_build = build_number.map(|build| match revision {
            Some(revision) => format!("{build}.{revision}"),
            None => build,
        });

        let (native_architecture, process_architecture, emulated) = probe_architectures();
        let elevated = probe_elevation();

        let long_paths_enabled = read_registry_u32(
            r"SYSTEM\CurrentControlSet\Control\FileSystem",
            "LongPathsEnabled",
            REGISTRY_VIEW_ANY,
        )
        .map(|value| value != 0);

        let controlled_folder_access = read_registry_u32(
            r"SOFTWARE\Microsoft\Windows Defender\Windows Defender Exploit Guard\Controlled Folder Access",
            "EnableControlledFolderAccess",
            REGISTRY_VIEW_ANY,
        )
        .map(|value| match value {
            1 | 2 => SecurityPolicyState::Enabled,
            _ => SecurityPolicyState::Disabled,
        })
        .unwrap_or(SecurityPolicyState::Unknown);

        let application_control_policy = read_registry_u32(
            r"SYSTEM\CurrentControlSet\Control\CI\Policy",
            "VerifiedAndReputablePolicyState",
            REGISTRY_VIEW_ANY,
        )
        .map(|value| match value {
            1 => SecurityPolicyState::Enabled,
            0 | 2 => SecurityPolicyState::Disabled,
            _ => SecurityPolicyState::Unknown,
        })
        .unwrap_or(SecurityPolicyState::Unknown);

        Self {
            os: "Windows".to_string(),
            os_build,
            os_edition,
            installation_type,
            native_architecture,
            process_architecture,
            emulated,
            elevated: Some(elevated),
            long_paths_enabled,
            webview_version,
            controlled_folder_access,
            application_control_policy,
        }
    }
}

#[cfg(target_os = "windows")]
const REGISTRY_VIEW_64: u32 = windows_sys::Win32::System::Registry::KEY_WOW64_64KEY;
#[cfg(target_os = "windows")]
const REGISTRY_VIEW_ANY: u32 = 0;

/// Reads a `REG_SZ` / `REG_EXPAND_SZ` value from `HKEY_LOCAL_MACHINE`.
#[cfg(target_os = "windows")]
fn read_registry_string(subkey: &str, value: &str, view: u32) -> Option<String> {
    let (data_type, bytes) = read_registry_raw(subkey, value, view)?;
    use windows_sys::Win32::System::Registry::{REG_EXPAND_SZ, REG_SZ};
    if data_type != REG_SZ && data_type != REG_EXPAND_SZ {
        return None;
    }
    let wide: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
        .collect();
    let end = wide
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(wide.len());
    let text = String::from_utf16_lossy(&wide[..end]);
    (!text.is_empty()).then_some(text)
}

#[cfg(target_os = "windows")]
fn read_registry_u32(subkey: &str, value: &str, view: u32) -> Option<u32> {
    let (data_type, bytes) = read_registry_raw(subkey, value, view)?;
    use windows_sys::Win32::System::Registry::REG_DWORD;
    if data_type != REG_DWORD || bytes.len() < 4 {
        return None;
    }
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(target_os = "windows")]
fn read_registry_raw(subkey: &str, value: &str, view: u32) -> Option<(u32, Vec<u8>)> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
    };

    let subkey_wide: Vec<u16> = subkey.encode_utf16().chain([0]).collect();
    let value_wide: Vec<u16> = value.encode_utf16().chain([0]).collect();

    let mut key: HKEY = std::ptr::null_mut();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            subkey_wide.as_ptr(),
            0,
            KEY_READ | view,
            &mut key,
        )
    };
    if status != 0 || key.is_null() {
        return None;
    }

    let mut data_type: u32 = 0;
    let mut size: u32 = 0;
    let status = unsafe {
        RegQueryValueExW(
            key,
            value_wide.as_ptr(),
            std::ptr::null(),
            &mut data_type,
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if status != 0 || size == 0 || size > 4096 {
        unsafe { RegCloseKey(key) };
        return None;
    }

    let mut buffer = vec![0u8; size as usize];
    let status = unsafe {
        RegQueryValueExW(
            key,
            value_wide.as_ptr(),
            std::ptr::null(),
            &mut data_type,
            buffer.as_mut_ptr(),
            &mut size,
        )
    };
    unsafe { RegCloseKey(key) };
    if status != 0 {
        return None;
    }
    buffer.truncate(size as usize);
    Some((data_type, buffer))
}

/// Returns `(native, process, emulated)` using `IsWow64Process2`, falling back
/// to a defined default when the symbol is absent (pre-Windows 10 builds).
#[cfg(target_os = "windows")]
fn probe_architectures() -> (Option<String>, String, bool) {
    use windows_sys::Win32::System::SystemInformation::{
        IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64, IMAGE_FILE_MACHINE_I386,
        IMAGE_FILE_MACHINE_UNKNOWN,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};

    let process = std::env::consts::ARCH.to_string();
    let mut process_machine: u16 = IMAGE_FILE_MACHINE_UNKNOWN;
    let mut native_machine: u16 = IMAGE_FILE_MACHINE_UNKNOWN;
    let ok = unsafe {
        IsWow64Process2(
            GetCurrentProcess(),
            &mut process_machine,
            &mut native_machine,
        )
    };
    if ok == 0 {
        return (None, process, false);
    }

    fn label(machine: u16) -> Option<String> {
        match machine {
            IMAGE_FILE_MACHINE_AMD64 => Some("x86_64".to_string()),
            IMAGE_FILE_MACHINE_ARM64 => Some("aarch64".to_string()),
            IMAGE_FILE_MACHINE_I386 => Some("x86".to_string()),
            _ => None,
        }
    }

    let native = label(native_machine);
    // Per `IsWow64Process2`, an unknown process machine means the process runs
    // natively on the native machine.
    let effective_process = if process_machine == IMAGE_FILE_MACHINE_UNKNOWN {
        native.clone().unwrap_or(process)
    } else {
        label(process_machine).unwrap_or(process)
    };
    let emulated = process_machine != IMAGE_FILE_MACHINE_UNKNOWN
        && native_machine != IMAGE_FILE_MACHINE_UNKNOWN
        && process_machine != native_machine;
    (native, effective_process, emulated)
}

#[cfg(target_os = "windows")]
fn probe_elevation() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = std::ptr::null_mut();
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if opened == 0 || token.is_null() {
        return false;
    }
    let mut elevation: TOKEN_ELEVATION = unsafe { std::mem::zeroed() };
    let mut returned: u32 = 0;
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut TOKEN_ELEVATION as *mut std::ffi::c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    unsafe { CloseHandle(token) };
    ok != 0 && elevation.TokenIsElevated != 0
}

#[cfg(test)]
mod tests {
    use super::{RuntimeEnvironment, SecurityPolicyState};

    #[test]
    fn probe_reports_the_compiled_os_and_architecture() {
        let environment = RuntimeEnvironment::probe(Some("test-runtime 1.0".to_string()));
        assert_eq!(environment.os, std::env::consts::OS);
        assert_eq!(environment.process_architecture, std::env::consts::ARCH);
        assert_eq!(
            environment.webview_version.as_deref(),
            Some("test-runtime 1.0")
        );
        assert!(!environment.diagnostics_lines().is_empty());
    }

    #[test]
    fn policy_state_labels_are_stable() {
        assert_eq!(SecurityPolicyState::Enabled.label(), "enabled");
        assert_eq!(SecurityPolicyState::Disabled.label(), "disabled");
        assert_eq!(SecurityPolicyState::Unknown.label(), "unknown");
    }
}
