//! The dialog that says what language a document is written in, and a passage of it.

use letters_editor::Lang;
use letters_test::{Harness, Role};
use letters_ui::Access;

const DOCUMENT: &str = "Language of the document";
const SAME: &str = "Same as the document";
/// What a command is in the palette's list.
const LISTED: Role = Role::ListBoxOption;

/// What a person does before the dialog opens.
type Act = fn(&mut Harness);

fn letter() -> Harness {
    let mut window = Harness::doc("# Roof repairs\n\nHarland Roofing sent a quote.\n\nBonjour tout le monde.\n");
    window.ui().set_languages(["fr-FR", "de-DE"]);
    window.select("Harland");
    window
}

/// Opens the dialog from the menu, over whatever is selected.
fn written(window: &mut Harness) -> &mut Harness {
    window.click(Role::Button, "More").click(Role::MenuItem, "Language").assert_open(Role::Dialog, "Language")
}

/// What a select of the dialog says is chosen.
fn chosen(window: &mut Harness, select: &str) -> Option<String> {
    window.find(Role::Button, select).and_then(|select| select.description)
}

/// The items an open list ticks.
fn ticked(window: &mut Harness) -> Vec<String> {
    let items = window.controls().into_iter().filter(|found| found.role == Role::MenuItemCheckBox);
    let ticked = items.filter(|found| found.toggled == Some(true));
    ticked.map(|found| found.name).collect()
}

/// Whether the palette, asked for the dialog by name, lists it.
fn in_palette(window: &mut Harness) -> bool {
    window.key("Mod+Shift+P").type_text("Language");
    let listed = window.controls().into_iter().any(|found| (found.role, found.name.as_str()) == (LISTED, "Language"));
    window.key("Escape");
    listed
}

#[test]
fn the_dialog_opens_from_the_menu_or_the_palette_and_changes_nothing_by_being_looked_at() {
    for route in ["the menu", "the palette"] {
        let mut window = letter();
        let before = (window.markdown(), window.editor().doc().language.clone());
        match route {
            "the menu" => window.click(Role::Button, "More").click(Role::MenuItem, "Language"),
            _ => window.key("Mod+Shift+P").type_text("Language").key("Enter"),
        };
        window.assert_open(Role::Dialog, "Language");
        // The dialog has the keyboard: what is typed does not land on the selected word behind it.
        window.type_text("x");
        window.key("Escape").assert_closed(Role::Dialog, "Language");
        assert_eq!((window.markdown(), window.editor().doc().language.clone()), before, "{route}");
        assert!(window.focused().is_some_and(|found| found.role == Role::Document), "{route}: back on the page");
    }
}

#[test]
fn only_one_who_writes_the_document_is_offered_its_language() {
    for access in [Access::Edits, Access::Comments, Access::Reads] {
        let mut window = letter();
        window.ui().set_access(access);
        window.click(Role::Button, "More");
        assert_eq!(window.find(Role::MenuItem, "Language").is_some(), access == Access::Edits, "{access:?}: the menu");
        window.key("Escape");
        assert_eq!(in_palette(&mut window), access == Access::Edits, "{access:?}: the palette");
    }
}

#[test]
fn the_dialog_lies_over_the_middle_of_the_window_and_shields_it() {
    let mut window = letter();
    let before = window.markdown();
    let middle = written(&mut window).find(Role::Dialog, "Language").expect("it is open").center();
    assert!((middle.x - 640.0).abs() <= 1.0 && (middle.y - 400.0).abs() <= 1.0, "{middle:?}");

    window.click(Role::Button, "Bold").assert_closed(Role::Dialog, "Language");
    window.assert_markdown(&before);
}

#[test]
fn the_dialog_and_the_settings_take_each_others_place() {
    let mut window = letter();
    window.key("Mod+,").assert_open(Role::Dialog, "Settings");
    window.key("Mod+Shift+P").type_text("Language").key("Enter");
    window.assert_open(Role::Dialog, "Language").assert_closed(Role::Dialog, "Settings");
    window.key("Mod+,").assert_open(Role::Dialog, "Settings").assert_closed(Role::Dialog, "Language");
}

#[test]
fn the_document_is_written_in_one_of_the_languages_there_are_dictionaries_for() {
    let mut window = letter();
    let at_first = window.editor().doc().language.clone();
    written(&mut window);
    window.click(Role::Button, DOCUMENT);
    // The kinds of English that are built in, then by name the document's own and the shell's.
    let items = window.controls().into_iter().filter(|found| found.role == Role::MenuItemCheckBox);
    let offered: Vec<String> = items.map(|found| found.name).collect();
    let (english, others) = offered.split_at(4);
    assert!(english.iter().all(|name| name.starts_with("English (")), "{offered:?}");
    assert_eq!(others, ["English", "French (France)", "German (Germany)"], "{offered:?}");
    assert_eq!(ticked(&mut window), ["English"], "the list ticks what the document is written in");
    assert_eq!(chosen(&mut window, DOCUMENT).as_deref(), Some("English"));

    window.click(Role::MenuItemCheckBox, "German (Germany)");
    assert_eq!(window.editor().doc().language, "de-DE");
    assert_eq!(chosen(&mut window, DOCUMENT).as_deref(), Some("German (Germany)"));
    assert_eq!(window.editor().lang(), Lang::Document, "no passage was given a language by it");

    // It is an edit like any other, which undo takes back.
    window.key("Escape").key("Mod+z");
    assert_eq!(window.editor().doc().language, at_first);
}

#[test]
fn a_passage_takes_a_language_of_its_own_and_gives_it_back() {
    // What is selected, and what the dialog calls it.
    let cases: [(Act, &str); 2] = [
        (|window| _ = window.select("Bonjour").key("Left"), "Language of this paragraph"),
        (|window| _ = window.select("Bonjour tout le monde."), "Language of selected text"),
    ];
    for (select, passage) in cases {
        let mut window = letter();
        select(&mut window);
        written(&mut window);
        assert_eq!(chosen(&mut window, passage).as_deref(), Some(SAME), "{passage}");

        window.click(Role::Button, passage);
        assert_eq!(ticked(&mut window), [SAME], "{passage}");
        window.click(Role::MenuItemCheckBox, "French (France)");
        assert_eq!(window.editor().lang(), Lang::Tag("fr-FR".into()), "{passage}");
        assert_eq!(chosen(&mut window, passage).as_deref(), Some("French (France)"), "{passage}");
        assert_ne!(window.editor().doc().language, "fr-FR", "{passage}: the document keeps its own");

        window.click(Role::Button, passage);
        assert_eq!(ticked(&mut window), ["French (France)"], "{passage}");
        window.click(Role::MenuItemCheckBox, SAME);
        assert_eq!(window.editor().lang(), Lang::Document, "{passage}");
        assert_eq!(chosen(&mut window, passage).as_deref(), Some(SAME), "{passage}");
    }
}

#[test]
fn a_selection_in_more_than_one_language_says_so_and_can_be_given_one() {
    const PASSAGE: &str = "Language of selected text";
    let mut window = letter();
    window.select("Bonjour tout le monde.");
    written(&mut window).click(Role::Button, PASSAGE).click(Role::MenuItemCheckBox, "French (France)");
    window.key("Escape").select("Harland Roofing sent a quote.").key("Shift+Down");
    written(&mut window);
    assert_eq!(chosen(&mut window, PASSAGE).as_deref(), Some("More than one"));

    window.click(Role::Button, PASSAGE).click(Role::MenuItemCheckBox, "German (Germany)");
    assert_eq!(window.editor().lang(), Lang::Tag("de-DE".into()));
}

#[test]
fn where_nothing_can_have_a_language_the_dialog_says_so() {
    let mut window = Harness::doc("Before\n\n```rust\nlet a = 1;\n```\n");
    window.select("let");
    written(&mut window);
    assert!(window.find(Role::Label, "Code, formulas and pictures have no language.").is_some());
    let passages = window.controls().into_iter().filter(|found| found.name.starts_with("Language of "));
    assert_eq!(passages.map(|found| found.name).collect::<Vec<_>>(), [DOCUMENT], "the document's is still chosen");
}
