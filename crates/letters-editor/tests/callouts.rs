//! Changing a callout after it is made.

mod common;

use common::{Harness, canon};
use letters_editor::{CalloutEdit, Command};
use letters_model::{BlockKind, Tone};

/// The first callout of the document.
fn callout(editor: &Harness) -> Option<BlockKind> {
    let doc = editor.editor.doc();
    doc.walk().find(|block| matches!(block.kind, BlockKind::Callout { .. })).map(|block| block.kind.clone())
}

fn chosen(tone: Tone, title: Option<&str>, icon: Option<&str>, color: Option<&str>) -> BlockKind {
    let own = |text: Option<&str>| text.map(str::to_owned);
    BlockKind::Callout { tone, title: own(title), icon: own(icon), color: own(color) }
}

#[test]
fn a_callout_takes_a_tone_a_title_an_icon_and_a_colour() {
    use CalloutEdit::{Color, Icon, Reset, Title, Tone as To};
    let some = |text: &str| Some(text.to_owned());
    // Each case: the edits made in turn to an info callout, and the callout after them.
    let cases = [
        (vec![To(Tone::Danger)], BlockKind::callout(Tone::Danger)),
        (vec![Title(some("Heads up"))], chosen(Tone::Info, Some("Heads up"), None, None)),
        (vec![Title(some(""))], chosen(Tone::Info, Some(""), None, None)),
        (vec![Title(some("two\nlines"))], chosen(Tone::Info, Some("two lines"), None, None)),
        (vec![Icon(some("flame"))], chosen(Tone::Info, None, Some("flame"), None)),
        (vec![Icon(some("none"))], chosen(Tone::Info, None, Some("none"), None)),
        (vec![Color(some("#AA3366"))], chosen(Tone::Info, None, None, Some("#aa3366"))),
        // What cannot be read as a colour changes nothing.
        (vec![Color(some("#aa3366")), Color(some("pink"))], chosen(Tone::Info, None, None, Some("#aa3366"))),
        // What a tone has anyway is not kept as a choice, so it follows the tone when that changes.
        (vec![Title(some("Info")), Icon(some("info"))], BlockKind::callout(Tone::Info)),
        (vec![Title(some("Warning")), To(Tone::Warn)], BlockKind::callout(Tone::Warn)),
        // What was chosen stays through a change of tone, and goes when the defaults are asked for.
        (
            vec![Title(some("Heads up")), Color(some("#aa3366")), To(Tone::Tip)],
            chosen(Tone::Tip, Some("Heads up"), None, Some("#aa3366")),
        ),
        (vec![Title(some("Heads up")), Icon(some("bug")), Reset], BlockKind::callout(Tone::Info)),
        (vec![Title(some("Heads up")), Icon(None), Title(None), Color(None)], BlockKind::callout(Tone::Info)),
    ];
    for (edits, after) in cases {
        let mut editor = Harness::new("info> p one|\ninfo> p two");
        for edit in &edits {
            editor.run(Command::Callout(edit.clone()));
        }
        assert_eq!(callout(&editor), Some(after), "after {edits:?}");
        assert_eq!(editor.state(), canon(&editor.state()), "the text and the caret stay: {edits:?}");
        assert!(editor.state().contains("one|"), "the caret stays after {edits:?}");
    }
}

#[test]
fn a_change_is_one_step_to_undo_and_applies_only_inside_a_callout() {
    let mut editor = Harness::new("p zero\ninfo> p one|");
    editor.run(Command::Callout(CalloutEdit::Title(Some("Heads up".into()))));
    editor.run(Command::Undo);
    assert_eq!(callout(&editor), Some(BlockKind::callout(Tone::Info)));

    let outside = Harness::new("p zero|\ninfo> p one");
    for edit in [CalloutEdit::Tone(Tone::Tip), CalloutEdit::Reset, CalloutEdit::Remove] {
        assert!(editor.editor.applies(&Command::Callout(edit.clone())), "{edit:?} inside");
        assert!(!outside.editor.applies(&Command::Callout(edit.clone())), "{edit:?} outside");
    }
}

#[test]
fn the_callout_the_caret_is_in_is_the_one_changed_and_removing_it_leaves_its_text() {
    let mut editor = Harness::new("info> p one\np between\nwarn> p two|\nwarn> ul three");
    editor.run(Command::Callout(CalloutEdit::Tone(Tone::Danger)));
    assert_eq!(editor.state(), canon("info> p one\np between\ndanger> p two|\ndanger> ul three"));
    editor.run(Command::Callout(CalloutEdit::Remove));
    assert_eq!(editor.state(), canon("info> p one\np between\np two|\nul three"));
}

#[test]
fn other_ways_to_change_the_tone_keep_what_was_chosen() {
    let titled = |editor: &mut Harness| editor.run(Command::Callout(CalloutEdit::Title(Some("Heads up".into()))));
    // The block command with another tone, and the tag typed at the top of the callout.
    let mut by_command = Harness::new("info> p one|");
    titled(&mut by_command);
    by_command.run(Command::SetBlock(BlockKind::callout(Tone::Warn)));
    let mut by_tag = Harness::new("info> p |");
    titled(&mut by_tag);
    by_tag.type_text("[!WARN] ");
    for editor in [&by_command, &by_tag] {
        assert_eq!(callout(editor), Some(chosen(Tone::Warn, Some("Heads up"), None, None)));
    }
    // The block command with the tone it has takes the callout away, whatever was chosen for it.
    by_command.run(Command::SetBlock(BlockKind::callout(Tone::Warn)));
    assert_eq!(by_command.state(), canon("p one|"));
}
