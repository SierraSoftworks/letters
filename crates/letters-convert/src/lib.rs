//! Import and export. Conversion is best effort in both directions.
//!
//! Markdown is first: it is the exchange format and what interaction tests assert on. HTML is an export, one
//! file that stands alone, and what the clipboard carries to and from other applications.

mod html;
mod markdown;

pub use html::{Assets, from_html, to_html, to_html_fragment};
pub use markdown::{from_markdown, to_markdown};

/// Something an import had to simplify, to be shown as an import note on the affected block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportNote {
    /// The id of the affected block.
    pub block: String,
    pub message: String,
}
