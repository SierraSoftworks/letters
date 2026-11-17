//! The import card: what the window shows for a file that has to be imported, and what it asks the shell.
//! And the toast that counts what an import left on the document it made.

use letters_model::{Anchor, Block, BlockKind, Document, Note, NoteKind, Notes, Pos, Run, Selection};
use letters_render::Point;
use letters_test::{Found, Harness, Role};
use letters_ui::{ImportFolder, ImportKind, ImportOffer, Landing, Request};

const CARD: &str = "Import Trip notes.md";
const COMMENTS: &str = "Bring its 4 comments";

/// A new document, as a window with nothing opened holds.
fn blank() -> Harness {
    Harness::open(Document { blocks: vec![Block::text("p", BlockKind::Paragraph, "")], ..Document::default() })
}

/// A file with pictures beside it and in a folder it is not in, some of them missing, and comments.
fn offer() -> ImportOffer {
    let folder = |path: &str, shown: &str, pictures, chosen| ImportFolder {
        path: path.into(),
        shown: shown.into(),
        pictures,
        chosen,
    };
    ImportOffer {
        name: "Trip notes.md".into(),
        kind: ImportKind::Markdown,
        folders: vec![
            folder("/notes/trip", "Notes / Trip", 3, true),
            folder("/notes/shared", "Notes / Shared", 1, false),
        ],
        missing: 2,
        comments: 4,
        destination: "Family / Travel".into(),
    }
}

/// What the card says beside its ticks, in one string.
fn notes(window: &mut Harness) -> String {
    let notes = window.controls().into_iter().filter(|found| found.role == Role::Note);
    notes.map(|found| found.name).collect::<Vec<_>>().join(" ")
}

fn import(folders: &[&str], comments: bool) -> Request {
    Request::Import { folders: folders.iter().map(|path| (*path).to_owned()).collect(), comments }
}

/// Whether each tick on the card is on, by its name.
fn ticks(window: &mut Harness) -> Vec<(String, Option<bool>)> {
    let ticks = window.controls().into_iter().filter(|found| found.role == Role::CheckBox);
    ticks.map(|found| (found.name, found.toggled)).collect()
}

#[test]
fn the_import_card_says_what_a_file_brings() {
    let mut window = blank();
    assert!(!window.ui().importing(), "no card until the shell offers a file");
    window.ui().set_landing(Some(Landing::default()));
    window.ui().offer_import(Some(offer()));
    window.assert_open(Role::Dialog, CARD).assert_closed(Role::Dialog, "Open a document");
    let shown = [("Notes / Trip", Some(true)), ("Notes / Shared", Some(false)), (COMMENTS, Some(true))];
    assert_eq!(ticks(&mut window), shown.map(|(name, on)| (name.to_owned(), on)), "ticked as the shell suggested");
    let shared = window.find(Role::CheckBox, "Notes / Shared").expect("the second folder");
    assert_eq!(shared.description.as_deref(), Some("1 picture"));
    let said = notes(&mut window);
    for part in ["Family / Travel", "original file is left", "2 pictures", "stay links"] {
        assert!(said.contains(part), "{part} in {said:?}");
    }

    // A file that brings everything itself has nothing to tick, and nothing missing to say.
    let word = ImportOffer { name: "Trip notes.md".into(), kind: ImportKind::Word, ..ImportOffer::default() };
    window.ui().offer_import(Some(word));
    window.assert_open(Role::Dialog, CARD);
    assert_eq!(ticks(&mut window), [], "nothing to choose");
    let said = notes(&mut window);
    assert!(said.contains("original file is left") && !said.contains("found"), "{said:?}");
}

#[test]
fn importing_asks_the_shell_with_what_is_ticked() {
    // What is done on the card before Import, and what the shell is then asked.
    let cases: [(&str, &[&str], Request); 4] = [
        ("Enter at once", &[], import(&["/notes/trip"], true)),
        ("another folder", &["Notes / Shared"], import(&["/notes/trip", "/notes/shared"], true)),
        ("no folder and no comments", &["Notes / Trip", COMMENTS], import(&[], false)),
        ("a tick taken back", &["Notes / Shared", "Notes / Shared"], import(&["/notes/trip"], true)),
    ];
    for (case, clicks, request) in cases {
        let mut window = blank();
        window.ui().offer_import(Some(offer()));
        for tick in clicks {
            window.click(Role::CheckBox, tick);
        }
        match clicks.is_empty() {
            true => window.key("Enter"),
            false => window.click(Role::Button, "Import"),
        };
        assert_eq!(window.ui().take_requests(), [request], "{case}");
        assert!(window.ui().importing(), "{case}: the card stays until the shell has the document");

        // While the shell imports, the card cannot ask again or change its mind.
        let waiting = window.find(Role::Button, "Importing").expect("the button says it is importing");
        assert!(waiting.disabled, "{case}");
        let before = ticks(&mut window);
        window.click(Role::CheckBox, "Notes / Trip").key("Enter");
        assert_eq!((ticks(&mut window), window.ui().take_requests()), (before, vec![]), "{case}");

        window.ui().offer_import(None);
        window.assert_closed(Role::Dialog, CARD);
    }
}

#[test]
fn only_cancel_and_escape_put_the_import_card_away() {
    for way in ["a click on the page", "typing", "Cancel", "Escape"] {
        let mut window = blank();
        window.ui().set_landing(Some(Landing::default()));
        window.ui().offer_import(Some(offer()));
        match way {
            "a click on the page" => window.click_at(Point { x: 80.0, y: 700.0 }),
            "typing" => window.type_text("D"),
            "Cancel" => window.click(Role::Button, "Cancel"),
            _ => window.key("Escape"),
        };
        let kept = matches!(way, "a click on the page" | "typing");
        assert_eq!(window.ui().importing(), kept, "{way}");
        assert_eq!(window.ui().take_requests(), if kept { vec![] } else { vec![Request::CancelImport] }, "{way}");
        assert_eq!(window.markdown().trim(), "", "{way}: nothing is written behind the card");
        if !kept {
            window.assert_open(Role::Dialog, "Open a document");
        }
    }
}

/// Why the shell could not import, in its own words.
const WHY: &str = "Trip notes.md could not be read. It may have been moved.";

#[test]
fn an_import_that_failed_is_said_on_the_card_which_can_ask_again() {
    let mut window = blank();
    window.ui().offer_import(Some(offer()));
    window.key("Enter").ui().take_requests();
    window.ui().refuse_import(WHY);
    assert!(window.ui().importing(), "the card stays with the file");
    assert!(window.find(Role::Alert, WHY).is_some(), "and says why");

    // It waits no longer: what is ticked can be changed, and Import asks again.
    window.click(Role::CheckBox, "Notes / Shared").click(Role::Button, "Import");
    assert_eq!(window.ui().take_requests(), [import(&["/notes/trip", "/notes/shared"], true)]);
    assert_eq!(window.find(Role::Alert, WHY), None, "the reason goes while the shell tries again");

    // With no card there is nowhere to say it.
    let mut window = blank();
    window.ui().refuse_import(WHY);
    assert_eq!(window.find(Role::Alert, WHY), None);
}

/// A note on a block, of a kind, added on a Tuesday.
fn note(id: &str, kind: NoteKind, block: &str, says: &str) -> Note {
    Note {
        id: id.into(),
        kind,
        anchor: Anchor::block(block),
        author: "Letters import".into(),
        created: "2026-11-10T08:00:00Z".into(),
        resolved: false,
        content: vec![Run::plain(says)],
        replies: Vec::new(),
        extra: Default::default(),
    }
}

/// What an import left on a block.
fn left(id: &str, block: &str, says: &str) -> Note {
    note(id, NoteKind::Import, block, says)
}

/// A letter of three lines, `a`, `b` and `c`, with these notes on it. The shell has not said where it
/// came from.
fn letter(notes: Vec<Note>) -> Harness {
    let line = |id: &str, text: &str| Block::text(id, BlockKind::Paragraph, text);
    let blocks = vec![line("a", "Dear Ada,"), line("b", "The costs are in the table."), line("c", "Yours, Sam")];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.ui().set_notes(Notes { notes, ..Notes::default() });
    window
}

/// The toast about an import, when it shows.
fn toast(window: &mut Harness) -> Option<Found> {
    window.controls().into_iter().find(|found| found.role == Role::Status && found.name.starts_with("Imported"))
}

/// What the note says whose card is the open one, of those beside the page.
fn reading(window: &mut Harness) -> Option<String> {
    let controls = window.controls();
    let open = controls.iter().position(|found| found.role == Role::Group && found.selected == Some(true))?;
    controls[open..].iter().find(|found| found.role == Role::Paragraph).map(|words| words.name.clone())
}

#[test]
fn a_toast_counts_what_an_import_left_on_the_document() {
    let settled = Note { resolved: true, ..left("old", "a", "Text box flattened") };
    let asked = note("asked", NoteKind::Comment, "a", "Is this the last draft?");
    // The notes on the document, what the toast says, and the button that leads to the first.
    let cases: [(Vec<Note>, &str, Option<&str>); 3] = [
        (vec![], "Imported with no notes", None),
        (vec![left("split", "b", "Merged cells split")], "Imported with 1 note", Some("Show it")),
        (
            vec![left("moved", "c", "Footnote moved here"), asked, settled, left("split", "b", "Merged cells split")],
            "Imported with 2 notes",
            Some("Show the first"),
        ),
    ];
    for (notes, says, button) in cases {
        let mut window = letter(notes);
        assert_eq!(toast(&mut window), None, "{says}: nothing until the shell says it was imported");
        window.ui().say_imported();
        assert_eq!(toast(&mut window).map(|toast| toast.name).as_deref(), Some(says));
        for name in ["Show it", "Show the first"] {
            assert_eq!(window.find(Role::Button, name).is_some(), button == Some(name), "{says}: {name}");
        }
    }
}

#[test]
fn the_toast_leads_to_the_first_note_in_the_order_of_the_page() {
    // The note on the last line was added first.
    let notes = vec![left("moved", "c", "Footnote moved here"), left("split", "b", "Merged cells split")];
    let mut window = letter(notes);
    window.ui().say_imported();
    assert_eq!(reading(&mut window), None);
    window.click(Role::Button, "Show the first");
    assert_eq!(reading(&mut window).as_deref(), Some("Merged cells split"));
    assert_eq!(toast(&mut window), None, "the toast has done what it was for");

    // The keyboard comes to it straight from the page.
    let mut window = letter(vec![left("split", "b", "Merged cells split")]);
    window.ui().say_imported();
    window.key("Tab");
    assert_eq!(window.focused().map(|found| found.name).as_deref(), Some("Show it"));
    window.key("Enter");
    assert_eq!(reading(&mut window).as_deref(), Some("Merged cells split"));
}

#[test]
fn the_toast_goes_by_itself_and_leaves_the_page_to_the_pointer() {
    let mut window = letter(vec![left("split", "b", "Merged cells split")]);
    window.resize(1000.0, 700.0);
    window.ui().say_imported();
    window.advance(0).advance(5000);
    assert!(toast(&mut window).is_some(), "the toast stays long enough to be read and pressed");
    window.advance(5000);
    assert_eq!(toast(&mut window), None, "and then goes");

    // It is about the page, and waits while Print view has the window.
    window.ui().say_imported();
    window.key("Mod+P");
    assert_eq!(toast(&mut window), None);
    window.key("Escape");

    // It can be put away sooner.
    window.click(Role::Button, "Dismiss");
    assert_eq!(toast(&mut window), None);
    assert_eq!(reading(&mut window), None, "which opens no note");

    // Beside it the page takes a press as it does without it.
    window.ui().say_imported();
    window.ui().select(Selection::caret(Pos::new("c", 4)));
    window.paint();
    let caret = window.ui().caret_rect().expect("the page shows a caret");
    window.ui().select(Selection::caret(Pos::new("a", 0)));
    let level = toast(&mut window).expect("the toast shows again").bounds;
    assert!(level.y > caret.y + caret.height, "the toast lies at the foot of the page: {level:?}");
    window.click_at(Point { x: caret.x + 1.0, y: caret.y + caret.height / 2.0 });
    assert_eq!(window.editor().selection().map(|selection| selection.head.block.as_str()), Some("c"));
    assert!(toast(&mut window).is_some(), "and the toast stays");
}
