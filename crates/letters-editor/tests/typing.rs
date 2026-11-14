mod common;

use common::{Harness, canon};
use letters_editor::{Command, InputEvent};
use letters_model::Mark;

#[test]
fn the_notation_reads_back_what_it_writes() {
    let states = [
        "p Hello| world",
        "h2 A ^title|",
        "p **bold** *italic* ~~struck~~ `code` __under__ ==lit==\n  ul nested **|**item\nhr",
        "> p quoted\n> todo [a link](https://example.com) and \\*stars\\*\ntip> done ^ticked\ncode:rust fn main() {}⏎|",
        "p |",
    ];
    for state in states {
        assert_eq!(Harness::new(state).state(), canon(state), "{state:?}");
    }
}

#[test]
fn typing_replaces_the_selection() {
    let cases = [
        ("p |", "Hello", "p Hello|"),
        ("p He|o", "ll", "p Hell|o"),
        ("p H^ell|o", "ipp", "p Hipp|o"),
        ("p H|ell^o", "ipp", "p Hipp|o"),
        ("p héllo |", "wörld ✍️", "p héllo wörld ✍️|"),
        ("h1 On^e\np two\nul th|ree", "c", "h1 Onc|ree"),
        ("p ^one\n> p two\n> p th|ree\np four", "x", "> p x|ree\np four"),
        ("code a|", "\n", "code a⏎|"),
    ];
    for (before, text, after) in cases {
        let mut editor = Harness::new(before);
        editor.type_text(text);
        assert_eq!(editor.state(), canon(after), "typing {text:?} into {before:?}");
    }
}

#[test]
fn typing_into_an_empty_document_starts_a_paragraph() {
    let mut editor = Harness::new("");
    assert!(editor.editor.selection().is_none());
    editor.type_text("Hi");
    assert_eq!(editor.state(), "p Hi|");
    editor.key("Mod+z");
    assert_eq!(editor.state(), "");
}

#[test]
fn a_line_break_in_typed_text_starts_a_new_block() {
    let mut editor = Harness::new("ul one|");
    editor.input(InputEvent::Insert("\r\ntwo\nthree".into()));
    assert_eq!(editor.state(), "ul one\nul two\nul three|");
}

#[test]
fn typed_text_takes_the_marks_at_the_caret() {
    let cases = [
        // Inside a span, and at its end, typing continues it.
        ("p **bo|ld** text", "x", "p **box|ld** text"),
        ("p **bold|** text", "x", "p **boldx|** text"),
        // Text before a span stays plain.
        ("p plain |**bold**", "x", "p plain x|**bold**"),
        ("p |**bold**", "x", "p x|**bold**"),
        // A mark that shows no marker carries over from the first character of a block.
        ("p |__under__", "x", "p __x|under__"),
        // A selection is replaced in the marks of its first character.
        ("p a ^**bold** b|", "x", "p a **x|**"),
        // A link grows only from inside.
        ("p [li|nk](https://a.example)", "x", "p [lix|nk](https://a.example)"),
        ("p [link|](https://a.example)", "x", "p [link](https://a.example)x|"),
        // Marks mean nothing in code.
        ("code **let|**", "x", "code **let**x|"),
    ];
    for (before, text, after) in cases {
        let mut editor = Harness::new(before);
        editor.type_text(text);
        assert_eq!(editor.state(), canon(after), "typing {text:?} into {before:?}");
    }
}

#[test]
fn a_mark_switched_on_at_a_caret_applies_to_what_is_typed_next() {
    let mut editor = Harness::new("p plain |");
    editor.run(Command::ToggleMark(Mark::Bold));
    assert!(editor.marks().contains(&Mark::Bold));
    editor.type_text("bold");
    editor.run(Command::ToggleMark(Mark::Bold));
    editor.type_text(" plain");
    assert_eq!(editor.state(), canon("p plain **bold** plain|"));
}

#[test]
fn edits_report_the_blocks_they_touched() {
    let mut editor = Harness::new("p one|\np two");
    let head = editor.editor.selection().unwrap().head.block.clone();
    let typed = editor.input(InputEvent::Insert("!".into()));
    assert!(typed.blocks.contains(&head) && !typed.structure);
    let split = editor.run(Command::Split);
    assert!(split.structure);
    let moved = editor.run(Command::SelectAll);
    assert!(moved.blocks.is_empty() && !moved.structure);
}
