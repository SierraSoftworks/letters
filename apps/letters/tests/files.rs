mod common;

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use common::{Writer, comment, comments};
use letters::files::{self, Exchange, Home, Opened};
use letters_format::{Archive, OPLOG, Package, Saved, conflict_copy_of};
use letters_model::{Block, BlockKind, Document, Notes, Page, ReplicaId, Style};

/// A scratch folder that is removed when the test ends.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("letters-files-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A style that is easy to tell from the built-in one.
fn narrow() -> Style {
    Style { page: Page { width: 400.0, ..Page::default() }, ..Style::default() }
}

fn open(path: &Path) -> Result<Opened, files::Error> {
    files::open(path, Style::default)
}

fn document(text: &str) -> Document {
    paragraphs(&[text])
}

/// A document of paragraphs with ids `p0`, `p1` and so on.
fn paragraphs(texts: &[&str]) -> Document {
    let block = |(n, text): (usize, &&str)| Block::text(format!("p{n}").as_str(), BlockKind::Paragraph, *text);
    Document { blocks: texts.iter().enumerate().map(block).collect(), ..Document::default() }
}

const ASSET: &str = "assets/0a1b.png";

/// A file as another tool would leave it: no history, and an entry the app itself never reads.
fn write_full(path: &Path, text: &str) -> Package {
    let mut package = Package {
        id: "8f14e45f-ceea-467f-a0e6-5b1d3c2a9f00".into(),
        created: "2026-11-14T20:00:00Z".into(),
        document: document(text),
        entries: [(ASSET.to_string(), vec![0x89, b'P'])].into(),
    };
    package.set_style(&narrow());
    write(path, &package);
    package
}

fn write(path: &Path, package: &Package) {
    let mut out = Cursor::new(Vec::new());
    letters_format::write(&mut out, package).unwrap();
    fs::write(path, out.into_inner()).unwrap();
}

fn read(path: &Path) -> Package {
    Archive::open(Cursor::new(fs::read(path).unwrap())).unwrap().package().unwrap()
}

/// What makes a file the document it was: its identity, and the entries a save only carries.
fn identity(package: &Package) -> (&str, &str, Option<&Vec<u8>>, Style) {
    (&package.id, &package.created, package.entries.get(ASSET), package.style().unwrap())
}

#[test]
fn a_new_document_is_saved_and_opens_again() {
    let folder = Folder::new("new");
    let path = folder.path("Notes.let");
    let home = Home::new(&path);
    assert_eq!(home.title(), "Notes");

    let mut writer = Writer::new(&document("First"), Style::default());
    let (home, first) = writer.save(&home, &document("First"));
    let (home, _) = writer.save(&home, &document("Second"));
    assert!(matches!(first, Saved::Written), "{first:?}");

    assert_eq!(open(&path).unwrap().document, document("Second"));
    assert_eq!(home.path(), path);
    assert_eq!(folder.names(), ["Notes.let"], "a save leaves nothing else behind");
}

#[test]
fn saving_keeps_the_identity_and_every_other_entry_of_the_file() {
    let folder = Folder::new("carry");
    let path = folder.path("Rates.let");
    let original = write_full(&path, "Before");

    let opened = open(&path).unwrap();
    assert_eq!(opened.document, original.document);
    let (mut writer, home) = Writer::of(opened);
    // Two saves, so entries must survive being carried from a file the app wrote itself.
    let (home, _) = writer.save(&home, &document("Edited"));
    writer.save(&home, &document("After"));

    let written = read(&path);
    assert_eq!(written.document, document("After"));
    assert_eq!(identity(&written), identity(&original));
}

#[test]
fn two_writers_of_one_file_both_keep_their_edits() {
    let folder = Folder::new("merge");
    let path = folder.path("Shared.let");
    Writer::new(&paragraphs(&["One", "Two"]), Style::default()).save(&Home::new(&path), &paragraphs(&["One", "Two"]));
    let (mut ada, adas) = Writer::of(open(&path).unwrap());
    let (mut ben, bens) = Writer::of(open(&path).unwrap());

    ada.save(&adas, &paragraphs(&["One, says Ada", "Two"]));
    // Ben's save meets hers in the file, and hands him what he has not seen.
    ben.write(&paragraphs(&["One", "Two, says Ben"]));
    let seen = ben.model.revision();
    let Exchange { home, news, held, .. } =
        bens.exchange(&mut ben.copy, &ben.model.save(), Notes::default(), &seen, &[]).unwrap();
    ben.model.merge(&news.expect("the file held her edit")).unwrap();
    assert!(!held, "the file had to be written");

    let both = paragraphs(&["One, says Ada", "Two, says Ben"]);
    assert_eq!(ben.model.doc(), &both);
    assert_eq!(open(&path).unwrap().document, both);
    assert_eq!((home.path(), folder.names().len()), (path.as_path(), 1), "no conflict copy was needed");

    // A save with nothing new on either side has nothing to hand back, and finds it all in the file.
    let seen = ben.model.revision();
    let edits = ben.model.updates_since(&seen);
    let again = home.exchange(&mut ben.copy, &edits, Notes::default(), &seen, &[]).unwrap();
    assert!(again.news.is_none() && again.held);
}

#[test]
fn comments_are_saved_as_the_editor_holds_them_and_meet_those_of_other_writers() {
    let folder = Folder::new("comments");
    let path = folder.path("Shared.let");
    let text = paragraphs(&["One", "Two"]);
    Writer::new(&text, Style::default()).save(&Home::new(&path), &text);
    let (mut ada, adas) = Writer::of(open(&path).unwrap());
    let (mut ben, bens) = Writer::of(open(&path).unwrap());
    let holding =
        |ids: &[&str]| Notes { notes: ids.iter().map(|id| comment(id, "p0", "Why?")).collect(), ..Notes::default() };
    // What a writer's editor hands a save: nothing typed, and the notes it holds.
    let hand = |writer: &Writer, id| (writer.model.updates_since(&writer.model.revision()), holding(&[id]));

    // A save hands over the editor's notes, and hears nothing back when the file had no others.
    let (edits, notes) = hand(&ada, "ada");
    let saved = adas.exchange(&mut ada.copy, &edits, notes, &ada.model.revision(), &[]).unwrap();
    assert_eq!(saved.notes, None);
    // One that meets another writer's comment in the file is handed it, and the file keeps both.
    let (edits, notes) = hand(&ben, "ben");
    let saved = bens.exchange(&mut ben.copy, &edits, notes, &ben.model.revision(), &[]).unwrap();
    assert_eq!(saved.notes.as_ref().map(comments), Some(vec!["ada", "ben"]));
    assert_eq!(comments(&Writer::of(open(&path).unwrap()).0.copy.notes), ["ada", "ben"]);
}

#[test]
fn a_file_that_cannot_be_merged_is_never_overwritten() {
    let folder = Folder::new("conflict");
    let path = folder.path("Rates.let");
    write_full(&path, "Shared");
    let (mut writer, home) = Writer::of(open(&path).unwrap());

    // Another tool rewrote the file from its own start, so nothing says how the two lead to each other.
    let theirs = write_full(&path, "Theirs");
    let (home, saved) = writer.save(&home, &document("Mine"));

    assert!(matches!(saved, Saved::ConflictCopy { .. }), "{saved:?}");
    assert_eq!(read(&path), theirs, "their version is untouched");
    assert_eq!(home.path().parent(), path.parent());
    let name = home.path().file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(conflict_copy_of(&name).as_deref(), Some("Rates.let"), "{name}");
    let copy = read(home.path());
    assert_eq!(copy.document, document("Mine"));
    assert_eq!(identity(&copy), identity(&theirs), "the copy is a whole document");

    // Later saves stay with the copy, and a second conflict gets a name of its own.
    let (again, _) = writer.save(&home, &document("Mine, later"));
    assert_eq!(again.path(), home.path());
    write_full(again.path(), "Theirs again");
    let (third, _) = writer.save(&again, &document("Mine, third"));
    assert_eq!(folder.names().len(), 3);
    assert_eq!(read(third.path()).document, document("Mine, third"));
}

/// Histories as a stranger writes them, which would harm a document that took them in, and how.
const HARMFUL: [(&str, &[u8]); 3] = [
    ("Loro panics on it", include_bytes!("../../../fuzz/corpus/history/a-history-loro-panics-on.bin")),
    ("Loro panics on it too", include_bytes!("../../../fuzz/corpus/history/a-second-history-loro-panics-on.bin")),
    ("its save cannot be read", include_bytes!("../../../fuzz/corpus/history/a-history-loro-cannot-write-out.bin")),
];

#[test]
fn a_document_nobody_vouches_for_is_read_as_a_strangers() {
    let folder = Folder::new("wary");
    let path = folder.path("Rates.let");
    write_full(&path, "Shared");
    let (mut writer, home) = Writer::of(open(&path).unwrap());
    writer.save(&home, &document("Shared, and written on"));
    let sound = read(&path);
    let read_as = |wary: bool| {
        let asked = |id: &str| (id == sound.id && wary) || (id != sound.id && panic!("asked of {id}"));
        open(&path).unwrap().rest.read(ReplicaId::new("a window"), None, asked).unwrap().copy
    };

    // A history that does no harm is the document's either way. Only the wary copy checks what comes later.
    let (trusted, wary) = (read_as(false), read_as(true));
    assert_eq!((trusted.wary, wary.wary), (false, true));
    assert_eq!(wary.model().revision(), trusted.model().revision());

    // One that would is left out, and the document opens as its file shows it.
    for (why, history) in HARMFUL {
        let mut harmful = sound.clone();
        harmful.entries.insert(OPLOG.into(), history.to_vec());
        write(&path, &harmful);
        let opened = read_as(true);
        assert_eq!(opened.model().doc(), &sound.document, "{why}");
        assert!(opened.wary && opened.model().save() != history, "{why}");
    }
}

#[test]
fn a_file_whose_history_would_harm_is_not_merged() {
    let folder = Folder::new("harmful-merge");
    let (path, other) = (folder.path("Rates.let"), folder.path("Rates from Ben.let"));
    write_full(&path, "Shared");
    let (mut writer, home) = Writer::of(open(&path).unwrap());
    writer.save(&home, &document("Mine"));

    for (why, history) in HARMFUL {
        let mut harmful = read(&path);
        harmful.entries.insert(OPLOG.into(), history.to_vec());
        write(&other, &harmful);

        let seen = writer.model.revision();
        let (edits, others) = (writer.model.updates_since(&seen), std::slice::from_ref(&other));
        let exchange = home.exchange(&mut writer.copy, &edits, Notes::default(), &seen, others).unwrap();

        assert!(
            matches!(exchange.merged.as_slice(), [merged] if merged.result.is_err()),
            "{why}: {:?}",
            exchange.merged
        );
        assert!(
            exchange.news.is_none() && !writer.copy.wary,
            "{why}: the document is as it was, and its own as before"
        );
        assert_eq!(read(&path).document, document("Mine"), "{why}");
        assert_eq!(read(&other), harmful, "{why}: the other file is only read");
    }
}

#[test]
fn a_file_deleted_by_someone_else_is_written_back() {
    let folder = Folder::new("deleted");
    let path = folder.path("Rates.let");
    let original = write_full(&path, "Shared");
    let (mut writer, home) = Writer::of(open(&path).unwrap());
    fs::remove_file(&path).unwrap();

    let (home, _) = writer.save(&home, &document("Mine"));
    assert_eq!(home.path(), path);
    assert_eq!(read(&path).document, document("Mine"));
    assert_eq!(read(&path).id, original.id);
}

#[test]
fn what_the_system_refused_is_said_in_plain_words() {
    use std::io::{Error, ErrorKind};
    let cases = [
        (ErrorKind::NotFound, "because that folder does not exist"),
        (ErrorKind::NotADirectory, "because that folder does not exist"),
        (ErrorKind::PermissionDenied, "because Letters may not write to that folder"),
        (ErrorKind::ReadOnlyFilesystem, "because Letters may not write to that folder"),
        (ErrorKind::StorageFull, "because the disk is full"),
        (ErrorKind::AlreadyExists, "because a file with that name already exists"),
    ];
    for (kind, why) in cases {
        let said = files::unwritten("The document", &Error::new(kind, "Whatever (os error 20)"));
        assert_eq!(said, format!("The document could not be saved {why}."), "{kind:?}");
    }
    // Where the system has no reason a person can act on, its own words are for the log.
    let unknown = Error::other("Input/output error (os error 5)");
    assert_eq!(files::unwritten("The copy", &unknown), "The copy could not be saved.");
}

#[test]
fn a_save_into_a_folder_that_is_gone_says_so() {
    let folder = Folder::new("gone");
    let path = folder.path("Away/Rates.let");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_full(&path, "Shared");
    let (mut writer, home) = Writer::of(open(&path).unwrap());
    fs::remove_dir_all(path.parent().unwrap()).unwrap();

    writer.write(&document("Mine"));
    writer.copy.edit(&writer.model.save()).unwrap();
    let refused = home.save(&mut writer.copy).expect_err("there is nowhere to write");

    assert_eq!(refused.unsaved("Rates"), "Rates could not be saved because that folder does not exist.");
}

#[test]
fn files_that_are_not_documents_are_refused_with_a_reason() {
    let folder = Folder::new("bad");
    let text = folder.path("plain.let");
    fs::write(&text, "just text").unwrap();
    let truncated = folder.path("truncated.let");
    write_full(&truncated, "Whole");
    let bytes = fs::read(&truncated).unwrap();
    fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();

    for path in [folder.path("no such folder/missing.let"), text, truncated] {
        let error = open(&path).expect_err("not a document");
        assert!(!error.to_string().is_empty(), "{path:?}");
    }
}

#[test]
fn a_path_with_no_file_opens_blank_and_is_created_by_the_first_save() {
    let folder = Folder::new("fresh");
    let path = folder.path("Fresh.let");
    let opened = open(&path).unwrap();
    assert_eq!(opened.document.blocks.len(), 1);
    assert!(folder.names().is_empty(), "opening writes nothing");

    let (mut writer, home) = Writer::of(opened);
    writer.save(&home, &document("Typed"));
    assert_eq!(read(&path).document, document("Typed"));
}

#[test]
fn the_style_comes_from_the_file_and_falls_back_to_the_built_in_one() {
    let folder = Folder::new("style");
    let path = folder.path("Styled.let");
    let mut package = write_full(&path, "Text");
    assert_eq!(open(&path).unwrap().style, narrow());

    for (case, unreadable) in [("not a style", Some(&b"{}"[..])), ("no style", None)] {
        package.entries.remove("style.json");
        package.entries.extend(unreadable.map(|bytes| ("style.json".to_string(), bytes.to_vec())));
        write(&path, &package);
        let opened = open(&path).unwrap();
        assert_eq!(opened.style, Style::default(), "{case}");
        let (mut writer, home) = Writer::of(opened);
        writer.save(&home, &document("Edited"));
        let written = read(&path);
        assert_eq!(written.document, document("Edited"), "{case}");
        // A style this build cannot read may be a newer build's, so it stays exactly as it was.
        match unreadable {
            Some(bytes) => assert_eq!(written.entries.get("style.json").map(Vec::as_slice), Some(bytes), "{case}"),
            None => assert_eq!(written.style().unwrap(), Style::default(), "{case}"),
        }
    }
}

#[test]
fn a_new_document_carries_a_style_and_replaces_nothing() {
    let folder = Folder::new("in-the-way");
    let path = folder.path("Old.let");
    let theirs = write_full(&path, "Old");

    let (home, _) = Writer::new(&document("New"), narrow()).save(&Home::new(&path), &document("New"));
    assert_eq!(read(&path), theirs, "the file that was there is left as it was");
    assert_ne!(home.path(), path);
    let written = read(home.path());
    assert_eq!(written.document, document("New"));
    assert_eq!(written.style().unwrap(), narrow());
}

#[test]
fn a_move_takes_the_whole_file_and_replaces_nothing() {
    let folder = Folder::new("move");
    let (path, taken) = (folder.path("Rates.let"), folder.path("Taken.let"));
    let original = write_full(&path, "Rates");
    let theirs = write_full(&taken, "Theirs");
    let (mut writer, home) = Writer::of(open(&path).unwrap());

    let refused = home.move_to(&taken).unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(home.move_to(&folder.path("missing/Rates.let")).is_err());
    assert_eq!((read(&path), read(&taken)), (original.clone(), theirs), "a refused move changes nothing");
    assert_eq!(folder.names(), ["Rates.let", "Taken.let"]);

    fs::create_dir(folder.path("Work")).unwrap();
    let moved = home.move_to(&folder.path("Work/Rates 2027.let")).unwrap();
    assert_eq!(moved.title(), "Rates 2027");
    assert_eq!(read(moved.path()), original, "the whole file moves with the document");
    assert!(!path.exists());

    // The moved document is saved in its new place, as itself.
    let (moved, saved) = writer.save(&moved, &document("Edited"));
    assert!(matches!(saved, Saved::Written), "{saved:?}");
    assert_eq!(read(moved.path()).document, document("Edited"));
    assert_eq!(identity(&read(moved.path())), identity(&original));
    assert_eq!(folder.names(), ["Taken.let", "Work"]);
}

#[test]
fn a_file_that_appears_where_a_new_document_was_going_is_left_alone() {
    let folder = Folder::new("appears");
    let path = folder.path("Fresh.let");
    let (mut writer, home) = Writer::of(open(&path).unwrap());
    let theirs = write_full(&path, "Theirs");

    let (home, _) = writer.save(&home, &document("Mine"));
    assert_eq!(read(&path), theirs);
    assert_ne!(home.path(), path);
    assert_eq!(read(home.path()).document, document("Mine"));
}

#[test]
fn new_documents_take_the_users_default_style() {
    let folder = Folder::new("default-style");
    assert_eq!(files::new_style(None), Style::default());
    assert_eq!(files::new_style(Some(&folder.0)), Style::default(), "no default saved yet");

    fs::create_dir(folder.path(".letters")).unwrap();
    fs::write(folder.path(".letters/style.json"), "not a style").unwrap();
    assert_eq!(files::new_style(Some(&folder.0)), Style::default(), "an unreadable default");
    fs::write(folder.path(".letters/style.json"), narrow().to_json()).unwrap();
    assert_eq!(files::new_style(Some(&folder.0)), narrow());

    // A path with no file yet is a new document, and its first save carries that style.
    let path = folder.path("Fresh.let");
    let opened = files::open(&path, || files::new_style(Some(&folder.0))).unwrap();
    assert_eq!(opened.style, narrow());
    let (mut writer, home) = Writer::of(opened);
    writer.save(&home, &document("Typed"));
    assert_eq!(read(&path).style().unwrap(), narrow());
}

#[test]
fn a_style_kept_as_the_default_is_what_new_documents_start_with() {
    let folder = Folder::new("keep-style");
    // The first one makes the folder it is kept in, and a later one takes its place.
    for style in [narrow(), Style::default()] {
        files::keep_style(&folder.0, &style.to_json()).unwrap();
        assert_eq!(files::new_style(Some(&folder.0)), style);
    }
    assert_eq!(fs::read_dir(folder.path(".letters")).unwrap().count(), 1, "nothing is left beside it");
}

#[test]
fn a_stylesheet_is_written_as_a_letstyle_file() {
    let folder = Folder::new("stylesheet");
    let sheet = narrow().to_json();
    let cases = [
        ("Look.letstyle", "Look.letstyle"),
        ("Look.LETSTYLE", "Look.LETSTYLE"),
        ("Plain", "Plain.letstyle"),
        ("My.look", "My.look.letstyle"),
    ];
    for (chosen, written) in cases {
        let path = files::write_stylesheet(&folder.path(chosen), &sheet).unwrap();
        assert_eq!(path, folder.path(written), "{chosen}");
        assert_eq!(Style::from_json(&fs::read_to_string(path).unwrap()).unwrap(), narrow(), "{chosen}");
    }

    // The person was asked about replacing the name they chose, and not about one that was made from it.
    let other = Style::default().to_json();
    files::write_stylesheet(&folder.path("Look.letstyle"), &other).unwrap();
    assert_eq!(fs::read_to_string(folder.path("Look.letstyle")).unwrap(), other);
    let refused = files::write_stylesheet(&folder.path("Plain"), &other).unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(folder.path("Plain.letstyle")).unwrap(), sheet);
}

#[test]
fn a_blank_document_has_somewhere_to_type() {
    let blank = files::blank();
    assert_eq!(blank.blocks.len(), 1);
    assert_eq!(blank.blocks[0].kind, BlockKind::Paragraph);
    assert_ne!(files::blank().blocks[0].id, blank.blocks[0].id, "each blank page gets its own block id");
}
