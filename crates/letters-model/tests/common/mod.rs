//! The document and the replicas the tests of a history start from.
#![allow(dead_code)]

use letters_model::{Block, BlockId, BlockKind, Document, Model, Op, Pos, ReplicaId};

/// A paragraph `p` reading "Hello world", then a divider `d`, with every field of its own set.
pub fn doc() -> Document {
    let blocks = vec![Block::text("p", BlockKind::Paragraph, "Hello world"), Block::new("d", BlockKind::Divider)];
    let mut doc = Document { blocks, language: "en-GB".into(), ..Document::default() };
    doc.dictionary.insert("Letters".into());
    doc.extra.insert("future".into(), serde_json::json!({ "kept": true }));
    doc
}

/// Two replicas of [`doc`], as two devices that opened the same file.
pub fn pair() -> (Model, Model) {
    (Model::new(doc(), ReplicaId::from(1)), Model::new(doc(), ReplicaId::from(2)))
}

/// Gives each replica everything the other holds.
pub fn sync(a: &mut Model, b: &mut Model) {
    a.merge(&b.save()).unwrap();
    b.merge(&a.save()).unwrap();
}

/// A file as a session left it: its document, and the history that leads there.
pub fn saved() -> (Document, Vec<u8>) {
    let mut writer = Model::new(doc(), ReplicaId::from(1));
    writer.apply([typed("p", 11, "!")]).unwrap();
    (writer.doc().clone(), writer.save())
}

pub fn id(id: &str) -> BlockId {
    id.into()
}

pub fn typed(block: &str, offset: usize, text: &str) -> Op {
    Op::InsertText { at: Pos::new(block, offset), text: text.into() }
}

pub fn split(block: &str, offset: usize, new_id: &str) -> Op {
    Op::Split { at: Pos::new(block, offset), new_id: id(new_id) }
}

pub fn texts(model: &Model) -> Vec<String> {
    model.doc().walk().map(Block::plain_text).collect()
}
