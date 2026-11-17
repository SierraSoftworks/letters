//! The native calls winit does not make. This is the one crate of Letters that may use `unsafe`, and it
//! holds nothing else.
//!
//! Besides the window calls it maps font files into memory ([`map_font`]), which is unsafe for any crate,
//! asks macOS to download files a sync app keeps in the cloud ([`fetch_cloud_files`]), and gives the bare
//! program its icon in the Dock ([`set_dock_icon`]). On Windows it asks the system what the standard
//! library does not: the person's name and Documents folder, whether they want animations, and for the
//! console the program was started from.
//!
//! Every call is optional and is there on every system. Each says whether it worked, and where one did
//! not the app runs as it would without this crate: square corners and no shadow of its own on Wayland, a
//! title bar of the usual height on macOS.

#[cfg(target_os = "macos")]
mod cloud;
#[cfg(target_os = "macos")]
mod macos;
mod map;
#[cfg(target_os = "macos")]
mod print;
#[cfg(all(windows, feature = "probe"))]
pub mod probe;
#[cfg_attr(not(all(unix, not(target_os = "macos"))), path = "no_wayland.rs")]
pub mod wayland;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use cloud::fetch_cloud_files;
#[cfg(target_os = "macos")]
pub use macos::{lights, name_process, set_dock_icon, unify_title_bar};
pub use map::{Mapped, map_font};
#[cfg(target_os = "macos")]
pub use print::print_pdf;

/// Only macOS holds a cloud file back from a process. Nothing is asked, and `true` says none is held back.
#[cfg(not(target_os = "macos"))]
pub fn fetch_cloud_files() -> bool {
    true
}

/// Only macOS has a Dock whose icon a running app sets. Nothing is shown, and `false` says so.
#[cfg(not(target_os = "macos"))]
pub fn set_dock_icon(_png: &[u8]) -> bool {
    false
}

/// Only macOS has the print sheet this asks for. Nothing is shown, and `false` says so.
#[cfg(not(target_os = "macos"))]
pub fn print_pdf(_window: &impl raw_window_handle::HasWindowHandle, _pdf: &[u8], _title: &str) -> bool {
    false
}

/// Only macOS names an app in its menu bar after the process. Nothing changes.
#[cfg(not(target_os = "macos"))]
pub fn name_process(_name: &str) {}

/// Only macOS has the title bar this asks for. Nothing changes, and `None` says so.
#[cfg(not(target_os = "macos"))]
pub fn unify_title_bar(_window: &impl raw_window_handle::HasWindowHandle) -> Option<f32> {
    None
}

/// Where macOS stands a window's three lights, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lights {
    /// How far the line through their centres is below the window's top edge.
    pub centre: f32,
    /// How far the last of them reaches from the window's left edge.
    pub end: f32,
}

/// Only macOS has lights to look for. `None` says there are none.
#[cfg(not(target_os = "macos"))]
pub fn lights(_window: &impl raw_window_handle::HasWindowHandle) -> Option<Lights> {
    None
}

#[cfg(windows)]
pub use windows::{account_name, animations, attach_console, documents_folder};

/// Only a Windows program can be without the console it was started from. `true` says nothing is missing.
#[cfg(not(windows))]
pub fn attach_console() -> bool {
    true
}

/// Only Windows keeps a full name the standard library and the shell's tools do not give. `None` says so.
#[cfg(not(windows))]
pub fn account_name() -> Option<String> {
    None
}

/// Only Windows moves the Documents folder out of the home folder. `None` says it is where it always is.
#[cfg(not(windows))]
pub fn documents_folder(_home: &std::path::Path) -> Option<std::path::PathBuf> {
    None
}

/// Only Windows is asked here whether the person wants animations. `None` says nothing was asked.
#[cfg(not(windows))]
pub fn animations() -> Option<bool> {
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
