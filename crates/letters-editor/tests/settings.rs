//! The style and the document's own fields: changed by commands, and undone with the text.

mod common;

use std::collections::BTreeSet;

use common::{Harness, canon};
use letters_editor::{Command, InputEvent};
use letters_model::{Change, Page, Style, TextStyle};

/// Sets the style of first headings to the built-in one at another size.
fn heading(size: f32) -> Command {
    let style = TextStyle { size, ..Style::default().types["heading1"].clone() };
    Command::SetStyle { element: "heading1".into(), style }
}

/// A style that differs from the built-in one in its page and in a type.
fn narrow() -> Style {
    let mut style = Style { page: Page { width: 400.0, ..Page::default() }, ..Style::default() };
    style.types.get_mut("body").expect("the built-in style has body text").size = 13.5;
    style
}

/// Everything a setting can change.
fn shown(editor: &Harness) -> (Style, String, BTreeSet<String>) {
    let doc = editor.editor.doc();
    (editor.editor.model().style().clone(), doc.language.clone(), doc.dictionary.clone())
}

#[test]
fn a_setting_is_one_undo_step_that_leaves_the_text_alone() {
    // A name, the command, and whether it is to the style rather than the document.
    let cases = [
        ("a type's size", heading(30.0), true),
        ("the page", Command::SetPage(Page { width: 400.0, ..Page::default() }), true),
        ("a whole style", Command::ReplaceStyle(narrow()), true),
        ("the language", Command::SetLanguage("fr".into()), false),
        ("a word", Command::AddWord("mot".into()), false),
    ];
    // With a selection, and with no text to have one in.
    for state in ["h1 Ti^tle|\np text", ""] {
        for (name, command, style) in &cases {
            let mut editor = Harness::new(state);
            let before = shown(&editor);

            let change = editor.run(command.clone());
            let after = shown(&editor);
            assert_ne!(after, before, "{name} in {state:?}");
            assert_eq!((change.style, change.document), (*style, !*style), "{name}");
            assert!(change.blocks.is_empty() && !change.structure, "{name}: no block is laid out again for it");
            assert_eq!(editor.state(), canon(state), "{name}");
            assert_eq!(editor.run(command.clone()), Change::default(), "{name}: set again, it changes nothing");

            let undone = editor.run(Command::Undo);
            assert_eq!((undone.style, undone.document), (*style, !*style), "undoing {name}");
            assert_eq!((shown(&editor), editor.state()), (before, canon(state)), "undoing {name}");
            assert!(!editor.editor.can_undo(), "{name} was one step");
            editor.run(Command::Redo);
            assert_eq!(shown(&editor), after, "redoing {name}");
        }
    }
}

#[test]
fn settings_and_typing_are_undone_in_the_order_they_were_made() {
    let mut editor = Harness::new("p one|");
    let seen = |editor: &Harness| (editor.text(), editor.editor.doc().dictionary.contains("mot"));
    editor.type_text("a");
    editor.run(Command::AddWord("mot".into()));
    editor.type_text("b");
    editor.run(Command::RemoveWord("mot".into()));

    let mut steps = vec![seen(&editor)];
    while editor.key("Mod+z") {
        steps.push(seen(&editor));
    }
    let expected = [("p oneab", false), ("p oneab", true), ("p onea", true), ("p onea", false), ("p one", false)];
    assert_eq!(steps, expected.map(|(text, word)| (text.to_owned(), word)));
}

#[test]
fn a_style_the_file_format_would_refuse_changes_nothing() {
    let page = Page::default();
    let cases = [
        ("no size", heading(0.0)),
        ("margins that leave no room", Command::SetPage(Page { margin_left: page.width, ..page })),
    ];
    for (name, command) in cases {
        let mut editor = Harness::new("p one|");
        let before = shown(&editor);
        assert_eq!(editor.run(command), Change::default(), "{name}");
        assert_eq!(shown(&editor), before, "{name}");
        assert!(!editor.editor.can_undo(), "{name} left nothing to undo");
    }
}

#[test]
fn another_writers_setting_arrives_without_disturbing_the_writing() {
    let mut mine = Harness::new("p one|");
    mine.type_text(" two ");
    let mut theirs = mine.fork(2);
    theirs.run(heading(30.0));
    theirs.run(Command::AddWord("mot".into()));

    // A word is being composed when it arrives.
    mine.input(InputEvent::SetComposition { text: "th".into(), selection: None });
    let change = mine.editor.merge(&theirs.editor.model().save()).expect("a saved history is read");
    assert!(change.style && change.document, "{change:?}");
    assert_eq!(shown(&mine), shown(&theirs));
    assert_eq!(mine.state(), canon("p one two th|"));
    mine.input(InputEvent::Commit("three".into()));
    assert_eq!(mine.state(), canon("p one two three|"));

    // Undo takes back this writer's text, and not the other's settings.
    while mine.key("Mod+z") {}
    assert_eq!((mine.state(), shown(&mine)), (canon("p one|"), shown(&theirs)));
}

#[test]
fn a_style_changed_outside_is_shown_and_is_no_undo_step() {
    let mut editor = Harness::new("p on^e|");
    let change = editor.editor.rewrite_style(narrow()).expect("the newest revision takes a style");
    assert!(change.style && change.blocks.is_empty(), "{change:?}");
    assert_eq!(editor.editor.model().style(), &narrow());
    assert_eq!(editor.state(), canon("p on^e|"));
    assert!(!editor.editor.can_undo());
}
