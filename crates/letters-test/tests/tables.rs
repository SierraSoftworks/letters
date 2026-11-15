//! The handles round a table: adding, removing and moving its rows and columns.

use letters_render::Point;
use letters_test::{Harness, Role};

const NOTE: &str =
    "Intro line.\n\n| Band | Homes | Rate |\n| --- | --- | --- |\n| A | 120 | 4 |\n| B | 80 | 6 |\n\nAfter.";

/// A note with a table of three columns, a header and two rows between two paragraphs. The caret waits in
/// the first paragraph.
fn note() -> Harness {
    let mut window = Harness::doc(NOTE);
    window.resize(900.0, 700.0);
    window.select("Intro");
    window.paint();
    window
}

/// The rows of the note's table as Markdown writes them, without the rule under the header.
fn rows(window: &Harness) -> Vec<String> {
    let cells =
        |line: &str| line.split('|').map(str::trim).filter(|cell| !cell.is_empty()).collect::<Vec<_>>().join("|");
    let written = window.markdown();
    written.lines().filter(|line| line.starts_with('|') && !line.contains("---")).map(cells).collect()
}

fn button(window: &mut Harness, name: &str) -> Option<Point> {
    window.paint();
    window.find(Role::Button, name).map(|found| found.center())
}

/// The middle of the first place `text` is written on the page.
fn text(window: &mut Harness, text: &str) -> Point {
    let held = window.editor().selection().cloned();
    window.select(text);
    window.paint();
    let caret = window.ui().caret_rect().expect("the page shows a caret");
    if let Some(held) = held {
        window.ui().select(held);
    }
    window.paint();
    Point { x: caret.x - 2.0, y: caret.y + caret.height / 2.0 }
}

#[test]
fn a_table_shows_its_handles_under_the_pointer_and_round_the_caret() {
    let mut window = note();
    let all = ["Column 1", "Column 2", "Column 3", "Row 1", "Row 2", "Row 3"];
    let shown =
        |window: &mut Harness| all.iter().filter(|name| button(window, name).is_some()).copied().collect::<Vec<_>>();
    assert!(shown(&mut window).is_empty(), "nothing round a table that is left alone");

    let over = text(&mut window, "80");
    window.hover_at(over);
    assert_eq!(shown(&mut window), ["Column 2", "Row 3"], "those of the cell pointed at");
    let pluses =
        ["Add column before column 2", "Add column before column 3", "Add row before row 3", "Add row at the end"];
    assert!(pluses.iter().all(|name| button(&mut window, name).is_some()), "with a plus on each border beside them");

    let away = text(&mut window, "After");
    window.hover_at(away);
    assert!(shown(&mut window).is_empty(), "they go with the pointer");

    window.select("4");
    let caret = window.editor().selection().map(|selection| selection.head.clone()).expect("a selection");
    window.ui().select(letters_model::Selection::caret(caret));
    assert_eq!(shown(&mut window), ["Column 3", "Row 2"], "those of the caret's cell");
}

#[test]
fn a_plus_past_the_last_adds_a_column_or_a_row_at_the_end() {
    let cases = [
        ("Add column at the end", ["Band|Homes|Rate", "A|120|4", "B|80|6"].as_slice(), 4),
        ("Add row at the end", ["Band|Homes|Rate", "A|120|4", "B|80|6", ""].as_slice(), 3),
    ];
    for (bar, expected, columns) in cases {
        let mut window = note();
        let over = text(&mut window, "6");
        window.hover_at(over);
        let at = button(&mut window, bar).expect("the plus shows");
        window.click_at(at);
        assert_eq!(rows(&window), expected, "{bar}: what was written stays where it was");
        let cell = window.editor().cell().expect("the caret is in the table");
        assert_eq!(cell.columns, columns, "{bar}");
        assert_eq!(cell.rows, expected.len(), "{bar}");
        window.type_text("new");
        assert!(rows(&window).iter().any(|row| row.contains("new")), "{bar}: and the table is written in at once");
    }
}

#[test]
fn a_plus_on_a_border_adds_a_column_or_a_row_there() {
    // (the cell pointed at, the plus, the cell that then stands after the new one, the new column or row)
    let cases = [
        ("120", "Add column before column 2", "Homes", 1),
        ("120", "Add column before column 3", "Rate", 2),
        ("Band", "Add column before column 1", "Band", 0),
        ("120", "Add row before row 3", "80", 2),
        ("80", "Add row before row 3", "80", 2),
    ];
    for (cell, plus, after, added) in cases {
        let mut window = note();
        let over = text(&mut window, cell);
        window.hover_at(over);
        let at = button(&mut window, plus).unwrap_or_else(|| panic!("{plus} shows"));
        window.click_at(at);
        let caret = window.editor().cell().expect("the caret is in the table");
        let by_column = plus.contains("column");
        assert_eq!((caret.columns, caret.rows), if by_column { (4, 3) } else { (3, 4) }, "{plus}: one more");
        let stands = if by_column { caret.column } else { caret.row };
        assert_eq!(stands, added, "{plus}: the caret is in what was added");
        window.type_text("new");
        let rows = rows(&window);
        let all: Vec<&str> = rows.iter().flat_map(|row| row.split('|')).collect();
        let (new, after) = (all.iter().position(|cell| *cell == "new"), all.iter().position(|cell| *cell == after));
        assert!(new.is_some() && new < after, "{plus}: it stands before {after:?}: {rows:?}");
    }
}

#[test]
fn a_press_on_a_handle_opens_the_menu_of_its_column_or_row() {
    // (the cell pointed at, the handle, its menu, the item, the rows after)
    let cases = [
        ("80", "Column 2", "Column", "Remove column", ["Band|Rate", "A|4", "B|6"].as_slice()),
        ("80", "Row 3", "Row", "Remove row", ["Band|Homes|Rate", "A|120|4"].as_slice()),
        ("120", "Column 2", "Column", "Move column right", ["Band|Rate|Homes", "A|4|120", "B|6|80"].as_slice()),
        ("120", "Row 2", "Row", "Move row down", ["Band|Homes|Rate", "B|80|6", "A|120|4"].as_slice()),
        ("Band", "Column 1", "Column", "Add column to the left", ["Band|Homes|Rate", "A|120|4", "B|80|6"].as_slice()),
    ];
    for (cell, handle, menu, item, expected) in cases {
        let mut window = note();
        let over = text(&mut window, cell);
        window.hover_at(over);
        let at = button(&mut window, handle).unwrap_or_else(|| panic!("{handle} shows"));
        window.click_at(at);
        window.assert_open(Role::Menu, menu);
        window.click(Role::MenuItem, item);
        window.assert_closed(Role::Menu, menu);
        assert_eq!(rows(&window), expected, "{item} from {handle}");
    }
    let mut window = note();
    let over = text(&mut window, "80");
    window.hover_at(over);
    let at = button(&mut window, "Column 2").expect("the handle shows");
    window.click_at(at);
    window.key("Escape");
    window.assert_closed(Role::Menu, "Column");
    assert_eq!(rows(&window).len(), 3, "a menu put away changes nothing");
}

#[test]
fn a_handle_is_carried_to_another_place_among_its_columns_or_rows() {
    // (the cell pointed at, the handle, where it is let go: a cell and how far from its start, the rows after)
    let cases = [
        ("Band", "Column 1", Some(("Rate", 400.0, 0.0)), ["Homes|Rate|Band", "120|4|A", "80|6|B"].as_slice()),
        ("Rate", "Column 3", Some(("Band", -400.0, 0.0)), ["Rate|Band|Homes", "4|A|120", "6|B|80"].as_slice()),
        ("Band", "Column 1", Some(("Homes", 4.0, 0.0)), ["Band|Homes|Rate", "A|120|4", "B|80|6"].as_slice()),
        ("A", "Row 2", Some(("80", 0.0, 12.0)), ["Band|Homes|Rate", "B|80|6", "A|120|4"].as_slice()),
        // Carried a little way and let go over its own place.
        ("Homes", "Column 2", None, ["Band|Homes|Rate", "A|120|4", "B|80|6"].as_slice()),
    ];
    for (cell, handle, to, expected) in cases {
        let mut window = note();
        let over = text(&mut window, cell);
        window.hover_at(over);
        let from = button(&mut window, handle).unwrap_or_else(|| panic!("{handle} shows"));
        let past = match to {
            Some((cell, right, down)) => {
                let start = text(&mut window, cell);
                Point { x: start.x + right, y: start.y + down }
            }
            None => Point { x: from.x + 6.0, y: from.y - 6.0 },
        };
        window.press_at(from);
        window.hover_at(Point { x: (from.x + past.x) / 2.0, y: (from.y + past.y) / 2.0 });
        window.release_at(past);
        assert_eq!(rows(&window), expected, "{handle} to {to:?}");
        assert!(window.find(Role::Menu, "Column").is_none(), "{handle}: a handle that was carried opens no menu");
    }
}
