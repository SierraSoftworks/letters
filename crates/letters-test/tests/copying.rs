//! The card that saves a copy of a document, or writes it in another format.

use letters_model::{Anchor, Block, BlockKind, Document, Note, NoteKind, Notes, Run};
use letters_render::image::Image;
use letters_test::{Harness, Role};
use letters_ui::{CopyFormat, Folder, Landing, Location, Request};

const HOUSE: &str = "/Family/House";
const QUOTES: &str = "/Family/House/Quotes";
/// What the card calls the list of what a format will simplify.
const LOSSES: &str = "Simplified in this copy";

/// Where the cottage's plan is kept: in House, in the space called Family.
fn family() -> Location {
    let folder = |path: &str| Folder {
        path: path.into(),
        name: path.rsplit('/').next().unwrap_or_default().into(),
        depth: path.matches('/').count() as u8 - 1,
    };
    let folders = ["/Family", HOUSE, QUOTES, "/Family/School"];
    Location {
        name: Some("Cottage renovation".into()),
        folder: HOUSE.into(),
        folders: folders.map(folder).into(),
        ..Location::default()
    }
}

/// Where a server keeps the plan: folders of the server's, and nowhere else for the plan to go.
fn served() -> Location {
    let folder = |path: &str, name: &str, depth| Folder { path: path.into(), name: name.into(), depth };
    Location {
        name: Some("Cottage renovation".into()),
        folder: "shared/plans".into(),
        folders: vec![folder("shared", "Shared", 0), folder("shared/plans", "Plans", 1)],
        closed: true,
    }
}

/// A short letter, kept where `location` says.
fn letter(location: Location) -> Harness {
    let blocks = vec![
        Block::text("h1", BlockKind::Heading { level: 1 }, "Roof repairs"),
        Block::text("p1", BlockKind::Paragraph, "Harland Roofing sent a quote."),
    ];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.ui().set_location(location);
    window
}

/// The letter with a code block in Rust and a comment on its first paragraph, kept in House.
fn annotated() -> Harness {
    let blocks = vec![
        Block::text("h1", BlockKind::Heading { level: 1 }, "Roof repairs"),
        Block::text("p1", BlockKind::Paragraph, "Harland Roofing sent a quote."),
        Block::text("c1", BlockKind::Code { language: Some("rust".into()) }, "let quote = 4_200;"),
    ];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.ui().set_location(family());
    let note = Note {
        id: "n1".into(),
        kind: NoteKind::Comment,
        anchor: Anchor::range("p1", 0..7),
        author: "Alice Moreau".into(),
        created: "2026-11-10T08:00:00Z".into(),
        resolved: false,
        content: vec![Run::plain("Is this the final quote?")],
        replies: Vec::new(),
        extra: Default::default(),
    };
    window.ui().set_notes(Notes { notes: vec![note], ..Notes::default() });
    window
}

/// The lines of what the card says the picked format will simplify.
fn simplified(window: &mut Harness) -> Vec<String> {
    let Some(list) = window.find(Role::List, LOSSES) else { return Vec::new() };
    let (top, bottom) = (list.bounds.y, list.bounds.y + list.bounds.height);
    let within = |found: &letters_test::Found| {
        found.role == Role::ListItem && found.bounds.y >= top && found.bounds.y + found.bounds.height <= bottom
    };
    window.controls().into_iter().filter(within).map(|found| found.name).collect()
}

/// Runs a command from the palette.
fn run(window: &mut Harness, command: &str) {
    window.key("Mod+Shift+P").type_text(command).key("Enter");
}

/// The letter with the card open, as the key opens it.
fn copying() -> Harness {
    let mut window = letter(family());
    window.key("Mod+Shift+S").assert_open(Role::Dialog, "Save a copy");
    window
}

/// What the name field holds.
fn name(window: &mut Harness) -> String {
    window.find(Role::TextInput, "Name of the copy").expect("the name field").value.unwrap_or_default()
}

/// The format that is picked.
fn format(window: &mut Harness) -> Vec<String> {
    let picked = |found: &letters_test::Found| found.role == Role::Button && found.toggled == Some(true);
    let formats = CopyFormat::ALL.map(CopyFormat::name);
    let on = window.controls().into_iter().filter(picked).map(|found| found.name);
    on.filter(|name| formats.contains(&name.as_str())).collect()
}

/// Chooses a command from the More menu.
fn menu(window: &mut Harness, command: &str) {
    window.click(Role::Button, "More").click(Role::MenuItem, command);
}

fn copy(name: &str, format: CopyFormat, folder: Option<&str>, replace: bool) -> Request {
    Request::SaveCopy { name: name.into(), format, folder: folder.map(str::to_owned), replace }
}

#[test]
fn the_card_opens_from_the_key_and_the_menu_set_to_a_format() {
    type Open = fn(&mut Harness);
    let cases: [(&str, Open, &str, &str); 4] = [
        ("the key", |window| _ = window.key("Mod+Shift+S"), "Letters", "Cottage renovation copy"),
        // A clean copy is a document too, so it cannot share the name either.
        ("the palette", |window| run(window, "Export clean copy"), "Clean copy", "Cottage renovation clean"),
        // The menu names the key beside the command.
        ("the menu", |window| menu(window, "Save a copy Ctrl+Shift+S"), "Letters", "Cottage renovation copy"),
        ("Export", |window| menu(window, "Export"), "PDF", "Cottage renovation"),
    ];
    for (case, open, picked, named) in cases {
        let mut window = letter(family());
        open(&mut window);
        window.assert_open(Role::Dialog, "Save a copy");
        assert_eq!((format(&mut window), name(&mut window)), (vec![picked.to_owned()], named.to_owned()), "{case}");
        let focused = window.focused().expect("something has focus");
        assert_eq!((focused.role, focused.name.as_str()), (Role::TextInput, "Name of the copy"), "{case}");
        let here = window.find(Role::TreeItem, "House").expect("the folder the document is in");
        assert_eq!((here.selected, here.description.as_deref()), (Some(true), Some("It is here")), "{case}");
    }
}

#[test]
fn the_card_stays_away_while_there_is_nothing_to_copy() {
    let mut window = letter(family());
    window.ui().set_landing(Some(Landing::default()));
    window.key("Mod+Shift+S").assert_closed(Role::Dialog, "Save a copy");
}

#[test]
fn the_name_follows_the_format_until_one_is_typed() {
    let mut window = copying();
    window.click(Role::Button, "Word");
    assert_eq!(name(&mut window), "Cottage renovation", "a file in another format can share the name");
    window.click(Role::Button, "Letters");
    assert_eq!(name(&mut window), "Cottage renovation copy", "another document cannot");

    window.key("Mod+A").type_text("Plan B").click(Role::Button, "Markdown");
    assert_eq!((name(&mut window), format(&mut window)), ("Plan B".to_owned(), vec!["Markdown".to_owned()]));
}

#[test]
fn a_name_the_card_suggests_is_typed_over_whole() {
    // The name a format suggests may be longer or shorter than the one before it.
    for format in ["Clean copy", "Word"] {
        let mut window = copying();
        window.click(Role::Button, format).type_text("Plan B");
        assert_eq!(name(&mut window), "Plan B", "{format}");
    }
}

#[test]
fn the_shell_is_asked_for_the_copy_and_the_card_goes() {
    type Ask = fn(&mut Harness);
    let cases: [(&str, Ask, Request); 5] = [
        (
            "a copy without the history",
            |window| _ = window.click(Role::Button, "Clean copy").click(Role::Button, "Export to House"),
            copy("Cottage renovation clean", CopyFormat::Clean, Some(HOUSE), false),
        ),
        (
            "Enter in the name",
            |window| _ = window.key("Enter"),
            copy("Cottage renovation copy", CopyFormat::Letters, Some(HOUSE), false),
        ),
        (
            "a format and a folder",
            |window| {
                window
                    .click(Role::Button, "PDF")
                    .click(Role::TreeItem, "Quotes")
                    .click(Role::Button, "Export to Quotes");
            },
            copy("Cottage renovation", CopyFormat::Pdf, Some(QUOTES), false),
        ),
        (
            "a name of the person's own",
            |window| _ = window.key("Mod+A").type_text("  Plan B ").click(Role::Button, "Save copy in House"),
            copy("Plan B", CopyFormat::Letters, Some(HOUSE), false),
        ),
        (
            "somewhere else",
            |window| _ = window.click(Role::Button, "Web page").click(Role::Button, "Other location\u{2026}"),
            copy("Cottage renovation", CopyFormat::WebPage, None, false),
        ),
    ];
    for (case, ask, request) in cases {
        let mut window = copying();
        ask(&mut window);
        assert_eq!(window.ui().take_requests(), [request], "{case}");
        window.assert_closed(Role::Dialog, "Save a copy");
    }
}

#[test]
fn the_card_lists_what_the_picked_format_will_simplify() {
    // A format, what its list names of a letter with a comment and a code block in Rust, and what it
    // must not: Word carries comments, and the others the code's language.
    let cases: [(&str, &[&str], &[&str]); 5] = [
        ("Clean copy", &["history", "comment"], &["code block"]),
        ("Word", &["code block"], &["comment", "history"]),
        ("PDF", &["comment"], &["code block", "history"]),
        ("Web page", &["comment"], &["code block", "history"]),
        ("Markdown", &["comment"], &["code block", "history"]),
    ];
    let mut window = annotated();
    window.key("Mod+Shift+S");
    assert!(window.find(Role::List, LOSSES).is_none(), "a copy with the history holds all the document does");
    for (format, lost, kept) in cases {
        let listed = simplified(window.click(Role::Button, format)).join("\n").to_lowercase();
        for word in lost {
            assert!(listed.contains(word), "{format}: {word} in {listed:?}");
        }
        for word in kept {
            assert!(!listed.contains(word), "{format}: no {word} in {listed:?}");
        }
    }
    window.click(Role::Button, "Letters");
    assert!(window.find(Role::List, LOSSES).is_none(), "the list goes with the format it was of");

    // A card that opens on a format says at once what that format leaves out.
    window.key("Escape");
    run(&mut window, "Export clean copy");
    let listed = simplified(&mut window).join("\n").to_lowercase();
    assert!(listed.contains("history") && listed.contains("comment"), "{listed:?}");
}

#[test]
fn a_linked_picture_that_shows_is_no_loss_in_a_pdf() {
    const LINK: &str = "https://example.com/roof.png";
    let figure = BlockKind::Figure {
        asset: None,
        src: Some(LINK.into()),
        alt: "A slate roof".into(),
        width: 300.0,
        height: 150.0,
        numbered: false,
    };
    let blocks = vec![Block::text("p1", BlockKind::Paragraph, "The roof today:"), Block::new("g", figure)];
    // Whether the window shows the picture, and whether a PDF and a web page then lose it: a shell hands
    // a PDF the pictures that show, and a web page is written with the link either way.
    for (shown, pdf, page) in [(false, true, true), (true, false, true)] {
        let mut window = Harness::open(Document { blocks: blocks.clone(), ..Document::default() });
        window.ui().set_location(family());
        if shown {
            let image = Image::from_rgba8(2, 1, vec![255; 8]).expect("two pixels");
            window.ui().set_image(LINK, Some(image));
        }
        window.key("Mod+Shift+S");
        for (format, lost) in [("PDF", pdf), ("Web page", page)] {
            let listed = simplified(window.click(Role::Button, format)).join("\n");
            assert_eq!(listed.contains("linked picture"), lost, "{format}, shown: {shown}: {listed:?}");
        }
    }
}

#[test]
fn a_format_that_carries_everything_lists_nothing() {
    let mut window = copying();
    for format in CopyFormat::ALL.map(CopyFormat::name) {
        let listed = simplified(window.click(Role::Button, format));
        // A plain letter loses nothing but its history, and that only in a clean copy.
        assert_eq!(listed.len(), usize::from(format == "Clean copy"), "{format}: {listed:?}");
    }
}

#[test]
fn the_card_is_put_away_with_nothing_asked() {
    type Leave = fn(&mut Harness);
    let cases: [(&str, Leave); 3] = [
        ("Escape", |window| _ = window.key("Escape")),
        ("a press on the page", |window| {
            let page = window.find(Role::Document, "Cottage renovation").expect("the page").bounds;
            window.click_at(letters_render::Point { x: page.x + 20.0, y: page.y + page.height - 20.0 });
        }),
        ("no name", |window| _ = window.key("Mod+A").key("Backspace").key("Enter").key("Escape")),
    ];
    for (case, leave) in cases {
        let mut window = copying();
        leave(&mut window);
        window.assert_closed(Role::Dialog, "Save a copy");
        assert_eq!(window.ui().take_requests(), [], "{case}");
        window.type_text("!");
        assert!(window.markdown().contains('!'), "{case}: writing goes on in the document");
    }
}

#[test]
fn a_copy_that_was_written_is_told_in_the_bar() {
    let cases = [("Letters", "House", "Copy saved in House"), ("PDF", "Downloads", "Exported to Downloads")];
    for (format, folder, told) in cases {
        let mut window = copying();
        window.click(Role::Button, format).key("Enter");
        window.ui().take_requests();
        window.ui().set_copied(folder);
        window.assert_open(Role::Label, told).assert_closed(Role::Dialog, "Save a copy");
        // The note goes after a few seconds, and the bar says how saving stands again.
        window.advance(0).advance(10_000).assert_open(Role::Label, "Saved");
    }
}

#[test]
fn a_refused_copy_comes_back_as_it_was_with_the_reason() {
    let mut window = copying();
    window.key("Mod+A").type_text("Plan B").click(Role::TreeItem, "Quotes").key("Enter");
    window.ui().take_requests();
    window.ui().refuse_copy("There is already a document called \u{201c}Plan B\u{201d} in that folder.", false);

    window.assert_open(Role::Dialog, "Save a copy");
    window.assert_open(Role::Alert, "There is already a document called \u{201c}Plan B\u{201d} in that folder.");
    assert_eq!(name(&mut window), "Plan B");
    window.assert_open(Role::Button, "Save copy in Quotes");

    // The reason stands until something about the copy changes.
    window.type_text("!");
    assert!(window.controls().iter().all(|found| found.role != Role::Alert));
}

#[test]
fn a_file_in_the_way_is_replaced_once_the_person_says_so() {
    let mut window = copying();
    window.click(Role::Button, "PDF").key("Enter");
    assert_eq!(window.ui().take_requests(), [copy("Cottage renovation", CopyFormat::Pdf, Some(HOUSE), false)]);
    window.ui().refuse_copy("\u{201c}Cottage renovation.pdf\u{201d} is already in that folder.", true);
    window.click(Role::Button, "Replace it");
    assert_eq!(window.ui().take_requests(), [copy("Cottage renovation", CopyFormat::Pdf, Some(HOUSE), true)]);

    // Another folder, name or format is another file, which nobody said to replace.
    type Change = fn(&mut Harness);
    let changes: [(&str, Change, &str); 3] = [
        ("a folder", |window| _ = window.click(Role::TreeItem, "Quotes"), "Export to Quotes"),
        ("a name", |window| _ = window.type_text("2"), "Export to House"),
        ("a format", |window| _ = window.click(Role::Button, "Word"), "Export to House"),
    ];
    for (case, change, button) in changes {
        let mut window = copying();
        window.click(Role::Button, "PDF").key("Enter");
        window.ui().refuse_copy("It is already there.", true);
        change(&mut window);
        window.assert_closed(Role::Button, "Replace it").click(Role::Button, button);
        let asked = window.ui().take_requests();
        assert!(matches!(asked.last(), Some(Request::SaveCopy { replace: false, .. })), "{case}: {asked:?}");
    }
}

#[test]
fn a_new_folder_is_made_from_the_card_and_picked() {
    let mut window = copying();
    window.click(Role::TreeItem, "Quotes").click(Role::Button, "New folder");
    window.type_text("Drafts").key("Enter");
    assert_eq!(window.ui().take_requests(), [Request::NewFolder { parent: QUOTES.into(), name: "Drafts".into() }]);

    let mut location = family();
    location.folders.insert(3, Folder { path: "/Family/House/Quotes/Drafts".into(), name: "Drafts".into(), depth: 3 });
    window.ui().set_location(location);
    window.assert_open(Role::Dialog, "Save a copy").click(Role::Button, "Save copy in Drafts");
    let asked = copy("Cottage renovation copy", CopyFormat::Letters, Some("/Family/House/Quotes/Drafts"), false);
    assert_eq!(window.ui().take_requests(), [asked]);
}

#[test]
fn a_copy_of_a_document_kept_elsewhere_goes_among_the_folders_the_shell_names_for_copies() {
    let mut window = letter(served());
    window.ui().set_copy_location(Some(family()));
    window.key("Mod+Shift+S").assert_open(Role::TreeItem, "Quotes").assert_closed(Role::TreeItem, "Plans");
    window.click(Role::TreeItem, "Quotes").click(Role::Button, "New folder");
    window.type_text("Drafts").key("Enter");
    assert_eq!(window.ui().take_requests(), [Request::NewFolder { parent: QUOTES.into(), name: "Drafts".into() }]);

    // What the shell says of the document's own place meanwhile is not the card's.
    window.ui().set_location(served());
    let mut copies = family();
    copies.folders.insert(3, Folder { path: "/Family/House/Quotes/Drafts".into(), name: "Drafts".into(), depth: 3 });
    window.ui().set_copy_location(Some(copies));
    window.click(Role::Button, "Save copy in Drafts");
    let asked = copy("Cottage renovation copy", CopyFormat::Letters, Some("/Family/House/Quotes/Drafts"), false);
    assert_eq!(window.ui().take_requests(), [asked]);

    // With no place of their own, copies go where the document is.
    window.ui().set_copy_location(None);
    window.key("Mod+Shift+S").assert_open(Role::TreeItem, "Plans").assert_closed(Role::TreeItem, "Quotes");
}

#[test]
fn a_place_with_nowhere_else_to_go_offers_no_other_location() {
    let other = "Other location\u{2026}";
    for closed in [false, true] {
        let mut window = letter(Location { closed, ..family() });
        window.key("Mod+Shift+S").assert_open(Role::Tree, "Folder");
        assert_eq!(window.find(Role::Button, other).is_some(), !closed, "closed: {closed}: on the card");
        window.key("Escape").click(Role::Button, "Cottage renovation").assert_open(Role::Tree, "Move to");
        assert_eq!(window.find(Role::Button, other).is_some(), !closed, "closed: {closed}: under the name");
    }
}

#[test]
fn a_shell_that_lists_no_folders_is_left_to_say_where_the_copy_goes() {
    let mut window = letter(Location { name: Some("Cottage renovation".into()), ..Location::default() });
    window.key("Mod+Shift+S").assert_closed(Role::Tree, "Folder").click(Role::Button, "Markdown");
    window.click(Role::Button, "Export");
    assert_eq!(window.ui().take_requests(), [copy("Cottage renovation", CopyFormat::Markdown, None, false)]);
}
