//! A writer whose sessions end without warning: whatever reached the journal before a crash is in the
//! document the next session opens, whether or not a file holds the rest, and a save that finds it all in
//! the file lets the journal go.

use std::io::Cursor;

use letters_format::{Archive, Journal, Save, Saved, Unsaved, WorkingCopy, journal_name, save};
use letters_model::{Model, Revision, Style};

use crate::edits::{ops, sample, shown};
use crate::folder::Folder;
use crate::input::Script;

const NAME: &str = "doc.let";
const ID: &str = "8f14e45f-ceea-467f-a0e6-5b1d3c2a9f00";
const CREATED: &str = "2026-11-15T00:00:00Z";
const STEPS: usize = 40;

/// One run of the app on the document.
struct Session {
    copy: WorkingCopy,
    journal: Journal,
    /// The revision the journal has all of. `None` until it has the whole history.
    sent: Option<Revision>,
}

fn file(folder: &Folder) -> Option<Archive<Cursor<Vec<u8>>>> {
    Archive::open(Cursor::new(folder.files.get(NAME)?.clone())).ok()
}

impl Session {
    /// Starts as the app does: from the file when there is one, and with whatever the journal kept.
    fn start(folder: &Folder, state: &mut Folder, replica: u64) -> Self {
        let home = folder.files.contains_key(NAME).then_some(NAME);
        let (journal, unsaved) = Journal::open(state, ID, CREATED, home).expect("the journal of this document opens");
        let (copy, sent) = match (file(folder), unsaved) {
            (Some(mut file), unsaved) => {
                let mut copy = WorkingCopy::open(&mut file, replica.into()).expect("a saved file opens");
                if let Some(unsaved) = &unsaved {
                    copy.replay(unsaved).expect("a journal replays onto the file it was kept for");
                }
                let sent = copy.model().revision();
                (copy, Some(sent))
            }
            (None, Some(unsaved)) => {
                let copy = WorkingCopy::recover(&unsaved, replica.into()).expect("a journal that stands alone is read");
                let copy = copy.expect("the journal of a document with no file stands alone");
                let sent = copy.model().revision();
                (copy, Some(sent))
            }
            (None, None) => (WorkingCopy::new(ID, CREATED, sample(), Style::default(), replica.into()), None),
        };
        Self { copy, journal, sent }
    }

    /// Hands the journal what the document holds beyond what it was last given.
    fn record(&mut self, state: &mut Folder) {
        let model = self.copy.model();
        let edits = self.sent.as_ref().map_or_else(|| model.save(), |sent| model.updates_since(sent));
        self.sent = Some(model.revision());
        self.journal.edits(state, &edits).expect("a folder that takes every write is journalled to");
    }
}

pub fn run(data: &[u8]) {
    let mut script = Script::new(data);
    let (mut folder, mut state) = (Folder::default(), Folder::default());
    let journal = journal_name(ID);
    let mut session = Session::start(&folder, &mut state, 1);
    // Everything the journal or the file is known to hold, as a history, and the same before the journal's
    // last record: a crash may tear that record and no other.
    let mut held = Model::new(sample(), 80.into()).save();
    let mut before: Option<(Vec<u8>, usize)> = None;

    for nth in 0..STEPS {
        if script.done() {
            break;
        }
        match script.pick(12) {
            0..=5 => {
                let ops = ops(&mut script, &shown(session.copy.model()), 2);
                if session.copy.apply(ops).is_ok_and(|change| change != Default::default()) {
                    let length = state.files.get(&journal).map_or(0, Vec::len);
                    before = Some((std::mem::take(&mut held), length));
                    session.record(&mut state);
                    held = session.copy.model().save();
                }
            }
            // An autosave, which tells the journal how it ended.
            6..=8 => {
                let mark = session.journal.mark();
                let order = Save { name: NAME, token: &format!("n{nth}"), label: "fuzz", attempts: 2 };
                let saved =
                    save(&mut folder, &mut session.copy, &order).expect("a folder that takes every write is saved to");
                session.journal.moved(&mut state, NAME).expect("a folder that takes every write is journalled to");
                session
                    .journal
                    .saved(&mut state, mark, &saved)
                    .expect("a folder that takes every write is journalled to");
                if matches!(saved, Saved::Unchanged) && session.sent == Some(session.copy.model().revision()) {
                    assert!(session.journal.is_empty(), "a save that found everything in the file left records behind");
                    // A crash can leave a journal that is its header alone, which stays until it is written to.
                    let left = Unsaved::read(&mut state, &journal).expect("the journal folder reads");
                    assert!(left.is_none(), "a journal that was saved away still holds work");
                }
                // What another writer saved has come in, and the journal builds on it from here.
                held = session.copy.model().save();
                before = None;
            }
            // Another writer saves to the file.
            9 => {
                let Some(mut file) = file(&folder) else { continue };
                let mut other = WorkingCopy::open(&mut file, (100 + nth as u64).into()).expect("a saved file opens");
                let _ = other.apply(ops(&mut script, &shown(other.model()), 2));
                let order = Save { name: NAME, token: "other", label: "other", attempts: 2 };
                save(&mut folder, &mut other, &order).expect("a folder that takes every write is saved to");
            }
            // The app dies, perhaps as the journal's last record was on its way to the disk.
            _ => {
                let torn = before.take().filter(|_| script.flag());
                if let (Some((earlier, length)), Some(bytes)) = (torn, state.files.get_mut(&journal)) {
                    let kept = length + script.pick(bytes.len() - length);
                    bytes.truncate(kept);
                    held = earlier;
                }
                let mut expected = Model::load(&held, 81.into()).expect("a saved history loads");
                if let Some(mut file) = file(&folder) {
                    let theirs = WorkingCopy::open(&mut file, 82.into()).expect("a saved file opens");
                    expected.merge(&theirs.model().save()).expect("a replica reads another's history");
                }
                session = Session::start(&folder, &mut state, 2 + nth as u64);
                assert_eq!(shown(session.copy.model()), shown(&expected), "a crash lost or added work");
                held = session.copy.model().save();
            }
        }
    }

    // Left alone, the document reaches its file and the journal goes: one save writes, and the next reads back.
    for round in 0.. {
        let mark = session.journal.mark();
        let order = Save { name: NAME, token: "end", label: "fuzz", attempts: 2 };
        let saved = save(&mut folder, &mut session.copy, &order).expect("a folder that takes every write is saved to");
        session.journal.saved(&mut state, mark, &saved).expect("a folder that takes every write is journalled to");
        if matches!(saved, Saved::Unchanged) {
            break;
        }
        assert!(round < 3, "the saves never find everything in the file");
    }
    assert!(
        Unsaved::read(&mut state, &journal).expect("the journal folder reads").is_none(),
        "a journal outlived its save"
    );
    let mut file = file(&folder).expect("a saved file opens");
    let reopened = WorkingCopy::open(&mut file, 99.into()).expect("a saved file opens");
    assert_eq!(
        shown(reopened.model()),
        shown(session.copy.model()),
        "the file holds another document than was written"
    );
}

pub fn seeds() -> Vec<Vec<u8>> {
    let edit = [0, 0, 1, 0, 2, 1, 3];
    let mut seeds: Vec<Vec<u8>> = (6..12u8)
        .map(|step| [&edit[..], &[step, 1, 2, 3], &edit, &[11, 1, 9], &edit, &[6], &[step, 0, 7, 3, 4, 5]].concat())
        .collect();
    seeds.push((0..=255).collect());
    seeds
}
