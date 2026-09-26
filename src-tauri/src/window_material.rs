//! Native glass belongs to the desktop window adapter, not the domain layer.
use objc2::{rc::Retained, runtime::AnyClass, MainThreadMarker};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSGlassEffectView, NSGlassEffectViewStyle, NSView};
use tauri::{Runtime, WebviewWindow};

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
