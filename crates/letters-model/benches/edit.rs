//! How long the model takes to edit a 500-page document, against the budgets in the design.
//!
//! Run with `cargo bench -p letters-model`. It exits with an error when a budget is broken. A key press
//! has 8 ms to reach the screen on the reference machine, an M1 Pro, and the model should use little of it.

use std::hint::black_box;
use std::time::{Duration, Instant};

use letters_model::{Block, BlockId, BlockKind, Body, Document, Hold, Mark, Model, Op, Pos, ReplicaId, Run};

/// An edit inside one block: typing, deleting, formatting.
const EDIT: Duration = Duration::from_micros(300);
/// An edit that adds or removes a block, and any undo step.
const STRUCTURE: Duration = Duration::from_millis(4);

/// Showing a document, and taking over its history once that is ready. Both hold up the window.
const OPEN: Duration = Duration::from_millis(10);

/// 500 pages of dense prose hold about this many paragraphs.
const BLOCKS: usize = 10_000;

fn document() -> Document {
    let plain = "The quick brown fox jumps over the lazy dog and keeps on running. ";
    let block = |i: usize| {
        let kind = if i.is_multiple_of(40) { BlockKind::Heading { level: 2 } } else { BlockKind::Paragraph };
        let runs = vec![Run::plain(plain), Run::marked("Bold words here. ", [Mark::Bold]), Run::plain(plain)];
        Block { body: Body::Text(runs), ..Block::new(id(i), kind) }
    };
    Document { blocks: (0..BLOCKS).map(block).collect(), ..Document::default() }
}

fn id(i: usize) -> BlockId {
    BlockId::new(format!("b{i}"))
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let out = f();
    (out, start.elapsed())
}

/// Prints how long `runs` calls took. Returns whether nearly all of them kept to `budget`.
fn report(name: &str, runs: usize, budget: Option<Duration>, mut f: impl FnMut(usize)) -> bool {
    let mut times: Vec<Duration> = (0..runs).map(|run| timed(|| f(run)).1).collect();
    times.sort();
    let us = |time: Duration| time.as_secs_f64() * 1e6;
    let (mean, most) = (times.iter().sum::<Duration>() / runs as u32, times[runs * 99 / 100]);
    let kept = budget.is_none_or(|budget| most <= budget);
    let verdict = match budget {
        Some(budget) if kept => format!("within {:.0} µs", us(budget)),
        Some(budget) => format!("OVER {:.0} µs", us(budget)),
        None => String::new(),
    };
    println!("{name:<28} mean {:>9.1} µs   p99 {:>9.1} µs   {verdict}", us(mean), us(most));
    kept
}

fn main() {
    let doc = document();
    let bytes: usize = doc.walk().map(|block| block.plain_text().len()).sum();
    println!("{BLOCKS} blocks, {:.1} MB of text", bytes as f64 / 1e6);

    // What the window waits for: the document on screen, and later the history handed over.
    let (shown, copy) = (doc.clone(), doc.clone());
    let (mut model, new) = timed(|| Model::new(shown, ReplicaId::from(1)));
    let (prepared, start) = timed(|| Model::new(copy, ReplicaId::from(1)).prepare(None).expect("a new history"));
    let (_, attach) = timed(|| model.attach(prepared));
    println!("new {new:.1?}, attach {attach:.1?}, within {OPEN:.0?} each");
    let mut kept = new <= OPEN && attach <= OPEN;

    // What runs away from the window.
    let (history, save) = timed(|| model.save());
    let (_, read) =
        timed(|| Model::new(doc, ReplicaId::from(2)).prepare(Some(&history)).expect("a saved history loads"));
    let (loaded, load) = timed(|| Model::load(&history, ReplicaId::from(2)).expect("a saved history loads"));
    let size = history.len() as f64 / 1e6;
    println!("prepare {start:.0?}, or {read:.0?} with a history; save {save:.0?} ({size:.1} MB), load {load:.0?}");
    let mut other = loaded.fork(ReplicaId::from(3));
    let first = model.revision();

    let places = [
        ("type in the first block", 0),
        ("type in a middle block", BLOCKS / 2),
        ("type in the last block", BLOCKS - 1),
    ];
    for (name, block) in places {
        let op = |run: usize| Op::InsertText { at: Pos { block: id(block), offset: 10 + run }, text: "x".into() };
        kept &= report(name, 2_000, Some(EDIT), |run| drop(model.apply([op(run)]).expect("typed")));
    }
    let block = id(BLOCKS / 2);
    let delete = |run: usize| Op::DeleteText { block: block.clone(), range: 10 + run..11 + run };
    kept &= report("delete a character", 500, Some(EDIT), |run| drop(model.apply([delete(run)]).expect("deleted")));
    let mark =
        |run: usize| Op::SetMark { block: block.clone(), range: 3 * run..3 * run + 2, mark: Mark::Italic, on: true };
    kept &= report("mark two characters", 100, Some(EDIT), |run| drop(model.apply([mark(run)]).expect("marked")));
    kept &= report("undo an edit", 100, Some(EDIT), |_| drop(model.undo().expect("a step to undo")));
    kept &= report("redo an edit", 100, Some(EDIT), |_| drop(model.redo().expect("a step to redo")));

    let new_id = |run: usize| BlockId::new(format!("split{run}"));
    let split = |run: usize| Op::Split { at: Pos { block: block.clone(), offset: 20 }, new_id: new_id(run) };
    kept &= report("split a paragraph", 100, Some(STRUCTURE), |run| drop(model.apply([split(run)]).expect("split")));
    kept &= report("join two paragraphs", 100, Some(STRUCTURE), |run| {
        drop(model.apply([Op::Join { block: new_id(99 - run) }]).expect("joined"))
    });
    kept &= report("undo a join", 100, Some(STRUCTURE), |_| drop(model.undo().expect("a step to undo")));
    // The first search after blocks came or went reads where every block is. Later ones do not.
    let found = |block: usize| black_box(model.place(&id(block))).is_some();
    report("find a block, the first time", 1, None, |_| assert!(found(BLOCKS - 1)));
    kept &= report("find a block by its id", 2_000, Some(EDIT), |run| assert!(found(run * 5)));

    // What another replica typed arrives while this one has moved on.
    report("merge a remote key press", 100, None, |run| {
        other.apply([Op::InsertText { at: Pos { block: id(7), offset: run }, text: "y".into() }]).expect("typed");
        model.merge(&other.updates_since(&loaded.revision())).expect("a replica reads another's edits");
    });
    let heading = |run: usize| Op::SetKind { block: id(9), kind: BlockKind::Heading { level: 1 + (run % 3) as u8 } };
    report("merge a remote kind change", 100, None, |run| {
        other.apply([heading(run)]).expect("a new kind");
        model.merge(&other.updates_since(&loaded.revision())).expect("a replica reads another's edits");
    });
    // A character in a block the edits above left alone.
    let text = model.doc().block(&id(BLOCKS / 4)).expect("the block").plain_text();
    let (offset, held) = text.char_indices().nth(30).expect("a long block");
    let here = Pos { block: id(BLOCKS / 4), offset };
    let kept_text = model.stable(&here, Hold::Next).expect("a position in text");
    report("resolve a stable position", 100, None, |_| drop(model.resolve(&kept_text).expect("its text is there")));
    let range = offset..offset + held.len_utf8();
    model.apply([Op::DeleteText { block: here.block.clone(), range }]).expect("deleted");
    report("resolve one whose text is gone", 20, None, |_| drop(model.resolve(&kept_text)));
    report("show the first revision", 5, None, |_| {
        model.checkout(&first).expect("the history holds its first revision");
        model.checkout_latest();
    });

    if !kept {
        eprintln!("a budget was broken");
        std::process::exit(1);
    }
}
