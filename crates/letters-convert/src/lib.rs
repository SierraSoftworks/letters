//! Import and export. Conversion is best effort in both directions.
//!
//! Markdown is first: it is the exchange format and what interaction tests assert on. HTML is an export, one
//! file that stands alone, and what the clipboard carries to and from other applications. Word documents are
//! read and written whole, styles included. PDF is an export on the pages Print view shows. A web page is read
//! for what a link preview shows of it.

mod dates;
mod docx;
mod figures;
mod html;
mod losses;
mod markdown;
mod pdf;
mod preview;

pub use dates::Dates;
pub use docx::{DocxError, DocxImport, from_docx, to_docx};
pub use html::{Assets, Column, HtmlPage, WebFont, from_html, to_html, to_html_fragment, to_html_themed, to_html_with};
pub use losses::{Loss, LossKind, Target, losses, losses_with};
pub use markdown::{MarkdownImport, from_markdown, import_markdown, markdown_assets, to_markdown};
pub use pdf::{Backups, Linked, PdfInfo, PdfSources, Words, to_pdf, to_pdf_with};
pub use preview::{PageSummary, page_summary};

/// Something an import had to simplify, to be shown as an import note on the affected block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportNote {
    /// The id of the affected block.
    pub block: String,
    pub message: String,
}
