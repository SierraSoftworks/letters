use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use letters::files::{self, Home};
use letters_format::{Archive, Package};
use letters_model::{Block, BlockKind, Document};

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
    let entries =
        [("style.json", b"{}".to_vec()), ("assets/0a1b.png", vec![0x89, b'P']), ("history/oplog", vec![1, 2])];
    let package = Package {
        id: "8f14e45f-ceea-467f-a0e6-5b1d3c2a9f00".into(),
        created: "2026-11-14T20:00:00Z".into(),
        document: document(text),
        entries: entries.into_iter().map(|(name, bytes)| (name.to_string(), bytes)).collect(),
    };
    let mut out = Cursor::new(Vec::new());
    letters_format::write(&mut out, &package).unwrap();
    fs::write(path, out.into_inner()).unwrap();
    package
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

    let home = home.save(&document("First")).unwrap();
    let home = home.save(&document("Second")).unwrap();

    let (reopened, _) = files::open(&path).unwrap();
    assert_eq!(reopened, document("Second"));
    assert_eq!(home.path(), path);
    assert_eq!(folder.names(), ["Notes.let"], "a save leaves nothing else behind");
}

#[test]
fn saving_keeps_the_identity_and_every_other_entry_of_the_file() {
    let folder = Folder::new("carry");
    let path = folder.path("Rates.let");
    let original = write_full(&path, "Before");

    let (opened, home) = files::open(&path).unwrap();
    assert_eq!(opened, original.document);
    // Two saves, so entries must survive being carried from a file the app wrote itself.
    let home = home.save(&document("Edited")).unwrap();
    home.save(&document("After")).unwrap();

    assert_eq!(read(&path), Package { document: document("After"), ..original });
}

#[test]
fn a_file_changed_by_someone_else_is_never_overwritten() {
    let folder = Folder::new("conflict");
    let path = folder.path("Rates.let");
    write_full(&path, "Shared");
    let (_, home) = files::open(&path).unwrap();

    let theirs = write_full(&path, "Theirs");
    let home = home.save(&document("Mine")).unwrap();

    assert_eq!(read(&path), theirs, "their version is untouched");
    assert_ne!(home.path(), path);
    assert_eq!(home.path().parent(), path.parent());
    assert!(home.title().starts_with("Rates (conflicted copy "), "{}", home.title());
    assert_eq!(read(home.path()).document, document("Mine"));
    assert_eq!(read(home.path()).id, theirs.id, "a conflict copy is the same document");

    // Later saves stay with the copy, and a second conflict gets a name of its own.
    let again = home.save(&document("Mine, later")).unwrap();
    assert_eq!(again.path(), home.path());
    write_full(again.path(), "Theirs again");
    let third = again.save(&document("Mine, third")).unwrap();
    assert_eq!(folder.names().len(), 3);
    assert_eq!(read(third.path()).document, document("Mine, third"));
}

#[test]
fn a_file_deleted_by_someone_else_is_written_back() {
    let folder = Folder::new("deleted");
    let path = folder.path("Rates.let");
    write_full(&path, "Shared");
    let (_, home) = files::open(&path).unwrap();
    fs::remove_file(&path).unwrap();

    let home = home.save(&document("Mine")).unwrap();
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

    for path in [folder.path("missing.let"), text, truncated] {
        let error = files::open(&path).expect_err("not a document");
        assert!(!error.to_string().is_empty(), "{path:?}");
    }
}

#[test]
fn a_blank_document_has_somewhere_to_type() {
    let blank = files::blank();
    assert_eq!(blank.blocks.len(), 1);
    assert_eq!(blank.blocks[0].kind, BlockKind::Paragraph);
    assert_ne!(files::blank().blocks[0].id, blank.blocks[0].id, "each blank page gets its own block id");
}
