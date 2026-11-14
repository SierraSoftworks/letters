mod common;

use std::collections::BTreeSet;

use common::Do::{K, T};
use common::{Harness, canon, key};
use letters_editor::{Clip, Command, InputEvent};
use letters_model::{BlockKind, Body, ListKind, Mark, Pos, Selection, Tone};
use proptest::prelude::*;

/// Presses undo until nothing changes, and returns each state passed through.
fn unwind(editor: &mut Harness) -> Vec<String> {
    std::iter::from_fn(|| editor.key("Mod+z").then(|| editor.state())).collect()
}

#[test]
fn typing_undoes_a_word_at_a_time() {
    let mut editor = Harness::new("p |");
    editor.type_text("one two  three");
    assert_eq!(unwind(&mut editor), ["p one two|", "p one|", "p |"]);
}

#[test]
fn a_pause_or_a_move_starts_a_new_step() {
    let mut editor = Harness::new("p |");
    editor.type_text("ab");
    editor.platform.now += 5_000;
    editor.type_text("cd");
    editor.play(&[K("Left"), K("Right"), T("ef")]);
    assert_eq!(unwind(&mut editor), ["p abcd|", "p ab|", "p |"]);
}

#[test]
fn deleting_undoes_as_one_step_and_apart_from_typing() {
    let mut editor = Harness::new("p |");
    editor.play(&[T("abcd"), K("Backspace"), K("Backspace"), T("x")]);
    assert_eq!(unwind(&mut editor), ["p ab|", "p abcd|", "p |"]);
}

#[test]
fn undo_and_redo_put_the_selection_back_with_the_text() {
    let mut editor = Harness::new("h1 on^e\nul tw|o");
    editor.type_text("x");
    editor.key("Mod+z");
    assert_eq!(editor.state(), canon("h1 on^e\nul tw|o"));
    editor.key("Mod+Shift+z");
    assert_eq!(editor.state(), canon("h1 onx|o"));
    editor.key("Mod+z");
    editor.key("Mod+y");
    assert_eq!(editor.state(), canon("h1 onx|o"));
}

#[test]
fn a_new_edit_forgets_what_was_undone() {
    let mut editor = Harness::new("p |");
    editor.play(&[T("a"), K("Mod+z"), T("b")]);
    assert!(!editor.editor.can_redo());
    assert!(!editor.key("Mod+Shift+z"), "there is nothing to redo, so the key is not used");
    assert_eq!(editor.state(), "p b|");
}

#[test]
fn undo_with_nothing_to_undo_is_left_to_the_shell() {
    let mut editor = Harness::new("p one|");
    assert!(!editor.editor.can_undo() && !editor.key("Mod+z"));
    editor.key("Left");
    assert!(!editor.key("Mod+z"), "moving the caret is not an edit");
}

/// One thing a writer or a shell can do to the editor.
#[derive(Clone, Debug)]
enum Action {
    Type(String),
    Key(&'static str),
    Run(Command),
    /// Selects from one block and offset to another, both wrapped into range.
    Select(usize, usize, usize, usize),
    Input(InputEvent),
    Wait(u64),
}

fn action() -> impl Strategy<Value = Action> {
    let typed = prop::sample::select(vec![
        "a",
        "b",
        " ",
        " ",
        "é",
        "👩‍👧",
        "*",
        "*",
        "`",
        "~",
        "#",
        "-",
        ">",
        "[",
        "]",
        "(",
        ")",
        "!",
        "x",
        "1",
        ".",
        "\n",
    ]);
    let keys = prop::sample::select(vec![
        "Left",
        "Right",
        "Up",
        "Down",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "Mod+Left",
        "Mod+Right",
        "Mod+Up",
        "Mod+Down",
        "Mod+Home",
        "Mod+End",
        "Shift+Left",
        "Shift+Right",
        "Shift+Up",
        "Shift+Down",
        "Shift+Mod+Left",
        "Shift+Mod+End",
        "Backspace",
        "Backspace",
        "Delete",
        "Mod+Backspace",
        "Mod+Delete",
        "Enter",
        "Enter",
        "Shift+Enter",
        "Mod+Enter",
        "Tab",
        "Shift+Tab",
        "Mod+b",
        "Mod+i",
        "Mod+e",
        "Mod+Shift+x",
        "Mod+u",
        "Mod+z",
        "Mod+z",
        "Mod+Shift+z",
        "Mod+a",
        "Mod+c",
        "Mod+x",
        "Mod+v",
        "Mod+v",
        "Mod+Shift+v",
    ]);
    let kinds = prop::sample::select(vec![
        BlockKind::Paragraph,
        BlockKind::Heading { level: 2 },
        BlockKind::ListItem { list: ListKind::Bullet, depth: 0 },
        BlockKind::ListItem { list: ListKind::Task { checked: true }, depth: 0 },
        BlockKind::Code { language: None },
        BlockKind::Quote,
        BlockKind::Callout { tone: Tone::Tip },
        BlockKind::Divider,
    ]);
    let commands = prop_oneof![
        kinds.prop_map(Command::SetBlock),
        Just(Command::SetLink(Some("https://a.example".into()))),
        Just(Command::SetLink(None)),
        Just(Command::SelectWord),
        Just(Command::SelectBlock),
        Just(Command::ToggleMark(Mark::Highlight)),
    ];
    let composed = prop::sample::select(vec!["", "k", "か", "かな"]);
    let inputs = prop_oneof![
        (composed.clone(), 0..8usize)
            .prop_map(|(text, at)| InputEvent::SetComposition { text: text.into(), selection: Some(at..at) }),
        composed.prop_map(|text| InputEvent::Commit(text.into())),
        (0..12usize, 0..6usize)
            .prop_map(|(start, len)| InputEvent::ReplaceRange { range: start..start + len, text: "fix".into() }),
        (0..4usize, 0..4usize).prop_map(|(before, after)| InputEvent::DeleteSurrounding { before, after }),
    ];
    prop_oneof![
        4 => typed.prop_map(|text| Action::Type(text.into())),
        4 => keys.prop_map(Action::Key),
        1 => commands.prop_map(Action::Run),
        1 => (0..16usize, 0..24usize, 0..16usize, 0..24usize).prop_map(|(a, b, c, d)| Action::Select(a, b, c, d)),
        1 => inputs.prop_map(Action::Input),
        1 => prop::sample::select(vec![10u64, 2_000]).prop_map(Action::Wait),
    ]
}

fn perform(editor: &mut Harness, action: &Action) {
    match action {
        Action::Type(text) => editor.type_text(text),
        Action::Key(name) => _ = editor.editor.key(key(name), &mut editor.platform),
        Action::Run(command) => _ = editor.run(command.clone()),
        Action::Input(event) => _ = editor.input(event.clone()),
        Action::Wait(ms) => editor.platform.now += ms,
        Action::Select(a, b, c, d) => {
            let ids: Vec<_> = editor.editor.doc().walk().map(|block| block.id.clone()).collect();
            if !ids.is_empty() {
                let pos = |block: &usize, offset: &usize| Pos::new(ids[block % ids.len()].clone(), *offset);
                editor.editor.select(Selection { anchor: pos(a, b), head: pos(c, d) });
            }
        }
    }
}

/// What must hold after anything: every block has its own id, and the selection sits on characters of text.
fn check(editor: &Harness) -> Result<(), TestCaseError> {
    let doc = editor.editor.doc();
    let ids: BTreeSet<_> = doc.walk().map(|block| &block.id).collect();
    prop_assert_eq!(ids.len(), doc.walk().count(), "two blocks share an id");
    if let Some(selection) = editor.editor.selection() {
        for pos in [&selection.anchor, &selection.head] {
            let block = doc.block(&pos.block).filter(|block| matches!(block.body, Body::Text(_)));
            let text = block.map(|block| block.plain_text());
            prop_assert!(text.is_some_and(|text| text.is_char_boundary(pos.offset)), "the selection is off the text");
        }
    }
    Ok(())
}

const SAMPLE: &str = "h1 A *title*|\np Some **bold** and [linked](https://a.example) text, ünïcödé 👩‍👧.\nul one\n  todo two\nhr\n> p quoted\n> p lines\ntip> p a tip\ncode:rust fn main() {⏎    body();⏎}\np";

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    #[test]
    fn any_script_can_be_undone_and_redone(actions in prop::collection::vec(action(), 1..48), wrap in prop::option::of(3..9usize)) {
        let mut editor = match wrap {
            Some(width) => Harness::wrapped(&SAMPLE.replace("ünïcödé 👩‍👧", "unicode"), width),
            None => Harness::new(SAMPLE),
        };
        editor.platform.clip = Some(Clip { text: "pasted\ntext".into(), html: None });
        let original = editor.text();
        for action in &actions {
            // The fake layout counts bytes, so it is only given text of one byte a character.
            let ascii = |action: &Action| match action {
                Action::Type(text) => text.is_ascii(),
                Action::Input(_) => false,
                _ => true,
            };
            if wrap.is_none() || ascii(action) {
                perform(&mut editor, action);
                check(&editor)?;
            }
        }
        // Finishing a composition makes everything so far an undo step.
        editor.key("Left");
        let edited = editor.text();

        let mut undone = 0;
        while editor.key("Mod+z") {
            undone += 1;
            check(&editor)?;
        }
        prop_assert_eq!(editor.text(), original);
        for _ in 0..undone {
            prop_assert!(editor.key("Mod+Shift+z"));
            check(&editor)?;
        }
        prop_assert_eq!(editor.text(), edited);
    }
}
