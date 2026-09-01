use crate::models::SelectedApplication;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::process::Command;

pub struct ApplicationPicker;

impl ApplicationPicker {
    #[cfg(target_os = "macos")]
    pub fn pick() -> Result<Option<SelectedApplication>, String> {
        let script = r#"
            try
                set selectedApp to choose file with prompt "Choose an application for Keep Awake" of type {"com.apple.application-bundle"} default location (path to applications folder)
                return POSIX path of selectedApp
            on error number -128
                return ""
            end try
        "#;
        let output = Command::new("osascript")
            .args(["-e", script])
            .output()
            .map_err(|error| format!("Could not open the application picker: {error}"))?;

        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        if path.as_os_str().is_empty() {
            return Ok(None);
        }
        Self::selection_from_app(&path).map(Some)
    }

    #[cfg(target_os = "windows")]
    pub fn pick() -> Result<Option<SelectedApplication>, String> {
        let script = r#"
            Add-Type -AssemblyName System.Windows.Forms
            $dialog = New-Object System.Windows.Forms.OpenFileDialog
            $dialog.Filter = "Executable files (*.exe)|*.exe|All files (*.*)|*.*"
            $dialog.Title = "Choose an application for Keep Awake"
            $progFiles = [Environment]::GetFolderPath("ProgramFiles")
            $dialog.InitialDirectory = $progFiles
            if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
                Write-Output $dialog.FileName
            }
        "#;
        let mut cmd = Command::new("powershell.exe");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        let output = crate::tooling::run_with_timeout(cmd, std::time::Duration::from_secs(60))
            .map_err(|error| format!("Could not open the application picker: {error}"))?;

        let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if path_str.is_empty() {
            return Ok(None);
        }
        let path = std::path::PathBuf::from(path_str);
        if !path.is_file() {
            return Ok(None);
        }
        Self::selection_from_windows_exe(&path).map(Some)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    pub fn pick() -> Result<Option<SelectedApplication>, String> {
        Err("Application selection is currently available on macOS and Windows only".into())
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn selection_from_app(path: &Path) -> Result<SelectedApplication, String> {
        if path.extension().and_then(|extension| extension.to_str()) != Some("app") {
            return Err("Please choose a macOS .app bundle".into());
        }
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "The selected application has an invalid name".to_string())?
            .to_string();
        let info_plist = path.join("Contents/Info.plist");
        let mut plutil_cmd = Command::new("plutil");
        plutil_cmd
            .args(["-extract", "CFBundleExecutable", "raw", "-o", "-"])
            .arg(&info_plist);
        let executable_pattern =
            crate::tooling::run_with_timeout(plutil_cmd, std::time::Duration::from_secs(3))
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| name.clone());

        Ok(SelectedApplication {
            name,
            executable_pattern,
            path: path.to_string_lossy().into_owned(),
        })
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) fn selection_from_windows_exe(path: &Path) -> Result<SelectedApplication, String> {
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "The selected application has an invalid name".to_string())?
            .to_string();
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(&name)
            .to_string();

        Ok(SelectedApplication {
            name,
            executable_pattern: file_name,
            path: path.to_string_lossy().into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ApplicationPicker;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn rejects_non_application_paths() {
        let dir = tempdir().unwrap();
        assert!(ApplicationPicker::selection_from_app(dir.path()).is_err());
    }

    #[test]
    fn falls_back_to_bundle_name_without_an_info_plist() {
        let dir = tempdir().unwrap();
        let app = dir.path().join("Render Worker.app");
        fs::create_dir(&app).unwrap();
        let selection = ApplicationPicker::selection_from_app(&app).unwrap();
        assert_eq!(selection.name, "Render Worker");
        assert_eq!(selection.executable_pattern, "Render Worker");
    }

    #[test]
    fn parses_windows_executable_selection() {
        let dir = tempdir().unwrap();
        let exe = dir.path().join("code.exe");
        fs::write(&exe, "binary").unwrap();
        let selection = ApplicationPicker::selection_from_windows_exe(&exe).unwrap();
        assert_eq!(selection.name, "code");
        assert_eq!(selection.executable_pattern, "code.exe");
        assert_eq!(selection.path, exe.to_string_lossy());
    }
}
