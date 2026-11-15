//! The menu about what is under the pointer: a strip of small buttons over the actions of that thing.

use letters_editor::Clip;
use letters_model::{Block, BlockKind, Document, Pos, Selection};
use letters_render::{Item, Point, Rect};
use letters_test::{Found, Harness, Role};
use letters_ui::Request;

const MENU: &str = "Context menu";
const ADDRESS: &str = "https://example.org/manual";

/// A note with one of each thing the menu knows: a line with a link, a list, code, a table, a callout, a
/// formula and a picture.
fn note() -> Harness {
    let markdown = format!(
        "The brown fox reads [the manual]({ADDRESS}) twice.\n\n- First item\n- Second item\n\n\
         ```rust\nfn main() {{}}\n```\n\n| Name | Count |\n| --- | --- |\n| Apples | 3 |\n| Pears | 12 |\n\n> [!TIP]\n> Mind the step.\n\nLast line."
    );
    let mut blocks = Harness::doc(&markdown).editor().doc().blocks.clone();
    let figure = BlockKind::Figure {
        asset: Some("assets/roof.png".into()),
        src: None,
        alt: "A slate roof".into(),
        width: 300.0,
        height: 150.0,
        numbered: false,
    };
    blocks.push(Block::text("sum", BlockKind::Formula, "a^2 + b^2"));
    blocks.push(Block::text("roof", figure, "Seen from the garden"));
    blocks.push(Block::text("blank", BlockKind::Paragraph, ""));
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.resize(900.0, 1400.0);
    window.paint();
    window
}

/// A point on the first place `text` stands on the page.
fn on(window: &mut Harness, text: &str) -> Point {
    window.select(text);
    let start = window.editor().selection().expect("a selection").anchor.clone();
    window.ui().select(Selection::caret(Pos::new(start.block, start.offset + 1)));
    window.paint();
    let caret = window.ui().caret_rect().expect("the page shows a caret");
    Point { x: caret.x, y: caret.y + caret.height / 2.0 }
}

/// A point on the picture the page shows.
fn on_picture(window: &mut Harness) -> Point {
    let drawn = |item: &Item| match item {
        Item::Image { rect, .. } => Some(*rect),
        Item::Fill { rect, .. } if (rect.width, rect.height) == (300.0, 150.0) => Some(*rect),
        _ => None,
    };
    let place: Rect = window.paint().items.iter().find_map(drawn).expect("a picture");
    Point { x: place.x + 150.0, y: place.y + 75.0 }
}

/// Asks for the menu on a place of the note: a word of its text, or its picture.
fn ask(window: &mut Harness, place: &str) {
    let at = match place {
        "the picture" => on_picture(window),
        "the formula" => {
            // It stands between the last line of text and the picture.
            let (over, under) = (on(window, "Last"), on_picture(window));
            Point { x: under.x, y: (over.y + under.y - 75.0) / 2.0 }
        }
        "the empty line" => {
            window.ui().select(Selection::caret(Pos::new("blank", 0)));
            window.paint();
            let caret = window.ui().caret_rect().expect("the page shows a caret");
            Point { x: caret.x + 40.0, y: caret.y + caret.height / 2.0 }
        }
        text => on(window, text),
    };
    // The caret starts somewhere else, as it would be.
    let first = window.editor().doc().blocks[0].id.clone();
    window.ui().select(Selection::caret(Pos::new(first, 0)));
    window.secondary_click_at(at);
}

/// The row of the menu for an action. A row is named with its key, where it has one.
fn row(window: &mut Harness, item: &str) -> Option<Found> {
    let named = |found: &Found| {
        matches!(found.role, Role::MenuItem | Role::MenuItemCheckBox)
            && found.name.strip_prefix(item).is_some_and(|key| key.is_empty() || key.starts_with(' '))
    };
    window.controls().into_iter().find(named)
}

fn listed(window: &mut Harness, item: &str) -> bool {
    row(window, item).is_some()
}

/// Presses a row of the menu.
#[track_caller]
fn choose(window: &mut Harness, item: &str) {
    let at = row(window, item).unwrap_or_else(|| panic!("the menu does not list {item:?}")).center();
    window.click_at(at);
}

fn kind_of<'a>(window: &'a Harness, block: &str) -> &'a BlockKind {
    &window.editor().doc().block(&block.into()).expect("the block").kind
}

#[test]
fn the_menu_lists_what_can_be_done_with_the_thing_it_is_asked_on() {
    // Where the menu is asked for, what it lists there, and what it leaves to other places.
    let cases: [(&str, &[&str], &[&str]); 9] = [
        (
            "brown",
            &["Turn into", "Paste as plain text", "Select all", "All commands"],
            &["Copy code", "Type", "Indent"],
        ),
        ("step", &["Edit callout", "Type", "Turn into body text"], &["Copy code", "Tip"]),
        ("manual", &["Open link", "Edit link", "Copy link address", "Remove link"], &["Copy code"]),
        ("Second", &["Indent", "Outdent"], &["Open link", "Add row above"]),
        ("main", &["Copy code", "Copy as picture"], &["Open link", "Remove row"]),
        ("Pears", &["Paste as plain text"], &["Copy code", "Turn into", "Remove row"]),
        ("the formula", &["Copy TeX source"], &["Copy code", "Remove row"]),
        ("the picture", &["Edit caption", "Edit alt text", "Delete image"], &["Open link", "Indent"]),
        ("the empty line", &["Table", "Image", "Divider"], &["Copy code", "Delete image"]),
    ];
    for (place, lists, leaves) in cases {
        let mut window = note();
        ask(&mut window, place);
        assert!(window.find(Role::Menu, MENU).is_some(), "on {place}: the menu opens");
        for item in lists {
            assert!(listed(&mut window, item), "on {place}: {item} is listed");
        }
        for item in leaves {
            assert!(!listed(&mut window, item), "on {place}: {item} is left out");
        }
        window.key("Escape").assert_closed(Role::Menu, MENU);
        assert!(window.focused().is_some_and(|found| found.role == Role::Document), "on {place}: back on the page");
    }
}

#[test]
fn the_strip_marks_text_and_stays_and_its_other_buttons_put_the_menu_away() {
    let mut window = note();
    let at = on(&mut window, "brown");
    window.select("brown").secondary_click_at(at);
    assert_eq!(window.find(Role::Button, "Bold").and_then(|bold| bold.toggled), Some(false));
    window.click(Role::Button, "Bold").click(Role::Button, "Italic").assert_open(Role::Menu, MENU);
    assert_eq!(window.find(Role::Button, "Bold").and_then(|bold| bold.toggled), Some(true), "the strip shows it");
    assert!(window.markdown().contains("***brown***"), "{}", window.markdown());

    window.click(Role::Button, "Copy").assert_closed(Role::Menu, MENU);
    assert_eq!(window.platform.clipboard.as_ref().map(|clip| clip.text.as_str()), Some("***brown***"));
}

#[test]
fn what_is_selected_has_its_history_in_the_menu() {
    let mut window = note();
    let at = on(&mut window, "brown");
    // A caret has no passage to show the history of.
    window.secondary_click_at(at);
    assert!(!listed(&mut window, "History of selection"));
    window.key("Escape").select("brown").secondary_click_at(at);
    choose(&mut window, "History of selection");
    assert!(window.find(Role::Complementary, "History of selection").is_some());
}

#[test]
fn the_strip_offers_only_what_has_something_to_act_on() {
    // Where the menu is asked for, a button of the strip, and whether it can be pressed there.
    let cases = [
        ("brown", "Cut", false),
        ("brown", "Paste", true),
        ("brown", "Bold", true),
        ("main", "Bold", false),
        ("the picture", "Bold", false),
        ("the picture", "Copy", true),
        ("brown", "Comment", false),
    ];
    for (place, button, offered) in cases {
        let mut window = note();
        ask(&mut window, place);
        let found = window.find(Role::Button, button).unwrap_or_else(|| panic!("on {place}: no {button}"));
        assert_eq!(!found.disabled, offered, "{button} on {place}");
    }
}

#[test]
fn a_choice_among_several_is_a_menu_of_its_own_beside_its_row() {
    // The row that opens the menu, how it is opened, what is chosen there, and the kind the block is then.
    let heading = |window: &Harness| matches!(kind_of(window, "b1"), BlockKind::Heading { level: 2 });
    for keys in [false, true] {
        let mut window = note();
        ask(&mut window, "brown");
        assert!(!listed(&mut window, "Heading 2"), "the kinds wait in their own menu");
        if keys {
            // The keyboard is on the list: the arrow that points at the menu opens it.
            while window.focused().is_some_and(|found| !found.name.starts_with("Turn into")) {
                window.key("Down");
            }
            window.key("Right");
            assert!(listed(&mut window, "Heading 2"), "the arrow opens it");
            window.key("Left");
            assert!(!listed(&mut window, "Heading 2"), "and the other arrow closes it");
            assert!(window.focused().is_some_and(|found| found.name.starts_with("Turn into")), "back on its row");
            window.key("Right");
        } else {
            let at = row(&mut window, "Turn into").expect("the row").center();
            window.hover_at(at);
        }
        let body = row(&mut window, "Body text").expect("the kind the block is");
        assert_eq!(body.toggled, Some(true), "the kind it is has the tick");
        choose(&mut window, "Heading 2");
        window.assert_closed(Role::Menu, MENU);
        assert!(heading(&window), "keys: {keys}: {}", window.markdown());
        assert!(window.focused().is_some_and(|found| found.role == Role::Document), "back on the page");
    }

    // A callout's types are such a menu too.
    let mut window = note();
    ask(&mut window, "step");
    let at = row(&mut window, "Type").expect("the row").center();
    window.hover_at(at);
    choose(&mut window, "Warning");
    assert!(window.markdown().contains("[!WARNING]"), "{}", window.markdown());
}

#[test]
fn the_strip_works_on_the_row_and_the_column_of_a_cell() {
    // The button, and the rows of the table as Markdown writes them afterwards.
    let cases = [
        ("Add row above", "| Apples | 3 |\n|  |  |\n| Pears | 12 |"),
        ("Add row below", "| Pears | 12 |\n|  |  |"),
        ("Move row up", "| Pears | 12 |\n| Apples | 3 |"),
        ("Remove row", "| Name | Count |\n| --- | --- |\n| Apples | 3 |\n\n"),
        ("Add column to the left", "| Apples |  | 3 |"),
        ("Add column to the right", "| Apples | 3 |  |"),
        ("Move column left", "| 12 | Pears |"),
        ("Remove column", "| Name |\n| --- |\n| Apples |\n| Pears |"),
        ("Align column right", "| --- | --: |"),
        ("Turn header row off", "|  |  |\n| --- | --- |\n| Name | Count |"),
    ];
    for (button, written) in cases {
        let mut window = note();
        ask(&mut window, "12");
        assert!(window.find(Role::Button, "Bold").is_some(), "the marks are there in a cell too");
        window.click(Role::Button, button).assert_open(Role::Menu, MENU);
        assert!(window.markdown().contains(written), "{button}: {}", window.markdown());
    }

    // What has nowhere to go cannot be pressed.
    let mut window = note();
    ask(&mut window, "12");
    for (button, offered) in [("Move row down", false), ("Move column right", false), ("Move row up", true)] {
        let found = window.find(Role::Button, button).unwrap_or_else(|| panic!("no {button}"));
        assert_eq!(!found.disabled, offered, "{button}");
    }
}

#[test]
fn misspelled_words_are_stepped_through_and_mended_by_keys() {
    let mut window = Harness::doc("The quik brown fox.\n\nA plain line.\n\nOne more lnie of it.");
    for _ in 0..200 {
        window.advance(50);
    }
    let word = |window: &Harness| {
        let caret = window.editor().selection().expect("a caret").head.clone();
        let text = window.editor().doc().block(&caret.block).expect("the block").plain_text();
        text[caret.offset..].split(' ').next().unwrap_or_default().to_owned()
    };
    // The key, and the misspelled word the menu is then about. The steps go on round the document's ends.
    for (key, at) in [("F7", "quik"), ("F7", "lnie"), ("F7", "quik"), ("Shift+F7", "lnie"), ("Shift+F7", "quik")] {
        window.key(key).assert_open(Role::Menu, MENU);
        assert_eq!(word(&window), at, "after {key}");
        assert!(listed(&mut window, "quick") == (at == "quik"), "the corrections are those of {at}");
    }
    // The number beside a correction takes it, alone or with the key the list names.
    let second = window.controls().into_iter().filter(|found| found.name.ends_with("+2")).count();
    assert_eq!(second, 1, "the second correction says how it is taken");
    assert!(row(&mut window, "quick").is_some_and(|found| found.name.ends_with("+2")), "quick is the second");
    window.key("Mod+2").assert_closed(Role::Menu, MENU);
    assert!(window.markdown().contains("The quick brown fox."), "{}", window.markdown());
    window.key("F7").assert_open(Role::Menu, MENU);
    assert_eq!(word(&window), "lnie");
    window.key("1").assert_closed(Role::Menu, MENU);
    assert!(!window.markdown().contains("lnie"), "{}", window.markdown());
    // With nothing left there is nothing to open.
    for _ in 0..200 {
        window.advance(50);
    }
    window.key("F7").assert_closed(Role::Menu, MENU);
}

#[test]
fn a_press_on_what_is_selected_keeps_it_and_one_elsewhere_moves_the_caret() {
    let mut window = note();
    let (inside, outside) = (on(&mut window, "rown"), on(&mut window, "twice"));
    window.select("brown").secondary_click_at(inside);
    let selection = window.editor().selection().expect("a selection");
    assert_eq!(selection.head.offset - selection.anchor.offset, 5, "the word stays selected");
    assert!(!window.find(Role::Button, "Cut").expect("the strip").disabled);

    window.key("Escape").select("brown").secondary_click_at(outside);
    assert!(window.editor().selection().is_some_and(Selection::is_caret), "the caret goes where the menu is");
    window.key("Escape");

    window.select("Last").menu_key().assert_open(Role::Menu, MENU);
}

#[test]
fn code_is_copied_as_it_is_written_or_as_a_picture_of_it() {
    let mut window = note();
    ask(&mut window, "main");
    choose(&mut window, "Copy code");
    window.assert_closed(Role::Menu, MENU);
    assert_eq!(window.platform.clipboard.as_ref().map(|clip| clip.text.as_str()), Some("fn main() {}"));

    ask(&mut window, "main");
    choose(&mut window, "Copy as picture");
    let clip = window.platform.clipboard.clone().expect("something was copied");
    assert_eq!(clip.text, "", "a picture alone, so that it is pasted as one");
    let [picture] = clip.pictures.as_slice() else { panic!("one picture, not {}", clip.pictures.len()) };
    assert_eq!(picture.mime, "image/png");
    assert!(picture.bytes.starts_with(&[0x89, b'P', b'N', b'G']), "the bytes are a PNG");
    assert!(picture.width > 400 && picture.height > 40, "{}x{} shows the block", picture.width, picture.height);
}

#[test]
fn the_items_about_a_link_a_table_and_a_picture_do_what_they_say() {
    let mut window = note();
    ask(&mut window, "manual");
    choose(&mut window, "Copy link address");
    assert_eq!(window.platform.clipboard.as_ref().map(|clip| clip.text.as_str()), Some(ADDRESS));
    ask(&mut window, "manual");
    choose(&mut window, "Edit link");
    window.assert_closed(Role::Menu, MENU).assert_open(Role::Dialog, "Link");
    assert!(window.focused().is_some_and(|found| found.name == "Link"), "the address is there to change");
    window.key("Escape");
    ask(&mut window, "manual");
    choose(&mut window, "Remove link");
    assert!(window.markdown().contains("reads the manual twice"), "{}", window.markdown());

    ask(&mut window, "Pears");
    window.click(Role::Button, "Remove row");
    assert!(!window.markdown().contains("Pears"), "{}", window.markdown());
    window.key("Escape").key("Mod+Z");
    assert!(window.markdown().contains("Pears"), "one undo brings the row back");

    ask(&mut window, "the picture");
    choose(&mut window, "Delete image");
    assert!(window.editor().doc().block(&"roof".into()).is_none(), "the picture is gone");
}

#[test]
fn the_keyboard_starts_on_the_list_and_a_paste_with_nothing_to_give_is_the_shells_to_do() {
    let mut window = note();
    ask(&mut window, "Second");
    assert!(window.focused().is_some_and(|found| found.name.starts_with("Indent")), "the first row has it");
    window.key("Enter").assert_closed(Role::Menu, MENU);
    assert!(window.markdown().contains("  - Second item"), "the row was taken: {}", window.markdown());

    // Whether the shell has the clipboard at once, the item, and what is asked of the shell.
    let cases = [
        (true, "Paste as plain text", vec![]),
        (false, "Paste as plain text", vec![Request::Paste { plain: true }]),
        (false, "Paste", vec![Request::Paste { plain: false }]),
    ];
    for (held, item, asked) in cases {
        let mut window = note();
        window.platform.clipboard = held.then(|| Clip { text: "pasted".into(), ..Clip::default() });
        ask(&mut window, "brown");
        // The strip has the one, and the list the other.
        match item {
            "Paste" => _ = window.click(Role::Button, item),
            item => choose(&mut window, item),
        }
        let mut requests = window.ui().take_requests();
        requests.retain(|request| matches!(request, Request::Paste { .. }));
        assert_eq!(requests, asked, "{item}, held: {held}");
        assert_eq!(window.markdown().contains("pasted"), held, "{item}, held: {held}");
    }
}
