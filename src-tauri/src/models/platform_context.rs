use serde::{Deserialize, Serialize};

use super::PlatformKind;

/// Platform vocabulary and locations the interface must not hardcode.
///
/// Copy such as "Move to Trash", "Menu Bar Quick Panel", or
/// `~/Library/Logs/Zenith` is only true on one platform. Deriving it from the
/// running backend keeps the Windows and Linux builds from describing
/// themselves with macOS nouns, and keeps a mocked browser preview from
/// inventing paths the native build would never produce.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PlatformContext {
    pub platform: PlatformKind,
    /// Directory holding the current log file, with the profile masked.
    pub log_directory: String,
    /// Label for revealing a path in the file manager.
    pub reveal_label: String,
    /// What this platform calls the recoverable-delete location.
    pub trash_label: String,
    /// What this platform calls per-application support data.
    pub app_data_label: String,
    /// Surface that hosts the quick panel.
    pub quick_panel_surface_label: String,
    /// Container runtime the empty state may suggest, when one exists.
    pub container_runtime_hint: Option<String>,
    /// True when the platform draws the main window caption bar.
    pub native_caption_bar: bool,
    /// True when the application draws an overlay title bar over the webview.
    pub overlay_title_bar: bool,
    /// Primary shortcut modifier for this platform (`meta` or `ctrl`).
    pub primary_accelerator: PlatformAccelerator,
    /// What this platform calls the identity of an installed application.
    pub app_identity_label: String,
    /// Terminal application the user is sent to.
    pub terminal_label: String,
    /// Release page a user can open manually when a fix is published.
    pub releases_url: String,
}

/// Primary shortcut modifier. A dismissal bound to `meta` only never fires on
/// Windows, where the accelerator is `ctrl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum PlatformAccelerator {
    Meta,
    Ctrl,
}

impl PlatformAccelerator {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Meta => "cmd",
            Self::Ctrl => "ctrl",
        }
    }
}

impl PlatformContext {
    pub const RELEASES_URL: &'static str = "https://github.com/jaeyoung0509/zenith/releases";

    /// Vocabulary for a stated platform. Pure, so every platform's copy can be
    /// asserted from any runner.
    pub fn for_platform(platform: PlatformKind, log_directory: String) -> Self {
        match platform {
            PlatformKind::Macos => Self {
                platform,
                log_directory,
                reveal_label: "Reveal in Finder".to_string(),
                trash_label: "Trash".to_string(),
                app_data_label: "Application Support".to_string(),
                quick_panel_surface_label: "menu bar".to_string(),
                container_runtime_hint: Some("Colima".to_string()),
                native_caption_bar: false,
                overlay_title_bar: true,
                primary_accelerator: PlatformAccelerator::Meta,
                app_identity_label: "bundle identifier".to_string(),
                terminal_label: "Terminal".to_string(),
                releases_url: Self::RELEASES_URL.to_string(),
            },
            PlatformKind::Windows => Self {
                platform,
                log_directory,
                reveal_label: "Show in File Explorer".to_string(),
                trash_label: "Recycle Bin".to_string(),
                app_data_label: "AppData".to_string(),
                quick_panel_surface_label: "notification area".to_string(),
                container_runtime_hint: Some("Docker Desktop".to_string()),
                // Windows draws a native caption bar; reserving overlay space
                // only creates dead pixels below it.
                native_caption_bar: true,
                overlay_title_bar: false,
                primary_accelerator: PlatformAccelerator::Ctrl,
                app_identity_label: "installed application".to_string(),
                terminal_label: "Windows Terminal".to_string(),
                releases_url: Self::RELEASES_URL.to_string(),
            },
            PlatformKind::Linux => Self {
                platform,
                log_directory,
                reveal_label: "Show in File Manager".to_string(),
                trash_label: "Trash".to_string(),
                app_data_label: "data directory".to_string(),
                quick_panel_surface_label: "system tray".to_string(),
                container_runtime_hint: None,
                native_caption_bar: true,
                overlay_title_bar: false,
                primary_accelerator: PlatformAccelerator::Ctrl,
                app_identity_label: "installed application".to_string(),
                terminal_label: "your terminal".to_string(),
                releases_url: Self::RELEASES_URL.to_string(),
            },
            PlatformKind::Other => Self {
                platform,
                log_directory,
                reveal_label: "Reveal".to_string(),
                trash_label: "Trash".to_string(),
                app_data_label: "application data".to_string(),
                quick_panel_surface_label: "tray".to_string(),
                container_runtime_hint: None,
                native_caption_bar: true,
                overlay_title_bar: false,
                primary_accelerator: PlatformAccelerator::Ctrl,
                app_identity_label: "installed application".to_string(),
                terminal_label: "your terminal".to_string(),
                releases_url: Self::RELEASES_URL.to_string(),
            },
        }
    }

    pub fn current(log_directory: String) -> Self {
        Self::for_platform(PlatformKind::current(), log_directory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(platform: PlatformKind) -> PlatformContext {
        PlatformContext::for_platform(platform, "/logs".to_string())
    }

    #[test]
    fn windows_copy_never_uses_macos_nouns() {
        let windows = context(PlatformKind::Windows);
        assert_eq!(windows.trash_label, "Recycle Bin");
        assert_eq!(windows.quick_panel_surface_label, "notification area");
        assert_eq!(windows.reveal_label, "Show in File Explorer");
        assert!(!windows.reveal_label.contains("Finder"));
        assert!(!windows.app_data_label.contains("Application Support"));
        assert_eq!(windows.primary_accelerator, PlatformAccelerator::Ctrl);
        assert!(windows.native_caption_bar);
        assert!(!windows.overlay_title_bar);
    }

    #[test]
    fn macos_copy_is_unchanged() {
        let macos = context(PlatformKind::Macos);
        assert_eq!(macos.trash_label, "Trash");
        assert_eq!(macos.quick_panel_surface_label, "menu bar");
        assert_eq!(macos.reveal_label, "Reveal in Finder");
        assert_eq!(macos.primary_accelerator, PlatformAccelerator::Meta);
        assert!(macos.overlay_title_bar);
        assert!(!macos.native_caption_bar);
    }

    #[test]
    fn accelerator_matches_the_platform_shortcut() {
        assert_eq!(
            context(PlatformKind::Macos).primary_accelerator.label(),
            "cmd"
        );
        for platform in [PlatformKind::Windows, PlatformKind::Linux] {
            assert_eq!(context(platform).primary_accelerator.label(), "ctrl");
        }
    }

    #[test]
    fn every_platform_points_at_the_same_release_page() {
        for platform in [
            PlatformKind::Macos,
            PlatformKind::Windows,
            PlatformKind::Linux,
            PlatformKind::Other,
        ] {
            assert_eq!(
                context(platform).releases_url,
                PlatformContext::RELEASES_URL
            );
        }
    }
}
