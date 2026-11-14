mod common;

use common::Do::{K, T};
use common::{Do, Harness, SAMPLE, action, canon, check, clip, perform};
use letters_editor::InputEvent;
use proptest::prelude::*;

/// Two writers on `state`: this one with its selection, and another who plays `script` from `theirs`.
/// Returns this writer after taking the other's edits in.
fn merged(state: &str, theirs: &str, script: &[Do]) -> Harness {
    let mut mine = Harness::new(state);
    let mut other = mine.fork(2);
    let (_, selection) = common::parse(theirs);
    other.editor.select(selection.expect("the other writer has a caret"));
    other.play(script);
    assert!(mine.merge(&other));
    mine
}

#[test]
fn the_selection_stays_with_its_text() {
    let cases = [
        // (this writer, the other writer, what they do, this writer afterwards)
        ("p one tw|o", "p |one two", vec![T("zero ")], "p zero one tw|o"),
        ("p one tw|o", "p one two|", vec![T(" three")], "p one tw|o three"),
        ("p one\np tw|o", "p one|\np two", vec![K("Enter"), T("more")], "p one\np more\np tw|o"),
        ("p one\np tw|o", "p one\np |two", vec![K("Backspace")], "p onetw|o"),
        // What is typed at the edges of a selection stays outside it, and at a caret lands after it.
        ("p one ^two| three", "p one |two three", vec![T("a ")], "p one a ^two| three"),
        ("p one ^two| three", "p one two| three", vec![T("s")], "p one ^two|s three"),
        ("p one| two", "p one| two", vec![T("s")], "p one|s two"),
        // Text that is gone leaves the selection where it stood.
        ("p one ^two| three", "p one ^two| three", vec![K("Backspace")], "p one | three"),
        ("p one\nhr\np tw|o", "p one^\nhr\np two|", vec![K("Backspace")], "p one|"),
        ("p one\np tw|o\np three", "p on^e\np two\np th|ree", vec![K("Backspace")], "p on|ree"),
    ];
    for (mine, theirs, script, after) in cases {
        let editor = merged(mine, theirs, &script);
        assert_eq!(editor.state(), canon(after), "{mine:?} with {script:?} at {theirs:?}");
    }
}

#[test]
fn undo_takes_back_only_this_writers_edits() {
    let mut mine = Harness::new("p one|");
    mine.type_text(" two");
    let mut other = mine.fork(2);
    other.play(&[K("Mod+Home"), T("zero ")]);
    assert!(mine.merge(&other));
    mine.type_text(" three");
    assert_eq!(mine.state(), canon("p zero one two three|"));

    let undone: Vec<String> = std::iter::from_fn(|| mine.key("Mod+z").then(|| mine.state())).collect();
    assert_eq!(undone, [canon("p zero one two|"), canon("p zero one|")], "typing went on as one step a word");
    assert!(mine.key("Mod+Shift+z") && mine.key("Mod+Shift+z"));
    assert_eq!(mine.state(), canon("p zero one two three|"));

    // Both writers end on the same document.
    assert!(other.merge(&mine));
    assert_eq!(other.text(), mine.text());
}

#[test]
fn a_composition_goes_on_through_a_merge() {
    let mut mine = Harness::new("p one |two");
    let mut other = mine.fork(2);
    mine.input(InputEvent::SetComposition { text: "か".into(), selection: None });
    other.play(&[K("Mod+Home"), T("zero ")]);
    assert!(mine.merge(&other));

    let (_, range) = mine.editor.composition().expect("the composition is still in progress");
    assert_eq!(&mine.text()[2..][range], "か", "the composed text is where the range says");
    mine.input(InputEvent::Commit("かな".into()));
    assert_eq!(mine.state(), canon("p zero one かな|two"));
    mine.key("Mod+z");
    assert_eq!(mine.state(), canon("p zero one |two"));
}

#[test]
fn a_history_that_cannot_be_read_changes_nothing() {
    let mut editor = Harness::new("p one|");
    assert!(editor.editor.merge(b"not a history").is_err());
    assert_eq!(editor.state(), canon("p one|"));
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    #[test]
    fn two_writers_end_on_the_same_document(
        turns in prop::collection::vec((any::<bool>(), action(), any::<bool>()), 1..40),
    ) {
        let mut first = Harness::new(SAMPLE);
        let mut second = first.fork(2);
        for writer in [&mut first, &mut second] {
            writer.platform.clip = Some(clip("pasted\ntext", None));
        }
        for (mine, action, share) in &turns {
            let (writer, other) = if *mine { (&mut first, &mut second) } else { (&mut second, &mut first) };
            perform(writer, action);
            check(writer)?;
            if *share {
                prop_assert!(other.merge(writer));
                check(other)?;
            }
        }
        // Each writer may also take back all of its own edits, whatever the other has done since.
        for turn in [true, false] {
            let (writer, other) = if turn { (&mut first, &mut second) } else { (&mut second, &mut first) };
            while writer.key("Mod+z") {
                check(writer)?;
            }
            prop_assert!(other.merge(writer));
            check(other)?;
        }
        prop_assert!(first.merge(&second));
        prop_assert_eq!(first.text(), second.text());
    }
}
