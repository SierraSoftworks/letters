//! The import card: what the window shows for a file that has to be imported, and what it asks the shell.

use letters_model::{Block, BlockKind, Document};
use letters_render::Point;
use letters_test::{Harness, Role};
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
