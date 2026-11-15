//! The page the editor starts in, as a server sends it: the document as HTML, readable before any of the
//! editor has arrived, and the same document as the editor reads it.
//!
//! The HTML is the document's export, shown in a frame of its own so that its styles and the page's cannot
//! reach each other. The editor opens the document the page carries and takes the frame away once its
//! first frame is on the canvas.

use letters_model::{Block, BlockId, BlockKind, Document};

/// The `id` of the frame that shows the document until the editor does.
pub const FIRST_PAINT: &str = "letters-first-paint";
/// The `id` of the script element that holds the document as `document.json` stores it.
pub const DOCUMENT: &str = "letters-document";

/// Where each goes in a page's template.
const FIRST_PAINT_SLOT: &str = "<!--letters:first-paint-->";
const DOCUMENT_SLOT: &str = "<!--letters:document-->";

/// Fills a page's template with a document: `html` is its export as a whole HTML file, and `json` its
/// `document.json`. Neither can end its element early, whatever the document holds.
pub fn compose(template: &str, html: &str, json: &str) -> String {
    // The frame may run nothing and reach nothing, like the export it shows.
    let mut frame = format!("<iframe id=\"{FIRST_PAINT}\" title=\"Document\" sandbox=\"\" srcdoc=\"");
    for c in html.chars() {
        match c {
            '&' => frame.push_str("&amp;"),
            '"' => frame.push_str("&quot;"),
            c => frame.push(c),
        }
    }
    frame.push_str("\"></iframe>");
    // Inside a script, `<` could begin the tag that ends it. JSON reads the escape as the same character.
    let json = json.replace('<', "\\u003c");
    let script = format!("<script type=\"application/json\" id=\"{DOCUMENT}\">{json}</script>");
    // The script goes in first: it holds no `<`, so it cannot be mistaken for the frame's place, while the
    // frame may hold anything.
    template.replacen(DOCUMENT_SLOT, &script, 1).replacen(FIRST_PAINT_SLOT, &frame, 1)
}

/// What a page shows when it was given no document of its own.
pub fn welcome(new_id: &mut dyn FnMut() -> String) -> Document {
    let mut block = |kind, text| Block::text(BlockId::new(new_id()), kind, text);
    Document {
        blocks: vec![
            block(BlockKind::Heading { level: 1 }, "Letters in the browser"),
            block(BlockKind::Paragraph, "The same engine as the desktop app, painted on a canvas. Type to try it."),
            block(BlockKind::Paragraph, ""),
        ],
        ..Document::default()
    }
}
