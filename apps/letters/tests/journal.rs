mod common;

use std::fs;
use std::path::Path;

use common::{Scratch, Writer};
use letters::files::{self, Exchange, Home, Opened, Rest, Working};
use letters::journal::{Journal, Journals};
use letters::platform;
use letters_model::{Block, BlockKind, Document, Model, ReplicaId, Revision, Style};

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
    /// The names and sizes of the pictures the window was given to show.
    pictures: Vec<(String, u32, u32)>,
}

impl Desk {
    /// Opens a document as a window does once its first frame is out.
    fn at(journals: &mut Journals, opened: Opened) -> Self {
        let replica = ReplicaId::new(&platform::block_id());
        let mut model = Model::new(opened.document, replica).with_style(opened.style);
        let rest = opened.rest.read(replica, Some(journals.folder())).unwrap();
        let Working { copy, history, news, claim, journalled, pictures } = rest;
        let pictures = pictures.into_iter().map(|(name, image)| (name, image.width(), image.height())).collect();
        if let Some(history) = history {
            model.attach(history);
        }
        if let Some(news) = news {
            model.merge(&news).unwrap();
        }
        let file = opened.home.as_ref().filter(|home| home.written()).map(Home::path);
        let journal = claim.map(|claim| journals.open(claim, &copy.id, &copy.created, file));
        Self { writer: Writer { model, copy }, home: opened.home, journal, seen: journalled, pictures }
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
        let Exchange { home, held, .. } = home.exchange(&mut self.writer.copy, &edits, &seen).unwrap();
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
    let shown = vec![("assets/pasted.png".to_owned(), 3, 2)];
    desk.journal().asset(&shown[0].0, &picture.bytes);
    desk.write(&document("With the picture"));

    let mut journals = crash(desk, journals);
    let mut back = brought_back(&journals);
    assert_eq!(back.len(), 1);
    let mut desk = Desk::at(&mut journals, back.remove(0));
    assert_eq!(desk.pictures, shown, "the window is given the picture again");

    // The save it is owed puts the picture in the file, where the next opening finds it.
    desk.save();
    drop((desk, journals));
    let mut journals = Journals::at(scratch.0.join("elsewhere"));
    let desk = Desk::at(&mut journals, files::open(&path, Style::default).unwrap());
    assert_eq!(desk.pictures, shown, "the file holds the picture");
}
