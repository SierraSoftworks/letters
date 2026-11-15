//! Who wrote an edit and when, the blocks two writers changed in the same words while apart, and how such
//! a block reads once the two meet.

mod common;

use std::cmp::Ordering;

use common::{id, saved, sync, texts, typed};
use letters_model::{Block, BlockKind, Conflict, Document, HistoryError, Mark, Model, Op, ReplicaId, Revision};

/// A morning in January 2027, in seconds since 1970.
const MORNING: u64 = 1_800_000_000;
const HOUR: u64 = 3_600;

/// A heading `h`, two paragraphs `p1` and `p2`, and an empty one `e`.
fn doc() -> Document {
    let blocks = vec![
        Block::text("h", BlockKind::Heading { level: 1 }, "Boats"),
        Block::text("p1", BlockKind::Paragraph, P1),
        Block::text("p2", BlockKind::Paragraph, P2),
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
    /// Puts the second text right before the first, or right after it.
    Before(&'static str, &'static str),
    After(&'static str, &'static str),
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
        Edit::Before(part, new) => vec![typed(block, range(part).start, new)],
        Edit::After(part, new) => vec![typed(block, range(part).end, new)],
        Edit::Add(new) => vec![typed(block, text.len(), new)],
        Edit::Bold(part) => vec![Op::SetMark { block: id(block), range: range(part), mark: Mark::Bold, on: true }],
        Edit::Kind(kind) => vec![Op::SetKind { block: id(block), kind }],
        Edit::Split(before) => vec![common::split(block, range(before).end, "new")],
        Edit::Remove => vec![Op::RemoveBlock { block: id(block) }],
    }
}

/// Edits to blocks, in the order a writer makes them.
type Edits = Vec<(&'static str, Edit)>;

fn write(model: &mut Model, edits: Edits) {
    for (block, edit) in edits {
        model.apply(ops(model, block, edit)).unwrap();
    }
}

/// Lets each writer make an edit unseen by the other, then gives each all the other holds. Returns the
/// revisions the two stood at before.
fn part(
    ana: &mut Model,
    ben: &mut Model,
    hers: (&'static str, Edit),
    his: (&'static str, Edit),
) -> (Revision, Revision) {
    write(ana, vec![hers]);
    write(ben, vec![his]);
    let stood = (ana.revision(), ben.revision());
    sync(ana, ben);
    stood
}

/// Gives a device what another one held, and settles what the two changed alike. Returns the conflicts.
fn meet(model: &mut Model, held: &[u8], ours: &Revision, theirs: &Revision) -> Vec<Conflict> {
    model.merge(held).unwrap();
    model.settle(ours, theirs).unwrap().1
}

/// Lets each writer make edits unseen by the other. Then each device takes in what the other held and
/// settles the conflicts by itself, with no word from the other about how.
fn settle(ana: &mut Model, ben: &mut Model, hers: Edits, his: Edits) -> [Vec<Conflict>; 2] {
    write(ana, hers);
    write(ben, his);
    let (her, him) = (ana.revision(), ben.revision());
    let (hers, his) = (ana.save(), ben.save());
    [meet(ana, &his, &her, &him), meet(ben, &hers, &him, &her)]
}

fn text(model: &Model, block: &str) -> Option<String> {
    model.block(&id(block)).map(Block::plain_text)
}

const P1: &str = "The quick brown fox jumps over the lazy dog.";
const P2: &str = "It was a calm day.";

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
        ("letters before a word one changes", ("p1", Before("quick", "un")), ("p1", Swap("quick", "fast")), &["p1"]),
        ("letters after a word one changes", ("p1", After("quick", "er")), ("p1", Swap("quick", "fast")), &["p1"]),
        ("text inside a word one changes", ("p1", After("qui", " ")), ("p1", Swap("quick", "fast")), &["p1"]),
        ("an ending for a word one changes", ("h", Add("wain")), ("h", Swap("Boats", "Ships")), &["h"]),
        ("words before a word one changes", ("p1", Before("quick", "very ")), ("p1", Swap("quick", "fast")), &[]),
        ("words after a word one changes", ("p1", After("quick", ", sly")), ("p1", Swap("quick", "fast")), &[]),
        ("words before a word and after it", ("p1", Before("quick", "very ")), ("p1", After("quick", ", sly")), &[]),
        ("text inside a word and after it", ("p1", After("qui", "-")), ("p1", After("quick", ", sly")), &[]),
        ("a start and a new first word", ("p2", Before("It", "So. ")), ("p2", Swap("It", "This")), &[]),
        ("a start each", ("p2", Before("It", "So. ")), ("p2", Before("It", "Well. ")), &[]),
        ("a start and an end for one word", ("h", Before("Boats", "Oh. ")), ("h", Add("!")), &[]),
        ("an end and a new last word", ("h", Add("!")), ("h", Swap("Boats", "Ships")), &[]),
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
    assert_eq!(note[0].block.as_ref(), versions[0].as_ref());

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

/// A name, what the writer who edits first and the one who edits last each do unseen by the other, and how
/// the blocks then read.
type Settled = (&'static str, Edits, Edits, &'static [&'static str]);

#[test]
fn a_settled_block_reads_as_the_writer_who_edited_last_left_it() {
    use Edit::*;
    let cases: Vec<Settled> = vec![
        (
            "the same word",
            vec![("p1", Swap("quick", "slow"))],
            vec![("p1", Swap("quick", "fast"))],
            &["Boats", "The fast brown fox jumps over the lazy dog.", P2, ""],
        ),
        (
            "words the first one cut",
            vec![("p1", Swap("quick brown ", ""))],
            vec![("p1", Swap("brown", "red"))],
            &["Boats", "The quick red fox jumps over the lazy dog.", P2, ""],
        ),
        (
            "a word the last one cut",
            vec![("p1", Swap("fox", "foxes"))],
            vec![("p1", Swap("fox ", ""))],
            &["Boats", "The quick brown jumps over the lazy dog.", P2, ""],
        ),
        ("two ends for one word", vec![("h", Add("man"))], vec![("h", Add("wain"))], &["Boatswain", P1, P2, ""]),
        (
            "a paragraph the first one removed",
            vec![("p2", Remove)],
            vec![("p2", Swap("calm", "windy"))],
            &["Boats", P1, "It was a windy day.", ""],
        ),
        (
            "a paragraph the first one removed, and the last cut short",
            vec![("p2", Remove)],
            vec![("p2", Swap(" calm", ""))],
            &["Boats", P1, "It was a day.", ""],
        ),
        (
            "a paragraph the first one removed, having written on in the one before",
            vec![("p1", Add(" The end.")), ("p2", Remove)],
            vec![("p2", Swap("calm", "windy"))],
            &["Boats", "The quick brown fox jumps over the lazy dog. The end.", "It was a windy day.", ""],
        ),
        (
            "two paragraphs the first one removed",
            vec![("p1", Remove), ("p2", Remove)],
            vec![("p1", Swap("quick", "fast")), ("p2", Swap("calm", "windy"))],
            &["Boats", "The fast brown fox jumps over the lazy dog.", "It was a windy day.", ""],
        ),
        (
            "a paragraph the last one removed",
            vec![("p2", Swap("calm", "windy"))],
            vec![("p2", Remove)],
            &["Boats", P1, ""],
        ),
        ("an empty paragraph the last one removed", vec![("e", Add("Notes"))], vec![("e", Remove)], &["Boats", P1, P2]),
        (
            "a paragraph the last one cut in two",
            vec![("p1", Swap("lazy", "sleepy"))],
            vec![("p1", Split("fox")), ("new", Swap("lazy", "idle"))],
            &["Boats", "The quick brown fox", " jumps over the idle dog.", P2, ""],
        ),
        (
            "a paragraph the first one cut in two",
            vec![("p1", Split("fox")), ("new", Swap("lazy", "idle"))],
            vec![("p1", Swap("lazy", "sleepy"))],
            &["Boats", "The quick brown fox jumps over the sleepy dog.", P2, ""],
        ),
    ];
    for (name, first, last, expected) in cases {
        let (mut ana, mut ben) = writers();
        let found = settle(&mut ana, &mut ben, first, last);
        assert!(!found[0].is_empty() && found[0].len() == found[1].len(), "{name}: each device finds the conflicts");
        assert_eq!(texts(&ana), expected, "{name}");
        assert_eq!(ana.doc(), ben.doc(), "{name}: each device settled it alone, and alike");
        assert_eq!(ana.compare(&ben.save()), Ok(Some(Ordering::Equal)), "{name}: by the very same change");
    }
}

/// A name, what each writer does to one block unseen by the other, and what each typed.
type Beside = (&'static str, (&'static str, Edit), (&'static str, Edit), [&'static str; 2]);

#[test]
fn text_beside_a_word_another_writer_changed_stays_on_the_page() {
    use Edit::*;
    let cases: Vec<Beside> = vec![
        ("before the word", ("p1", Before("quick", "very ")), ("p1", Swap("quick", "fast")), ["very ", "fast"]),
        ("after the word", ("p1", Swap("quick", "fast")), ("p1", After("quick", ", sly")), ["fast", ", sly"]),
        ("either side of one word", ("h", Before("Boats", "Oh. ")), ("h", Add("!")), ["Oh. ", "!"]),
        ("at the start of a block", ("p2", Before("It", "So. ")), ("p2", Swap("It", "This")), ["So. ", "This"]),
        ("at the end of a block", ("h", Swap("Boats", "Ships")), ("h", Add("!")), ["Ships", "!"]),
        ("at one place", ("p1", Add(" It ran.")), ("p1", Add(" It hid.")), [" It ran.", " It hid."]),
    ];
    for (name, hers, his, typed) in cases {
        let block = hers.0;
        let (mut ana, mut ben) = writers();
        let found = settle(&mut ana, &mut ben, vec![hers], vec![his]);
        assert_eq!(found, [vec![], vec![]], "{name}: neither changed the other's words");
        let text = text(&ana, block).unwrap_or_default();
        assert!(typed.iter().all(|typed| text.contains(typed)), "{name}: {text:?} lacks what one of them typed");
        assert_eq!(ana.doc(), ben.doc(), "{name}");
    }
}

#[test]
fn a_device_that_went_on_writing_elsewhere_settles_to_the_same_text() {
    let (mut ana, mut ben) = writers();
    write(&mut ana, vec![("p1", Edit::Swap("quick brown ", ""))]);
    write(&mut ben, vec![("p1", Edit::Swap("brown", "red"))]);
    let (her, him) = (ana.revision(), ben.revision());
    let (hers, his) = (ana.save(), ben.save());
    // Ana writes on in another paragraph before she sees what Ben did. Ben sees only what she did before.
    write(&mut ana, vec![("p2", Edit::Add(" We sailed."))]);
    let later = ana.revision();
    assert_eq!(meet(&mut ana, &his, &later, &him).len(), 1);
    assert_eq!(meet(&mut ben, &hers, &him, &her).len(), 1);
    let settled = "The quick red fox jumps over the lazy dog.";
    assert_eq!((text(&ana, "p1").as_deref(), text(&ben, "p1").as_deref()), (Some(settled), Some(settled)));

    // They merged different revisions, so each put the cut words back by a change of its own. Settled
    // again when they meet, the words stand once, and there is nothing left to choose.
    let (her, him) = (ana.revision(), ben.revision());
    let (hers, his) = (ana.save(), ben.save());
    let found = [meet(&mut ana, &his, &her, &him), meet(&mut ben, &hers, &him, &her)];
    assert_eq!(found, [vec![], vec![]]);
    assert_eq!(texts(&ana), ["Boats", settled, "It was a calm day. We sailed.", ""]);
    assert_eq!(ana.doc(), ben.doc());
    assert_eq!(ana.compare(&ben.save()), Ok(Some(Ordering::Equal)));
}

#[test]
fn a_block_two_devices_settled_apart_is_settled_again() {
    let (mut ana, mut ben) = writers();
    write(&mut ana, vec![("p1", Edit::Swap("quick brown ", ""))]);
    write(&mut ben, vec![("p1", Edit::Swap("brown", "red"))]);
    let (her, him) = (ana.revision(), ben.revision());
    let (hers, his) = (ana.save(), ben.save());
    // A third device meets both and settles the paragraph. Ben writes on in it, and then meets Ana.
    let mut eve = ana.fork(ReplicaId::from(3));
    assert_eq!(meet(&mut eve, &his, &her, &him).len(), 1);
    ben.set_time(MORNING + 2 * HOUR);
    write(&mut ben, vec![("p1", Edit::Swap("lazy", "idle"))]);
    let later = ben.revision();
    assert_eq!(meet(&mut ben, &hers, &later, &her).len(), 1);
    assert_eq!(text(&ben, "p1").as_deref(), Some("The quick red fox jumps over the idle dog."));

    // Each put the cut words back by a change of its own. The later of the two settlings stands, and
    // there is no conflict to tell of beyond the one each of them found.
    let (theirs, his) = (eve.revision(), ben.revision());
    let (held, kept) = (eve.save(), ben.save());
    let found = [meet(&mut eve, &kept, &theirs, &his), meet(&mut ben, &held, &his, &theirs)];
    assert_eq!(found, [vec![], vec![]]);
    assert_eq!(texts(&eve), ["Boats", "The quick red fox jumps over the idle dog.", P2, ""]);
    assert_eq!(eve.doc(), ben.doc());
    assert_eq!(eve.compare(&ben.save()), Ok(Some(Ordering::Equal)));
}

#[test]
fn settling_twice_changes_nothing_and_no_undo_step_is_lost_to_it() {
    let (mut ana, mut ben) = writers();
    write(&mut ana, vec![("p2", Edit::Add(" We sailed.")), ("p1", Edit::Swap("quick", "slow"))]);
    write(&mut ben, vec![("p1", Edit::Swap("quick", "fast"))]);
    let (her, him) = (ana.revision(), ben.revision());
    ana.merge(&ben.save()).unwrap();

    let (change, found) = ana.settle(&her, &him).unwrap();
    assert!(change.blocks.contains(&id("p1")), "the change names the block that now reads otherwise");
    let settled = (ana.doc().clone(), ana.revision());
    let (change, again) = ana.settle(&her, &him).unwrap();
    assert_eq!((again, change.blocks.len()), (found, 0), "the conflict is found again, and nothing done");
    assert_eq!((ana.doc(), ana.revision(), ana.is_latest()), (&settled.0, settled.1, true));

    // Ana can still take back her own two edits, the earlier of which the settling did not touch.
    assert!(ana.undo().is_some() && ana.undo().is_some());
    assert_eq!(text(&ana, "p2").as_deref(), Some(P2));
}

#[test]
fn a_model_that_shows_an_older_revision_settles_without_leaving_it() {
    let (mut ana, mut ben) = writers();
    let first = (ana.doc().clone(), ana.revision());
    let stood = part(&mut ana, &mut ben, ("p1", Edit::Swap("quick", "slow")), ("p1", Edit::Swap("quick", "fast")));
    ana.checkout(&first.1).unwrap();
    let (change, found) = ana.settle(&stood.0, &stood.1).unwrap();
    assert_eq!((blocks(&found), change.blocks.len()), (vec!["p1"], 0));
    assert_eq!((ana.doc(), ana.revision(), ana.is_latest()), (&first.0, first.1, false));
    ana.checkout_latest();
    assert_eq!(text(&ana, "p1").as_deref(), Some("The fast brown fox jumps over the lazy dog."));
}
