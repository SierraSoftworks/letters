//! Windows of two copies of the app that share a folder of journals: they open documents, write, save,
//! close and crash. No two windows hold one journal, what a journal was given comes back when an app next
//! starts, and no lock outlives its window.
//!
//! This runs the desktop shell's own journals, with their writing thread, lock files and a real folder.

use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use letters::files::{self, Exchange, Home, Opened, Working};
use letters::journal::{Journal, Journals};
use letters::platform;
use letters_format::{Archive, WorkingCopy, journal_name};
use letters_model::{Model, ReplicaId, Revision, Style};

use crate::edits::{Shown, ops, shown};
use crate::input::Script;

const STEPS: usize = 32;
/// The paths documents are saved at.
const SLOTS: usize = 3;
const WINDOWS: usize = 5;
const APPS: usize = 2;

/// A folder for one run, in memory where the system has such a place: every record waits for the disk.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let memory = Path::new("/dev/shm");
        let base = if memory.is_dir() { memory.to_owned() } else { std::env::temp_dir() };
        let path = base.join(format!("letters-fuzz-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("unsaved")).expect("a scratch folder can be made");
        Self(path)
    }

    fn slot(&self, slot: usize) -> PathBuf {
        self.0.join(format!("doc{slot}.let"))
    }

    fn journals(&self) -> PathBuf {
        self.0.join("unsaved")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A document open in a window.
struct Desk {
    app: usize,
    model: Model,
    copy: WorkingCopy,
    home: Option<Home>,
    /// `None` when another window has it.
    journal: Option<Journal>,
    /// The revision the journal has all of.
    seen: Option<Revision>,
}

impl Desk {
    /// Opens a document as a window does once its first frame is out.
    fn at(journals: &mut Journals, app: usize, opened: Opened) -> Self {
        let replica = ReplicaId::new(&platform::block_id());
        let mut model = Model::new(opened.document, replica).with_style(opened.style);
        let rest = opened.rest.read(replica, Some(journals.folder())).expect("the rest of an opened document reads");
        let Working { copy, history, news, claim, journalled, .. } = rest;
        if let Some(history) = history {
            model.attach(history);
        }
        if let Some(news) = news {
            model.merge(&news).expect("an editor reads what a journal kept");
        }
        let file = opened.home.as_ref().filter(|home| home.written()).map(Home::path);
        let journal = claim.map(|claim| journals.open(claim, &copy.id, &copy.created, file));
        Self { app, model, copy, home: opened.home, journal, seen: journalled }
    }

    fn write(&mut self, script: &mut Script) {
        let ops = ops(script, &shown(&self.model), 2);
        let changed = self.model.apply(ops).is_ok_and(|change| change != Default::default());
        if let Some(journal) = self.journal.as_mut().filter(|_| changed) {
            let model = &self.model;
            let edits = self.seen.as_ref().map_or_else(|| model.save(), |seen| model.updates_since(seen));
            self.seen = Some(model.revision());
            journal.edits(edits);
        }
    }

    /// Saves as an autosave does.
    fn save(&mut self) {
        let Some(home) = self.home.clone() else { return };
        if let Some(journal) = &mut self.journal {
            journal.saving();
        }
        let (edits, seen) = (self.model.save(), self.model.revision());
        // This editor writes no comments, so the notes are the copy's own.
        let notes = self.copy.notes.clone();
        let exchange = home.exchange(&mut self.copy, &edits, notes, &seen, &[]);
        let Exchange { home, news, held, .. } = exchange.expect("a save to a folder that takes every write ends");
        if let Some(news) = news {
            self.model.merge(&news).expect("an editor reads what its file held");
        }
        if let Some(journal) = &mut self.journal {
            journal.moved(home.path());
            journal.saved(held);
        }
        self.home = Some(home);
    }

    /// What this window leaves to its journal as it goes, if anything.
    fn leaves(&self) -> Option<Left> {
        let owed = self.journal.as_ref().is_some_and(Journal::owes);
        let home = self.home.as_ref().filter(|home| home.written()).map(|home| home.path().to_owned());
        owed.then(|| Left { app: self.app, history: self.model.save(), home })
    }
}

/// Unsaved work whose window is gone: what it showed, and the file its journal builds on.
struct Left {
    /// The copy of the app the window was in, whose writing thread lets go of the journal in its own time.
    app: usize,
    history: Vec<u8>,
    home: Option<PathBuf>,
}

impl Left {
    /// What must be shown when the work comes back, and whether it must come back at all. `None` when the
    /// file the journal builds on is gone or is another document's, and the journal waits.
    fn owed(&self, id: &str) -> Option<(Shown, bool)> {
        let mut model = Model::load(&self.history, ReplicaId::from(90)).expect("a saved history loads");
        // With no file to add to, the journal holds the whole document.
        let Some(home) = &self.home else { return Some((shown(&model), true)) };
        let mut archive = Archive::open(Cursor::new(fs::read(home).ok()?)).ok()?;
        let file = WorkingCopy::open(&mut archive, ReplicaId::from(91)).ok().filter(|file| file.id == id)?;
        model.merge(&file.model().save()).ok()?;
        // A file that holds it all needs nothing from the journal, which may then just go.
        let more = shown(&model) != shown(file.model());
        Some((shown(&model), more))
    }
}

struct World {
    scratch: Scratch,
    apps: Vec<Journals>,
    desks: Vec<Desk>,
    /// By document id.
    left: BTreeMap<String, Left>,
}

impl World {
    fn open(&mut self, app: usize, opened: Opened, new: bool) {
        let desk = Desk::at(&mut self.apps[app], app, opened);
        let id = &desk.copy.id;
        let held = self.desks.iter().any(|other| other.copy.id == *id && other.journal.is_some());
        assert!(!(held && desk.journal.is_some()), "two windows hold the journal of one document");
        assert!(desk.journal.is_some() || !new, "a new document's journal could not be claimed");
        if desk.journal.is_some() {
            // Whatever waited for this document is the window's from here on.
            self.left.remove(id);
        }
        self.desks.push(desk);
    }

    /// Ends a window. Its journal stays while it holds work, unless the document is not worth keeping.
    fn close(&mut self, index: usize, forget: bool) {
        let mut desk = self.desks.remove(index);
        if let Some(left) = desk.leaves().filter(|_| !forget) {
            self.left.insert(desk.copy.id.clone(), left);
        }
        if let Some(journal) = desk.journal.as_mut().filter(|_| forget) {
            journal.forget();
        }
    }

    /// Ends a copy of the app with no chance to save, and starts it again as it opens what was left.
    fn crash(&mut self, app: usize) {
        while let Some(index) = self.desks.iter().position(|desk| desk.app == app) {
            self.close(index, false);
        }
        self.apps[app] = Journals::at(self.scratch.journals());
        // Only this copy's journals are surely let go of by now, so only they must come back.
        let owed = |(id, left): (&String, &Left)| {
            let (expected, more) = left.owed(id)?;
            Some((id.clone(), (expected, more && left.app == app)))
        };
        let owed: BTreeMap<String, (Shown, bool)> = self.left.iter().filter_map(owed).collect();
        let back: Vec<Opened> = self.apps[app].orphans().into_iter().filter_map(files::reopen).collect();
        let mut returned = Vec::new();
        for opened in back {
            let desk = Desk::at(&mut self.apps[app], app, opened);
            if let Some((expected, _)) = owed.get(&desk.copy.id) {
                assert_eq!(&shown(&desk.model), expected, "unsaved work came back as another document");
            }
            returned.push(desk);
        }
        for (id, (_, more)) in owed {
            let came = returned.iter().any(|desk| desk.copy.id == id);
            assert!(came || !more, "unsaved work did not come back when the app started again");
        }
        // What came back is a window's again, and what did not has gone or waits for its file.
        self.left.retain(|id, _| self.scratch.journals().join(journal_name(id)).exists());
        for desk in &returned {
            self.left.remove(&desk.copy.id);
        }
        self.desks.extend(returned);
    }
}

pub fn run(data: &[u8]) {
    let mut script = Script::new(data);
    let scratch = Scratch::new();
    let apps = (0..APPS).map(|_| Journals::at(scratch.journals())).collect();
    let mut world = World { scratch, apps, desks: Vec::new(), left: BTreeMap::new() };

    for _ in 0..STEPS {
        if script.done() {
            break;
        }
        let (step, app) = (script.pick(10), script.pick(APPS));
        let window = (!world.desks.is_empty()).then(|| script.pick(world.desks.len()));
        match (step, window) {
            (0 | 1, _) if world.desks.len() < WINDOWS => {
                let path = world.scratch.slot(script.pick(SLOTS));
                let new = !path.exists();
                let opened = files::open(&path, Style::default).expect("a file Letters saved opens");
                world.open(app, opened, new);
            }
            (2..=4, Some(window)) => world.desks[window].write(&mut script),
            (5 | 6, Some(window)) => world.desks[window].save(),
            (7, Some(window)) => world.close(window, script.flag()),
            (8, _) => world.crash(app),
            _ => {}
        }
    }

    // With every window closed and both apps gone, nothing is locked.
    let World { scratch, apps, desks, .. } = world;
    drop((desks, apps));
    let entries = fs::read_dir(scratch.journals()).expect("the journals have a folder");
    let names = entries.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned());
    let locks: Vec<String> = names.filter(|name| name.ends_with(".lock")).collect();
    assert!(locks.is_empty(), "locks outlived their windows: {locks:?}");
}

pub fn seeds() -> Vec<Vec<u8>> {
    let edit = [2, 0, 0, 0, 1, 0, 2, 1, 3];
    // Open, write, save twice, write, crash, and the same with a second window on the document.
    let once = [&[0, 0, 0][..], &edit, &[5, 0, 0, 5, 0, 0], &edit, &[8, 0]].concat();
    let twice = [&[0, 0, 0][..], &edit, &[5, 0, 0, 0, 1, 0], &edit, &[3, 1, 1, 1, 2, 5, 1, 1, 8, 0, 8, 1]].concat();
    let closed = [&[1, 1, 2][..], &edit, &[7, 0, 0, 0, 1, 1], &edit, &[7, 0, 0, 1, 8, 1, 8, 0]].concat();
    vec![once, twice, closed, (0..=255).collect()]
}
