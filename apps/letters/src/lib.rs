//! The Letters desktop app: a native window around the shared engine.
//!
//! - [`app`] runs the winit event loop and owns the windows.
//! - [`input`] and [`platform`] adapt the desktop to the engine's interfaces.
//! - [`files`] opens and saves `.let` files.
//! - [`measure`] checks the performance budgets, and [`baseline`] is the empty window it compares with.

pub mod app;
pub mod baseline;
pub mod cli;
pub mod files;
pub mod gpu;
pub mod input;
pub mod measure;
pub mod platform;
pub mod present;
