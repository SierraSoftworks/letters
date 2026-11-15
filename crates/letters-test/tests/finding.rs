//! Find and replace: the bar under the top bar, from the keyboard, the palette and the menu on the page.

use letters_model::{Pos, Selection};
use letters_render::{Item, Point};
use letters_test::{Found, Harness, Role};

const BAR: &str = "Find and replace";

/// Three lines that hold "roof" four times, once with a capital and once inside a longer word.
fn page() -> Harness {
    let mut window = Harness::doc("Roof first, then the roof again.\n\nThe rooftop leaks.\n\nA roof (new).");
    window.resize(900.0, 700.0);
    window.paint();
    window
}

/// What the selection holds.
fn selected(window: &Harness) -> String {
    let selection = window.editor().selection().expect("a selection");
    let text = window.editor().doc().block(&selection.head.block).expect("its block").plain_text();
    let (from, to) =
        (selection.anchor.offset.min(selection.head.offset), selection.anchor.offset.max(selection.head.offset));
    text[from..to].to_owned()
}

/// What the bar says of what it found, or of the pattern it cannot read.
fn said(window: &mut Harness) -> Vec<String> {
    let bar = window.find(Role::Dialog, BAR).expect("the bar").bounds;
    let inside =
        |found: &Found| found.role == Role::Status && found.bounds.y >= bar.y && found.bounds.y <= bar.y + bar.height;
    window
        .controls()
        .into_iter()
        .filter(inside)
        .filter_map(|found| found.value)
        .filter(|said| !said.is_empty())
        .collect()
}

/// A point on the first place `text` stands on the page, asked for while the keyboard is the page's.
fn on(window: &mut Harness, text: &str) -> Point {
    let held = window.editor().selection().cloned();
    window.select(text);
    let start = window.editor().selection().expect("a selection").anchor.clone();
    window.ui().select(Selection::caret(Pos::new(start.block, start.offset + 1)));
    window.paint();
    let caret = window.ui().caret_rect().expect("the page shows a caret");
    if let Some(held) = held {
        window.ui().select(held);
    }
    Point { x: caret.x, y: caret.y + caret.height / 2.0 }
}

/// How many rectangles the window fills.
fn fills(window: &mut Harness) -> usize {
    window.paint().items.iter().filter(|item| matches!(item, Item::Fill { .. })).count()
}

#[test]
fn the_bar_opens_on_its_keys_with_the_keyboard_in_its_field_and_escape_puts_it_away() {
    // The key, and whether the line that replaces shows.
    for (key, replaces) in [("Mod+F", false), ("Mod+H", true), ("Mod+Alt+F", true)] {
        let mut window = page();
        window.assert_closed(Role::Dialog, BAR).key(key).assert_open(Role::Dialog, BAR);
        let field = window.focused().expect("something focused");
        assert_eq!((field.role, field.name.as_str()), (Role::TextInput, "Find"), "{key}");
        assert_eq!(window.find(Role::TextInput, "Replace with").is_some(), replaces, "{key}");
        let bar = window.find(Role::Dialog, BAR).expect("the bar").bounds;
        assert!(bar.y > 0.0 && bar.x > 300.0 && bar.x + bar.width > 800.0, "it hangs at the end, under the bar");

        window.key("Escape").assert_closed(Role::Dialog, BAR);
        window.type_text("X");
        assert!(window.markdown().starts_with("XRoof"), "the keyboard is the page's again");
    }
}

#[test]
fn what_is_typed_is_found_counted_and_stepped_through() {
    let mut window = page();
    window.key("Mod+F").type_text("roof");
    assert_eq!(said(&mut window), ["1 of 4"]);
    assert_eq!(selected(&window), "Roof", "the first place is selected");
    // The key, and the place it goes to.
    let steps = [("Enter", 2), ("Enter", 3), ("Mod+G", 4), ("Enter", 1), ("Shift+Enter", 4), ("Mod+Shift+G", 3)];
    for (key, place) in steps {
        window.key(key);
        assert_eq!(said(&mut window), [format!("{place} of 4")], "after {key}");
    }
    window.click(Role::Button, "Next").click(Role::Button, "Previous");
    assert_eq!(said(&mut window), ["3 of 4"]);
    assert_eq!(window.focused().map(|field| field.name), Some("Find".into()), "the keyboard stays in the field");

    window.type_text("x");
    assert_eq!(said(&mut window), ["No results"]);
    assert!(window.find(Role::Button, "Next").expect("the button").disabled);
}

#[test]
fn the_ways_of_looking_narrow_or_widen_what_is_found() {
    // The buttons pressed, what is typed, and what the bar says.
    let cases = [
        (vec![], "roof", "1 of 4"),
        (vec!["Match case"], "roof", "1 of 3"),
        (vec!["Whole words"], "roof", "1 of 3"),
        (vec!["Match case", "Whole words"], "roof", "1 of 2"),
        (vec![], "r.of", "No results"),
        (vec!["Regular expression"], "r.of", "1 of 4"),
        (vec!["Regular expression"], r"\(\w+\)", "1 of 1"),
        (vec!["Regular expression", "Match case"], "^[A-Z]", "1 of 3"),
    ];
    for (ways, typed, says) in cases {
        let mut window = page();
        window.key("Mod+F");
        for way in &ways {
            window.click(Role::Button, way);
            assert_eq!(window.find(Role::Button, way).and_then(|way| way.toggled), Some(true), "{way} is on");
        }
        window.type_text(typed);
        assert_eq!(said(&mut window), [says], "{typed:?} with {ways:?}");
    }
}

#[test]
fn a_pattern_that_does_not_read_is_said_to_be_none() {
    let mut window = page();
    window.key("Mod+F").click(Role::Button, "Regular expression").type_text("(roof");
    let said = said(&mut window);
    assert!(matches!(&said[..], [why] if why.starts_with("Not a pattern")), "{said:?}");
    window.type_text(")");
    assert_eq!(self::said(&mut window), ["1 of 4"], "and found again once it reads");
}

#[test]
fn what_is_selected_is_what_the_bar_looks_for() {
    let mut window = page();
    window.select("leaks").key("Mod+F");
    assert_eq!(window.find(Role::TextInput, "Find").and_then(|field| field.value), Some("leaks".into()));
    assert_eq!(said(&mut window), ["1 of 1"]);
    assert!(window.find(Role::Button, "Comment").is_none(), "no tools stand over a place that was found");
}

#[test]
fn the_page_is_written_on_under_the_bar_and_what_is_found_follows() {
    let mut window = page();
    let at = on(&mut window, "leaks");
    window.key("Mod+F").type_text("roofx");
    let none = fills(&mut window);
    window.key("Backspace");
    assert_eq!(fills(&mut window), none + 4, "three places are marked and the fourth is selected");
    window.click_at(at).assert_open(Role::Dialog, BAR);
    window.type_text("roof ");
    assert_eq!(said(&mut window), ["5 found"]);
    window.key("Mod+F");
    assert_eq!(window.focused().map(|field| field.name), Some("Find".into()), "the key takes the keyboard back");
}

#[test]
fn replacing_takes_one_place_at_a_time_or_all_at_once() {
    let mut window = page();
    let at = on(&mut window, "then");
    window.key("Mod+H").type_text("roof").click(Role::TextInput, "Replace with").type_text("lid");
    window.key("Enter");
    window.assert_markdown("lid first, then the roof again.\n\nThe rooftop leaks.\n\nA roof (new).");
    assert_eq!(said(&mut window), ["1 of 3"], "the next place is the one to look at");
    window.click(Role::Button, "Replace");
    window.assert_markdown("lid first, then the lid again.\n\nThe rooftop leaks.\n\nA roof (new).");
    window.click(Role::Button, "Replace all");
    window.assert_markdown("lid first, then the lid again.\n\nThe lidtop leaks.\n\nA lid (new).");
    assert_eq!(said(&mut window), ["No results"]);
    assert!(window.find(Role::Button, "Replace all").expect("the button").disabled);

    // Everything replaced at once comes back at once.
    window.click_at(at).key("Mod+Z");
    window.assert_markdown("lid first, then the lid again.\n\nThe rooftop leaks.\n\nA roof (new).");
    assert_eq!(said(&mut window), ["1 of 2"], "with the place that was selected");
}

#[test]
fn a_pattern_puts_what_its_groups_found_into_the_replacement() {
    let mut window = Harness::doc("Ada Lovelace; Alan Turing");
    window.key("Mod+H").click(Role::Button, "Regular expression").type_text(r"(\w+) (\w+)");
    window.click(Role::TextInput, "Replace with").type_text("$2, $1").click(Role::Button, "Replace all");
    window.assert_markdown("Lovelace, Ada; Turing, Alan");
}

#[test]
fn the_palette_and_the_menu_on_the_page_open_the_bar() {
    let mut window = page();
    window.key("Mod+Shift+P").type_text("find and").key("Enter");
    window.assert_closed(Role::Dialog, "Commands").assert_open(Role::Dialog, BAR);
    assert!(window.find(Role::TextInput, "Replace with").is_some());
    assert_eq!(window.focused().map(|field| field.name), Some("Find".into()));
    window.key("Escape");

    let at = on(&mut window, "leaks");
    window.secondary_click_at(at);
    let row =
        window.controls().into_iter().find(|found| found.role == Role::MenuItem && found.name.starts_with("Find "));
    window.click_at(row.expect("the menu offers to find").center());
    window.assert_closed(Role::Menu, "Context menu").assert_open(Role::Dialog, BAR);
}

#[test]
fn the_bar_keeps_to_the_page_and_folds_in_a_narrow_window() {
    let mut window = page();
    window.key("Mod+H").type_text("roof");
    for width in [900.0, 420.0, 320.0] {
        window.resize(width, 700.0);
        let bar = window.find(Role::Dialog, BAR).expect("the bar").bounds;
        assert!(bar.x >= 0.0 && bar.x + bar.width <= width, "{width}: {bar:?}");
        for (role, name) in [(Role::TextInput, "Find"), (Role::Button, "Regular expression"), (Role::Button, "Close")] {
            let part = window.find(role, name).unwrap_or_else(|| panic!("{name} at {width}")).bounds;
            assert!(part.x >= bar.x && part.x + part.width <= bar.x + bar.width, "{name} at {width}: {part:?}");
        }
        assert!(window.find(Role::TextInput, "Find").expect("the field").bounds.width >= 100.0, "{width}");
    }
}

#[test]
fn there_is_nothing_to_find_while_the_page_does_not_show() {
    let mut window = page();
    window.key("Mod+F").type_text("roof").key("Mod+P");
    window.assert_closed(Role::Dialog, BAR);
    window.key("Mod+F").assert_closed(Role::Dialog, BAR);
}
