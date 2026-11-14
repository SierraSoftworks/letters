//! The Letters desktop app: a native window around the shared engine.
//!
//! - [`app`] runs the winit event loop and owns the windows.
//! - [`input`], [`platform`] and [`access`] adapt the desktop to the engine's interfaces.
//! - [`files`] opens and saves `.let` files, and [`launch`] measures how long opening one takes.

pub mod cli;
pub mod files;
pub mod input;
pub mod platform;
