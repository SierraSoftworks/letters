//! The steps a history was written in, the versions marked on it, and going back to one.

mod common;

use common::{id, pair, split, texts, typed};
use letters_model::{
    Change, Document, HistoryError, Model, Op, Point, ReplicaId, Revision, Style, Tag, TagKind, TextStyle,
};

const MORNING: u64 = 1_800_000_000;

/// Two writers of one file who gave their names, an hour apart by their clocks.
fn writers() -> (Model, Model) {
    let (mut ana, mut ben) = pair();
    for (writer, name, time) in [(&mut ana, "Ana", MORNING), (&mut ben, "Ben", MORNING + 3600)] {
        writer.set_author(name);
        writer.set_time(time);
    }
    (ana, ben)
}

fn italic_asides() -> Op {
    Op::SetStyle { element: "aside".into(), style: TextStyle { italic: true, ..TextStyle::default() } }
}

fn shown(model: &Model) -> (Document, Style) {
    (model.doc().clone(), model.style().clone())
}

/// The steps someone put a name to, each with its writer and when it was begun.
fn signed(steps: &[Point]) -> Vec<(&str, u64)> {
    let named = steps.iter().filter(|step| !step.authors.is_empty());
    named.map(|step| (step.authors[0].0.as_str(), step.time)).collect()
}

/// A revision no history holds.
fn unknown() -> Revision {
    "7@99".parse().expect("a revision as it is written")
}

#[test]
fn the_timeline_tells_each_step_with_its_writer_and_time() {
    let (mut ana, mut ben) = writers();
    ana.apply([typed("p", 11, "!")]).unwrap();
    ben.merge(&ana.save()).unwrap();
    ben.apply([typed("p", 0, "Oh. ")]).unwrap();
    ana.merge(&ben.save()).unwrap();

    let steps = ana.timeline();
    assert_eq!(signed(&steps), [("Ana", MORNING), ("Ben", MORNING + 3600)]);
    assert!(steps.iter().all(|step| step.edits > 0 && step.size > 0), "every step changed something");
    // The steps follow on from each other, up to the newest revision.
    assert!(steps.windows(2).all(|pair| pair[0].after == pair[1].before));
    assert_eq!(steps.last().map(|step| &step.after), Some(&ana.revision()));

    // Either side of a step is a document to read, and the step says which blocks to look at.
    let last = steps.last().unwrap();
    let before = ana.document_at(&last.before).unwrap().0;
    assert_eq!(before.block(&id("p")).map(|block| block.plain_text()).as_deref(), Some("Hello world!"));
    assert_eq!(ana.document_at(&last.after).unwrap(), shown(&ana));
    assert_eq!(ana.changed(&last.before, &last.after).unwrap().blocks, [id("p")].into());
}

#[test]
fn every_device_lists_one_history_alike() {
    let (mut ana, mut ben) = writers();
    // Written apart, the two steps have no order of their own.
    ana.apply([typed("p", 11, "!")]).unwrap();
    ben.apply([typed("p", 0, "Oh. ")]).unwrap();
    ana.merge(&ben.save()).unwrap();
    ben.merge(&ana.save()).unwrap();
    let mut reader = Model::load(&ben.save(), ReplicaId::from(3)).unwrap();

    let listed = ana.timeline();
    assert_eq!(signed(&listed).len(), 2);
    assert_eq!(ben.timeline(), listed);
    assert_eq!(reader.timeline(), listed);
    // Whatever the order, each step starts from a revision the history can show.
    for step in &listed {
        assert!(ana.document_at(&step.before).is_ok() && ana.document_at(&step.after).is_ok());
    }
}

/// A name, a revision, and the steps it lacks: the writer of each and when it was begun.
type Lacking<'a> = (&'static str, &'a Revision, &'a [(&'static str, u64)]);

#[test]
fn since_tells_the_steps_a_revision_lacks_wherever_they_are_listed() {
    let (mut ana, mut ben) = writers();
    let start = ana.revision();
    // Ben writes a step offline while Ana writes two, and his comes in after she last looked.
    ben.apply([typed("p", 0, "Oh. ")]).unwrap();
    ana.apply([typed("p", 11, "!")]).unwrap();
    let first = ana.revision();
    ana.set_time(MORNING + 7200);
    ana.apply([typed("p", 12, " Again.")]).unwrap();
    let (seen, his) = (ana.revision(), ben.revision());
    ana.merge(&ben.save()).unwrap();

    let steps = ana.timeline();
    let (hers, later, bens) = (("Ana", MORNING), ("Ana", MORNING + 7200), ("Ben", MORNING + 3600));
    let cases: [Lacking; 5] = [
        ("the newest revision", &ana.revision(), &[]),
        ("her own last edit, which his step is listed before", &seen, &[bens]),
        ("her first edit", &first, &[bens, later]),
        ("his edit", &his, &[hers, later]),
        ("the start", &start, &[hers, bens, later]),
    ];
    for (name, revision, expected) in cases {
        let since = ana.since(revision).unwrap();
        assert_eq!(signed(&since), expected, "{name}");
        assert!(since.iter().all(|step| steps.contains(step)), "{name}: the steps are the timeline's own");
    }
    assert_eq!(ana.since(&unknown()), Err(HistoryError::UnknownRevision));
}

#[test]
fn a_revision_inside_a_step_is_read_and_counts_the_step_as_since() {
    let (mut ana, _) = writers();
    ana.apply([typed("p", 11, "!")]).unwrap();
    let (inside, steps) = (ana.revision(), ana.timeline().len());
    // Within the minute, she writes on in the same step.
    ana.apply([typed("p", 12, " Again.")]).unwrap();
    assert_eq!(ana.timeline().len(), steps, "one step holds both edits");

    assert_eq!(signed(&ana.since(&inside).unwrap()), [("Ana", MORNING)]);
    let read = ana.document_at(&inside).unwrap().0;
    assert_eq!(read.block(&id("p")).map(|block| block.plain_text()).as_deref(), Some("Hello world!"));
}

#[test]
fn a_version_is_read_without_leaving_the_one_shown() {
    let mut model = Model::new(common::doc(), ReplicaId::from(1));
    let (start, first) = (model.revision(), shown(&model));
    model.apply([typed("p", 11, "!"), italic_asides()]).unwrap();
    let (newest, last) = (model.revision(), shown(&model));

    assert_eq!(model.document_at(&start).unwrap(), first);
    assert_eq!((shown(&model), model.revision(), model.is_latest()), (last.clone(), newest.clone(), true));
    assert!(model.undo().is_some(), "reading a version keeps the undo steps");
    assert!(model.redo().is_some());

    // In history mode too, where an older revision is shown.
    model.checkout(&start).unwrap();
    assert_eq!(model.document_at(&newest).unwrap(), last);
    assert_eq!((shown(&model), model.revision(), model.is_latest()), (first, start, false));
}

#[test]
fn versions_read_as_they_were_written_in_whatever_order() {
    let (mut ana, mut ben) = writers();
    ben.apply([typed("p", 0, "Oh. ")]).unwrap();
    let hours: Vec<Vec<Op>> = vec![
        vec![typed("p", 11, "!")],
        vec![split("p", 5, "p2"), typed("p2", 0, "wide")],
        vec![Op::RemoveBlock { block: id("d") }, Op::DeleteText { block: id("p"), range: 0..2 }],
        vec![italic_asides(), Op::SetLanguage { language: "fr".into() }],
    ];
    let mut written = vec![(ana.revision(), shown(&ana))];
    for (hour, ops) in (1..).zip(hours) {
        // Another writer's work comes in between, and so does a mark on a version.
        match hour {
            2 => drop(ana.merge(&ben.save()).unwrap()),
            3 => ana.tag(&written[1].0, TagKind::Named, Some("First draft")).unwrap(),
            _ => {}
        }
        ana.set_time(MORNING + hour * 3600);
        ana.apply(ops).unwrap();
        written.push((ana.revision(), shown(&ana)));
    }

    // Stepping back, stepping forth, and going from one end to the other.
    let last = written.len() - 1;
    let orders = [(0..=last).rev().collect(), (0..=last).collect(), vec![0, last, 1, last - 1, 0]];
    for nth in orders.concat() {
        let (revision, then) = &written[nth];
        assert_eq!(ana.document_at(revision).as_ref(), Ok(then), "version {nth}");
    }
    // The history is written on, and what was read before reads the same.
    ana.set_time(MORNING + 24 * 3600);
    ana.apply([typed("p", 0, "So. ")]).unwrap();
    written.push((ana.revision(), shown(&ana)));
    for (nth, (revision, then)) in written.iter().enumerate().rev() {
        assert_eq!(ana.document_at(revision).as_ref(), Ok(then), "version {nth}, after more was written");
    }
    assert_eq!(ana.document_at(&unknown()), Err(HistoryError::UnknownRevision));
}

/// A name, an edit, and what differs either side of it: the blocks, and whether blocks came or went, the
/// style changed, or the document's own fields did.
type Differs = (&'static str, Vec<Op>, Vec<&'static str>, [bool; 3]);

#[test]
fn changed_tells_what_differs_between_two_revisions() {
    let cases: Vec<Differs> = vec![
        ("typing", vec![typed("p", 0, "Oh. ")], vec!["p"], [false, false, false]),
        ("a split", vec![split("p", 5, "p2")], vec!["p", "p2"], [true, false, false]),
        ("a block removed", vec![Op::RemoveBlock { block: id("d") }], vec!["d"], [true, false, false]),
        ("a style", vec![italic_asides()], vec![], [false, true, false]),
        ("a language", vec![Op::SetLanguage { language: "fr".into() }], vec![], [false, false, true]),
    ];
    for (name, ops, blocks, [structure, style, document]) in cases {
        let mut model = Model::new(common::doc(), ReplicaId::from(1));
        let before = model.revision();
        model.apply(ops).unwrap();
        let after = model.revision();
        let expected = Change { blocks: blocks.into_iter().map(id).collect(), structure, style, document };
        assert_eq!(model.changed(&before, &after), Ok(expected), "{name}");
        assert_eq!(model.changed(&after, &after), Ok(Change::default()), "{name}: a revision and itself");
    }
}

#[test]
fn a_step_counts_the_characters_it_put_in_and_took_out() {
    let cases: Vec<(&str, Vec<Op>, (usize, usize))> = vec![
        ("typing", vec![typed("p", 11, "!")], (1, 0)),
        ("a letter of two bytes", vec![typed("p", 0, "\u{e9}")], (1, 0)),
        ("a cut", vec![Op::DeleteText { block: id("p"), range: 0..6 }], (0, 6)),
        ("a word for another", vec![Op::DeleteText { block: id("p"), range: 6..11 }, typed("p", 6, "moon")], (4, 5)),
        ("a split", vec![split("p", 5, "p2")], (0, 0)),
        ("a block removed", vec![Op::RemoveBlock { block: id("p") }], (0, 11)),
        ("a style", vec![italic_asides()], (0, 0)),
    ];
    for (name, ops, expected) in cases {
        let mut model = Model::new(common::doc(), ReplicaId::from(1));
        model.apply(ops).unwrap();
        let steps = model.timeline();
        let last = steps.last().unwrap();
        assert_eq!((last.added, last.removed), expected, "{name}");
    }
}

#[test]
fn a_marked_version_travels_with_the_history() {
    let (mut ana, mut ben) = writers();
    ana.apply([typed("p", 11, "!")]).unwrap();
    let (draft, steps) = (ana.revision(), ana.timeline().len());
    ana.tag(&draft, TagKind::Named, Some("First draft")).unwrap();
    ana.apply([typed("p", 12, "?")]).unwrap();
    ben.merge(&ana.save()).unwrap();

    for writer in [&mut ana, &mut ben] {
        let tags = writer.tags();
        let told: Vec<_> =
            tags.iter().map(|tag| (&tag.at, tag.kind, tag.name.as_deref(), tag.author.as_deref())).collect();
        assert_eq!(told, [(&draft, TagKind::Named, Some("First draft"), Some("Ana"))]);
        assert_eq!(tags[0].time, MORNING);
        // The mark is no step of its own. It stands at the step that led to its revision.
        let timeline = writer.timeline();
        assert_eq!(timeline.len(), steps + 1);
        let marked: Vec<&Point> = timeline.iter().filter(|step| !step.tags.is_empty()).collect();
        assert_eq!(marked.iter().map(|step| (&step.after, &step.tags)).collect::<Vec<_>>(), [(&draft, &tags)]);
        assert_eq!(texts(writer), ["Hello world!?", ""], "a mark changes nothing of the document");
    }

    // A second mark on the revision takes the place of the first, and a mark taken off is gone everywhere.
    ben.tag(&draft, TagKind::Named, Some("Draft one")).unwrap();
    ana.merge(&ben.save()).unwrap();
    assert_eq!(ana.tags().iter().map(|tag| tag.name.as_deref()).collect::<Vec<_>>(), [Some("Draft one")]);
    ana.untag(&draft);
    ana.untag(&draft);
    ben.merge(&ana.save()).unwrap();
    assert_eq!(ben.tags(), Vec::<Tag>::new());
    assert_eq!(ben.timeline().len(), steps + 1);
    assert!(ana.undo().is_some(), "marking leaves the undo steps");
}

#[test]
fn a_version_is_marked_while_another_is_shown() {
    let mut model = Model::new(common::doc(), ReplicaId::from(1));
    let start = model.revision();
    model.apply([typed("p", 11, "!")]).unwrap();
    model.checkout(&start).unwrap();
    let before = shown(&model);

    model.tag(&start, TagKind::Session, None).unwrap();
    assert_eq!((shown(&model), model.revision(), model.is_latest()), (before, start.clone(), false));
    let kinds = |model: &mut Model| model.tags().iter().map(|tag| (tag.at.clone(), tag.kind)).collect::<Vec<_>>();
    assert_eq!(kinds(&mut model), [(start.clone(), TagKind::Session)]);
    model.checkout_latest();
    assert_eq!(kinds(&mut model), [(start, TagKind::Session)]);
    assert_eq!(texts(&model), ["Hello world!", ""]);
}

#[test]
fn restoring_a_version_is_one_edit_that_can_be_undone() {
    let (mut ana, mut ben) = writers();
    ana.apply([typed("p", 11, "!")]).unwrap();
    let (then, old) = (ana.revision(), shown(&ana));
    ana.apply([Op::RemoveBlock { block: id("d") }, typed("p", 0, "Oh. ")]).unwrap();
    ana.apply([italic_asides(), Op::SetLanguage { language: "fr".into() }]).unwrap();
    ben.merge(&ana.save()).unwrap();
    ben.apply([split("p", 3, "p2")]).unwrap();
    ana.merge(&ben.save()).unwrap();
    let (newest, new) = (ana.revision(), shown(&ana));
    assert_ne!(new, old);

    // From history mode, where the version is on screen.
    ana.checkout(&then).unwrap();
    ana.restore(&then).unwrap();
    assert!(ana.is_latest());
    assert_eq!(shown(&ana), old);
    // Nothing of the history is gone, and the restoring is marked with the version it brought back.
    assert_eq!(ana.document_at(&newest).unwrap(), new);
    let tags = ana.tags();
    let told: Vec<_> = tags.iter().map(|tag| (tag.kind, tag.restored.as_ref(), tag.author.as_deref())).collect();
    assert_eq!(told, [(TagKind::Restore, Some(&then), Some("Ana"))]);
    assert_eq!(ana.document_at(&tags[0].at).unwrap(), old);

    // It reaches another device like any edit, and its writer can take it back.
    ben.merge(&ana.save()).unwrap();
    assert_eq!(shown(&ben), old);
    assert!(ana.undo().is_some());
    assert_eq!(shown(&ana), new);
    assert!(ana.undo().is_none(), "a restore is one undo step");
    assert!(ana.redo().is_some());
    assert_eq!(shown(&ana), old);
}

#[test]
fn a_revision_the_history_lacks_is_refused() {
    let (mut model, _) = writers();
    model.apply([typed("p", 11, "!")]).unwrap();
    let (before, stood) = (shown(&model), model.revision());

    let lacks = Err(HistoryError::UnknownRevision);
    assert_eq!(model.document_at(&unknown()), lacks.clone().map(|()| before.clone()));
    assert_eq!(model.changed(&unknown(), &stood), lacks.clone().map(|()| Change::default()));
    assert_eq!(model.restore(&unknown()), lacks.clone().map(|()| Change::default()));
    assert_eq!(model.tag(&unknown(), TagKind::Named, Some("Nowhere")), lacks);
    model.untag(&unknown());
    assert_eq!((shown(&model), model.revision(), model.tags()), (before, stood.clone(), Vec::new()));

    // The version that is already there is restored by doing nothing.
    assert_eq!(model.restore(&stood), Ok(Change::default()));
    assert_eq!((model.revision(), model.tags()), (stood, Vec::new()));
    assert!(model.undo().is_some() && model.undo().is_none(), "nothing was added to take back");
}
