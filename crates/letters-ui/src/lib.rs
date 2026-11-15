//! The interface, drawn natively with yewi's design tokens: no browser engine and no system widgets.
//!
//! - [`Element`] is the one building block: a styled box holding children, text, an icon, a text input or a
//!   canvas. The [`widgets`] are builders that turn into elements.
//! - An [`App`] describes its window as a tree of elements and handles the messages they send. [`Ui`] hosts
//!   it: layout, painting, pointer and keyboard input, focus and the accessibility tree.
//! - [`Window`] is the editor's window, which the shells and the test harness drive through the same calls.

mod access;
mod callouts;
mod captions;
mod chrome;
mod code_copy;
pub mod color;
mod commands;
mod context;
mod copying;
mod describe;
mod element;
mod figures;
mod finding;
mod folders;
mod highlight;
mod history;
pub mod icon;
mod importing;
mod input;
mod insert;
mod layout;
mod maths;
pub mod motion;
mod notes;
mod opening;
mod paint;
mod pictures;
mod print;
mod refer;
mod references;
mod renaming;
mod restyle;
mod review;
mod safe;
mod settings;
mod spelling;
mod style;
mod styles;
mod tables;
mod tex;
mod text;
mod theme;
mod threads;
pub use yewi_tokens as tokens;
mod tree;
mod ui;
pub mod widgets;
mod window;
mod words;

pub use accesskit::Role;
pub use callouts::Calling;
pub use chrome::{Chrome, Edge, Frame, WindowButton};
pub use context::Contexting;
pub use copying::CopyFormat;
pub use element::{Cursor, Element, Fitted, Paint, Pointer};
pub use finding::{Finding, Way};
pub use icon::Icon;
pub use importing::{ImportFolder, ImportKind, ImportOffer};
pub use letters_layout::{Uncovered, WordList};
pub use opening::{Folder, Landing, Location, Recent, Request};
pub use pictures::PictureState;
pub use restyle::StyleEdit;
pub use safe::SafeMode;
pub use settings::{Appearance, Settings};
pub use style::{Align, Dir, Edges, Justify, Look, Ring, Style};
pub use text::{Family, FixedShaper, FontShaper, GlyphRun, Shaped, Stop, TextShaper, TextStyle};
pub use theme::Accent;
pub use tokens::{Color as Ink, Scheme};
pub use ui::{App, Idle, Keys, Ui};
pub use window::{DocLines, EditorApp, SaveState, Window, palette};
