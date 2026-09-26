//! Native glass belongs to the desktop window adapter, not the domain layer.
use objc2::{rc::Retained, runtime::AnyClass, MainThreadMarker};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSGlassEffectView, NSGlassEffectViewStyle, NSView};
use tauri::{Runtime, WebviewWindow};

#[derive(Debug, PartialEq, Eq)]
pub enum WindowMaterial {
    Vibrancy,
    LiquidGlass,
}

/// Keep the native effect, WebView marker, and installed host on one decision.
/// DESIGN.md locks both window labels to the user-approved v0.3.59 material.
pub fn configure(
    config: &mut tauri::utils::config::WindowConfig,
    available: bool,
) -> WindowMaterial {
    if available && matches!(config.label.as_str(), "main" | "quick") {
        // A window has one native material; never stack vibrancy under glass.
        config.window_effects = None;
        WindowMaterial::LiquidGlass
    } else {
        WindowMaterial::Vibrancy
    }
}

/// Runtime lookup keeps the existing vibrancy fallback on macOS before 26.
pub fn glass_available() -> bool {
    AnyClass::get(c"NSGlassEffectView").is_some()
}

pub fn install_glass<R: Runtime>(window: &WebviewWindow<R>) -> tauri::Result<()> {
    let radius = if window.label() == "quick" { 20.0 } else { 0.0 };
    window.with_webview(move |webview| {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        if !glass_available() {
            return;
        }
        // Tauri supplies a live WKWebView on the AppKit thread. Retain its parent
        // and let NSGlassEffectView own it as contentView; arbitrary siblings
        // are not guaranteed to composite above the glass by AppKit.
        unsafe {
            let Some(view) = Retained::retain(webview.inner().cast::<NSView>()) else {
                return;
            };
            let Some(parent) = view.superview() else {
                return;
            };
            let glass = NSGlassEffectView::initWithFrame(mtm.alloc(), view.frame());
            glass.setStyle(NSGlassEffectViewStyle::Regular);
            glass.setCornerRadius(radius);
            glass.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            view.removeFromSuperview();
            glass.setContentView(Some(&view));
            view.setFrame(glass.bounds());
            view.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            parent.addSubview(&glass);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::utils::{config::WindowConfig, WindowEffect, WindowEffectState};
    use zenith_platform::path_algebra::PathFlavor;

    #[test]
    fn both_windows_use_glass_without_stacked_vibrancy_when_available() {
        for label in ["main", "quick"] {
            let mut config = crate::platform_window_config(
                WindowConfig {
                    label: label.into(),
                    transparent: true,
                    ..Default::default()
                },
                PathFlavor::Posix,
            );
            assert_eq!(
                configure(&mut config, true),
                WindowMaterial::LiquidGlass,
                "{label}"
            );
            assert!(
                config.window_effects.is_none(),
                "{label} must have one material"
            );
            assert!(config.transparent);
            assert_eq!(
                config.background_color,
                Some(tauri::utils::config::Color(0, 0, 0, 0))
            );
        }
    }

    #[test]
    fn older_macos_keeps_the_configured_vibrancy_and_corner_policy() {
        for (label, effect, radius) in [
            ("main", WindowEffect::Sidebar, None),
            ("quick", WindowEffect::Popover, Some(20.0)),
        ] {
            let mut config = crate::platform_window_config(
                WindowConfig {
                    label: label.into(),
                    transparent: true,
                    ..Default::default()
                },
                PathFlavor::Posix,
            );
            assert_eq!(configure(&mut config, false), WindowMaterial::Vibrancy);
            let effects = config
                .window_effects
                .expect("older macOS needs native blur");
            assert_eq!(effects.effects, vec![effect]);
            assert_eq!(effects.radius, radius);
            assert_eq!(effects.state, Some(WindowEffectState::Active));
        }
    }
}
