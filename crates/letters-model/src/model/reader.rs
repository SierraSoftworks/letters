//! Old revisions, read by walking the history forward once.
//!
//! Loro takes a step forward through a history in what the step changed, and rebuilds a text from its
//! first edit for any step back. So a reader holds a copy of the history, walks it once from before its
//! first step to its newest, and keeps for each step the change that takes the document back over it.
//! From then on it moves its document either way by those changes alone, and a revision near the last
//! one read costs what differs between the two.

use std::collections::HashMap;
use std::mem;

use loro::{ExportMode, Frontiers, LoroDoc};

use super::{History, Model};
use crate::undo::Delta;
use crate::{Document, Revision, Style, crdt, versions};

/// How many edits a reader that is not asked anything outlives. A writer who has gone back to writing
/// does not keep a second copy of the history for good.
const IDLE: usize = 64;

pub(super) struct Reader {
    /// A copy of the history, which stays at the last revision walked. Its document shows revision `at`.
    model: Model,
    /// The revisions walked, oldest first: the history before anything was written, then the one each
    /// step of the timeline leads to.
    ends: Vec<Frontiers>,
    /// How to cross from each of them to the next: back for those before `at`, forth from there on.
    trail: Vec<Frame>,
    at: usize,
    /// Where in `ends` the document either side of each step of the timeline is found.
    places: HashMap<String, usize>,
    /// The newest revision of the history when it was last walked.
    made: Frontiers,
    /// Whether a change did not fit, and the document is no longer to be trusted.
    lost: bool,
    /// The edits made since it was last asked.
    idle: usize,
}

struct Frame {
    /// The changes to the sequence that cross it, in order.
    cross: Vec<Delta>,
    /// The document's own fields and the style on the far side, where the step changed them.
    set: Option<(Document, Style)>,
}

impl Reader {
    /// A reader of `main`'s history, walked to its newest step.
    fn open(main: &Model) -> Self {
        let loro = LoroDoc::new();
        crdt::configure(&loro);
        let history = History::open(loro, main.replica);
        // What it takes in is then shown step by step, and never all at once.
        history.loro.detach();
        let (model, ends) = (Model::of(history, main.replica), vec![Frontiers::default()]);
        let (trail, places, made) = (Vec::new(), HashMap::new(), main.loro().oplog_frontiers());
        let mut reader = Self { model, ends, trail, at: 0, places, made, lost: false, idle: 0 };
        reader.lost = !reader.walk(main);
        reader
    }

    /// Walks the steps `main`'s history has gained. False when one did not fit the document.
    fn walk(&mut self, main: &Model) -> bool {
        let loro = self.model.loro();
        let Ok(news) = main.loro().export(ExportMode::updates(&loro.oplog_vv())) else { return false };
        if self.lost || loro.import(&news).is_err() {
            return false;
        }
        let steps = versions::revisions(loro);
        // A step that was walked may lead elsewhere now: a writer's last step grows as they write on, and
        // a merge lists another writer's steps among those walked. The walk goes on from the last that holds.
        let same = self.ends[1..].iter().zip(&steps).take_while(|(end, step)| **end == step.1).count();
        if !self.seek(same) {
            return false;
        }
        if same + 1 < self.ends.len() {
            if self.model.loro().checkout(&self.ends[same]).is_err() {
                return false;
            }
            // The document went there by the trail.
            self.model.queue().clear();
            self.ends.truncate(same + 1);
            self.trail.truncate(same);
        }
        let mut held = (self.model.fields(), self.model.style.clone());
        for (_, end) in &steps[same..] {
            self.model.kept = Some(Vec::new());
            if self.model.loro().checkout(end).is_err() {
                return false;
            }
            let change = self.model.drain();
            let Some(mut cross) = self.model.kept.take() else { return false };
            cross.reverse();
            let set = change.document || change.style || self.model.doc.extra != held.0.extra;
            let set = set.then(|| mem::replace(&mut held, (self.model.fields(), self.model.style.clone())));
            self.trail.push(Frame { cross, set });
            self.at = self.ends.len();
            self.ends.push(end.clone());
        }
        self.places.clear();
        for (step, (before, after)) in steps.iter().enumerate() {
            // What stands between two steps marks versions, and leaves the document as it was.
            self.places.insert(Revision(before.clone()).to_string(), step);
            self.places.insert(Revision(after.clone()).to_string(), step + 1);
        }
        self.made = main.loro().oplog_frontiers();
        true
    }

    /// Moves the document to revision `to` of those walked. False when a change did not fit.
    fn seek(&mut self, to: usize) -> bool {
        while self.at != to && !self.lost {
            let back = self.at > to;
            let frame = &mut self.trail[self.at - usize::from(back)];
            // Each change gives the one that takes it back, which is the way across next time.
            let crossed: Option<Vec<Delta>> = frame.cross.iter().map(|delta| self.model.shift(delta, None).1).collect();
            let Some(mut crossed) = crossed else {
                self.lost = true;
                break;
            };
            crossed.reverse();
            frame.cross = crossed;
            if let Some((fields, style)) = &mut frame.set {
                let doc = &mut self.model.doc;
                mem::swap(&mut doc.language, &mut fields.language);
                mem::swap(&mut doc.dictionary, &mut fields.dictionary);
                mem::swap(&mut doc.properties, &mut fields.properties);
                mem::swap(&mut doc.previews, &mut fields.previews);
                mem::swap(&mut doc.extra, &mut fields.extra);
                mem::swap(&mut self.model.style, style);
            }
            self.at = if back { self.at - 1 } else { self.at + 1 };
        }
        !self.lost
    }

    /// The document and style at `revision`. `None` when it is not either side of a step of the
    /// timeline, or the reader lost its place.
    fn read(&mut self, revision: &Revision) -> Option<(Document, Style)> {
        let place = *self.places.get(&revision.to_string()).filter(|_| !self.lost)?;
        self.seek(place).then(|| (self.model.doc.clone(), self.model.style.clone()))
    }
}

impl Model {
    /// Reads `revision` through the reader, which is made or brought up to date first. `None` when the
    /// reader cannot tell, and the history itself has to be asked.
    pub(super) fn reads(&mut self, revision: &Revision) -> Option<(Document, Style)> {
        let newest = self.loro().oplog_frontiers();
        let mut reader = match self.reader.take() {
            Some(reader) if reader.made == newest => reader,
            Some(mut reader) => {
                if reader.walk(self) {
                    reader
                } else {
                    Box::new(Reader::open(self))
                }
            }
            None => Box::new(Reader::open(self)),
        };
        reader.idle = 0;
        let read = reader.read(revision);
        self.reader = Some(reader);
        read
    }

    /// Counts an edit against the reader, and lets go of one that has not been asked for long.
    pub(super) fn tire(&mut self) {
        let Some(reader) = &mut self.reader else { return };
        reader.idle += 1;
        if reader.idle > IDLE {
            self.reader = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Block, BlockKind, Op, Pos, ReplicaId, TextStyle};

    type Shown = Option<(Document, Style)>;

    /// The document either side of every step, newest first: as the reader tells it, and as the history
    /// reads.
    fn both(model: &mut Model) -> Vec<(Shown, Shown)> {
        let steps = model.timeline();
        let revisions = steps.iter().rev().flat_map(|step| [&step.after, &step.before]);
        let slow = |model: &Model, revision| model.read_at(revision).ok().map(|(doc, _, style)| (doc, style));
        revisions.map(|revision| (model.reads(revision), slow(model, revision))).collect()
    }

    fn typed(block: &str, offset: usize, text: &str) -> Op {
        Op::InsertText { at: Pos::new(block, offset), text: text.into() }
    }

    // Nothing a model shows tells whether its reader answered, so this asks the reader itself.
    #[test]
    fn a_reader_tells_every_step_as_the_history_grows() {
        let blocks = vec![Block::text("p", BlockKind::Paragraph, "Hello world"), Block::new("d", BlockKind::Divider)];
        let mut ana = Model::new(Document { blocks, ..Document::default() }, ReplicaId::from(1));
        let mut ben = ana.fork(ReplicaId::from(2));
        let italic = TextStyle { italic: true, ..TextStyle::default() };
        let rounds: [Vec<Op>; 4] = [
            vec![typed("p", 11, "!")],
            vec![Op::Split { at: Pos::new("p", 5), new_id: "q".into() }, typed("q", 0, "wide")],
            vec![Op::SetStyle { element: "aside".into(), style: italic }, Op::SetLanguage { language: "fr".into() }],
            vec![Op::RemoveBlock { block: "d".into() }, Op::DeleteText { block: "p".into(), range: 0..2 }],
        ];
        for (time, ops) in (1..).zip(rounds) {
            ana.set_time(time * 3600);
            for op in ops {
                ana.apply([op]).unwrap();
                // What was written since the last read is a new step, or more of the last one.
                for (read, held) in both(&mut ana) {
                    assert!(read.is_some(), "the reader lost its place");
                    assert_eq!(read, held);
                }
            }
        }
        // A merge lists the other writer's steps among those walked.
        ben.set_time(3600);
        ben.apply([typed("p", 0, "Oh. ")]).unwrap();
        ana.merge(&ben.save()).unwrap();
        ana.apply([typed("p", 0, "So. ")]).unwrap();
        let read = both(&mut ana);
        assert!(read.len() > 12, "the history has a step for each edit");
        for (read, held) in read {
            assert!(read.is_some(), "the reader lost its place");
            assert_eq!(read, held);
        }
    }
}
