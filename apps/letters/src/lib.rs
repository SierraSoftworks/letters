//! The Letters desktop app: a native window around the shared engine.
//!
//! - [`app`] runs the winit event loop and owns the windows.
//! - [`input`] and [`platform`] adapt the desktop to the engine's interfaces.
//! - [`files`] opens and saves `.let` files, and [`library`] decides where they live.
//! - [`journal`] keeps unsaved work through a crash, and [`conflicts`] finds the copies a sync tool left.
//! - [`words`] keeps the person's own dictionary, [`trust`] what they trust, and [`settings`] what they set.
//! - [`dictionaries`] finds the system's dictionaries for languages other than English, and [`fallback`]
//!   its fonts for what the bundled ones cannot draw.
//! - [`measure`] checks the performance budgets, and [`baseline`] is the empty window it compares with.
//! - [`convert`] runs the `export` and `import` subcommands, which need no window, and [`copies`] writes the
//!   copies and exports a window asks for.
//! - [`import`] makes a document of a Markdown or Word file, for that subcommand and for a window.
//! - [`strangers`] tries a history from elsewhere in a process of its own, where a panic ends nothing.

pub mod app;
pub mod baseline;
pub mod chrome;
pub mod cli;
pub mod conflicts;
pub mod connected;
pub mod convert;
pub mod copies;
pub mod dictionaries;
pub mod fallback;
pub mod files;
pub mod gpu;
pub mod import;
pub mod input;
pub mod journal;
pub mod keychain;
pub mod library;
pub mod links;
pub mod measure;
pub mod pictures;
pub mod platform;
pub mod present;
pub mod printing;
pub mod remote;
pub mod seen;
pub mod server;
pub mod servers;
pub mod session;
pub mod settings;
pub mod shadow;
pub mod signin;
pub mod socket;
pub mod strangers;
pub mod telemetry;
pub mod trust;
pub mod versions;
pub mod words;
pub mod workers;
pub mod writer;
