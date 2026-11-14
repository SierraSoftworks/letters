//! A document's history: merging replicas, moving between revisions, and positions that follow their text.

use std::cmp::Ordering;

use letters_model::{
    Block, BlockId, BlockKind, Document, HistoryError, Hold, Mark, Model, Op, OpError, Pos, ReplicaId, Revision,
    StablePos,
};

/// A paragraph `p` reading "Hello world", then a divider `d`, with every field of its own set.
fn doc() -> Document {
    let blocks = vec![Block::text("p", BlockKind::Paragraph, "Hello world"), Block::new("d", BlockKind::Divider)];
    let mut doc = Document { blocks, language: "en-GB".into(), ..Document::default() };
    doc.dictionary.insert("Letters".into());
    doc.extra.insert("future".into(), serde_json::json!({ "kept": true }));
    doc
}

/// Two replicas of [`doc`], as two devices that opened the same file.
fn pair() -> (Model, Model) {
    (Model::new(doc(), ReplicaId::from(1)), Model::new(doc(), ReplicaId::from(2)))
}

/// Gives each replica everything the other holds.
fn sync(a: &mut Model, b: &mut Model) {
    a.merge(&b.save()).unwrap();
    b.merge(&a.save()).unwrap();
}

fn id(id: &str) -> BlockId {
    id.into()
}

fn typed(block: &str, offset: usize, text: &str) -> Op {
    Op::InsertText { at: Pos::new(block, offset), text: text.into() }
}

fn split(block: &str, offset: usize, new_id: &str) -> Op {
    Op::Split { at: Pos::new(block, offset), new_id: id(new_id) }
}

fn texts(model: &Model) -> Vec<String> {
    model.doc().walk().map(Block::plain_text).collect()
}

/// A name, what each of two replicas does unseen by the other, and the text of every block once merged.
type Concurrent = (&'static str, Vec<Op>, Vec<Op>, Vec<&'static str>);

#[test]
fn concurrent_edits_all_survive_a_merge() {
    let cases: Vec<Concurrent> = vec![
        ("typing at both ends", vec![typed("p", 0, "Oh. ")], vec![typed("p", 11, "!")], vec!["Oh. Hello world!", ""]),
        ("typing after a split", vec![split("p", 5, "p2")], vec![typed("p", 11, "!")], vec!["Hello", " world!", ""]),
        (
            "typing before a split",
            vec![split("p", 5, "p2")],
            vec![typed("p", 0, "Oh. ")],
            vec!["Oh. Hello", " world", ""],
        ),
        (
            "deleting across a split",
            vec![split("p", 5, "p2")],
            vec![Op::DeleteText { block: id("p"), range: 3..8 }],
            vec!["Hel", "rld", ""],
        ),
        (
            "typing into a removed block",
            vec![Op::RemoveBlock { block: id("p") }],
            vec![typed("p", 5, ",")],
            vec![",", ""],
        ),
        (
            "typing at the start of a block that changes kind",
            vec![Op::SetKind { block: id("p"), kind: BlockKind::Heading { level: 2 } }],
            vec![typed("p", 0, "Oh. "), split("p", 0, "p0")],
            vec!["", "Oh. Hello world", ""],
        ),
        (
            "a new kind and a removal",
            vec![Op::SetKind { block: id("p"), kind: BlockKind::Heading { level: 2 } }],
            vec![Op::RemoveBlock { block: id("d") }],
            vec!["Hello world"],
        ),
    ];
    for (name, ours, theirs, expected) in cases {
        let (mut a, mut b) = pair();
        a.apply(ours).unwrap();
        b.apply(theirs).unwrap();
        sync(&mut a, &mut b);
        assert_eq!(a.doc(), b.doc(), "{name}");
        assert_eq!(texts(&a), expected, "{name}");
    }
}

#[test]
fn one_file_opened_twice_is_one_history() {
    let (mut a, mut b) = pair();
    assert_eq!(a.revision(), b.revision());
    assert_eq!(a.compare(&b.save()), Ok(Some(Ordering::Equal)));

    let change = a.merge(&b.save()).unwrap();
    assert!(change.blocks.is_empty() && !change.structure);
    b.apply([typed("p", 11, "!")]).unwrap();
    sync(&mut a, &mut b);
    assert_eq!(texts(&a), ["Hello world!", ""]);
}

#[test]
fn a_saved_history_loads_as_the_same_document() {
    let mut a = Model::new(doc(), ReplicaId::from(1));
    a.apply([split("p", 5, "p2"), typed("p2", 0, ",")]).unwrap();

    let mut b = Model::load(&a.save(), ReplicaId::from(2)).unwrap();
    assert_eq!(b.doc(), a.doc());
    assert_eq!(b.revision(), a.revision());
    assert!(b.undo().is_none(), "undo steps belong to the session that made them");

    // The loaded copy is a replica like any other.
    b.apply([typed("p", 0, "Oh. ")]).unwrap();
    a.merge(&b.save()).unwrap();
    assert_eq!(a.doc(), b.doc());
}

#[test]
fn updates_bring_a_replica_up_to_date() {
    let mut a = Model::new(doc(), ReplicaId::from(1));
    let mut b = a.fork(ReplicaId::from(2));
    a.apply([typed("p", 0, "Oh. ")]).unwrap();
    a.apply([Op::RemoveBlock { block: id("d") }]).unwrap();

    let change = b.merge(&a.updates_since(&b.revision())).unwrap();
    assert_eq!(b.doc(), a.doc());
    assert!(change.structure && change.blocks.contains(&id("p")));
}

#[test]
fn a_history_that_cannot_be_read_is_refused() {
    let mut m = Model::new(doc(), ReplicaId::from(1));
    let before = (m.doc().clone(), m.revision());
    for bytes in [&b""[..], b"not a history", &m.save()[..40]] {
        assert!(matches!(Model::load(bytes, ReplicaId::from(2)), Err(HistoryError::Unreadable(_))));
        assert!(matches!(m.merge(bytes), Err(HistoryError::Unreadable(_))));
        assert!(matches!(m.compare(bytes), Err(HistoryError::Unreadable(_))));
        assert_eq!((m.doc().clone(), m.revision()), before);
    }
}

#[test]
fn compare_tells_which_history_holds_more() {
    let cases = [
        (false, false, Some(Ordering::Equal)),
        (true, false, Some(Ordering::Greater)),
        (false, true, Some(Ordering::Less)),
        (true, true, None),
    ];
    for (ours, theirs, expected) in cases {
        let (mut a, mut b) = pair();
        if ours {
            a.apply([typed("p", 0, "A")]).unwrap();
        }
        if theirs {
            b.apply([typed("p", 0, "B")]).unwrap();
        }
        assert_eq!(a.compare(&b.save()), Ok(expected), "ours edited: {ours}, theirs edited: {theirs}");
    }
}

#[test]
fn undo_leaves_what_others_wrote() {
    let (mut a, mut b) = pair();
    a.apply([typed("p", 0, "Oh. ")]).unwrap();
    b.apply([typed("p", 11, "!")]).unwrap();
    sync(&mut a, &mut b);

    a.undo().unwrap();
    assert_eq!(texts(&a), ["Hello world!", ""]);
    assert!(a.undo().is_none(), "another replica's edit is not this one's to undo");
    a.redo().unwrap();
    assert_eq!(texts(&a), ["Oh. Hello world!", ""]);

    // An undo is an edit like any other, and merges.
    b.undo().unwrap();
    sync(&mut a, &mut b);
    assert_eq!(texts(&a), ["Oh. Hello world", ""]);
    assert_eq!(a.doc(), b.doc());
}

#[test]
fn undo_takes_back_only_its_own_edit_after_a_merge() {
    // This replica makes `ours`, takes in `theirs`, and undoes. A redo then gives the merged text back.
    let cases: Vec<Concurrent> = vec![
        (
            "text typed inside its own",
            vec![typed("p", 5, ", dear")],
            vec![typed("p", 8, "!")],
            vec!["Hello! world", ""],
        ),
        (
            "part of its text deleted",
            vec![typed("p", 5, ", dear")],
            vec![Op::DeleteText { block: id("p"), range: 6..9 }],
            vec!["Hello world", ""],
        ),
        (
            "text typed where it deleted",
            vec![Op::DeleteText { block: id("p"), range: 5..11 }],
            vec![typed("p", 5, "!")],
            vec!["Hello! world", ""],
        ),
        ("text typed after its split", vec![split("p", 5, "p2")], vec![typed("p2", 6, "!")], vec!["Hello world!", ""]),
        (
            "a mark set across its text",
            vec![typed("p", 5, ", dear")],
            vec![Op::SetMark { block: id("p"), range: 0..17, mark: Mark::Bold, on: true }],
            vec!["Hello world", ""],
        ),
    ];
    for (name, ours, theirs, expected) in cases {
        let (mut a, mut b) = pair();
        a.apply(ours).unwrap();
        sync(&mut a, &mut b);
        b.apply(theirs).unwrap();
        a.merge(&b.save()).unwrap();
        let merged = a.doc().clone();

        a.undo().unwrap();
        assert_eq!(texts(&a), expected, "{name}");
        a.redo().unwrap();
        assert_eq!(a.doc(), &merged, "{name}, redone");

        a.undo().unwrap();
        sync(&mut a, &mut b);
        assert_eq!(a.doc(), b.doc(), "{name}");
        assert_eq!(texts(&b), expected, "{name}, on the other replica");
    }
}

#[test]
fn an_older_revision_is_shown_read_only() {
    let (mut a, mut b) = pair();
    let first = a.revision();
    a.apply([typed("p", 0, "Oh. ")]).unwrap();
    b.apply([typed("p", 11, "!")]).unwrap();

    let change = a.checkout(&first).unwrap();
    assert!(change.blocks.contains(&id("p")));
    assert_eq!(a.doc(), &doc());
    assert_eq!(a.revision(), first);
    assert!(!a.is_latest());
    assert_eq!(a.apply([typed("p", 0, "x")]), Err(OpError::ReadOnly));
    assert!(a.undo().is_none());

    // What arrives in the meantime waits for the return to the newest revision.
    a.merge(&b.save()).unwrap();
    assert_eq!(a.doc(), &doc());
    a.checkout_latest();
    assert!(a.is_latest());
    assert_eq!(texts(&a), ["Oh. Hello world!", ""]);
    a.apply([typed("p", 0, "x")]).unwrap();
}

#[test]
fn a_revision_the_history_lacks_is_refused() {
    let (mut a, mut b) = pair();
    b.apply([typed("p", 0, "B")]).unwrap();
    let before = a.doc().clone();
    assert_eq!(a.checkout(&b.revision()), Err(HistoryError::UnknownRevision));
    assert_eq!(a.doc(), &before);
    assert!(a.is_latest());
}

#[test]
fn revisions_and_positions_survive_being_written_down() {
    let (mut a, mut b) = pair();
    a.apply([typed("p", 0, "A")]).unwrap();
    b.apply([typed("p", 0, "B")]).unwrap();
    sync(&mut a, &mut b);

    let revision = a.revision();
    assert_eq!(revision.to_string().parse(), Ok(revision));
    for (pos, hold) in [(Pos::new("p", 3), Hold::Next), (Pos::new("p", 3), Hold::Prev), (Pos::new("p", 13), Hold::Next)]
    {
        let stable = a.stable(&pos, hold).unwrap();
        assert_eq!(stable.to_string().parse(), Ok(stable.clone()));
        assert_eq!(a.resolve(&stable), Some(pos));
    }

    for text in ["x", "1@", "@2", "-1@2", "1@2,x"] {
        assert!(text.parse::<Revision>().is_err(), "{text:?} read as a revision");
        assert!(text.parse::<StablePos>().is_err(), "{text:?} read as a position");
    }
}

#[test]
fn a_stable_position_follows_its_text() {
    // Each position is taken on one replica, and the edits arrive from another.
    let cases: Vec<(&str, Pos, Hold, Vec<Op>, Pos)> = vec![
        ("text typed before it", Pos::new("p", 6), Hold::Next, vec![typed("p", 0, "é字 ")], Pos::new("p", 12)),
        ("text typed after it", Pos::new("p", 6), Hold::Next, vec![typed("p", 8, "x")], Pos::new("p", 6)),
        (
            "text typed there, held by what follows",
            Pos::new("p", 6),
            Hold::Next,
            vec![typed("p", 6, "new ")],
            Pos::new("p", 10),
        ),
        (
            "text typed there, held by what precedes",
            Pos::new("p", 6),
            Hold::Prev,
            vec![typed("p", 6, "new ")],
            Pos::new("p", 6),
        ),
        (
            "text typed at the end of its block",
            Pos::new("p", 11),
            Hold::Prev,
            vec![typed("p", 11, "!")],
            Pos::new("p", 11),
        ),
        ("its block is split before it", Pos::new("p", 6), Hold::Next, vec![split("p", 5, "p2")], Pos::new("p2", 1)),
        (
            "its text is deleted",
            Pos::new("p", 6),
            Hold::Next,
            vec![Op::DeleteText { block: id("p"), range: 4..8 }],
            Pos::new("p", 4),
        ),
        (
            "the block before is removed",
            Pos::new("q", 2),
            Hold::Next,
            vec![Op::RemoveBlock { block: id("p") }],
            Pos::new("q", 2),
        ),
    ];
    for (name, pos, hold, ops, expected) in cases {
        let (mut a, mut b) = pair();
        a.apply([Op::InsertBlock { parent: None, index: 2, block: Block::text("q", BlockKind::Paragraph, "Last") }])
            .unwrap();
        sync(&mut a, &mut b);
        let stable = a.stable(&pos, hold).unwrap();
        b.apply(ops).unwrap();
        a.merge(&b.save()).unwrap();
        assert_eq!(a.resolve(&stable), Some(expected.clone()), "{name}");
        assert_eq!(b.resolve(&stable), Some(expected), "{name}, on the other replica");
    }
}

#[test]
fn only_a_position_in_text_can_be_made_stable() {
    let m = Model::new(doc(), ReplicaId::from(1));
    for pos in [Pos::new("d", 0), Pos::new("p", 99), Pos::new("nope", 0)] {
        assert_eq!(m.stable(&pos, Hold::Next), None, "{pos:?}");
    }
}
