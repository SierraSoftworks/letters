//! When a window marks a version in its document's history.

use std::time::{Duration, Instant};

use letters::versions::{PAUSE, Rests};
use letters_model::{Block, BlockKind, Document, Model, ReplicaId, Revision, TagKind};

/// Revisions of one document, each a word further on.
fn revisions(count: usize) -> Vec<Revision> {
    let document = |words: usize| Document {
        blocks: vec![Block::text("p0", BlockKind::Paragraph, "word ".repeat(words))],
        ..Document::default()
    };
    let mut model = Model::new(document(0), ReplicaId::new("writer"));
    let step = |words| {
        model.rewrite(document(words)).unwrap();
        model.revision()
    };
    (1..=count).map(step).collect()
}

#[test]
fn where_writing_stopped_is_marked_when_it_goes_on() {
    let (at, start) = (revisions(4), Instant::now());
    let minutes = |minutes: u64| start + Duration::from_secs(60 * minutes);
    let mut rests = Rests::default();
    rests.opened(at[0].clone());
    // When each edit is made, the revision it leads to, and what is marked before it.
    let edits = [
        (minutes(0), &at[1], Some((&at[0], TagKind::Session))),
        (minutes(4), &at[2], None),
        (minutes(4) + PAUSE, &at[3], Some((&at[2], TagKind::Pause))),
        (minutes(5) + PAUSE, &at[3], None),
    ];
    for (now, after, marked) in edits {
        let mark = rests.edited(now);
        rests.rest(after.clone(), now);

        assert_eq!(mark.as_ref().map(|(at, kind)| (at, *kind)), marked, "{:?} in", now - start);
    }
}

#[test]
fn a_new_document_has_no_session_before_its_first() {
    let (at, now) = (revisions(1), Instant::now());
    let mut made = Rests::default();

    assert_eq!(made.edited(now), None);
    made.rest(at[0].clone(), now);
    assert_eq!(made.edited(now + Duration::from_secs(1)), None);
}

#[test]
fn merges_that_come_close_together_are_marked_once() {
    let start = Instant::now();
    let mut rests = Rests::default();
    let cases = [(Duration::ZERO, true), (Duration::from_secs(5), false), (PAUSE / 2, false), (PAUSE, true)];
    for (after, marked) in cases {
        assert_eq!(rests.merged(start + after), marked, "{after:?} in");
    }
}
