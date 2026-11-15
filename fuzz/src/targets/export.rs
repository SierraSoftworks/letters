//! Any document written out: as a PDF, as a Word file that reads back, and as a web page.

use letters_convert::{PdfInfo, from_docx, from_html, to_docx, to_html, to_html_fragment, to_markdown, to_pdf};
use letters_layout::Fonts;
use letters_model::{Notes, Style};

use crate::checks::{ids, sound};
use crate::edits::{written, written_seeds};

/// A PNG of one pixel.
const PIXEL: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\x0d\x49\x44\x41\x54\x78\xda\x63\x64\x60\xf8\x5f\x0f\x00\x02\x87\x01\x80\xeb\x47\xba\x92\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82";

/// A picture for every asset whose name ends in a digit, bytes that are no picture for the others with an
/// even length of name, and nothing for the rest.
fn asset(name: &str) -> Option<Vec<u8>> {
    match name.bytes().last() {
        Some(b'0'..=b'9') => Some(PIXEL.to_vec()),
        _ if name.len().is_multiple_of(2) => Some(name.as_bytes().to_vec()),
        _ => None,
    }
}

pub fn run(data: &[u8]) {
    let Some((doc, style)) = written(data) else { return };
    sound(&doc, "a document to export");

    let info = PdfInfo { title: "Fuzz".into(), authors: vec!["Ana".into()], created: Some(1_800_000_000) };
    let fonts = Fonts { faces: letters_fonts::bundled() };
    let pdf = to_pdf(&doc, &style, fonts, &mut asset, &info).expect("a document is written as a PDF");
    assert!(pdf.starts_with(b"%PDF-"), "what was written is no PDF");

    let word = to_docx(&doc, &style, &Notes::default(), "Fuzz", &mut asset);
    let back = from_docx(&word, &Style::default(), "2026-11-15T00:00:00Z", &mut ids());
    let back = back.expect("a Word file Letters wrote reads");
    sound(&back.document, "a Word file read back");

    to_html(&doc, &style, "Fuzz", &mut asset);
    let (page, _) = from_html(&to_html_fragment(&doc.blocks), &mut ids());
    sound(&page, "HTML read back");
    to_markdown(&doc);
}

pub fn seeds() -> Vec<Vec<u8>> {
    written_seeds()
}
