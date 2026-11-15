//! Markdown from anywhere: it imports into a sound document, and what is written of that document is
//! read back as the same Markdown.

use letters_convert::{from_markdown, to_markdown};

use crate::checks::{ids, noted, sound};
use crate::input::text;

pub fn run(data: &[u8]) {
    let markdown = text(data);
    let (imported, notes) = from_markdown(&markdown, &mut ids());
    sound(&imported, "imported Markdown");
    noted(&imported, &notes, "imported Markdown");

    let written = to_markdown(&imported);
    let (again, _) = from_markdown(&written, &mut ids());
    sound(&again, "Markdown read back");
    assert_eq!(to_markdown(&again), written, "Markdown changes on its second round trip");
}

pub fn seeds() -> Vec<Vec<u8>> {
    let files = [
        include_str!("../../../crates/letters-convert/tests/corpus/report.md"),
        include_str!("../../../crates/letters-convert/tests/corpus/notice.md"),
    ];
    // A table of contents is written as its list, which no file of the corpus holds one to become.
    let listed = "# Guide\n\n[TOC]\n\n## Moorings\n\n### Fees & *dues*\n\n## Moorings\n\n[[_TOC_]]\n";
    files.iter().chain([&listed]).map(|file| file.as_bytes().to_vec()).collect()
}
