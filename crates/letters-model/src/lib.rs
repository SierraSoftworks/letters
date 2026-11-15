//! The Letters document: its saved tree, the operations that change it and the positions that address it.
//!
//! - [`Document`] is the tree that `document.json` stores.
//! - [`Model`] owns a document and its history, and is the only way to change one.
//! - [`Pos`] and [`Selection`] address text inside it.

mod crdt;
mod doc;
mod flat;
mod history;
mod json;
mod markers;
mod model;
mod notes;
mod pos;
mod style;
mod undo;

pub use doc::{
    Block, BlockId, BlockKind, Body, BodyShape, Document, Extra, FORMAT, ListKind, Mark, Properties, PropertiesFormat,
    Run, Tone,
};
pub use history::{HistoryError, Hold, ReplicaId, Revision, StablePos};
pub use json::JsonError;
pub use markers::{Marker, Markers};
pub use model::{Change, Model, Op, OpError, Place, Prepared};
pub use notes::{Anchor, NOTES_FORMAT, Note, NoteKind, Notes, Reply, Version};
pub use pos::{Pos, Selection};
pub use style::{Align, Page, STYLE_FORMAT, Style, TextStyle};
