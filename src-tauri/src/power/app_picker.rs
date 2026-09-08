use crate::models::SelectedApplication;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::process::Command;

pub struct ApplicationPicker;

#[cfg(any(windows, test))]
const WINDOWS_APP_PICKER_SCRIPT: &str = r#"
    $ErrorActionPreference = 'Stop'
    [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
    Add-Type -AssemblyName System.Windows.Forms
    $dialog = New-Object System.Windows.Forms.OpenFileDialog
    try {
        $dialog.Filter = 'Executable files (*.exe)|*.exe'
        $dialog.Title = 'Choose an application for Keep Awake'
        $dialog.InitialDirectory = [Environment]::GetFolderPath('ProgramFiles')
        if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
            [Console]::Write($dialog.FileName)
        }
    } finally {
        $dialog.Dispose()
    }
"#;

#[cfg(any(windows, test))]
fn decode_picker_path(bytes: Vec<u8>) -> Result<Option<std::path::PathBuf>, String> {
    let value = String::from_utf8(bytes)
        .map_err(|_| "The application picker returned invalid UTF-8".to_string())?;
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(std::path::PathBuf::from(value)))
    }
}

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
        let output = crate::tooling::command("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-STA",
                "-Command",
                WINDOWS_APP_PICKER_SCRIPT,
            ])
            .output()
            .map_err(|error| format!("Could not open the application picker: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Could not open the application picker: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let Some(path) = decode_picker_path(output.stdout)? else {
            return Ok(None);
        };
        if !path.is_file() {
            return Err("The selected application no longer exists".into());
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
        if !path
            .extension()
            .is_some_and(|value| value.eq_ignore_ascii_case("exe"))
        {
            return Err("Please choose a Windows .exe application".into());
        }
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
    fn picker_preserves_korean_and_distinguishes_invalid_encoding_from_cancel() {
        let path = "D:\\사용자\\홍 길동\\도구 앱.exe";
        assert_eq!(
            super::decode_picker_path(path.as_bytes().to_vec()).unwrap(),
            Some(path.into())
        );
        assert!(super::decode_picker_path(vec![]).unwrap().is_none());
        assert!(super::decode_picker_path(vec![0xff, 0xfe]).is_err());
        assert!(super::WINDOWS_APP_PICKER_SCRIPT.contains("[Console]::OutputEncoding"));
        let dir = tempdir().unwrap();
        let selection =
            ApplicationPicker::selection_from_windows_exe(&dir.path().join("한글 도구.EXE"))
                .unwrap();
        assert_eq!(selection.executable_pattern, "한글 도구.EXE");
        assert!(
            ApplicationPicker::selection_from_windows_exe(&dir.path().join("도구.txt")).is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_powershell_emits_korean_paths_as_utf8() {
        let script = r#"[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); [Console]::Write('D:\사용자\홍 길동\도구 앱.exe')"#;
        let mut command = crate::tooling::command("powershell.exe");
        command.args(["-NoLogo", "-NoProfile", "-Command", script]);
        let output =
            crate::tooling::run_with_timeout(command, std::time::Duration::from_secs(15)).unwrap();
        assert!(output.status.success());
        assert_eq!(
            super::decode_picker_path(output.stdout).unwrap(),
            Some(r"D:\사용자\홍 길동\도구 앱.exe".into())
        );
    }

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
