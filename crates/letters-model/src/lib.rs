//! The Letters document: its saved tree, the operations that change it and the positions that address it.
//!
//! - [`Document`] is the tree that `document.json` stores.
//! - [`Model`] owns a document and is the only way to change one.
//! - [`Pos`] and [`Selection`] address text inside it.

mod doc;
mod json;
mod model;
mod pos;
mod style;

pub use doc::{Block, BlockId, BlockKind, Body, BodyShape, Document, Extra, FORMAT, ListKind, Mark, Run, Tone};
pub use json::JsonError;
pub use model::{Change, Model, Op, OpError};
pub use pos::{Pos, Selection};
pub use style::{Align, Page, Style, TextStyle};
