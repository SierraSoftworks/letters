//! The menu about what is under the pointer: a strip of small buttons over the actions of that thing.

use letters_editor::Clip;
use letters_model::{Align, Block, BlockKind, Document, ListKind, Pos, Selection};
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
        found.role == Role::MenuItem
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
        ("brown", &["Paste as plain text", "Select all", "All commands"], &["Copy code", "Add row above", "Indent"]),
        ("step", &["Edit callout", "Turn into body text"], &["Copy code", "Remove row"]),
        ("manual", &["Open link", "Edit link", "Copy link address", "Remove link"], &["Copy code"]),
        ("Second", &["Indent", "Outdent"], &["Open link", "Add row above"]),
        ("main", &["Copy code", "Copy as picture"], &["Open link", "Remove row"]),
        ("Pears", &["Add row above", "Add column to the right", "Remove row", "Turn header row off"], &["Copy code"]),
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
fn the_strip_turns_a_block_into_another_kind_and_lines_a_column_up() {
    let mut window = note();
    ask(&mut window, "Second");
    assert_eq!(window.find(Role::Button, "Bulleted list").and_then(|list| list.toggled), Some(true));
    window.click(Role::Button, "Numbered list");
    let second = window.editor().selection().expect("a caret").head.block.clone();
    assert!(matches!(kind_of(&window, second.as_str()), BlockKind::ListItem { list: ListKind::Ordered, .. }));

    let mut window = note();
    ask(&mut window, "12");
    assert!(window.find(Role::Button, "Heading 1").is_none(), "a cell is no heading");
    window.click(Role::Button, "Align column right");
    let aligned = window.editor().doc().walk().find_map(|block| match &block.kind {
        BlockKind::Table { columns, .. } => Some(columns.clone()),
        _ => None,
    });
    assert_eq!(aligned.as_deref(), Some(&[Align::Start, Align::End][..]));
    assert_eq!(window.find(Role::Button, "Align column right").and_then(|right| right.toggled), Some(true));
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
    choose(&mut window, "Remove row");
    assert!(!window.markdown().contains("Pears"), "{}", window.markdown());
    window.key("Mod+Z");
    assert!(window.markdown().contains("Pears"), "one undo brings the row back");

    ask(&mut window, "the picture");
    choose(&mut window, "Delete image");
    assert!(window.editor().doc().block(&"roof".into()).is_none(), "the picture is gone");
}

#[test]
fn the_keyboard_starts_on_the_list_and_a_paste_with_nothing_to_give_is_the_shells_to_do() {
    let mut window = note();
    ask(&mut window, "Pears");
    assert_eq!(window.focused().map(|found| found.name), Some("Add row above".to_owned()));
    window.key("Down").key("Enter").assert_closed(Role::Menu, MENU);
    assert_eq!(window.markdown().matches("|  |  |").count(), 1, "the second row was taken: {}", window.markdown());

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
