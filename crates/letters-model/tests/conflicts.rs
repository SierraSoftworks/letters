//! Who wrote an edit and when, and the blocks two writers changed in the same words while apart.

mod common;

use std::cmp::Ordering;

use common::{id, saved, sync, typed};
use letters_model::{Block, BlockKind, Conflict, Document, HistoryError, Mark, Model, Op, ReplicaId, Revision};

/// A morning in January 2027, in seconds since 1970.
const MORNING: u64 = 1_800_000_000;
const HOUR: u64 = 3_600;

/// A heading `h`, two paragraphs `p1` and `p2`, and an empty one `e`.
fn doc() -> Document {
    let blocks = vec![
        Block::text("h", BlockKind::Heading { level: 1 }, "Boats"),
        Block::text("p1", BlockKind::Paragraph, "The quick brown fox jumps over the lazy dog."),
        Block::text("p2", BlockKind::Paragraph, "It was a calm day."),
        Block::text("e", BlockKind::Paragraph, ""),
    ];
    Document { blocks, ..Document::default() }
}

/// Ana and Ben with the same file open, each on a device of their own. Ben's clock is an hour on.
fn writers() -> (Model, Model) {
    let writer = |replica: u64, name: &str, time: u64| {
        let mut model = Model::new(doc(), ReplicaId::from(replica));
        model.set_author(name);
        model.set_time(time);
        model
    };
    (writer(1, "Ana", MORNING), writer(2, "Ben", MORNING + HOUR))
}

/// What a writer does to one block.
enum Edit {
    /// Puts the second text where the first stands.
    Swap(&'static str, &'static str),
    Add(&'static str),
    Bold(&'static str),
    Kind(BlockKind),
    Split(&'static str),
    Remove,
}

fn ops(model: &Model, block: &str, edit: Edit) -> Vec<Op> {
    let text = model.doc().block(&id(block)).unwrap().plain_text();
    let range = |part: &str| text.find(part).map(|at| at..at + part.len()).unwrap();
    match edit {
        Edit::Swap(old, new) => {
            vec![Op::DeleteText { block: id(block), range: range(old) }, typed(block, range(old).start, new)]
        }
        Edit::Add(new) => vec![typed(block, text.len(), new)],
        Edit::Bold(part) => vec![Op::SetMark { block: id(block), range: range(part), mark: Mark::Bold, on: true }],
        Edit::Kind(kind) => vec![Op::SetKind { block: id(block), kind }],
        Edit::Split(before) => vec![common::split(block, range(before).end, "new")],
        Edit::Remove => vec![Op::RemoveBlock { block: id(block) }],
    }
}

/// Lets each writer make an edit unseen by the other, then gives each all the other holds. Returns the
/// revisions the two stood at before.
fn part(ana: &mut Model, ben: &mut Model, hers: (&str, Edit), his: (&str, Edit)) -> (Revision, Revision) {
    ana.apply(ops(ana, hers.0, hers.1)).unwrap();
    ben.apply(ops(ben, his.0, his.1)).unwrap();
    let stood = (ana.revision(), ben.revision());
    sync(ana, ben);
    stood
}

/// A name, what each writer does to a block unseen by the other, and the blocks that then conflict.
type Apart = (&'static str, (&'static str, Edit), (&'static str, Edit), &'static [&'static str]);

fn blocks(found: &[Conflict]) -> Vec<&str> {
    found.iter().map(|conflict| conflict.block.as_str()).collect()
}

#[test]
fn only_changes_to_the_same_words_conflict() {
    use Edit::*;
    let heading = BlockKind::Heading { level: 2 };
    let cases: Vec<Apart> = vec![
        ("the same word", ("p1", Swap("quick", "slow")), ("p1", Swap("quick", "fast")), &["p1"]),
        ("a word one cuts and one adds to", ("p1", Swap("fox ", "")), ("p1", Swap("fox", "foxes")), &["p1"]),
        ("two ends for one word", ("h", Add("man")), ("h", Add("wain")), &["h"]),
        ("a paragraph one removes", ("p2", Remove), ("p2", Swap("calm", "windy")), &["p2"]),
        ("an empty paragraph one removes", ("e", Add("Notes")), ("e", Remove), &["e"]),
        ("other words of one paragraph", ("p1", Swap("quick", "slow")), ("p1", Swap("lazy", "idle")), &[]),
        ("other paragraphs", ("p1", Swap("quick", "slow")), ("p2", Swap("calm", "windy")), &[]),
        ("the same cut", ("p1", Swap("brown ", "")), ("p1", Swap("brown ", "")), &[]),
        ("a paragraph both remove", ("p2", Remove), ("p2", Remove), &[]),
        ("a new kind and a word", ("p1", Kind(heading)), ("p1", Swap("quick", "slow")), &[]),
        ("formatting and a word", ("p1", Bold("quick")), ("p1", Swap("quick", "fast")), &[]),
        ("a sentence each at the end", ("p1", Add(" It ran.")), ("p1", Add(" It hid.")), &[]),
        ("a split and the word before it", ("p1", Split("fox")), ("p1", Swap("fox", "foxes")), &[]),
    ];
    for (name, hers, his, expected) in cases {
        let (mut ana, mut ben) = writers();
        let (her, him) = part(&mut ana, &mut ben, hers, his);
        let found = ana.conflicts(&her, &him).unwrap();
        assert_eq!(blocks(&found), expected, "{name}");
    }
}

#[test]
fn a_conflict_holds_both_versions_and_who_wrote_each_when() {
    let (mut ana, mut ben) = writers();
    let (hers, his) = (ops(&ana, "p1", Edit::Swap("quick", "slow")), ops(&ben, "p1", Edit::Swap("quick", "fast")));
    ana.apply(hers).unwrap();
    ben.apply(his).unwrap();
    let (her, him) = (ana.revision(), ben.revision());
    let versions = [ana.doc().block(&id("p1")).cloned(), ben.doc().block(&id("p1")).cloned()];
    sync(&mut ana, &mut ben);

    let found = ana.conflicts(&her, &him).unwrap();
    let [conflict] = found.as_slice() else { panic!("one block conflicts, not {}", found.len()) };
    assert_eq!(conflict.block, id("p1"));
    assert_eq!(conflict.sides.each_ref().map(|side| side.block.clone()), versions);
    let written = conflict.sides.each_ref().map(|side| (side.author.as_str(), side.edited));
    assert_eq!(written, [("Ana", MORNING), ("Ben", MORNING + HOUR)]);
    assert_eq!(conflict.later, 1, "Ben wrote last");

    // A note keeps the same, with its times written out.
    let note = conflict.versions().unwrap();
    assert_eq!((note[1].author.as_str(), note[1].edited.as_str()), ("Ben", "2027-01-15T09:00:00Z"));
    assert_eq!(Some(&note[0].block), versions[0].as_ref());

    // Ben's device finds the same conflict, and the same side to show until one is picked.
    let mirrored = ben.conflicts(&him, &her).unwrap();
    let [mirror] = mirrored.as_slice() else { panic!("one block conflicts, not {}", mirrored.len()) };
    assert_eq!((&mirror.sides[0], &mirror.sides[1], mirror.later), (&conflict.sides[1], &conflict.sides[0], 0));
}

#[test]
fn a_block_one_side_took_away_has_no_version_there() {
    let (mut ana, mut ben) = writers();
    let stood = part(&mut ana, &mut ben, ("p2", Edit::Remove), ("p2", Edit::Swap("calm", "windy")));
    let found = ana.conflicts(&stood.0, &stood.1).unwrap();
    let [conflict] = found.as_slice() else { panic!("one block conflicts, not {}", found.len()) };
    assert_eq!(conflict.sides[0].block, None);
    assert_eq!(conflict.sides[1].block.as_ref().map(Block::plain_text).as_deref(), Some("It was a windy day."));
    assert_eq!((conflict.sides[0].author.as_str(), conflict.sides[1].author.as_str()), ("Ana", "Ben"));
    assert!(conflict.versions().is_none(), "a note of two versions cannot hold it");
}

#[test]
fn the_search_leaves_the_document_as_it_was_shown() {
    let (mut ana, mut ben) = writers();
    let first = (ana.doc().clone(), ana.revision());
    let stood = part(&mut ana, &mut ben, ("p1", Edit::Swap("quick", "slow")), ("p1", Edit::Swap("quick", "fast")));
    let merged = (ana.doc().clone(), ana.revision());

    assert_eq!(ana.conflicts(&stood.0, &stood.1).unwrap().len(), 1);
    assert_eq!((ana.doc(), ana.revision(), ana.is_latest()), (&merged.0, merged.1.clone(), true));
    ana.apply([typed("p2", 0, "So. ")]).unwrap();
    assert_eq!(ana.doc().block(&id("p2")).map(Block::plain_text).as_deref(), Some("So. It was a calm day."));

    // A model that shows an older revision goes on showing it.
    ana.checkout(&first.1).unwrap();
    assert_eq!(ana.conflicts(&stood.0, &stood.1).unwrap().len(), 1);
    assert_eq!((ana.doc(), ana.revision(), ana.is_latest()), (&first.0, first.1, false));
}

#[test]
fn revisions_that_did_not_part_have_no_conflicts() {
    let (mut ana, mut ben) = writers();
    let first = ana.revision();
    let stood = part(&mut ana, &mut ben, ("p1", Edit::Swap("quick", "slow")), ("p1", Edit::Swap("quick", "fast")));
    let merged = ana.revision();
    for (name, ours, theirs) in [("one before the other", &first, &stood.1), ("one holding both", &merged, &stood.0)] {
        assert_eq!(ana.conflicts(ours, theirs), Ok(Vec::new()), "{name}");
    }
    let unknown: Revision = "7@99".parse().unwrap();
    assert_eq!(ana.conflicts(&stood.0, &unknown), Err(HistoryError::UnknownRevision));
}

#[test]
fn a_history_names_everyone_who_wrote_in_it() {
    let (mut ana, mut ben) = writers();
    let mut guest = Model::new(doc(), ReplicaId::from(3));
    assert!(ana.authors().is_empty(), "no one has written yet");
    ana.apply([typed("p2", 0, "So. ")]).unwrap();
    ben.apply([typed("h", 5, "!")]).unwrap();
    guest.apply([typed("e", 0, "Hello")]).unwrap();
    sync(&mut ana, &mut ben);
    sync(&mut ana, &mut guest);
    assert_eq!(ana.authors().into_iter().collect::<Vec<_>>(), ["Ana", "Ben"]);
    assert_eq!(ana.authors(), guest.authors());
}

#[test]
fn an_edit_is_never_dated_before_one_it_builds_on() {
    let (mut ana, mut ben) = writers();
    // Ben's clock is wrong by a day when he reads what Ana wrote and writes on.
    ana.apply(ops(&ana, "p1", Edit::Swap("quick", "slow"))).unwrap();
    let mut eve = ana.fork(ReplicaId::from(3));
    ben.merge(&ana.save()).unwrap();
    ben.set_time(MORNING - 24 * HOUR);
    ben.apply(ops(&ben, "p1", Edit::Swap("slow", "fast"))).unwrap();
    eve.apply(ops(&eve, "p1", Edit::Swap("slow", "idle"))).unwrap();
    let (his, hers) = (ben.revision(), eve.revision());
    sync(&mut ben, &mut eve);

    let found = ben.conflicts(&his, &hers).unwrap();
    let [conflict] = found.as_slice() else { panic!("one block conflicts, not {}", found.len()) };
    assert_eq!(conflict.sides[0].edited, MORNING, "an edit on top of Ana's is no older than hers");
    // Eve gave no name and told no time, so hers is as old as what she built on.
    assert_eq!((conflict.sides[1].author.as_str(), conflict.sides[1].edited), ("", MORNING));
}

#[test]
fn an_outside_edit_is_the_same_change_whoever_finds_it_and_when() {
    let (file, history) = saved();
    let mut outside = file;
    outside.blocks[0] = Block::text("p", BlockKind::Paragraph, "Hello there world!");
    let find = |replica: u64, name: &str, time: u64| {
        let mut model = Model::load(&history, ReplicaId::from(replica)).unwrap();
        model.set_author(name);
        model.set_time(time);
        model.rewrite(outside.clone()).unwrap();
        model
    };
    let (ana, ben) = (find(2, "Ana", MORNING), find(3, "Ben", MORNING + HOUR));
    assert_eq!(ana.compare(&ben.save()), Ok(Some(Ordering::Equal)));
    assert_eq!(ana.save(), ben.save(), "the change differs in nothing, its time included");
    assert!(ana.authors().is_empty(), "neither of them wrote it");
}
