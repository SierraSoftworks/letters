//! The interface, drawn natively with yewi's design tokens: no browser engine and no system widgets.
//!
//! - [`Element`] is the one building block: a styled box holding children, text, an icon, a text input or a
//!   canvas. The [`widgets`] are builders that turn into elements.
//! - An [`App`] describes its window as a tree of elements and handles the messages they send. [`Ui`] hosts
//!   it: layout, painting, pointer and keyboard input, focus and the accessibility tree.

mod access;
pub mod color;
mod element;
pub mod icon;
mod input;
mod layout;
mod paint;
mod style;
mod text;
pub mod tokens;
mod tree;
mod ui;
pub mod widgets;

pub use accesskit::Role;
pub use element::{Cursor, Element, Paint, Pointer};
pub use icon::Icon;
pub use style::{Align, Dir, Edges, Justify, Look, Ring, Style};
pub use text::{Family, FixedShaper, FontShaper, GlyphRun, Shaped, Stop, TextShaper, TextStyle};
pub use tokens::{Color as Ink, Scheme};
pub use ui::{App, Ui};
