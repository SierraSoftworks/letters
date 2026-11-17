//! Bringing a file in as a document: what there is to choose, and what the choices give.

use std::fs;
use std::path::{Path, PathBuf};

use letters::import::{self, Choices, Kind, Plan};
use letters_convert::to_docx;
use letters_model::{Anchor, Block, BlockKind, Document, Note, NoteKind, Notes, Run, Style};

/// A picture of 40 by 20 pixels.
const PICTURE: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x28\x00\x00\x00\x14\x01\x00\x00\x00\x00\xd7\x3d\x42\x16\x00\x00\x00\x0c\x49\x44\x41\x54\x78\xda\x63\x60\x18\x18\x00\x00\x00\x78\x00\x01\x35\x33\x6e\xa4\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82";

/// A scratch folder that is removed when the test ends.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("letters-import-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        // The folder as the system spells it, since a plan works its paths out in full.
        Self(path.canonicalize().unwrap())
    }

    fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Notes that show a picture beside them, one below them, two in a folder outside theirs and one that is gone.
fn notes(folder: &Folder) -> PathBuf {
    for picture in ["notes/cat.png", "notes/img/dog.png", "shared/owl.png", "shared/fox.png"] {
        folder.write(picture, PICTURE);
    }
    let pictures = ["cat.png", "img/dog.png", "../shared/owl.png", "../shared/fox.png", "gone.png"];
    let markdown: String = pictures.iter().map(|path| format!("![A picture]({path})\n\n")).collect();
    folder.write("notes/Animals.md", format!("# Animals\n\n{markdown}"))
}

/// How many pictures a document holds, and how many of its blocks show one.
fn pictures(document: &Document, entries: usize) -> (usize, usize) {
    let shown = |block: &&Block| matches!(&block.kind, BlockKind::Figure { asset: Some(_), .. });
    (entries, document.walk().filter(shown).count())
}

#[test]
fn a_plan_says_where_a_files_pictures_are() {
    let folder = Folder::new("plan");
    let plan = Plan::of(&notes(&folder), &Style::default()).expect("Markdown can be brought in");

    assert_eq!((plan.kind, plan.name.as_str()), (Kind::Markdown, "Animals"));
    let found: Vec<(&Path, usize, bool)> =
        plan.folders.iter().map(|found| (found.path.as_path(), found.pictures, found.beside)).collect();
    let at = |name: &str| folder.0.join(name);
    let expected = [(at("notes"), 1, true), (at("notes/img"), 1, true), (at("shared"), 2, false)];
    assert_eq!(
        found,
        expected.iter().map(|(path, count, beside)| (path.as_path(), *count, *beside)).collect::<Vec<_>>()
    );
    assert_eq!(plan.missing, 1);
    // Unasked, only what came with the file is brought in.
    assert_eq!(plan.choices().pictures, [at("notes"), at("notes/img")]);
}

#[test]
fn a_document_is_as_old_as_the_file_it_was_made_of() {
    let folder = Folder::new("dates");
    // 7 June 2021, 08:09:10 in UTC.
    let changed = 1_623_053_350;
    let word = to_docx(&Document::default(), &Style::default(), &Notes::default(), "Quote", &mut |_| None);
    let dated = "---\ndate: 2019-05-04\n---\n\nWords.\n";
    // The file, and when its document was made: when the file says, or else no later than it last changed.
    let cases: [(&str, Vec<u8>, Option<&str>); 3] = [
        ("Plain.md", "Words.".into(), None),
        ("Dated.md", dated.into(), Some("2019-05-04T00:00:00Z")),
        ("Quote.docx", word, None),
    ];
    for (name, bytes, said) in cases {
        let source = folder.write(name, bytes);
        let last = std::time::UNIX_EPOCH + std::time::Duration::from_secs(changed);
        fs::File::options().write(true).open(&source).unwrap().set_modified(last).unwrap();

        let plan = Plan::of(&source, &Style::default()).expect(name);
        let imported = plan.import(&Choices::default());
        assert_eq!(imported.modified, changed, "{name}: last changed when the file was");
        match said {
            Some(said) => assert_eq!(imported.package.created, said, "{name}"),
            None => assert_eq!(imported.package.created, "2021-06-07T08:09:10Z", "{name}"),
        }
    }
}

#[test]
fn pictures_come_from_the_folders_that_were_chosen() {
    let folder = Folder::new("choices");
    let source = notes(&folder);
    let plan = || Plan::of(&source, &Style::default()).unwrap();
    let all: Vec<PathBuf> = plan().folders.iter().map(|found| found.path.clone()).collect();
    // The folders chosen, and how many of the file's five pictures the document then shows.
    let cases = [(plan().choices().pictures, 2), (all.clone(), 4), (all[2..].to_vec(), 2), (Vec::new(), 0)];
    for (chosen, shown) in cases {
        let imported = plan().import(&Choices { pictures: chosen.clone(), comments: true });
        let package = &imported.package;
        // Every picture here holds the same bytes, so the file keeps them once.
        let held = package.entries.keys().filter(|name| name.starts_with("assets/")).count();
        assert_eq!(pictures(&package.document, held), (usize::from(shown > 0), shown), "{chosen:?}");
        // What was left out is still named, as a link, and noted on its passage.
        assert_eq!(imported.notes, 5 - shown, "{chosen:?}");
        assert_eq!(package.document.blocks[0].plain_text(), "Animals");
    }
}

#[test]
fn a_word_files_comments_come_along_if_asked() {
    let folder = Folder::new("word");
    let blocks = vec![Block::text("p", BlockKind::Paragraph, "Rates change in January.")];
    let comment = Note {
        id: "n1".into(),
        kind: NoteKind::Comment,
        anchor: Anchor::range("p", 0..5),
        author: "Ada".into(),
        created: "2026-11-14T20:00:00Z".into(),
        resolved: false,
        content: vec![Run::plain("Which rates?")],
        replies: Vec::new(),
        extra: Default::default(),
    };
    let notes = Notes { notes: vec![comment], ..Notes::default() };
    let document = Document { blocks, ..Document::default() };
    let word = to_docx(&document, &Style::default(), &notes, "Rates", &mut |_| None);
    // Told from what the file holds, whatever it is called.
    let source = folder.write("Rates.txt", word);
    let plan = || Plan::of(&source, &Style::default()).expect("a Word file can be brought in");

    assert_eq!((plan().kind, plan().comments, plan().folders.len()), (Kind::Word, 1, 0));
    assert!(plan().choices().comments);
    for comments in [true, false] {
        let imported = plan().import(&Choices { comments, ..Choices::default() });
        let kept = imported.package.notes().map(|notes| notes.notes.len()).unwrap_or_default();
        assert_eq!(kept, usize::from(comments), "comments asked for: {comments}");
        assert_eq!(imported.package.document.blocks[0].plain_text(), "Rates change in January.");
    }
}

#[test]
fn what_cannot_be_brought_in_is_refused_before_anything_is_asked() {
    let folder = Folder::new("refused");
    let cases: [(&str, &[u8]); 3] = [
        ("Sheet.docx", b"PK\x03\x04 a zip that is no Word file"),
        ("Photo.md", &[0xff, 0xfe, 0x00, 0xd8]),
        ("Old.doc", &[0xd0, 0xcf, 0x11, 0xe0, 1]),
    ];
    // Each says why, as it ends the sentence the person reads.
    for (name, bytes) in cases {
        let why = Plan::of(&folder.write(name, bytes), &Style::default()).expect_err(name).why();
        assert!(why.starts_with(" because ") && !why.ends_with('.'), "{name}: {why}");
    }
    let missing = Plan::of(&folder.0.join("Missing.md"), &Style::default()).expect_err("there is no such file");
    assert_eq!(missing.why(), " because it is not there");
}

#[test]
fn a_plan_and_what_it_makes_can_cross_threads() {
    // A window plans and imports off its own thread.
    fn sendable<T: Send>() {}
    sendable::<Plan>();
    sendable::<letters::import::Imported>();
}

#[test]
fn a_file_is_offered_for_import_by_its_ending() {
    let cases = [
        ("Notes.md", true),
        ("notes/Trip.MARKDOWN", true),
        ("Report.docx", true),
        ("Report.let", false),
        ("Report.doc", false),
        ("md", false),
        ("Picture.png", false),
    ];
    for (path, offered) in cases {
        assert_eq!(import::offered(Path::new(path)), offered, "{path}");
    }
}

#[test]
fn an_imported_document_is_kept_under_a_free_name_and_replaces_nothing() {
    let folder = Folder::new("keep");
    let library = folder.0.join("Letters");
    let import = || Plan::of(&notes(&folder), &Style::default()).map(|plan| plan.import(&plan_choices(&folder)));
    fn plan_choices(folder: &Folder) -> Choices {
        Choices { pictures: vec![folder.0.join("notes")], comments: true }
    }

    let first = import().unwrap().keep(&library, "Animals").expect("the folder is made for it");
    let second = import().unwrap().keep(&library, "Animals").expect("another name is free");

    assert_eq!(first, library.join("Animals.let"));
    assert_ne!(second, first, "the first is not replaced");
    for kept in [&first, &second] {
        let opened = letters::files::open(kept, Style::default).expect("what was kept opens as a document");
        assert_eq!(pictures(&opened.document, 0).1, 2, "{}", kept.display());
    }
    assert!(folder.0.join("notes/Animals.md").exists(), "the file it came from is left alone");
}

#[test]
fn a_plan_whose_document_could_not_be_kept_makes_it_again() {
    let folder = Folder::new("again");
    let plan = Plan::of(&notes(&folder), &Style::default()).unwrap();
    let choices = Choices { pictures: vec![folder.0.join("notes")], comments: true };
    // A file lies where the documents' folder should be.
    let taken = folder.write("Taken", "a file");

    let refused =
        plan.clone().import(&choices).keep(&taken.join("Letters"), "Animals").expect_err("no folder is there");
    let said = letters::files::unwritten("The document", &refused);
    assert_eq!(said, "The document could not be saved because that folder does not exist.");
    let kept = plan.import(&choices).keep(&folder.0.join("Letters"), "Animals").expect("the plan is whole still");

    let opened = letters::files::open(&kept, Style::default).expect("what was kept opens as a document");
    assert_eq!(pictures(&opened.document, 0).1, 2);
}

#[test]
fn a_picture_folder_is_named_so_that_its_own_name_shows() {
    let (beside, home) = (Path::new("/home/ada/notes/trip"), Path::new("/home/ada"));
    let cases = [
        ("/home/ada/notes/trip", "trip"),
        ("/home/ada/notes/trip/img/day one", "trip / img / day one"),
        ("/home/ada/notes/shared", "../shared"),
        ("/home/ada/pictures/2026", "../../pictures/2026"),
        ("/home/ada/notes", "~/notes"),
        ("/home/ada", "~"),
        ("/home/grace/pictures", "/home/grace/pictures"),
        ("/srv/pictures", "/srv/pictures"),
    ];
    for (folder, shown) in cases {
        assert_eq!(import::shown(Path::new(folder), beside, Some(home)), shown, "{folder}");
    }
    assert_eq!(import::shown(Path::new("/srv/pictures"), beside, None), "/srv/pictures", "with no home folder");
}
