use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use letters::files::{self, Home, Opened};
use letters_format::{Archive, Package, Saved, conflict_copy_of};
use letters_model::{Block, BlockKind, Document, Page, Style};

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

fn document(text: &str) -> Document {
    Document { blocks: vec![Block::text("p", BlockKind::Paragraph, text)], ..Document::default() }
}

/// A file as another writer would leave it, with entries the app itself never touches.
fn write_full(path: &Path, text: &str) -> Package {
    let entries = [("assets/0a1b.png", vec![0x89, b'P']), ("history/oplog", vec![1, 2])];
    let mut package = Package {
        id: "8f14e45f-ceea-467f-a0e6-5b1d3c2a9f00".into(),
        created: "2026-11-14T20:00:00Z".into(),
        document: document(text),
        entries: entries.into_iter().map(|(name, bytes)| (name.to_string(), bytes)).collect(),
    };
    package.set_style(&Style { page: Page { width: 400.0, ..Page::default() }, ..Style::default() });
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

#[test]
fn a_new_document_is_saved_and_opens_again() {
    let folder = Folder::new("new");
    let path = folder.path("Notes.let");
    let home = Home::new(&path);
    assert_eq!(home.title(), "Notes");

    let (home, first) = home.save(&document("First")).unwrap();
    let (home, _) = home.save(&document("Second")).unwrap();
    assert!(matches!(first, Saved::Written), "{first:?}");

    assert_eq!(files::open(&path).unwrap().document, document("Second"));
    assert_eq!(home.path(), path);
    assert_eq!(folder.names(), ["Notes.let"], "a save leaves nothing else behind");
}

#[test]
fn saving_keeps_the_identity_and_every_other_entry_of_the_file() {
    let folder = Folder::new("carry");
    let path = folder.path("Rates.let");
    let original = write_full(&path, "Before");

    let Opened { document: opened, home, .. } = files::open(&path).unwrap();
    assert_eq!(opened, original.document);
    // Two saves, so entries must survive being carried from a file the app wrote itself.
    let (home, _) = home.save(&document("Edited")).unwrap();
    home.save(&document("After")).unwrap();

    assert_eq!(read(&path), Package { document: document("After"), ..original });
}

#[test]
fn a_file_changed_by_someone_else_is_never_overwritten() {
    let folder = Folder::new("conflict");
    let path = folder.path("Rates.let");
    write_full(&path, "Shared");
    let home = files::open(&path).unwrap().home;

    let theirs = write_full(&path, "Theirs");
    let (home, saved) = home.save(&document("Mine")).unwrap();

    assert!(matches!(saved, Saved::ConflictCopy { .. }), "{saved:?}");
    assert_eq!(read(&path), theirs, "their version is untouched");
    assert_eq!(home.path().parent(), path.parent());
    let name = home.path().file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(conflict_copy_of(&name).as_deref(), Some("Rates.let"), "{name}");
    assert_eq!(read(home.path()), Package { document: document("Mine"), ..theirs }, "the copy is a whole document");

    // Later saves stay with the copy, and a second conflict gets a name of its own.
    let (again, _) = home.save(&document("Mine, later")).unwrap();
    assert_eq!(again.path(), home.path());
    write_full(again.path(), "Theirs again");
    let (third, _) = again.save(&document("Mine, third")).unwrap();
    assert_eq!(folder.names().len(), 3);
    assert_eq!(read(third.path()).document, document("Mine, third"));
}

#[test]
fn a_file_deleted_by_someone_else_is_written_back() {
    let folder = Folder::new("deleted");
    let path = folder.path("Rates.let");
    write_full(&path, "Shared");
    let home = files::open(&path).unwrap().home;
    fs::remove_file(&path).unwrap();

    let (home, _) = home.save(&document("Mine")).unwrap();
    assert_eq!(home.path(), path);
    assert_eq!(read(&path).document, document("Mine"));
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
        let error = files::open(&path).expect_err("not a document");
        assert!(!error.to_string().is_empty(), "{path:?}");
    }
}

#[test]
fn a_path_with_no_file_opens_blank_and_is_created_by_the_first_save() {
    let folder = Folder::new("fresh");
    let path = folder.path("Fresh.let");
    let opened = files::open(&path).unwrap();
    assert_eq!(opened.document.blocks.len(), 1);
    assert!(folder.names().is_empty(), "opening writes nothing");

    opened.home.save(&document("Typed")).unwrap();
    assert_eq!(read(&path).document, document("Typed"));
}

#[test]
fn the_style_comes_from_the_file_and_falls_back_to_the_built_in_one() {
    let folder = Folder::new("style");
    let path = folder.path("Styled.let");
    let mut package = write_full(&path, "Text");
    assert_eq!(files::open(&path).unwrap().style.page.width, 400.0);

    for (case, unreadable) in [("not a style", Some(&b"{}"[..])), ("no style", None)] {
        package.entries.remove("style.json");
        package.entries.extend(unreadable.map(|bytes| ("style.json".to_string(), bytes.to_vec())));
        write(&path, &package);
        let opened = files::open(&path).unwrap();
        assert_eq!(opened.style, Style::default(), "{case}");
        opened.home.save(&document("Edited")).unwrap();
        assert_eq!(read(&path).entries.get("style.json").map(Vec::as_slice), unreadable, "{case} is left as it was");
    }
}

#[test]
fn a_new_document_replaces_the_file_its_user_chose_and_carries_a_style() {
    let folder = Folder::new("replace");
    let path = folder.path("Old.let");
    write_full(&path, "Old");

    Home::new(&path).save(&document("New")).unwrap();
    let written = read(&path);
    assert_eq!(written.document, document("New"));
    assert_eq!(written.style().unwrap(), Style::default());
    assert!(!written.entries.keys().any(|name| name.starts_with("history/")), "nothing of the old file remains");
    assert_eq!(folder.names(), ["Old.let"]);
}

#[test]
fn a_file_that_appears_where_a_new_document_was_going_is_left_alone() {
    let folder = Folder::new("appears");
    let path = folder.path("Fresh.let");
    let opened = files::open(&path).unwrap();
    let theirs = write_full(&path, "Theirs");

    let (home, _) = opened.home.save(&document("Mine")).unwrap();
    assert_eq!(read(&path), theirs);
    assert_ne!(home.path(), path);
    assert_eq!(read(home.path()).document, document("Mine"));
}

#[test]
fn a_blank_document_has_somewhere_to_type() {
    let blank = files::blank();
    assert_eq!(blank.blocks.len(), 1);
    assert_eq!(blank.blocks[0].kind, BlockKind::Paragraph);
    assert_ne!(files::blank().blocks[0].id, blank.blocks[0].id, "each blank page gets its own block id");
}
