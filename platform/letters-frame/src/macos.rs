//! The unified title bar, which winit 0.30 has no attribute for.

use objc2::rc::Retained;
use objc2_app_kit::{NSToolbar, NSView, NSWindowToolbarStyle};
use objc2_foundation::MainThreadMarker;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Heights a unified title bar can have, in points. A measurement outside them is not believed: 28 pt
/// is the plain title bar's, which a window not laid out yet may still report.
const BELIEVED: std::ops::RangeInclusive<f64> = 40.0..=96.0;

/// Gives a window the tall title bar of a window with a toolbar, which centres its lights, and returns the
/// bar's height in points. `None` off the main thread, or when the window is not AppKit's: nothing has
/// changed then.
pub fn unify_title_bar(window: &impl HasWindowHandle) -> Option<f32> {
    let main = MainThreadMarker::new()?;
    let RawWindowHandle::AppKit(handle) = window.window_handle().ok()?.as_raw() else { return None };
    // SAFETY: the handle holds the window's content view, an `NSView` that lives while `window` is
    // borrowed, and views are only used on the main thread, which the marker says this is.
    let view: Retained<NSView> = unsafe { Retained::retain(handle.ns_view.as_ptr().cast()) }?;
    let window = view.window()?;
    // SAFETY: plain property setters of `NSToolbar` and `NSWindow`, called on the main thread with
    // objects that are alive. The window retains the toolbar.
    unsafe {
        // The style is ignored while a window has no toolbar, so it is given an empty one.
        let toolbar = NSToolbar::new(main);
        toolbar.setShowsBaselineSeparator(false);
        window.setToolbar(Some(&toolbar));
        window.setToolbarStyle(NSWindowToolbarStyle::Unified);
    }
    // SAFETY: a getter with no arguments, on the main thread.
    let below = unsafe { window.contentLayoutRect() };
    // What the title bar covers of a window whose content runs under it.
    let height = window.frame().size.height - below.size.height;
    // 52 pt is the unified bar's height from macOS 11 to 15, for a window not laid out yet.
    Some(if BELIEVED.contains(&height) { height as f32 } else { 52.0 })
}
