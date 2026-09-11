use std::path::Path;
use std::process::Command;

#[cfg(target_os = "windows")]
fn ms_settings_uri_registered() -> bool {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, HKEY_CLASSES_ROOT, KEY_READ,
    };

    let subkey: Vec<u16> = "ms-settings".encode_utf16().chain([0]).collect();
    let mut key = std::ptr::null_mut();
    let status =
        unsafe { RegOpenKeyExW(HKEY_CLASSES_ROOT, subkey.as_ptr(), 0, KEY_READ, &mut key) };
    if status == 0 && !key.is_null() {
        unsafe { RegCloseKey(key) };
        true
    } else {
        false
    }
}

/// Boundary for opening validated paths in native system utilities.
pub trait SystemActionProvider: Send + Sync {
    fn reveal_path(&self, path: &Path) -> Result<(), String>;
    fn open_folder(&self, path: &Path) -> Result<(), String>;
    fn open_terminal(&self, path: &Path) -> Result<(), String>;
    fn open_storage_settings(&self) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NativeSystemActions;

impl NativeSystemActions {
    pub fn new() -> Self {
        Self
    }
}

impl SystemActionProvider for NativeSystemActions {
    fn reveal_path(&self, path: &Path) -> Result<(), String> {
        if !path.exists() {
            return Err("Target path does not exist.".to_string());
        }

        #[cfg(target_os = "macos")]
        {
            Command::new("open")
                .arg("-R")
                .arg(path)
                .spawn()
                .map_err(|error| format!("Could not open Finder: {error}"))?;
            Ok(())
        }

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            let norm_path = crate::platform::NativePlatformPaths::normalize_verbatim_path(path);
            if norm_path.is_dir() {
                Command::new("explorer.exe")
                    .arg(&norm_path)
                    .spawn()
                    .map_err(|error| format!("Could not open Explorer: {error}"))?;
            } else {
                let mut cmd = Command::new("explorer.exe");
                cmd.raw_arg(format!("/select,\"{}\"", norm_path.to_string_lossy()));
                cmd.spawn()
                    .map_err(|error| format!("Could not open Explorer: {error}"))?;
            }
            Ok(())
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = path;
            Err("Reveal in file manager is unavailable on this platform.".to_string())
        }
    }

    fn open_folder(&self, path: &Path) -> Result<(), String> {
        if !path.is_dir() {
            return Err("Target directory does not exist.".to_string());
        }

        #[cfg(target_os = "macos")]
        {
            Command::new("open")
                .arg(path)
                .spawn()
                .map_err(|error| format!("Could not open Finder: {error}"))?;
            Ok(())
        }

        #[cfg(target_os = "windows")]
        {
            let norm_path = crate::platform::NativePlatformPaths::normalize_verbatim_path(path);
            Command::new("explorer.exe")
                .arg(&norm_path)
                .spawn()
                .map_err(|error| format!("Could not open Explorer: {error}"))?;
            Ok(())
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = path;
            Err("Opening a directory is unavailable on this platform.".to_string())
        }
    }

    fn open_terminal(&self, path: &Path) -> Result<(), String> {
        if !path.is_dir() {
            return Err("Project folder is no longer available.".to_string());
        }

        #[cfg(target_os = "macos")]
        {
            Command::new("open")
                .args(["-a", "Terminal"])
                .arg(path)
                .spawn()
                .map_err(|error| format!("Could not open Terminal: {error}"))?;
            Ok(())
        }

        #[cfg(target_os = "windows")]
        {
            // Try Windows Terminal (wt.exe) first
            let wt_result = Command::new("wt.exe")
                .args(["-d", &path.to_string_lossy()])
                .spawn();

            if wt_result.is_ok() {
                return Ok(());
            }

            // Fallback to PowerShell with current_dir (no shell injection)
            let ps_result = Command::new("powershell.exe").current_dir(path).spawn();

            if ps_result.is_ok() {
                return Ok(());
            }

            // Fallback to Command Prompt
            Command::new("cmd.exe")
                .current_dir(path)
                .spawn()
                .map_err(|error| format!("Could not open terminal: {error}"))?;
            Ok(())
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = path;
            Err("Opening a terminal is unavailable on this platform.".to_string())
        }
    }

    fn open_storage_settings(&self) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            Command::new("open")
                .arg("x-apple.systempreferences:com.apple.Storage-Settings.extension")
                .spawn()
                .map_err(|error| format!("Could not open Storage settings: {error}"))?;
            Ok(())
        }

        #[cfg(target_os = "windows")]
        {
            // explorer.exe spawns successfully even when the ms-settings URI
            // is not registered, so verify the handler exists before claiming
            // the settings page opened.
            if !ms_settings_uri_registered() {
                return Err(
                    "The Settings app URI (ms-settings) is not registered on this machine. Open Settings > System > Storage manually."
                        .to_string(),
                );
            }
            Command::new("explorer.exe")
                .arg("ms-settings:storagesense")
                .spawn()
                .map_err(|error| format!("Could not open Storage settings: {error}"))?;
            Ok(())
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err("Storage settings are unavailable on this platform.".to_string())
        }
    }
}

#[cfg(test)]
pub struct MockSystemActions {
    pub reveal_calls: std::sync::Mutex<Vec<std::path::PathBuf>>,
    pub folder_calls: std::sync::Mutex<Vec<std::path::PathBuf>>,
    pub terminal_calls: std::sync::Mutex<Vec<std::path::PathBuf>>,
}

#[cfg(test)]
impl Default for MockSystemActions {
    fn default() -> Self {
        Self {
            reveal_calls: std::sync::Mutex::new(Vec::new()),
            folder_calls: std::sync::Mutex::new(Vec::new()),
            terminal_calls: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[cfg(test)]
impl SystemActionProvider for MockSystemActions {
    fn reveal_path(&self, path: &Path) -> Result<(), String> {
        self.reveal_calls.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }

    fn open_folder(&self, path: &Path) -> Result<(), String> {
        self.folder_calls.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }

    fn open_terminal(&self, path: &Path) -> Result<(), String> {
        self.terminal_calls.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }

    fn open_storage_settings(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn mock_system_actions_tracks_invocations() {
        let mock = MockSystemActions::default();
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "content").unwrap();

        assert!(mock.reveal_path(&file).is_ok());
        assert!(mock.open_folder(dir.path()).is_ok());
        assert!(mock.open_terminal(dir.path()).is_ok());
        assert_eq!(mock.reveal_calls.lock().unwrap().len(), 1);
        assert_eq!(mock.folder_calls.lock().unwrap().len(), 1);
        assert_eq!(mock.terminal_calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn native_system_actions_rejects_missing_path() {
        let actions = NativeSystemActions::new();
        let missing = Path::new("/path/that/does/not/exist/zenith_test_missing_123");
        assert!(actions.reveal_path(missing).is_err());
        assert!(actions.open_folder(missing).is_err());
        assert!(actions.open_terminal(missing).is_err());
    }
}
