//! A Word file from anywhere: it imports into a sound document or is refused, and what was imported can
//! be written as a Word file that reads again.

use letters_convert::{from_docx, from_markdown, to_docx};
use letters_model::{Block, BlockKind, Mark, Notes, Style};

use crate::archive::{entries, spelled, unzipped, zipped};
use crate::checks::{ids, noted, sound};
use crate::input::Script;

const NOW: &str = "2026-11-15T00:00:00Z";

pub fn run(data: &[u8]) {
    let file = zipped(&entries(&mut Script::new(data)));
    let Ok(import) = from_docx(&file, &Style::default(), NOW, &mut ids()) else { return };
    let doc = &import.document;
    sound(doc, "an imported Word file");
    noted(doc, &import.notes, "an imported Word file");
    for comment in &import.comments {
        let text = doc.block(&comment.anchor.block).map(Block::plain_text);
        assert!(text.is_some(), "{:?} is on no block", comment.anchor);
        if let Some(range) = &comment.anchor.range {
            assert!(text.unwrap_or_default().get(range.clone()).is_some(), "{range:?} is not in its block");
        }
    }
    for block in doc.walk() {
        if let BlockKind::Figure { asset: Some(asset), .. } = &block.kind {
            assert!(import.assets.contains_key(asset), "{asset} is not kept");
        }
        for mark in block.body.runs().iter().flat_map(|run| &run.marks) {
            if let Mark::Ref(to) = mark {
                let kind = doc.block(to).map(|figure| &figure.kind);
                let pictured = matches!(kind, Some(BlockKind::Figure { .. } | BlockKind::Gallery { .. }));
                assert!(pictured, "{to:?} refers to {kind:?}");
            }
        }
    }
    let stored = Style::from_json(&import.style.to_json());
    assert_eq!(stored.as_ref(), Ok(&import.style), "an imported style does not read back as it is written");

    let notes = Notes { notes: import.comments.clone(), ..Notes::default() };
    let written = to_docx(doc, &import.style, &notes, "Fuzz", &mut |name| import.assets.get(name).cloned());
    let back = from_docx(&written, &Style::default(), NOW, &mut ids()).expect("a Word file Letters wrote reads");
    sound(&back.document, "a Word file read back");
}

/// Word files as Letters writes them, of documents with every kind of block.
pub fn seeds() -> Vec<Vec<u8>> {
    let files = [
        include_str!("../../../crates/letters-convert/tests/corpus/report.md"),
        include_str!("../../../crates/letters-convert/tests/corpus/notice.md"),
    ];
    let word = |markdown: &&str| {
        let (doc, _) = from_markdown(markdown, &mut ids());
        let file = to_docx(&doc, &Style::default(), &Notes::default(), "Seed", &mut |_| None);
        spelled(&unzipped(&file))
    };
    files.iter().map(word).collect()
}
