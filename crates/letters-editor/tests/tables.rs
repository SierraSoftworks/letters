//! Tables: the line that becomes one, and the commands that change rows and columns.

mod common;

use common::Do::{K, T};
use common::{Do, Harness, canon, check};
use letters_editor::{Cell, Command, TableEdit};
use letters_model::{Align, BlockKind};

const TABLE: &str = "p one\nr1> th A\nr1> th B\nr2> td C\nr2> td D\nr3> td E\nr3> td F\np two";

/// The table above with the caret after `text`.
fn at(text: &str) -> String {
    assert!(TABLE.contains(text), "the table has no {text:?}");
    TABLE.replacen(text, &format!("{text}|"), 1)
}

/// The same table with other rows.
fn rows(rows: &str) -> String {
    format!("p one\n{rows}\np two")
}

#[test]
fn a_line_of_cells_between_bars_becomes_a_table_on_enter() {
    let cases: [(&str, &[Do], &str); 7] = [
        ("p |", &[T("| a | b |"), K("Enter")], "r1> th a\nr1> th b\nr2> td |\nr2> td"),
        ("p one\np |\np two", &[T("|a||"), K("Enter"), T("x")], "p one\nr1> th a\nr1> th\nr2> td x|\nr2> td\np two"),
        // The cells keep their formatting, and a bar in code is part of the code.
        ("p |", &[T("| **a** | `x|y` |"), K("Enter")], "r1> th **a**\nr1> th `x\\|y`\nr2> td |\nr2> td"),
        // Backspace straight after gives the line back as it was typed.
        ("p |", &[T("| a | b |"), K("Enter"), K("Backspace")], "p \\| a \\| b \\||"),
        // One cell is no table, and neither is a row in a heading or with text after it.
        ("p |", &[T("|a|"), K("Enter")], "p \\|a\\|\np |"),
        ("h2 |", &[T("|a|b|"), K("Enter")], "h2 \\|a\\|b\\|\np |"),
        ("p |", &[T("|a|b| c"), K("Enter")], "p \\|a\\|b\\| c\np |"),
    ];
    for (before, script, after) in cases {
        let mut editor = Harness::new(before);
        editor.play(script);
        assert_eq!(editor.state(), canon(after), "{script:?} on {before:?}");
        assert!(check(&editor).is_ok(), "{script:?} on {before:?} leaves a table that is not whole");
    }
}

#[test]
fn a_table_is_put_in_after_the_caret_or_in_place_of_an_empty_line() {
    let empty = "r1> th |\nr1> th\nr2> td\nr2> td";
    let cases = [
        ("p one|\np two", format!("p one\n{empty}\np two")),
        ("p one\np |\np two", format!("p one\n{empty}\np two")),
        ("p o^ne\np tw|o\np three", format!("p one\np two\n{empty}\np three")),
        ("fig* Boats\np two", format!("fig Boats\n{empty}\np two")),
        ("r1> td A|\nr2> td B\np two", format!("r1> td A\nr2> td B\n{empty}\np two")),
        ("", empty.to_owned()),
    ];
    for (before, after) in cases {
        let mut editor = Harness::new(before);
        editor.run(Command::InsertTable { rows: 2, columns: 2 });
        assert_eq!(editor.state(), canon(&after), "in {before:?}");
        assert!(editor.key("Mod+z"), "putting a table in {before:?} is one step to undo");
        assert_eq!(editor.text(), Harness::new(before).text(), "undone in {before:?}");
    }
}

#[test]
fn rows_and_columns_are_added_removed_and_moved_around_the_caret() {
    use TableEdit::*;
    let cases = [
        // (the edit with the caret after D, the table afterwards)
        (AddRow { below: true }, "r1> th A\nr1> th B\nr2> td C\nr2> td D\nr3> td\nr3> td |\nr4> td E\nr4> td F"),
        (AddRow { below: false }, "r1> th A\nr1> th B\nr2> td\nr2> td |\nr3> td C\nr3> td D\nr4> td E\nr4> td F"),
        (
            AddColumn { after: true },
            "r1> th A\nr1> th B\nr1> th\nr2> td C\nr2> td D\nr2> td |\nr3> td E\nr3> td F\nr3> td",
        ),
        (
            AddColumn { after: false },
            "r1> th A\nr1> th\nr1> th B\nr2> td C\nr2> td |\nr2> td D\nr3> td E\nr3> td\nr3> td F",
        ),
        // The caret stays in its column, or beside it.
        (RemoveRow, "r1> th A\nr1> th B\nr2> td E\nr2> td F|"),
        (RemoveColumn, "r1> th A\nr2> td C|\nr3> td E"),
        // A row or a column moves with what is written in it, and the first row is the header.
        (MoveRow { to: 0 }, "r1> th C\nr1> th D|\nr2> td A\nr2> td B\nr3> td E\nr3> td F"),
        (MoveRow { to: 9 }, "r1> th A\nr1> th B\nr2> td E\nr2> td F\nr3> td C\nr3> td D|"),
        (MoveColumn { to: 0 }, "r1> th B\nr1> th A\nr2> td D|\nr2> td C\nr3> td F\nr3> td E"),
        (Header(false), "r1> td A\nr1> td B\nr2> td C\nr2> td D|\nr3> td E\nr3> td F"),
    ];
    for (edit, after) in cases {
        let mut editor = Harness::new(&at("D"));
        editor.run(Command::Table(edit.clone()));
        assert_eq!(editor.state(), canon(&rows(after)), "{edit:?}");
        assert!(check(&editor).is_ok(), "{edit:?} leaves a table that is not whole");
        assert!(editor.key("Mod+z"), "{edit:?} is one step to undo");
        assert_eq!(editor.state(), canon(&at("D")), "{edit:?} undone");
    }
}

#[test]
fn the_last_row_or_column_takes_the_table_with_it() {
    let cases = [
        ("p one\nr1> td A|\nr1> td B\np two", TableEdit::RemoveRow, "p one\np |two"),
        ("p one\nr1> td A|\nr2> td B", TableEdit::RemoveColumn, "p one|"),
        ("r1> td A|", TableEdit::RemoveRow, ""),
    ];
    for (before, edit, after) in cases {
        let mut editor = Harness::new(before);
        editor.run(Command::Table(edit.clone()));
        assert_eq!(editor.state(), canon(after), "{edit:?} on {before:?}");
    }
}

#[test]
fn a_column_is_lined_up_without_touching_the_others() {
    let mut editor = Harness::new(&at("D"));
    editor.run(Command::Table(TableEdit::Align(Align::End)));
    let aligned = |editor: &Harness| {
        editor.editor.doc().walk().find_map(|block| match &block.kind {
            BlockKind::Table { columns, .. } => Some(columns.clone()),
            _ => None,
        })
    };
    assert_eq!(aligned(&editor), Some(vec![Align::Start, Align::End]));
    assert_eq!(editor.state(), canon(&at("D")), "the text and the caret stay as they were");
}

#[test]
fn a_table_left_ragged_by_a_merge_is_filled_out_by_the_next_command() {
    let cases = [
        // A row that is short of cells, and one that has more than the table has columns.
        ("r1> th A\nr1> th B\nr2> td C|", "r1> th A\nr1> th B\nr2> td C|\nr2> td"),
        ("r1> th A\nr2> td B|\nr2> td C", "r1> th A\nr1> th\nr2> td B|\nr2> td C"),
    ];
    for (before, after) in cases {
        let mut editor = Harness::new(before);
        assert!(check(&editor).is_err(), "{before:?} is ragged");
        editor.run(Command::Table(TableEdit::Align(Align::Center)));
        assert_eq!(editor.state(), canon(after), "{before:?}");
        assert!(check(&editor).is_ok(), "{before:?} is whole afterwards");
    }
}

#[test]
fn the_caret_says_where_in_a_table_it_is() {
    let place = |state: &str| Harness::new(state).editor.cell();
    let found = place(&at("D")).expect("the caret is in a cell");
    let Cell { row, column, rows, columns, .. } = found;
    assert_eq!((row, column, rows, columns), (1, 1, 3, 2));
    assert_eq!(place(&TABLE.replacen("one", "one|", 1)), None, "text outside a table is in no cell");
    // A command for a table has nothing to act on there, and changes nothing.
    let mut outside = Harness::new(&TABLE.replacen("one", "one|", 1));
    outside.run(Command::Table(TableEdit::RemoveRow));
    assert_eq!(outside.text(), Harness::new(TABLE).text());
}
