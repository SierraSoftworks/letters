//! Import and export. Conversion is best effort in both directions.
//!
//! Markdown is first: it is the exchange format and what interaction tests assert on. HTML is an export, one
//! file that stands alone, and what the clipboard carries to and from other applications. Word documents are
//! read and written whole, styles included. PDF is an export on the pages Print view shows.

mod docx;
mod figures;
mod html;
mod markdown;
mod pdf;

pub use docx::{DocxError, DocxImport, from_docx, to_docx};
pub use html::{Assets, from_html, to_html, to_html_fragment, to_html_themed};
pub use markdown::{MarkdownImport, from_markdown, import_markdown, markdown_assets, to_markdown};
pub use pdf::{PdfInfo, to_pdf};

/// Something an import had to simplify, to be shown as an import note on the affected block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportNote {
    /// The id of the affected block.
    pub block: String,
    pub message: String,
}
