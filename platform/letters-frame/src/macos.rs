//! The unified title bar, which winit 0.30 has no attribute for, and where its lights stand.

use objc2::rc::Retained;
use objc2_app_kit::{NSToolbar, NSView, NSWindow, NSWindowButton, NSWindowToolbarStyle};
use objc2_foundation::{MainThreadMarker, NSProcessInfo, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::Lights;

/// Heights a unified title bar can have, in points. A measurement outside them is not believed: 28 pt
/// is the plain title bar's, which a window not laid out yet may still report.
const BELIEVED: std::ops::RangeInclusive<f64> = 40.0..=96.0;

/// AppKit's window for a winit window. `None` off the main thread, or when the window is not AppKit's.
fn appkit(window: &impl HasWindowHandle) -> Option<(MainThreadMarker, Retained<NSWindow>)> {
    let main = MainThreadMarker::new()?;
    let RawWindowHandle::AppKit(handle) = window.window_handle().ok()?.as_raw() else { return None };
    // SAFETY: the handle holds the window's content view, an `NSView` that lives while `window` is
    // borrowed, and views are only used on the main thread, which the marker says this is.
    let view: Retained<NSView> = unsafe { Retained::retain(handle.ns_view.as_ptr().cast()) }?;
    Some((main, view.window()?))
}

/// Names the process, which is what the menu bar calls an app that runs outside a bundle: without it the
/// app's own menu carries the name of the program's file. It has to be called before the app is launched.
/// An app in a bundle is called what the bundle says, whatever this is given.
pub fn name_process(name: &str) {
    // SAFETY: a property setter of the shared `NSProcessInfo`, which may be used from any thread, with a
    // string that is alive for the call. The process info copies it.
    unsafe { NSProcessInfo::processInfo().setProcessName(&NSString::from_str(name)) };
}

/// Gives a window the tall title bar of a window with a toolbar, and returns the bar's height in points
/// as far as it can be told before the window is shown. Where the lights stand in it is for [`lights`]
/// to say once it is. `None` off the main thread, or when the window is not AppKit's: nothing has
/// changed then.
pub fn unify_title_bar(window: &impl HasWindowHandle) -> Option<f32> {
    let (main, window) = appkit(window)?;
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

/// Where the system stands the window's lights now. No release of macOS promises where that is, and the
/// title bar's height does not tell it: on one Mac the bar came out some 64 pt tall with the lights on a
/// line 26 pt down. It holds once the window was laid out. `None` while the lights are not all in the
/// window, as in full screen, or stand where no title bar this crate asks for would put them.
pub fn lights(window: &impl HasWindowHandle) -> Option<Lights> {
    let (_, window) = appkit(window)?;
    let top = window.frame().size.height;
    let (mut centre, mut end) = (0.0_f64, 0.0_f64);
    for which in [
        NSWindowButton::NSWindowCloseButton,
        NSWindowButton::NSWindowMiniaturizeButton,
        NSWindowButton::NSWindowZoomButton,
    ] {
        let light = window.standardWindowButton(which)?;
        // A full screen window's lights live in a window of the system's own.
        if light.window().is_none_or(|theirs| Retained::as_ptr(&theirs) != Retained::as_ptr(&window)) {
            return None;
        }
        // In the window's own points, which are counted from its bottom left.
        let at = light.convertRect_toView(light.bounds(), None);
        centre = top - (at.origin.y + at.size.height / 2.0);
        end = end.max(at.origin.x + at.size.width);
    }
    // Half the heights a unified title bar can have.
    let believed = (BELIEVED.start() / 2.0)..=(BELIEVED.end() / 2.0);
    believed.contains(&centre).then_some(Lights { centre: centre as f32, end: end as f32 })
}
