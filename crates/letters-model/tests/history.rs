//! A document's history: merging replicas, moving between revisions, and positions that follow their text.

mod common;

use std::cmp::Ordering;

use common::{doc, id, pair, saved, split, sync, texts, typed};
use letters_model::{
    Block, BlockKind, Body, Change, Document, Extent, HistoryError, Hold, Mark, Model, Op, OpError, Page, Pos,
    ReplicaId, Revision, Run, StablePos, Style,
};

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
        // Text that replaces a word stands where the word stood, behind what was typed before it.
        (
            "typing before a word that is replaced",
            vec![typed("p", 6, "big ")],
            vec![Op::DeleteText { block: id("p"), range: 6..11 }, typed("p", 6, "moon")],
            vec!["Hello big moon", ""],
        ),
        (
            "replacing a word that is typed before",
            vec![Op::DeleteText { block: id("p"), range: 6..11 }, typed("p", 6, "moon")],
            vec![typed("p", 6, "big ")],
            vec!["Hello big moon", ""],
        ),
        (
            "typing after a word that is replaced",
            vec![typed("p", 11, ", again")],
            vec![Op::DeleteText { block: id("p"), range: 6..11 }, typed("p", 6, "moon")],
            vec!["Hello moon, again", ""],
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
fn replicas_that_trade_extents_send_only_what_the_other_lacks() {
    let cases = [(false, false), (true, false), (false, true), (true, true)];
    for (ours, theirs) in cases {
        let (mut a, mut b) = pair();
        if ours {
            a.apply([typed("p", 0, "A")]).unwrap();
        }
        if theirs {
            b.apply([typed("p", 11, "B")]).unwrap();
        }
        let (held_a, held_b) = (a.extent(), b.extent());
        let expected = match (ours, theirs) {
            (false, false) => Some(Ordering::Equal),
            (true, false) => Some(Ordering::Greater),
            (false, true) => Some(Ordering::Less),
            (true, true) => None,
        };
        assert_eq!(held_a.partial_cmp(&held_b), expected, "ours edited: {ours}, theirs edited: {theirs}");
        assert!(held_a.shares(&held_b) && !held_a.is_empty());

        // Each is told the other's extent as bytes, and answers with the rest.
        let (to_b, to_a) = (a.updates_for(&round_trip(&held_b)), b.updates_for(&round_trip(&held_a)));
        // An update reaches as far as its edits, so what was traded is known without reading the models.
        let mut known = held_b.clone();
        known.merge(&Extent::of(&to_b).unwrap());
        assert!(known >= held_a, "ours edited: {ours}, theirs edited: {theirs}");
        assert_eq!(a.merge(&to_a).unwrap() == Change::default(), !theirs);
        assert_eq!(b.merge(&to_b).unwrap() == Change::default(), !ours);
        assert_eq!(a.doc(), b.doc());
        assert_eq!(a.extent(), b.extent());
    }
}

#[test]
fn an_update_that_arrives_before_what_it_builds_on_waits_and_the_extent_tells() {
    let (mut a, mut b) = pair();
    let start = a.extent();
    a.apply([typed("p", 0, "1")]).unwrap();
    let (first, between) = (a.updates_for(&start), a.extent());
    a.apply([typed("p", 0, "2")]).unwrap();
    let second = a.updates_for(&between);
    let reach = Extent::of(&second).unwrap();
    let holds = |model: &Model| model.extent() >= reach;

    let held = b.extent();
    assert_eq!(b.merge(&second), Ok(Change::default()), "an update with a gap before it showed");
    assert!(b.extent() == held && !holds(&b), "an update that waits counts as held");

    b.merge(&first).unwrap();
    assert_eq!(texts(&b)[0], "21Hello world", "the update did not land once the gap was filled");
    assert!(holds(&b) && b.extent() == a.extent());
}

fn round_trip(extent: &Extent) -> Extent {
    Extent::from_bytes(&extent.to_bytes()).unwrap()
}

#[test]
fn an_extent_tells_copies_of_other_documents_apart() {
    let (a, _) = pair();
    let stranger =
        Model::new(Document { blocks: vec![Block::text("p", BlockKind::Paragraph, "Other")], ..doc() }, 3.into());
    assert!(!a.extent().shares(&stranger.extent()));
    assert!(Extent::default().is_empty() && !a.extent().shares(&Extent::default()));
    // A copy with nothing is sent everything, which is a history that loads.
    let whole = a.updates_for(&Extent::default());
    assert_eq!(Model::load(&whole, ReplicaId::from(4)).unwrap().doc(), a.doc());
    for bytes in [&b"\xff\xff\xff\xff\xff"[..], b"not an extent at all, and far too long to be one"] {
        assert!(matches!(Extent::from_bytes(bytes), Err(HistoryError::Unreadable(_))), "{bytes:?}");
    }
    assert!(matches!(Extent::of(b"not a history"), Err(HistoryError::Unreadable(_))));
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
fn an_edit_that_others_left_nothing_of_is_still_a_step_to_undo() {
    // This replica types twice, and the other deletes what the second edit typed.
    let (mut a, mut b) = pair();
    a.apply([typed("p", 0, "Oh. ")]).unwrap();
    a.apply([typed("p", 15, "!")]).unwrap();
    sync(&mut a, &mut b);
    b.apply([Op::DeleteText { block: id("p"), range: 15..16 }]).unwrap();
    a.merge(&b.save()).unwrap();
    assert_eq!(texts(&a), ["Oh. Hello world", ""]);

    // Whoever counts its edits steps back as many: over the second, which changes nothing, to the first.
    assert_eq!(a.undo(), Some(Change::default()));
    a.undo().unwrap();
    assert_eq!(texts(&a), ["Hello world", ""]);
    assert!(a.undo().is_none());
    a.redo().unwrap();
    assert_eq!(a.redo(), Some(Change::default()));
    assert_eq!(texts(&a), ["Oh. Hello world", ""]);
    assert!(a.redo().is_none());
}

#[test]
fn ops_of_one_batch_name_blocks_that_share_an_id_as_they_stood_before_it() {
    // Each replica adds a block under the same id. Merged, the second goes by a number.
    let (mut a, mut b) = pair();
    let added =
        |text: &str| Op::InsertBlock { parent: None, index: 1, block: Block::text("n", BlockKind::Paragraph, text) };
    a.apply([added("one")]).unwrap();
    b.apply([added("two")]).unwrap();
    sync(&mut a, &mut b);
    let second = a.block(&id("n~2")).expect("the second of the two").plain_text();

    // Taking the first away renames the second, which the ops after it still call by its number.
    let ops = [Op::RemoveBlock { block: id("n") }, typed("n~2", 0, "> "), Op::RemoveBlock { block: id("d") }];
    a.apply(ops).unwrap();
    assert_eq!(texts(&a), ["Hello world".to_string(), format!("> {second}")]);
    assert!(a.block(&id("n~2")).is_none());
    a.undo().unwrap();
    assert_eq!(a.block(&id("n~2")).map(Block::plain_text), Some(second));
}

#[test]
fn a_block_named_like_a_copy_is_told_from_the_copies() {
    // "n~2" is a block of its own. Each replica adds a block under the id "n" after it, and merged, the
    // second of those goes by the next free number.
    let own = Document { blocks: vec![Block::text("n~2", BlockKind::Paragraph, "own")], ..Document::default() };
    let (mut a, mut b) = (Model::new(own.clone(), ReplicaId::from(1)), Model::new(own, ReplicaId::from(2)));
    let added =
        |text: &str| Op::InsertBlock { parent: None, index: 1, block: Block::text("n", BlockKind::Paragraph, text) };
    a.apply([added("one")]).unwrap();
    b.apply([added("two")]).unwrap();
    sync(&mut a, &mut b);
    let second = a.block(&id("n~3")).expect("the second of the two").plain_text();

    // With the first of the two gone, an op that names the second still means it.
    a.apply([Op::RemoveBlock { block: id("n") }, typed("n~3", 0, "> ")]).unwrap();
    assert_eq!(texts(&a), ["own".to_string(), format!("> {second}")]);
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

#[test]
fn a_document_opens_as_its_history_holds_it() {
    let paragraph = |id: &str, text: &str| Block::text(id, BlockKind::Paragraph, text);
    let with = |block: Block, body: Body| Block { body, ..block };
    let runs = vec![
        Run::plain("a"),
        Run::plain(""),
        Run::plain("b\u{1e}c"),
        Run::marked("d", [Mark::Other("bold".into())]),
        Run::marked("e", [Mark::Bold]),
    ];
    let mut unknown = paragraph("x", "kept");
    unknown.extra.insert("lang".into(), "fr".into());
    unknown.extra.insert("type".into(), "heading".into());
    let figure = Block::new("u", BlockKind::Unknown { type_name: "figure".into() });
    let twice =
        with(Block::new("q", BlockKind::Quote), Body::Blocks(vec![paragraph("p", "inside"), paragraph("p", "")]));
    let cases = vec![
        ("ids used more than once", vec![paragraph("p", "one"), twice, paragraph("p~2", "two")]),
        ("runs that could be one", vec![with(paragraph("m", ""), Body::Text(runs))]),
        ("fields and types this build does not know", vec![unknown, figure]),
        ("a level no heading has", vec![Block::text("h", BlockKind::Heading { level: 9 }, "Title")]),
        ("a body of the wrong shape", vec![with(paragraph("w", ""), Body::Blocks(vec![paragraph("in", "lost")]))]),
    ];
    for (name, blocks) in cases {
        let model = Model::new(Document { blocks, ..doc() }, ReplicaId::from(1));
        let stored = Model::load(&model.save(), ReplicaId::from(2)).unwrap();
        assert_eq!(model.doc(), stored.doc(), "{name}");
        assert_eq!(Model::new(model.doc().clone(), ReplicaId::from(3)).doc(), model.doc(), "{name}, opened again");
        assert_eq!(Document::from_json(&model.doc().to_json()).as_ref(), Ok(model.doc()), "{name}, as a file");
    }
}

#[test]
fn a_prepared_history_is_attached_without_a_change() {
    let (file, history) = saved();
    for history in [None, Some(&history[..])] {
        let mut model = Model::new(file.clone(), ReplicaId::from(2));
        let prepared = Model::new(file.clone(), ReplicaId::from(2)).prepare(history).unwrap();
        assert_eq!(model.attach(prepared), Change::default());
        assert_eq!(model.doc(), &file);
        if let Some(history) = history {
            assert_eq!(model.compare(history), Ok(Some(Ordering::Equal)));
        }
        model.apply([typed("p", 0, "Oh. ")]).unwrap();
        assert_eq!(texts(&model), ["Oh. Hello world!", ""]);
    }
    assert!(matches!(
        Model::new(file, ReplicaId::from(2)).prepare(Some(b"not a history")),
        Err(HistoryError::Unreadable(_))
    ));
}

#[test]
fn a_file_without_a_history_keeps_the_one_its_model_started() {
    let mut model = Model::new(doc(), ReplicaId::from(2));
    model.apply([typed("p", 0, "Oh. ")]).unwrap();
    let (revision, held) = (model.revision(), model.stable(&Pos::new("p", 4), Hold::Next).unwrap());

    let prepared = Model::new(doc(), ReplicaId::from(2)).prepare(None).unwrap();
    assert_eq!(model.attach(prepared), Change::default());
    assert_eq!(model.revision(), revision);
    assert_eq!(model.resolve(&held), Some(Pos::new("p", 4)));
    model.undo().unwrap();
    assert_eq!(model.doc(), &doc());
}

#[test]
fn edits_made_before_a_history_is_attached_are_kept() {
    let (file, history) = saved();
    let mut model = Model::new(file.clone(), ReplicaId::from(2));
    model.apply([typed("p", 0, "Oh. ")]).unwrap();
    let edited = model.doc().clone();

    let prepared = Model::new(file, ReplicaId::from(2)).prepare(Some(&history)).unwrap();
    assert_eq!(model.attach(prepared), Change::default());
    assert_eq!(model.doc(), &edited);
    assert_eq!(model.compare(&history), Ok(Some(Ordering::Greater)));

    // The edit merges with whatever else the file's history leads to, and is still this session's to undo.
    let mut other = Model::load(&history, ReplicaId::from(3)).unwrap();
    other.apply([typed("p", 12, "?")]).unwrap();
    sync(&mut model, &mut other);
    assert_eq!(texts(&model), ["Oh. Hello world!?", ""]);
    model.undo().unwrap();
    assert_eq!(texts(&model), ["Hello world!?", ""]);
}

#[test]
fn a_file_changed_outside_is_one_more_change_in_its_history() {
    let (file, history) = saved();
    let mut outside = file.clone();
    outside.blocks[0] = Block::text("p", BlockKind::Heading { level: 1 }, "Hello, world!");
    outside.blocks.insert(0, Block::text("new", BlockKind::Paragraph, "Added"));
    outside.blocks.pop();
    outside.language = "fr".into();
    outside.dictionary.insert("mot".into());

    let mut model = Model::new(outside.clone(), ReplicaId::from(2));
    let prepared = Model::new(outside.clone(), ReplicaId::from(2)).prepare(Some(&history)).unwrap();
    assert_eq!(model.attach(prepared), Change::default());
    assert_eq!(model.doc(), &outside);
    assert_eq!(model.compare(&history), Ok(Some(Ordering::Greater)));

    // A copy that knows only the old history merges with it, and what it typed lands in the changed text.
    let mut other = Model::load(&history, ReplicaId::from(3)).unwrap();
    other.apply([typed("p", 12, "?")]).unwrap();
    sync(&mut model, &mut other);
    assert_eq!(model.doc(), other.doc());
    assert_eq!(texts(&model), ["Added", "Hello, world!?"]);
    assert_eq!((model.doc().language.as_str(), model.doc().dictionary.contains("mot")), ("fr", true));
}

#[test]
fn a_rewrite_changes_only_what_differs() {
    // What the document is rewritten to, and whether `p` is still the block it was.
    type Rewrite = (&'static str, fn(&mut Document), bool);
    let cases: Vec<Rewrite> = vec![
        ("text changed at its end", |doc| doc.blocks[0] = Block::text("p", BlockKind::Paragraph, "Hello world."), true),
        ("a new kind", |doc| doc.blocks[0].kind = BlockKind::Heading { level: 3 }, true),
        ("a block added and one removed", |doc| doc.blocks[1] = Block::text("n", BlockKind::Paragraph, "New"), true),
        ("everything replaced", |doc| doc.blocks = vec![Block::text("z", BlockKind::Paragraph, "Other")], false),
        ("nothing", |_| {}, true),
    ];
    for (name, change, kept) in cases {
        let (mut a, mut b) = pair();
        let mut target = doc();
        change(&mut target);
        b.apply([typed("p", 5, ",")]).unwrap();

        let changed = a.rewrite(target.clone()).unwrap();
        assert_eq!(a.doc(), &target, "{name}");
        assert_eq!(changed == Change::default(), target == doc(), "{name}");
        assert!(a.undo().is_none(), "{name}: a rewrite is not this session's edit");

        sync(&mut a, &mut b);
        assert_eq!(a.doc(), b.doc(), "{name}");
        let typed = a.doc().block(&id("p")).is_some_and(|p| p.plain_text().starts_with("Hello,"));
        assert_eq!(typed, kept, "{name}: what another replica typed into a kept block stays");
    }
}

#[test]
fn the_same_outside_edit_is_one_change_on_every_device() {
    let (file, history) = saved();
    let mut outside = file.clone();
    outside.blocks[0].body =
        Body::Text(vec![Run::plain("Hello "), Run::marked("there", [Mark::Bold, Mark::Italic]), Run::plain(" world!")]);
    outside.blocks.insert(1, Block::text("q", BlockKind::Paragraph, "Aside"));
    outside.dictionary.insert("mot".into());
    let style = Style { page: Page { width: 400.0, ..Page::default() }, ..Style::default() };

    // Two devices open the file another tool changed, each knowing only the history it holds.
    let open = |replica: u64, doc: &Document| {
        let file = || Model::new(doc.clone(), ReplicaId::from(replica)).with_style(style.clone());
        let mut model = file();
        model.attach(file().prepare(Some(&history)).unwrap());
        model
    };
    let (mut a, b) = (open(2, &outside), open(3, &outside));
    assert_eq!(a.compare(&b.save()), Ok(Some(Ordering::Equal)), "both hold the same history");
    assert_eq!(a.merge(&b.save()), Ok(Change::default()));
    assert_eq!((a.doc(), a.style()), (&outside, &style));

    // A different change to the same file is another change, and both are kept.
    let mut other = file;
    other.blocks[0] = Block::text("p", BlockKind::Paragraph, "Hello world! Again");
    let mut c = open(4, &other);
    assert_eq!(a.compare(&c.save()), Ok(None));
    sync(&mut a, &mut c);
    assert_eq!(a.doc(), c.doc());
    assert_eq!(texts(&a), ["Hello there world! Again", "Aside", ""]);
}

#[test]
fn histories_that_share_nothing_are_told_apart() {
    let (mut a, b) = pair();
    a.apply([typed("p", 0, "Oh. ")]).unwrap();
    let stranger =
        Model::new(Document { blocks: vec![Block::text("p", BlockKind::Paragraph, "Other")], ..doc() }, 3.into());
    let cases = [
        ("a copy of the same file", b.save(), true),
        ("a fork", a.fork(ReplicaId::from(4)).save(), true),
        ("another document", stranger.save(), false),
    ];
    for (name, history, related) in cases {
        assert_eq!(a.related(&history), Ok(related), "{name}");
    }
    assert!(matches!(a.related(b"not a history"), Err(HistoryError::Unreadable(_))));
}
