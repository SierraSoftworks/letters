mod common;

use common::Do::{K, T};
use common::{Action, Do, Harness, SAMPLE, action, canon, check, clip, parse, perform, sound};
use letters_editor::{Command, InputEvent, Spot, TableEdit};
use letters_model::{BlockId, BlockKind, Model, ReplicaId};
use proptest::prelude::*;

/// Two writers on `state`: this one with its selection, and another who plays `script` from `theirs`.
/// Returns this writer after taking the other's edits in.
fn merged(state: &str, theirs: &str, script: &[Do]) -> Harness {
    let mut mine = Harness::new(state);
    let mut other = mine.fork(2);
    let (_, selection) = parse(theirs);
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
fn edits_carry_their_writer_and_the_time_the_platform_tells() {
    // On the minute, as edits are dated to the minute.
    const MORNING: u64 = 1_800_000_000;
    let mut ana = Harness::new("p one|");
    let mut ben = ana.fork(2);
    for (writer, name, clock) in [(&mut ana, "Ana", Some(MORNING)), (&mut ben, "Ben", None)] {
        writer.editor.set_author(name);
        writer.platform.clock = clock;
    }
    ana.type_text("s");
    ben.play(&[K("Mod+Home"), T("so")]);
    ben.platform.clock = Some(MORNING + 3600);
    ben.type_text("me");
    assert_eq!(ben.editor.model().authors(), ["Ben".to_owned()].into(), "a writer is named from the first edit");

    // Both changed the one word, which a reader of the two histories is told with who and when.
    let (hers, his) = (ana.editor.model().revision(), ben.editor.model().revision());
    let mut reader = Model::load(&ana.editor.model().save(), ReplicaId::from(3)).unwrap();
    reader.merge(&ben.editor.model().save()).unwrap();
    let found = reader.conflicts(&hers, &his).unwrap();
    let written: Vec<_> =
        found.iter().flat_map(|conflict| &conflict.sides).map(|side| (&*side.author, side.edited)).collect();
    assert_eq!(written, [("Ana", MORNING), ("Ben", MORNING + 3600)]);
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
fn a_file_changed_outside_is_taken_in_around_the_caret() {
    let cases = [
        // (as it is being edited, as another tool left the file, afterwards)
        ("p one tw|o", "p zero one two", "p zero one tw|o"),
        ("p one\np ^two|", "h1 one\nul two more\np three", "h1 one\nul ^two| more\np three"),
        ("p one\np tw|o", "p one", "p one|"),
    ];
    for (mine, theirs, after) in cases {
        let mut editor = Harness::new(mine);
        assert!(editor.editor.rewrite(parse(theirs).0).is_ok());
        assert_eq!(editor.state(), canon(after), "{mine:?} rewritten as {theirs:?}");
    }
}

#[test]
fn the_files_history_is_attached_under_the_editing() {
    let file = "p one two";
    let saved = Model::new(parse(file).0, ReplicaId::from(7)).save();
    // The document is on screen and edited before its history has been read.
    let mut mine = Harness::new("p one| two");
    mine.type_text("s");
    let prepared = Model::new(parse(file).0, ReplicaId::from(1)).prepare(Some(&saved)).expect("the history loads");
    mine.editor.attach(prepared);
    assert_eq!(mine.state(), canon("p ones| two"));

    // From here on it merges with anyone who has the file.
    let mut other = Harness::on(Model::load(&saved, ReplicaId::from(2)).expect("the history loads"), 2);
    other.play(&[K("Mod+Home"), T("zero ")]);
    assert!(mine.merge(&other));
    mine.type_text("!");
    assert_eq!(mine.state(), canon("p zero ones!| two"));
    assert!(other.merge(&mine));
    assert_eq!(other.text(), mine.text());

    // What was typed before the history arrived can still be taken back.
    while mine.key("Mod+z") {}
    assert_eq!(mine.text(), canon("p zero one two"));
}

#[test]
fn a_history_that_cannot_be_read_changes_nothing() {
    let mut editor = Harness::new("p one|");
    assert!(editor.editor.merge(b"not a history").is_err());
    assert_eq!(editor.state(), canon("p one|"));
}

/// The lines of a state in an order of their own, for where two blocks that are alike may stand either way.
fn lines(state: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = state.lines().collect();
    lines.sort_unstable();
    lines
}

#[test]
fn a_block_that_two_writers_moved_at_once_is_two_blocks_that_still_move() {
    let line = |block: &str, below| Spot::Line { block: BlockId::new(block), below };
    // (before, what both writers do at once, what one of them does next, after)
    let cases = [
        // Both put a line in a quote, and one takes its copy out again.
        (
            "p one\np |two",
            vec![Action::Run(Command::SetBlock(BlockKind::Quote))],
            vec![Action::Key("Backspace")],
            "p one\np |two\n> p two",
        ),
        // Both take a line out of a quote, and one quotes its copy again by shorthand.
        (
            "> p |two\n> p three",
            vec![Action::Key("Backspace")],
            vec![Action::Type("> ".into())],
            "p two\n> p |two\n> p three",
        ),
        // Both move a figure to the top, and one moves its copy to the end, where it is left selected.
        (
            "p one\nfig* Boats\np two",
            vec![Action::Run(Command::MoveFigure(line("b1", false)))],
            vec![Action::Run(Command::MoveFigure(line("b3", true)))],
            "fig Boats\np one\np two\nfig* Boats",
        ),
        // Both move a row down, and one moves its copy back up with the caret in it.
        (
            "r1> td A|\nr1> td B\nr2> td C\nr2> td D",
            vec![Action::Run(Command::Table(TableEdit::MoveRow { to: 1 }))],
            vec![Action::Run(Command::Table(TableEdit::MoveRow { to: 0 }))],
            "r1> td A|\nr1> td B\nr2> td C\nr2> td D\nr3> td A\nr3> td B",
        ),
    ];
    for (before, both, next, after) in cases {
        let mut first = Harness::new(before);
        let mut second = first.fork(2);
        // The other writer stands where this one does.
        second.editor.select(first.editor.selection().cloned().expect("the writers have a caret"));
        if let Some(picture) = first.editor.picture().cloned() {
            second.editor.select_picture(&picture);
        }
        for writer in [&mut first, &mut second] {
            both.iter().for_each(|action| perform(writer, action));
        }
        assert!(first.merge(&second) && second.merge(&first));
        let twice = first.text();
        assert_eq!(twice.lines().count(), canon(after).lines().count(), "each writer's block is there: {twice:?}");

        next.iter().for_each(|action| perform(&mut first, action));
        assert_eq!(lines(&first.state()), lines(&canon(after)), "{next:?} after both did {both:?} to {before:?}");
        assert!(check(&first).is_ok(), "{next:?} after both did {both:?} to {before:?}");
        assert!(second.merge(&first));
        assert_eq!(second.text(), first.text(), "the other writer after {next:?}");
        while first.text() != twice && first.key("Mod+z") {}
        assert_eq!(first.text(), twice, "{next:?} undone");
    }
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
            sound(writer)?;
            if *share {
                prop_assert!(other.merge(writer));
                sound(other)?;
            }
        }
        // Each writer may also take back all of its own edits, whatever the other has done since.
        for turn in [true, false] {
            let (writer, other) = if turn { (&mut first, &mut second) } else { (&mut second, &mut first) };
            while writer.key("Mod+z") {
                sound(writer)?;
            }
            prop_assert!(other.merge(writer));
            sound(other)?;
        }
        prop_assert!(first.merge(&second));
        prop_assert_eq!(first.text(), second.text());
    }
}
