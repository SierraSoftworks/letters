mod common;

use std::fs;
use std::path::Path;

use common::{Scratch, Writer, comment, comments};
use letters::files::{self, Exchange, Home, Opened, Rest, Working};
use letters::journal::{self, Journal, Journals};
use letters::platform;
use letters_model::{Block, BlockKind, Document, Model, Notes, ReplicaId, Revision, Style};

fn document(text: &str) -> Document {
    Document { blocks: vec![Block::text("p0", BlockKind::Paragraph, text)], ..Document::default() }
}

/// A new document that is to be saved at `home`, if anywhere.
fn new(home: Option<&Path>) -> Opened {
    let (document, style) = (document(""), Style::default());
    Opened { rest: Rest::new(&document, &style), document, style, home: home.map(Home::new) }
}

/// A document open in a window: its writer, where it is saved, and its journal.
struct Desk {
    writer: Writer,
    home: Option<Home>,
    /// `None` when another window has it.
    journal: Option<Journal>,
    /// The revision the journal has seen all of.
    seen: Option<Revision>,
}

impl Desk {
    /// Opens a document as a window does once its first frame is out.
    fn at(journals: &mut Journals, opened: Opened) -> Self {
        let replica = ReplicaId::new(&platform::block_id());
        let mut model = Model::new(opened.document, replica).with_style(opened.style);
        let rest = opened.rest.read(replica, Some(journals.folder()), |_| false).unwrap();
        let Working { copy, history, news, claim, journalled } = rest;
        if let Some(history) = history {
            model.attach(history);
        }
        if let Some(news) = news {
            model.merge(&news).unwrap();
        }
        let file = opened.home.as_ref().filter(|home| home.written()).map(Home::path);
        let journal = claim.map(|claim| journals.open(claim, &copy.id, &copy.created, file));
        Self { writer: Writer { model, copy }, home: opened.home, journal, seen: journalled }
    }

    fn journal(&mut self) -> &mut Journal {
        self.journal.as_mut().expect("no other window has the journal")
    }

    /// Types until the document reads as `document`, and lets the journal have it.
    fn write(&mut self, document: &Document) {
        self.writer.write(document);
        let model = &self.writer.model;
        let edits = self.seen.as_ref().map_or_else(|| model.save(), |seen| model.updates_since(seen));
        self.seen = Some(model.revision());
        self.journal().edits(edits);
    }

    /// Saves as an autosave does, and returns whether the file already held everything.
    fn save(&mut self) -> bool {
        let home = self.home.clone().expect("the document has somewhere to go");
        self.journal().saving();
        let (edits, seen) = (self.writer.model.save(), self.writer.model.revision());
        let notes = self.writer.copy.notes.clone();
        let Exchange { home, held, .. } = home.exchange(&mut self.writer.copy, &edits, notes, &seen, &[]).unwrap();
        self.journal().moved(home.path());
        self.journal().saved(held);
        self.home = Some(home);
        held
    }
}

/// Ends a session with no chance to save, and starts the next one.
fn crash(desk: Desk, journals: Journals) -> Journals {
    let folder = journals.folder().to_owned();
    drop((desk, journals));
    Journals::at(folder)
}

/// What the next session opens again as it starts.
fn brought_back(journals: &Journals) -> Vec<Opened> {
    journals.orphans().into_iter().filter_map(files::reopen).collect()
}

#[test]
fn unsaved_work_comes_back_after_a_crash() {
    for (case, saved) in [("a document that never reached a file", false), ("a document changed since its save", true)]
    {
        let scratch = Scratch::new(&format!("journal-back-{saved}"));
        let path = scratch.0.join("Notes.let");
        let mut journals = Journals::at(scratch.0.join("unsaved"));
        let mut desk = Desk::at(&mut journals, new(saved.then_some(&path)));
        if saved {
            desk.write(&document("First thoughts"));
            desk.save();
        }
        let typed = document("First thoughts, and second ones");
        desk.write(&typed);

        let mut journals = crash(desk, journals);
        let mut back = brought_back(&journals);
        assert_eq!(back.len(), 1, "{case}");
        let opened = back.remove(0);
        assert_eq!(opened.document, typed, "{case}");
        assert_eq!(opened.home.as_ref().map(Home::path), saved.then_some(path.as_path()), "{case}");

        // It is owed a save, and the journal goes once one finds it all in the file.
        let mut desk =
            Desk::at(&mut journals, Opened { home: Some(opened.home.unwrap_or(Home::new(&path))), ..opened });
        assert_eq!(desk.writer.model.doc(), &typed, "{case}");
        assert!(desk.journal().owes(), "{case}");
        assert!(!desk.save() && desk.journal().owes(), "{case}: a file that was written is not yet read back");
        assert!(desk.save() && !desk.journal().owes(), "{case}");
        assert_eq!(files::open(&path, Style::default).unwrap().document, typed, "{case}");
        drop((desk, journals));
        assert_eq!(scratch.names("unsaved"), [""; 0], "{case}");
    }
}

#[test]
fn a_crash_after_the_file_was_written_brings_nothing_back() {
    let scratch = Scratch::new("journal-written");
    let mut journals = Journals::at(scratch.0.join("unsaved"));
    let mut desk = Desk::at(&mut journals, new(Some(&scratch.0.join("Notes.let"))));
    desk.write(&document("All of it"));
    assert!(!desk.save());

    let journals = crash(desk, journals);
    assert_eq!(scratch.names("unsaved").len(), 1, "the journal was not known to be in the file");
    assert!(brought_back(&journals).is_empty());
    assert_eq!(scratch.names("unsaved"), [""; 0]);
}

#[test]
fn a_journal_another_window_has_is_left_alone() {
    let scratch = Scratch::new("journal-held");
    let path = scratch.0.join("Notes.let");
    let mut journals = Journals::at(scratch.0.join("unsaved"));
    let mut desk = Desk::at(&mut journals, new(Some(&path)));
    desk.write(&document("One"));
    desk.save();
    desk.write(&document("One, two"));

    // Another copy of the app starts, and opens the same document.
    let mut others = Journals::at(journals.folder());
    assert!(brought_back(&others).is_empty());
    let other = Desk::at(&mut others, files::open(&path, Style::default).unwrap());
    assert!(other.journal.is_none());
    assert_eq!(other.writer.model.doc(), &document("One"), "what the first window has not saved stays its own");

    drop((other, others));
    let journals = crash(desk, journals);
    assert_eq!(brought_back(&journals).len(), 1);
}

#[test]
fn a_document_not_worth_a_file_leaves_no_journal() {
    let scratch = Scratch::new("journal-forgotten");
    let mut journals = Journals::at(scratch.0.join("unsaved"));
    let mut desk = Desk::at(&mut journals, new(None));
    desk.write(&document("x"));
    desk.write(&document(""));
    desk.journal().forget();

    drop((desk, journals));
    assert_eq!(scratch.names("unsaved"), [""; 0]);
}

#[test]
fn a_journal_waits_for_a_file_that_was_moved() {
    let scratch = Scratch::new("journal-moved");
    let (path, moved) = (scratch.0.join("Notes.let"), scratch.0.join("Moved.let"));
    let mut journals = Journals::at(scratch.0.join("unsaved"));
    let mut desk = Desk::at(&mut journals, new(Some(&path)));
    desk.write(&document("One"));
    desk.save();
    desk.save();
    let typed = document("One, two");
    desk.write(&typed);

    let mut journals = crash(desk, journals);
    fs::rename(&path, &moved).unwrap();
    assert!(brought_back(&journals).is_empty(), "the journal adds to a file that is not there");
    assert_eq!(scratch.names("unsaved").len(), 1, "and is kept for it");

    // The file is opened where it is now, and takes what the journal held.
    let mut desk = Desk::at(&mut journals, files::open(&moved, Style::default).unwrap());
    assert_eq!(desk.writer.model.doc(), &typed);
    assert!(desk.journal().owes());
}

#[test]
fn a_picture_put_in_before_a_crash_comes_back_and_reaches_the_file() {
    let scratch = Scratch::new("journal-picture");
    let path = scratch.0.join("Album.let");
    let mut journals = Journals::at(scratch.0.join("unsaved"));
    let mut desk = Desk::at(&mut journals, new(Some(&path)));
    desk.write(&document("Before the picture"));
    desk.save();

    // The asset is journalled ahead of the edit that names it, as a window does.
    let picture = letters::pictures::from_rgba(3, 2, &[0x80; 24]).unwrap();
    let name = "assets/pasted.png";
    desk.journal().asset(name, &picture.bytes);
    desk.write(&document("With the picture"));

    let mut journals = crash(desk, journals);
    let mut back = brought_back(&journals);
    assert_eq!(back.len(), 1);
    let mut desk = Desk::at(&mut journals, back.remove(0));
    assert_eq!(desk.writer.copy.entries.get(name), Some(&picture.bytes), "the window has the picture again");
    assert_eq!(files::entry(&path, name), None, "which the file does not hold yet");

    // The save it is owed puts the picture in the file, where the next opening finds it.
    desk.save();
    assert_eq!(files::entry(&path, name), Some(picture.bytes), "the file holds the picture");
}

#[test]
fn a_comment_written_before_a_crash_comes_back_and_reaches_the_file() {
    let scratch = Scratch::new("journal-comment");
    let path = scratch.0.join("Rates.let");
    let mut journals = Journals::at(scratch.0.join("unsaved"));
    let mut desk = Desk::at(&mut journals, new(Some(&path)));
    desk.write(&document("Rates change in January."));
    desk.save();

    let notes = Notes { notes: vec![comment("n1", "p0", "Which rates?")], ..Notes::default() };
    desk.journal().notes(&notes);

    let mut journals = crash(desk, journals);
    let mut back = brought_back(&journals);
    assert_eq!(back.len(), 1);
    let mut desk = Desk::at(&mut journals, back.remove(0));
    assert_eq!(comments(&desk.writer.copy.notes), ["n1"], "the window has the comment again");
    assert!(desk.journal().owes());

    // The save it is owed puts the comment in the file, where the next opening finds it.
    desk.save();
    let (reader, _) = Writer::of(files::open(&path, Style::default).unwrap());
    assert_eq!(comments(&reader.copy.notes), ["n1"], "the file holds the comment");
}

#[test]
fn a_copy_that_became_its_own_document_comes_back_as_that_after_a_crash() {
    let scratch = Scratch::new("journal-forked");
    let (path, copied) = (scratch.0.join("Notes.let"), scratch.0.join("Notes copy.let"));
    let mut journals = Journals::at(scratch.0.join("unsaved"));
    let mut desk = Desk::at(&mut journals, new(Some(&path)));
    desk.write(&document("First thoughts"));
    assert!(!desk.save() && desk.save());
    let original = desk.writer.copy.id.clone();
    drop(desk);
    fs::copy(&path, &copied).unwrap();

    // The copy is written in: it takes an id of its own, and a journal under that id, before anything
    // of the edit is journalled.
    let mut desk = Desk::at(&mut journals, files::open(&copied, Style::default).unwrap());
    let own = platform::document_id();
    desk.writer.copy.fork(own.clone());
    let claim = journal::claim(journals.folder(), &own).expect("nobody has a journal of that id");
    desk.journal = Some(journals.open(claim, &own, &desk.writer.copy.created, Some(&copied)));
    let typed = document("First thoughts, and others in the copy");
    desk.write(&typed);

    // Its file still says it is the original, and the journal says what it became.
    let mut journals = crash(desk, journals);
    let mut back = brought_back(&journals);
    assert_eq!(back.len(), 1);
    let opened = back.remove(0);
    assert_eq!((&opened.document, opened.home.as_ref().map(Home::path)), (&typed, Some(copied.as_path())));
    let mut desk = Desk::at(&mut journals, opened);
    assert!(!desk.save() && desk.save());

    let id = |path: &Path| Writer::of(files::open(path, Style::default).unwrap()).0.copy.id.clone();
    assert_eq!((id(&copied), id(&path)), (own, original), "each file is one document, and they are two");
    assert_eq!(files::open(&copied, Style::default).unwrap().document, typed);
    assert_eq!(files::open(&path, Style::default).unwrap().document, document("First thoughts"));
}
