//! The native window calls winit does not make. This is the one crate of Letters that may use `unsafe`,
//! and it holds nothing else.
//!
//! Every call is optional and is there on every system. Each says whether it worked, and where one did
//! not the app runs as it would without this crate: square corners and no shadow of its own on Wayland, a
//! title bar of the usual height on macOS.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod print;
#[cfg_attr(not(all(unix, not(target_os = "macos"))), path = "no_wayland.rs")]
pub mod wayland;

#[cfg(target_os = "macos")]
pub use macos::unify_title_bar;
#[cfg(target_os = "macos")]
pub use print::print_pdf;

/// Only macOS has the print sheet this asks for. Nothing is shown, and `false` says so.
#[cfg(not(target_os = "macos"))]
pub fn print_pdf(_window: &impl raw_window_handle::HasWindowHandle, _pdf: &[u8], _title: &str) -> bool {
    false
}

/// Only macOS has the title bar this asks for. Nothing changes, and `None` says so.
#[cfg(not(target_os = "macos"))]
pub fn unify_title_bar(_window: &impl raw_window_handle::HasWindowHandle) -> Option<f32> {
    None
}

/// One piece of a shadow: pixels to lie under and around a window.
#[derive(Clone, Copy, Debug)]
pub struct Strip<'a> {
    /// Where its top left corner is from the window's, in the window's own units.
    pub x: i32,
    pub y: i32,
    /// Its size in pixels.
    pub width: u32,
    pub height: u32,
    /// How many of its pixels make one of the window's units, each way.
    pub scale: u32,
    /// `0xAARRGGBB` words with the colours multiplied by alpha, row by row from the top left.
    pub pixels: &'a [u32],
}
