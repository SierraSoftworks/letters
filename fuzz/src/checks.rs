//! What holds of every document, wherever it came from.

use std::collections::BTreeSet;

use letters_convert::ImportNote;
use letters_model::{Block, BlockId, Document, Model};

/// Ids for the blocks of an import, counted from 1.
pub fn ids() -> impl FnMut() -> String {
    let mut next = 0;
    move || {
        next += 1;
        format!("b{next}")
    }
}

/// Fails unless `doc` is one a file can hold: every block has an id of its own and holds only what its kind
/// may, and the document reads back as it is written.
pub fn sound(doc: &Document, from: &str) {
    let mut ids = BTreeSet::new();
    assert!(doc.walk().all(|block| ids.insert(&block.id)), "two blocks of {from} share an id");
    for outer in doc.walk() {
        let strays: Vec<&Block> = outer.body.blocks().iter().filter(|part| !outer.kind.admits(&part.kind)).collect();
        assert!(strays.is_empty(), "in {from}, {:?} holds {strays:?}", outer.kind);
    }
    let stored = Document::from_json(&doc.to_json());
    assert_eq!(stored.as_ref(), Ok(doc), "{from} does not read back as it is written");
}

/// Fails unless a history can start from `doc` and tell it again. Slow beside [`sound`], so only for the
/// targets that are about histories.
pub fn opens(doc: &Document, from: &str) {
    let model = Model::new(doc.clone(), 1.into());
    let reread = Model::load(&model.save(), 2.into()).expect("a saved history loads");
    assert_eq!(reread.doc(), model.doc(), "the history of {from} tells another document");
    sound(model.doc(), from);
}

/// Fails when an import's note is on no block of what it imported.
pub fn noted(doc: &Document, notes: &[ImportNote], from: &str) {
    for note in notes {
        assert!(doc.block(&BlockId::new(&note.block)).is_some(), "{note:?} is on no block of {from}");
    }
}
