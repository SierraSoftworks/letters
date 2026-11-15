//! An editor that reads and does not write: what a session has that was let in to read or to comment.

mod common;

use common::Do::{K, T};
use common::{Action, Harness, SAMPLE, action, canon, clip, perform, sound};
use letters_editor::{Clip, Command, InputEvent, Motion};
use letters_model::{Change, Document, Mark, Markers, Revision, Style, TagKind, TextStyle};
use proptest::prelude::*;

/// An editor on a document that it may only read, with something on the clipboard.
fn reader(state: &str) -> Harness {
    let mut editor = Harness::new(state);
    editor.platform.clip = Some(clip("pasted\ntext", None));
    editor.editor.set_read_only(true);
    editor
}

/// All that writing changes: the document, its style, how far its history goes and the marks in it.
fn written(editor: &mut Harness) -> (Document, Style, Revision, usize) {
    let marks = editor.editor.timeline().iter().map(|point| point.tags.len()).sum();
    let model = editor.editor.model();
    (model.doc().clone(), model.style().clone(), model.revision(), marks)
}

fn run(command: Command) -> Action {
    Action::Run(command)
}

#[test]
fn nothing_that_writes_changes_a_document_that_is_only_read() {
    let page = "https://a.example";
    let found = |title: &str| Command::Fetched {
        url: page.into(),
        title: Some(title.into()),
        description: None,
        picture: None,
    };
    let heading = TextStyle { size: 30.0, ..Style::default().types["heading1"].clone() };
    let composing = InputEvent::SetComposition { text: "x".into(), selection: None };
    let cases = [
        // Typing, however it comes in, and into a document with nowhere to type yet.
        ("p one|", Action::Type("x".into())),
        ("", Action::Type("x".into())),
        ("p one|", Action::Input(composing)),
        ("p one|", Action::Input(InputEvent::Commit("x".into()))),
        ("p one|", Action::Input(InputEvent::DeleteSurrounding { before: 1, after: 0 })),
        ("p one|", Action::Input(InputEvent::ReplaceRange { range: 0..3, text: "two".into() })),
        // Keys.
        ("p one|", Action::Key("Backspace")),
        ("p one|", Action::Key("Enter")),
        ("ul one\nul two|", Action::Key("Tab")),
        ("p o^ne|", Action::Key("Mod+b")),
        ("p one|", Action::Key("Mod+v")),
        ("p one|", Action::Key("Mod+Shift+v")),
        // Commands of every kind: text, blocks, blocks that hold no text, pictures, the style.
        ("p o^ne|", run(Command::Delete(Motion::Left))),
        ("p o^ne|", run(Command::ToggleMark(Mark::Italic))),
        ("p o^ne|", run(Command::SetLink(Some(page.into())))),
        ("p one|", run(Command::InsertTable { rows: 2, columns: 2 })),
        ("p one|", run(Command::InsertContents)),
        ("p one|", run(Command::InsertPreview(page.into()))),
        ("card https://a.example\np one|", run(found("A page"))),
        ("p [one|](https://a.example)", run(found("A page"))),
        ("p [https://a.example|](https://a.example)", run(Command::LinkAsCard)),
        ("p one|", Action::Figures(2, None)),
        ("h1 one|", run(Command::SetStyle { element: "heading1".into(), style: heading })),
        ("p one|", run(Command::SetLanguage("fr".into()))),
        ("p one|", run(Command::AddWord("mot".into()))),
        // Cutting takes nothing and copies nothing.
        ("p o^ne|", Action::Key("Mod+x")),
        // A move that makes a line for a caret that writes makes none here.
        ("code let a;|", Action::Key("Down")),
        ("code let a;|", Action::Key("Right")),
        ("> p quoted|", Action::Key("Down")),
        ("hr", run(Command::LineBelow)),
    ];
    for (state, action) in cases {
        let mut editor = reader(state);
        let (before, held) = (written(&mut editor), editor.platform.clip.clone());

        perform(&mut editor, &action);
        assert_eq!(written(&mut editor), before, "{action:?} in {state:?}");
        assert_eq!(editor.state(), canon(state), "{action:?} in {state:?}");
        assert_eq!(editor.platform.clip, held, "{action:?} in {state:?} leaves the clipboard");
        assert!(editor.platform.assets.is_empty(), "{action:?} in {state:?} keeps no picture");
        assert!(editor.editor.take_unfetched().is_empty(), "{action:?} in {state:?} asks for no fetch");

        // The same writes once writing is allowed, so the case is one that would have.
        editor.editor.set_read_only(false);
        perform(&mut editor, &action);
        assert_ne!(written(&mut editor), before, "{action:?} in {state:?} writes when it may");
    }
}

#[test]
fn what_cannot_be_done_is_said_as_for_any_command_with_nothing_to_do() {
    let mut editor = reader("p o^ne|");
    assert_eq!(editor.run(Command::Delete(Motion::Left)), Change::default());
    assert_eq!(editor.input(InputEvent::Insert("x".into())), Change::default());
    assert!(!editor.key("Backspace"), "the key is left to the shell");
    let refused = [Command::Paste, Command::Cut, Command::ToggleMark(Mark::Bold), Command::Split, Command::Undo];
    for command in refused {
        assert!(!editor.editor.applies(&command), "{command:?} is shown as of no use");
    }
}

#[test]
fn the_caret_still_moves_and_selects_and_the_selection_is_copied() {
    let cases: [(&str, &[common::Do], &str); 5] = [
        ("p one| two\np three", &[K("Right")], "p one |two\np three"),
        ("p one| two\np three", &[K("Shift+Mod+Right")], "p one^ two|\np three"),
        ("p one| two\np three", &[K("Mod+a")], "p ^one two\np three|"),
        ("p one| two\np three", &[K("Mod+End")], "p one two\np three|"),
        ("p one\ncode let a;|", &[K("Up")], "p one|\ncode let a;"),
    ];
    for (state, script, after) in cases {
        let mut editor = reader(state);
        editor.play(script);
        assert_eq!(editor.state(), canon(after), "{script:?} in {state:?}");
    }

    let mut editor = reader("p on|e two");
    assert_eq!(editor.run(Command::SelectWord), Change::default());
    assert_eq!(editor.state(), "p ^one| two");
    editor.run(Command::LineBelow);
    assert_eq!(editor.state(), "p one two|", "a click under the text puts the caret at its end");

    editor.editor.select(common::parse("p one ^two|").1.expect("a selection"));
    assert!(editor.key("Mod+c"));
    assert_eq!(editor.platform.clip.as_ref().map(|clip: &Clip| clip.text.as_str()), Some("two"));
}

#[test]
fn no_marker_shows_or_stops_the_caret_of_one_who_only_reads() {
    for state in ["h1 A *ti|tle*", "p **one|** two", "p a $x|y$ b", "tip> p in|side"] {
        let mut editor = Harness::new(state);
        assert_ne!(editor.editor.markers(), Markers::default(), "{state:?} shows its markers to a writer");
        editor.editor.set_read_only(true);
        assert_eq!(editor.editor.markers(), Markers::default(), "{state:?}");
    }

    // Each press of an arrow moves the caret a character, past the edges of the marked text too.
    let walk = |editor: &mut Harness, key: &str, times: usize| -> Vec<Option<usize>> {
        let press = |_| {
            editor.key(key);
            editor.editor.selection().map(|selection| selection.head.offset)
        };
        (0..times).map(press).collect()
    };
    let mut editor = reader("p |a**b**`c`$d$e");
    assert_eq!(walk(&mut editor, "Right", 5), [1, 2, 3, 4, 5].map(Some));
    assert_eq!(walk(&mut editor, "Left", 5), [4, 3, 2, 1, 0].map(Some));

    // A writer's caret stops at each marker on the way.
    let mut editor = Harness::new("p |a**b**`c`$d$e");
    assert_ne!(walk(&mut editor, "Right", 5), [1, 2, 3, 4, 5].map(Some));
}

#[test]
fn what_other_writers_write_still_arrives() {
    let mut writer = Harness::new("p one|");
    let mut reader = writer.fork(2);
    reader.editor.set_read_only(true);
    writer.type_text(" two");

    assert!(reader.merge(&writer));
    assert_eq!(reader.text(), "p one two");
    reader.type_text("x");
    assert_eq!(reader.text(), "p one two", "and it is still only read");
}

#[test]
fn undo_waits_for_writing_to_be_allowed_again() {
    let mut editor = Harness::new("p |");
    editor.play(&[T("one"), K("Mod+z"), T("two")]);
    editor.run(Command::InsertPreview("https://a.example".into()));
    let shown = editor.state();

    editor.editor.set_read_only(true);
    assert!(!editor.editor.can_undo() && !editor.editor.can_redo());
    assert!(!editor.key("Mod+z") && !editor.key("Mod+Shift+z"));
    assert_eq!(editor.state(), shown);
    assert!(editor.editor.take_unfetched().is_empty(), "a page put in before is not asked for");

    editor.editor.set_read_only(false);
    assert!(editor.editor.can_undo());
    editor.play(&[K("Mod+z"), K("Mod+z")]);
    assert_eq!(editor.state(), "p |", "what was written before is taken back as it would have been");
}

#[test]
fn a_composition_ends_where_it_stands_when_writing_is_no_longer_allowed() {
    let mut editor = Harness::new("p one|");
    editor.input(InputEvent::SetComposition { text: "tw".into(), selection: None });
    editor.editor.set_read_only(true);
    assert_eq!(editor.editor.composition(), None);
    editor.input(InputEvent::Commit("two".into()));
    assert_eq!(editor.text(), "p onetw", "what was composed so far stays, and no more comes");
}

#[test]
fn no_version_is_marked_or_named_and_no_reference_is_in_the_making() {
    let mut editor = Harness::new("fig# Boats\np See Figure |");
    editor.type_text("1");
    let now = editor.editor.model().revision();
    assert!(editor.editor.referring().is_some(), "a writer is offered the figure");

    editor.editor.set_read_only(true);
    assert_eq!(editor.editor.referring(), None);
    let before = written(&mut editor);
    editor.editor.name_version(&now, Some("Draft")).expect("nothing to refuse");
    editor.editor.mark_version(&now, TagKind::Session, None).expect("nothing to refuse");
    assert_eq!(written(&mut editor), before);

    editor.editor.set_read_only(false);
    editor.editor.name_version(&now, Some("Draft")).expect("a revision of this history");
    assert_ne!(written(&mut editor), before, "a writer's name is kept");
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    #[test]
    fn no_script_writes_a_document_that_is_only_read(actions in prop::collection::vec(action(), 1..48)) {
        let mut editor = Harness::new(SAMPLE);
        editor.platform.clip = Some(clip("pasted\ntext", None));
        // Something to undo, which a script may well try.
        editor.type_text("written");
        editor.editor.set_read_only(true);
        let before = written(&mut editor);
        for action in &actions {
            perform(&mut editor, action);
            prop_assert_eq!(&written(&mut editor), &before, "after {:?}", action);
            sound(&editor)?;
        }
        prop_assert!(editor.platform.assets.is_empty(), "a picture was kept");

        // A writer again, the script that changed nothing left nothing in the way of undo.
        editor.editor.set_read_only(false);
        while editor.key("Mod+z") {}
        prop_assert_eq!(editor.text(), canon(SAMPLE).replace(['|', '^'], ""));
    }
}
