//! The public entries of a `.let` file: each reads or is refused, and what reads is written back the same.

use letters_model::{Document, Notes, Style};

use crate::checks::{opens, sound};
use crate::input::text;

pub fn run(data: &[u8]) {
    let text = text(data);
    if let Ok(document) = Document::from_json(&text) {
        sound(&document, "a document.json");
        opens(&document, "a document.json");
    }
    if let Ok(style) = Style::from_json(&text) {
        assert_eq!(Style::from_json(&style.to_json()).as_ref(), Ok(&style), "a style.json reads back differently");
    }
    if let Ok(notes) = Notes::from_json(&text) {
        assert_eq!(Notes::from_json(&notes.to_json()).as_ref(), Ok(&notes), "a notes.json reads back differently");
    }
}

/// The corpus letters-format keeps of each entry, and each case of the ones it refuses.
pub fn seeds() -> Vec<Vec<u8>> {
    let whole = [
        include_str!("../../../crates/letters-format/corpus/document/report.json"),
        include_str!("../../../crates/letters-format/corpus/document/notice.json"),
        include_str!("../../../crates/letters-format/corpus/style/report.json"),
        include_str!("../../../crates/letters-format/corpus/notes/report.json"),
        include_str!("../../../crates/letters-model/tests/corpus/every-block.json"),
    ];
    whole.iter().map(|json| json.as_bytes().to_vec()).collect()
}
