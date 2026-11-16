mod common;

use std::fs;
use std::path::Path;

use common::{Scratch, Writer, comment, comments};
use letters::files::{self, Exchange, Home, Opened, Rest, Working};
use letters::journal::{self, Aside, Journal, Journals, Why};
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
    /// The journal that was set aside for the one the window has.
    aside: Option<Aside>,
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
        let aside = claim.as_ref().and_then(|claim| claim.aside().cloned());
        let journal = claim.map(|claim| journals.open(claim, &copy.id, &copy.created, file));
        Self { writer: Writer { model, copy }, home: opened.home, journal, seen: journalled, aside }
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
    drop(desk);
    crash_of(journals)
}

/// Ends a session that has no window, and starts the next one.
fn crash_of(journals: Journals) -> Journals {
    let folder = journals.folder().to_owned();
    drop(journals);
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

/// Found by fuzzing a journal (issue #4): an edit Loro panics on in a document that lacks its writer.
const PANICS: &[u8] = include_bytes!("../../../fuzz/corpus/history/a-first-edit-that-builds-on-one-before-it.bin");

#[test]
fn a_journal_the_app_ended_at_is_set_aside_and_its_document_opens_as_saved() {
    let cases: [(&str, Option<&[u8]>); 2] =
        [("what the session wrote", None), ("an edit Loro panics on", Some(PANICS))];
    for (nth, (case, edit)) in cases.into_iter().enumerate() {
        let scratch = Scratch::new(&format!("journal-aside-{nth}"));
        let path = scratch.0.join("Notes.let");
        let mut journals = Journals::at(scratch.0.join("unsaved"));
        let mut desk = Desk::at(&mut journals, new(Some(&path)));
        let saved = document("First thoughts");
        desk.write(&saved);
        assert!(!desk.save() && desk.save(), "{case}");
        desk.write(&document("First thoughts, and second ones"));
        if let Some(edit) = edit {
            desk.journal().edits(edit.to_vec());
        }
        let journals = crash(desk, journals);
        let [journal] = &scratch.names("unsaved")[..] else { panic!("{case}: one journal is left") };
        let left = fs::read(scratch.0.join("unsaved").join(journal)).unwrap();

        // The next session ends as it brings the journal back, which lets go of what it held.
        let claims = journals.orphans();
        let ended = std::panic::catch_unwind(|| claims[0].replay(|_| panic!("the app ends here")));
        assert!(ended.is_err(), "{case}");
        drop(claims);

        // The one after it leaves the journal alone and shows the document as its file has it.
        let mut journals = crash_of(journals);
        let claims = journals.orphans();
        let aside: Vec<_> = claims.iter().filter_map(|claim| claim.aside().cloned()).collect();
        let mut back: Vec<Opened> = claims.into_iter().filter_map(files::reopen).collect();
        assert_eq!((aside.len(), back.len()), (1, 1), "{case}");
        assert_eq!(aside[0].home.as_deref(), Some(path.as_path()), "{case}");
        assert_eq!(fs::read(&aside[0].kept).unwrap(), left, "{case}: what was set aside is kept whole");
        let kept = aside[0].kept.file_name().unwrap().to_string_lossy();
        assert!(aside[0].said(true).contains(&*kept), "{case}: the person is told where it is");
        assert_eq!(back[0].document, saved, "{case}");

        // The document is journalled as any other from here on, and what was set aside is not read again.
        let mut desk = Desk::at(&mut journals, back.remove(0));
        let typed = document("First thoughts, and third ones");
        desk.write(&typed);
        let journals = crash(desk, journals);
        let claims = journals.orphans();
        assert!(claims.iter().all(|claim| claim.aside().is_none()), "{case}");
        let back: Vec<Opened> = claims.into_iter().filter_map(files::reopen).collect();
        assert_eq!(back.iter().map(|opened| &opened.document).collect::<Vec<_>>(), [&typed], "{case}");
        assert!(aside[0].kept.exists(), "{case}");
    }
}

#[test]
fn a_journal_with_an_edit_that_does_not_stand_comes_back_up_to_it_and_is_set_aside() {
    // Whether the document has a file, what was typed since, and whether anything is to come back.
    let cases = [
        ("a document changed since its save", true, Some("First thoughts, and second ones"), true),
        ("a document that never reached a file", false, Some("Thoughts with no file"), true),
        ("a document with nothing else unsaved", true, None, false),
    ];
    for (nth, (case, saved, typed, back)) in cases.into_iter().enumerate() {
        let scratch = Scratch::new(&format!("journal-part-{nth}"));
        let path = scratch.0.join("Notes.let");
        let mut journals = Journals::at(scratch.0.join("unsaved"));
        let mut desk = Desk::at(&mut journals, new(saved.then_some(&path)));
        let mut expected = document("First thoughts");
        if saved {
            desk.write(&expected);
            assert!(!desk.save() && desk.save(), "{case}");
        }
        if let Some(typed) = typed {
            expected = document(typed);
            desk.write(&expected);
        }
        // Then the edit Loro panics on, and more that was typed after it.
        desk.journal().edits(PANICS.to_vec());
        desk.write(&document("Typed after the edit that does not stand"));
        let mut journals = crash(desk, journals);
        let [journal] = &scratch.names("unsaved")[..] else { panic!("{case}: one journal is left") };
        let left = fs::read(scratch.0.join("unsaved").join(journal)).unwrap();

        let mut opened = brought_back(&journals);
        assert_eq!(opened.len(), 1, "{case}");
        assert_eq!(opened[0].document, expected, "{case}");
        let desk = Desk::at(&mut journals, opened.remove(0));
        let aside = desk.aside.clone().expect(case);
        assert_eq!(aside.why, Why::Refused { back }, "{case}");
        assert_eq!(fs::read(&aside.kept).unwrap(), left, "{case}: what was set aside is kept whole");
        let kept = aside.kept.file_name().unwrap().to_string_lossy();
        assert!(aside.said(true).contains(&*kept), "{case}: the person is told where it is");

        // What came back outlives another crash, and what was set aside is not read again.
        let mut journals = crash(desk, journals);
        let mut again = brought_back(&journals);
        assert_eq!(
            again.iter().map(|opened| &opened.document).collect::<Vec<_>>(),
            [&expected; 1][..usize::from(back)]
        );
        if back {
            assert_eq!(Desk::at(&mut journals, again.remove(0)).aside, None, "{case}");
        }
        assert!(aside.kept.exists(), "{case}");
    }
}
