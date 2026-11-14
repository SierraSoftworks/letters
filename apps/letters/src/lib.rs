//! The Letters desktop app: a native window around the shared engine.
//!
//! - [`app`] runs the winit event loop and owns the windows.
//! - [`input`] and [`platform`] adapt the desktop to the engine's interfaces.
//! - [`files`] opens and saves `.let` files, and [`library`] decides where they live.
//! - [`measure`] checks the performance budgets, and [`baseline`] is the empty window it compares with.
//! - [`convert`] runs the `export` and `import` subcommands, which need no window.

pub mod app;
pub mod baseline;
pub mod chrome;
pub mod cli;
pub mod convert;
pub mod files;
pub mod gpu;
pub mod input;
pub mod library;
pub mod measure;
pub mod platform;
pub mod present;
