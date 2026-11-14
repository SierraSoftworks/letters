//! Import and export. Conversion is best effort in both directions.
//!
//! Markdown is first: it is the exchange format and what interaction tests assert on.

use letters_model::Document;

/// Something an import had to simplify, to be shown as an import note on the affected block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportNote {
    /// The id of the affected block.
    pub block: String,
    pub message: String,
}

/// Reads CommonMark and GFM. `new_id` supplies an id for each block.
pub fn from_markdown(_markdown: &str, _new_id: &mut dyn FnMut() -> String) -> (Document, Vec<ImportNote>) {
    todo!("convert")
}

/// Writes Markdown. Reading the result back gives an equal document, apart from block ids.
pub fn to_markdown(_doc: &Document) -> String {
    todo!("convert")
}
