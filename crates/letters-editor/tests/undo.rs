mod common;

use common::Do::{K, T};
use common::{Action, Harness, SAMPLE, action, canon, check, clip, perform};
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

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    #[test]
    fn any_script_can_be_undone_and_redone(actions in prop::collection::vec(action(), 1..48), wrap in prop::option::of(3..9usize)) {
        let mut editor = match wrap {
            Some(width) => Harness::wrapped(&SAMPLE.replace("ünïcödé 👩‍👧", "unicode"), width),
            None => Harness::new(SAMPLE),
        };
        editor.platform.clip = Some(clip("pasted\ntext", None));
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
