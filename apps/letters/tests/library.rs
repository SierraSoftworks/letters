//! Naming, renaming and moving documents, and remembering the ones opened lately.

use std::fs;
use std::path::{Path, PathBuf};

use letters::files;
use letters::library::{Library, Place, Refused, free_path, has_content};
use letters_model::{Block, BlockId, BlockKind, Document, Style};

const H1: BlockKind = BlockKind::Heading { level: 1 };
const TEXT: BlockKind = BlockKind::Paragraph;

/// A user's home folder for one test, removed when the test ends.
struct Desk(PathBuf);

impl Desk {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("letters-library-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn library(&self) -> Library {
        Library::of(&self.0)
    }

    /// A folder under the library's root, made if it is not there.
    fn folder(&self, inside: &str) -> PathBuf {
        let folder = self.library().root().join(inside);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// A document someone else keeps at `inside` the root.
    fn theirs(&self, inside: &str) -> PathBuf {
        let path = self.folder("").join(inside);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        files::Home::new(&path, Style::default()).save(&document(&[(TEXT, "Theirs")])).unwrap();
        path
    }

    /// The names of the files and folders at `inside` the root, sorted.
    fn names(&self, inside: &str) -> Vec<String> {
        let entries = fs::read_dir(self.library().root().join(inside)).unwrap();
        let mut names: Vec<String> = entries.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }
}

impl Drop for Desk {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A document of text blocks, with ids `b0`, `b1` and so on.
fn document(blocks: &[(BlockKind, &str)]) -> Document {
    let block =
        |(n, (kind, text)): (usize, &(BlockKind, &str))| Block::text(format!("b{n}").as_str(), kind.clone(), *text);
    Document { blocks: blocks.iter().enumerate().map(block).collect(), ..Document::default() }
}

/// Saves as the app does, with the caret in block `caret`. Returns where the document ended up.
fn save(place: &mut Place, document: &Document, caret: Option<&str>, library: &Library) -> Option<PathBuf> {
    let caret = caret.map(BlockId::from);
    let home = place.settle(document, caret.as_ref(), &Style::default(), library).unwrap()?.clone();
    let (home, _) = home.save(document).unwrap();
    place.saved(home);
    place.path().map(Path::to_owned)
}

fn read(path: &Path) -> Document {
    files::open(path, Style::default).unwrap().document
}

#[test]
fn a_new_document_has_no_file_until_it_has_content() {
    let desk = Desk::new("content");
    let library = desk.library();
    let mut place = Place::new();

    for empty in [files::blank(), document(&[(H1, "  ")]), Document::default()] {
        assert!(!has_content(&empty));
        assert_eq!(save(&mut place, &empty, None, &library), None);
    }
    assert!(!library.root().exists(), "nothing is made for a document with nothing in it");

    let written = document(&[(TEXT, "Dear all")]);
    let path = save(&mut place, &written, None, &library).unwrap();
    assert_eq!(path, library.root().join("Untitled.let"));
    assert_eq!(read(&path), written);
}

#[test]
fn a_new_document_is_named_once() {
    let desk = Desk::new("naming");
    let library = desk.library();
    let plan = document(&[(H1, "Plan: 2027/28"), (TEXT, "First, the budget.")]);
    // What is in the document, what was typed in the bar, and the name the file gets.
    let cases = [
        ("its first Heading 1", plan.clone(), None, "Plan 2027 28"),
        ("no heading", document(&[(TEXT, "Dear all")]), None, "Untitled"),
        ("a heading with nothing to name a file", document(&[(H1, "???"), (TEXT, "Dear all")]), None, "Untitled 2"),
        ("a typed name wins over the heading", plan.clone(), Some("Budget"), "Budget"),
        ("a name that is taken gets a number", plan.clone(), None, "Plan 2027 28 2"),
        ("and the next one the next number", plan, None, "Plan 2027 28 3"),
    ];
    for (case, written, typed, name) in cases {
        let mut place = Place::new();
        if let Some(typed) = typed {
            place.rename(typed, &library).unwrap();
        }
        let path = save(&mut place, &written, None, &library).unwrap();
        assert_eq!(place.name(), name, "{case}");
        assert_eq!(read(&path), written, "{case}");

        // Later changes to the heading leave the name alone.
        let retitled = document(&[(H1, "Something else"), (TEXT, "Dear all")]);
        assert_eq!(save(&mut place, &retitled, None, &library).unwrap(), path, "{case}");
    }
    assert_eq!(desk.names("").len(), 6, "every document kept its own file: {:?}", desk.names(""));
}

#[test]
fn a_heading_still_being_typed_names_the_file_when_it_is_left() {
    let desk = Desk::new("titling");
    let library = desk.library();
    let mut place = Place::new();

    let typing = document(&[(H1, "Quart")]);
    assert_eq!(save(&mut place, &typing, Some("b0"), &library).unwrap(), library.root().join("Untitled.let"));

    let typed = document(&[(H1, "Quarterly report"), (TEXT, "Sales rose.")]);
    let path = save(&mut place, &typed, Some("b1"), &library).unwrap();
    assert_eq!(path, library.root().join("Quarterly report.let"));
    assert_eq!(desk.names(""), ["Quarterly report.let"], "the file was renamed, not copied");
    assert_eq!(read(&path), typed);

    let retitled = document(&[(H1, "Annual report"), (TEXT, "Sales rose.")]);
    assert_eq!(save(&mut place, &retitled, Some("b1"), &library).unwrap(), path, "the heading names it once");
}

#[test]
fn a_rename_never_replaces_another_document() {
    let desk = Desk::new("rename");
    let library = desk.library();
    let theirs = desk.theirs("Budget.let");
    let mine = document(&[(H1, "Plan"), (TEXT, "First, the budget.")]);
    let mut place = Place::new();
    let path = save(&mut place, &mine, None, &library).unwrap();

    let refused = [
        ("Budget", "taken"),
        ("  Budget ", "taken, however it is spaced"),
        ("", "nothing"),
        ("///", "nothing a file can be called"),
    ];
    for (typed, case) in refused {
        let answer = place.rename(typed, &library);
        match typed.trim() {
            "Budget" => assert!(matches!(answer, Err(Refused::Taken(_))), "{case}: {answer:?}"),
            _ => assert!(matches!(answer, Err(Refused::NoName)), "{case}: {answer:?}"),
        }
        assert_eq!(place.path(), Some(path.as_path()), "{case}");
        assert_eq!(read(&path), mine, "{case}");
        assert_eq!(read(&theirs), document(&[(TEXT, "Theirs")]), "{case}");
    }

    place.rename("Plan", &library).expect("its own name is no clash");
    place.rename(" Plan for 2027 ", &library).unwrap();
    assert_eq!(place.name(), "Plan for 2027");
    assert_eq!(desk.names(""), ["Budget.let", "Plan for 2027.let"]);
    assert_eq!(read(place.path().unwrap()), mine);

    // The renamed document goes on being saved where it now is.
    let edited = document(&[(H1, "Plan"), (TEXT, "Second thoughts.")]);
    save(&mut place, &edited, None, &library);
    assert_eq!(read(&library.root().join("Plan for 2027.let")), edited);
    assert_eq!(desk.names(""), ["Budget.let", "Plan for 2027.let"]);
}

#[test]
fn a_move_keeps_the_name_and_never_replaces_another_document() {
    let desk = Desk::new("move");
    let library = desk.library();
    let (work, home) = (desk.folder("Work"), desk.folder("Home"));
    let theirs = desk.theirs("Home/Plan.let");
    let mine = document(&[(H1, "Plan"), (TEXT, "First, the budget.")]);
    let mut place = Place::new();
    let path = save(&mut place, &mine, None, &library).unwrap();

    let answer = place.move_to(&home);
    assert!(matches!(answer, Err(Refused::Taken(_))), "{answer:?}");
    assert!(place.move_to(&desk.0.join("nowhere")).is_err());
    assert_eq!(place.path(), Some(path.as_path()));
    assert_eq!((read(&path), read(&theirs)), (mine.clone(), document(&[(TEXT, "Theirs")])));

    place.move_to(&work).unwrap();
    assert_eq!(place.path(), Some(work.join("Plan.let").as_path()));
    assert_eq!(read(&work.join("Plan.let")), mine);
    assert!(!path.exists(), "a move leaves nothing behind");
}

#[test]
fn a_document_with_no_file_yet_can_be_named_and_placed() {
    let desk = Desk::new("early");
    let library = desk.library();
    let work = desk.folder("Work");
    desk.theirs("Work/Budget.let");
    let mut place = Place::new();
    assert_eq!((place.name().as_str(), place.folder(&library)), ("Untitled", library.root().to_owned()));

    place.move_to(&work).unwrap();
    assert!(matches!(place.rename("Budget", &library), Err(Refused::Taken(_))));
    place.rename("Minutes", &library).unwrap();
    assert_eq!((place.name().as_str(), place.folder(&library)), ("Minutes", work.clone()));
    assert_eq!(desk.names("Work"), ["Budget.let"], "still nothing on disk");

    let path = save(&mut place, &document(&[(H1, "Plan"), (TEXT, "Dear all")]), None, &library).unwrap();
    assert_eq!(path, work.join("Minutes.let"));
}

#[test]
fn recent_documents_are_remembered_newest_first_while_they_exist() {
    let desk = Desk::new("recents");
    let library = desk.library();
    assert_eq!(library.recents(), []);
    assert_eq!(library.default_folder(), library.root());

    let (plan, budget, notes) = (desk.theirs("Plan.let"), desk.theirs("Work/Budget.let"), desk.theirs("Notes.let"));
    library.opened(&plan, "2026-11-12T09:00:00Z");
    library.opened(&budget, "2026-11-13T09:00:00Z");
    library.opened(&notes, "2026-11-13T10:00:00Z");
    library.opened(&budget, "2026-11-14T09:00:00Z");

    // Another window, or the app the next day, reads the same memory.
    let library = desk.library();
    let listed = |library: &Library| -> Vec<(String, String)> {
        library.recents().iter().map(|recent| (recent.name(), recent.opened[..10].to_owned())).collect()
    };
    let expected = [("Budget", "2026-11-14"), ("Notes", "2026-11-13"), ("Plan", "2026-11-12")];
    assert_eq!(listed(&library), expected.map(|(name, day)| (name.to_owned(), day.to_owned())));
    assert_eq!(library.default_folder(), desk.folder("Work"), "new documents go where the last one was");

    // A document that moved is listed where it is now. One that is gone is not listed, and neither is its
    // folder used.
    let renamed = plan.with_file_name("Plan for 2027.let");
    fs::rename(&plan, &renamed).unwrap();
    library.moved(&plan, &renamed, "2026-11-14T10:00:00Z");
    library.opened(&budget, "2026-11-14T11:00:00Z");
    fs::remove_file(&notes).unwrap();
    fs::remove_dir_all(desk.folder("Work")).unwrap();
    assert_eq!(listed(&library), [("Plan for 2027".to_owned(), "2026-11-14".to_owned())]);
    assert_eq!(library.default_folder(), library.root());
}

#[test]
fn a_memory_that_cannot_be_read_is_an_empty_one() {
    let desk = Desk::new("memory");
    let library = desk.library();
    let plan = desk.theirs("Plan.let");
    fs::create_dir_all(desk.0.join(".letters")).unwrap();
    for unreadable in ["", "not json", "[]", r#"{"recent": [{"path": 4}, "x"], "folder": 7}"#] {
        fs::write(desk.0.join(".letters/recent.json"), unreadable).unwrap();
        assert_eq!(library.recents(), [], "{unreadable}");
        assert_eq!(library.default_folder(), library.root(), "{unreadable}");
    }
    library.opened(&plan, "2026-11-14T09:00:00Z");
    assert_eq!(library.recents().len(), 1);
}

#[test]
fn the_folders_to_move_to_are_the_tree_under_the_root() {
    let desk = Desk::new("folders");
    let library = desk.library();
    for folder in ["Work/Clients/Acme", "work notes", "Home", ".hidden/inside"] {
        desk.folder(folder);
    }
    desk.theirs("Work/Plan.let");

    let tree = |current: &Path| -> Vec<(String, u8)> {
        library.folders(current).into_iter().map(|folder| (folder.name, folder.depth)).collect()
    };
    let expected = [("Letters", 0), ("Home", 1), ("Work", 1), ("Clients", 2), ("Acme", 3), ("work notes", 1)];
    let expected = expected.map(|(name, depth)| (name.to_owned(), depth));
    assert_eq!(tree(&desk.folder("Work")), expected);

    // A document kept somewhere else can stay where it is.
    let elsewhere = desk.0.join("Desktop");
    let listed = tree(&elsewhere);
    assert_eq!(listed[0], ("Desktop".to_owned(), 0));
    assert_eq!(listed[1..], expected);
    assert!(library.folders(&elsewhere).iter().all(|folder| folder.path.is_absolute()));
}

#[test]
fn a_free_path_is_the_first_number_nothing_has() {
    let desk = Desk::new("free");
    let root = desk.folder("");
    assert_eq!(free_path(&root, "Plan"), root.join("Plan.let"));
    desk.theirs("Plan.let");
    desk.theirs("Plan 2.let");
    desk.folder("Plan 3.let");
    assert_eq!(free_path(&root, "Plan"), root.join("Plan 4.let"));
}
