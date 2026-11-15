//! References to figures: taking one, typing one, and how the caret and deletion treat one.

mod common;

use common::Do::{self, K, T};
use common::{Harness, canon, check};
use letters_editor::Command;
use letters_model::{Block, BlockId, BlockKind, Mark, Pos, Selection};

fn play(before: &str, script: &[Do]) -> String {
    let mut editor = Harness::new(before);
    editor.play(script);
    editor.state()
}

/// The block each reference in the document names, in document order.
fn targets(editor: &Harness) -> Vec<BlockId> {
    let named = |block: &Block| {
        let marks = block.body.runs().iter().flat_map(|run| run.marks.clone());
        marks.filter_map(|mark| if let Mark::Ref(to) = mark { Some(to) } else { None }).collect::<Vec<_>>()
    };
    editor.editor.doc().walk().flat_map(named).collect()
}

fn figures(editor: &Harness) -> Vec<BlockId> {
    let figure = |block: &&Block| matches!(block.kind, BlockKind::Figure { .. });
    editor.editor.doc().walk().filter(figure).map(|block| block.id.clone()).collect()
}

#[test]
fn numbered_figures_are_listed_in_order_with_their_captions() {
    let editor = Harness::new("fig# Boats\nfig Not counted\ng#> fig\ng#> fig\ng#> cap Two more\np |\nfig# Last");
    let listed = editor.editor.numbered_figures();
    let shown: Vec<(u32, &str)> = listed.iter().map(|figure| (figure.number, figure.caption.as_str())).collect();
    assert_eq!(shown, [(1, "Boats"), (2, "Two more"), (3, "Last")]);
    let kinds: Vec<_> = listed.iter().filter_map(|figure| editor.editor.doc().block(&figure.id)).collect();
    assert!(matches!(kinds[1].kind, BlockKind::Gallery { .. }), "a gallery is referred to as one figure");
}

#[test]
fn the_word_for_a_figure_before_the_caret_asks_for_a_reference() {
    let cases = [
        ("fig# One\np see Figure|", Some(("Figure", None))),
        ("fig# One\np see figure |", Some(("figure ", None))),
        ("fig# One\np see Figure 1| and on", Some(("Figure 1", Some(1)))),
        ("fig# One\np Figure 12|", Some(("Figure 12", Some(12)))),
        // Part of another word, or meant as written.
        ("fig# One\np Configure 1|", None),
        ("fig# One\np see `Figure 1|`", None),
        ("fig# One\ncode Figure 1|", None),
        // A caption's own label, a selection, and a reference that is already made.
        ("fig# One\nfig Figure 2|", None),
        ("fig# One\np see ^Figure 1|", None),
        ("fig# One\np see ⟦Figure 1⟧(b1)|", None),
        // Nothing to refer to.
        ("fig One\np see Figure 1|", None),
    ];
    for (state, expected) in cases {
        let editor = Harness::new(state);
        let found = editor.editor.referring().map(|typed| {
            let block = editor.editor.doc().block(&typed.block).expect("the words are in a block");
            (block.plain_text()[typed.range].to_owned(), typed.number)
        });
        assert_eq!(found, expected.map(|(words, number)| (words.to_owned(), number)), "in {state:?}");
    }
}

#[test]
fn a_reference_is_taken_in_place_of_the_words_typed_for_it() {
    let cases = [
        ("fig# One\np see Figure|.", "b1", "fig# One\np see ⟦Figure 1⟧(b1)|."),
        ("fig# One\np see figure 1|", "b1", "fig# One\np see ⟦Figure 1⟧(b1)|"),
        ("fig# One\np see |", "b1", "fig# One\np see ⟦Figure 1⟧(b1)|"),
        ("fig# One\np see ^this| one", "b1", "fig# One\np see ⟦Figure 1⟧(b1)| one"),
        ("fig# One\nfig# Two\np see Figure 1|", "b2", "fig# One\nfig# Two\np see ⟦Figure 2⟧(b2)|"),
        ("fig# One\np **see Figure|**", "b1", "fig# One\np **see ⟦Figure 1⟧(b1)|**"),
    ];
    for (before, to, after) in cases {
        let mut editor = Harness::new(before);
        editor.run(Command::TakeReference(BlockId::new(to)));
        assert_eq!(editor.state(), canon(after), "referring to {to} in {before:?}");
        assert!(editor.key("Mod+z"), "referring to {to} in {before:?} is a step to undo");
        assert_eq!(editor.text(), Harness::new(before).text(), "referring to {to} in {before:?}, undone");
    }
    // A gallery is referred to as the one figure it is.
    let mut editor = Harness::new("g#> fig\ng#> fig\np |");
    let gallery = editor.editor.numbered_figures()[0].id.clone();
    editor.run(Command::TakeReference(gallery.clone()));
    assert_eq!(targets(&editor), [gallery], "in {:?}", editor.text());
}

#[test]
fn only_a_numbered_figure_is_referred_to_and_only_in_prose() {
    let cases = [
        // Not numbered, not a figure, not there.
        ("fig One\np see |", "b1", true),
        ("fig# One\np se^e| it", "b2", true),
        ("fig# One\np see |", "nope", true),
        // Code is meant as written, and a picture takes no text.
        ("fig# One\ncode see |", "b1", false),
        ("fig# One\np `see|`", "b1", false),
        ("g#> fig*\ng#> fig\np", "b3", false),
    ];
    for (state, to, place) in cases {
        let mut editor = Harness::new(state);
        let command = Command::TakeReference(BlockId::new(to));
        assert_eq!(editor.editor.applies(&command), place, "whether a reference can go where {state:?} has its caret");
        editor.run(command);
        assert_eq!(editor.state(), canon(state), "referring to {to} in {state:?}");
    }
}

#[test]
fn a_figures_number_typed_after_its_word_refers_to_it() {
    let cases = [
        ("fig# One\np see |", &[T("Figure 1 ")][..], "fig# One\np see ⟦Figure 1⟧(b1) |"),
        ("fig# One\np see |", &[T("figure 1.")], "fig# One\np see ⟦Figure 1⟧(b1).|"),
        ("fig# One\nfig# Two\np |", &[T("Figure 2 is")], "fig# One\nfig# Two\np ⟦Figure 2⟧(b2) is|"),
        ("fig# One\np **see |**", &[T("Figure 1 ")], "fig# One\np **see ⟦Figure 1⟧(b1) |**"),
        ("fig# One\nul |", &[T("Figure 1 ")], "fig# One\nul ⟦Figure 1⟧(b1) |"),
        // Enter takes it, and breaks no line.
        ("fig# One\np see |", &[T("Figure 1"), K("Enter")], "fig# One\np see ⟦Figure 1⟧(b1)|"),
    ];
    for (before, script, after) in cases {
        let mut editor = Harness::new(before);
        editor.play(script);
        assert_eq!(editor.state(), canon(after), "{script:?} in {before:?}");
        assert!(check(&editor).is_ok(), "{script:?} in {before:?} leaves the caret inside the reference");
    }
}

#[test]
fn words_stay_words_when_they_name_no_figure_or_are_said_to_be_words() {
    let cases = [
        // No figure has the number, or any number.
        ("fig# One\np see |", &[T("Figure 7 ")][..], "fig# One\np see Figure 7 |"),
        ("fig# One\np see |", &[T("Figure 7"), K("Enter")], "fig# One\np see Figure 7\np |"),
        ("fig One\np see |", &[T("Figure 1 ")], "fig One\np see Figure 1 |"),
        // Another word, and code.
        ("fig# One\np |", &[T("Configure 1 ")], "fig# One\np Configure 1 |"),
        ("fig# One\ncode |", &[T("Figure 1 ")], "fig# One\ncode Figure 1 |"),
        // Esc leaves the words as words.
        ("fig# One\np see |", &[T("Figure 1"), K("Escape"), T(" ")], "fig# One\np see Figure 1 |"),
        ("fig# One\np see |", &[T("Figure 1"), K("Escape"), K("Enter")], "fig# One\np see Figure 1\np |"),
        // Backspace or Undo straight after gives back what was typed.
        ("fig# One\np see |", &[T("Figure 1 "), K("Backspace")], "fig# One\np see Figure 1 |"),
        ("fig# One\np see |", &[T("figure 1."), K("Mod+z")], "fig# One\np see figure 1.|"),
    ];
    for (before, script, after) in cases {
        assert_eq!(play(before, script), canon(after), "{script:?} in {before:?}");
    }
}

#[test]
fn esc_is_for_the_words_at_the_caret_only() {
    let mut editor = Harness::new("fig# One\np see |");
    assert!(!editor.key("Escape"), "Esc has no use where no reference is being typed");
    editor.type_text("Figure");
    assert!(editor.editor.referring().is_some());
    assert!(editor.key("Escape"), "Esc dismisses the words typed for a reference");
    editor.type_text(" 1");
    assert!(editor.editor.referring().is_none(), "the words stay dismissed while they are typed on");
    editor.type_text(" and Figure 1 ");
    assert_eq!(editor.state(), canon("fig# One\np see Figure 1 and ⟦Figure 1⟧(b1) |"));
}

#[test]
fn the_caret_takes_a_reference_in_one_step() {
    let cases = [
        ("p a ⟦Figure 1⟧(f)| b", "Left", "p a |⟦Figure 1⟧(f) b"),
        ("p a |⟦Figure 1⟧(f) b", "Right", "p a ⟦Figure 1⟧(f)| b"),
        ("p a ⟦Figure 1⟧(f)| b", "Shift+Left", "p a |⟦Figure 1⟧(f)^ b"),
        ("p a |⟦Figure 1⟧(f) b", "Shift+Right", "p a ^⟦Figure 1⟧(f)| b"),
        ("p a ⟦Figure 1⟧(f)| b", "Mod+Left", "p a |⟦Figure 1⟧(f) b"),
        ("p a |⟦Figure 1⟧(f) b", "Mod+Right", "p a ⟦Figure 1⟧(f)| b"),
        // Deleting by the word takes all of it.
        ("p a ⟦Figure 1⟧(f)| b", "Mod+Backspace", "p a | b"),
        ("p a |⟦Figure 1⟧(f) b", "Mod+Delete", "p a | b"),
    ];
    for (before, key, after) in cases {
        assert_eq!(play(before, &[K(key)]), canon(after), "{key} in {before:?}");
    }
}

#[test]
fn a_press_inside_a_reference_lands_at_its_nearer_end_and_selects_it_whole() {
    // The reference is bytes 2 to 10 of its paragraph.
    let state = "p a ⟦Figure 1⟧(f) b";
    for (pressed, lands) in [(2, 2), (3, 2), (5, 2), (6, 10), (9, 10), (10, 10)] {
        let mut editor = Harness::new(state);
        editor.editor.select(Selection::caret(Pos::new("b1", pressed)));
        let head = editor.editor.selection().map(|selection| selection.head.offset);
        assert_eq!(head, Some(lands), "a press at byte {pressed}");
        // Typing there is typing beside it.
        editor.type_text("x");
        assert_eq!(targets(&editor).len(), 1, "typing after a press at byte {pressed}");
        assert_eq!(editor.text().matches("⟦Figure 1⟧").count(), 1, "typing after a press at byte {pressed}");
    }
    let mut editor = Harness::new(state);
    editor.editor.select(Selection::caret(Pos::new("b1", 4)));
    editor.run(Command::SelectWord);
    assert_eq!(editor.state(), canon("p a ^⟦Figure 1⟧(f)| b"), "a word picked in a reference is all of it");
}

#[test]
fn text_typed_beside_a_reference_is_no_part_of_it() {
    let cases = [
        ("p a ⟦Figure 1⟧(f)| b", "p a ⟦Figure 1⟧(f)x| b"),
        ("p a |⟦Figure 1⟧(f) b", "p a x|⟦Figure 1⟧(f) b"),
        ("p |⟦Figure 1⟧(f)", "p x|⟦Figure 1⟧(f)"),
        ("p ⟦Figure 1⟧(f)|", "p ⟦Figure 1⟧(f)x|"),
        ("p ⟦Figure 1⟧(f)|⟦Figure 2⟧(g)", "p ⟦Figure 1⟧(f)x|⟦Figure 2⟧(g)"),
    ];
    for (before, after) in cases {
        assert_eq!(play(before, &[T("x")]), canon(after), "typing in {before:?}");
    }
}

#[test]
fn backspace_or_delete_on_a_reference_leaves_the_words_it_shows() {
    let cases = [
        ("fig# One\np a ⟦Figure 1⟧(b1)| b", "Backspace", "fig# One\np a Figure 1| b"),
        ("fig# One\np a |⟦Figure 1⟧(b1) b", "Delete", "fig# One\np a |Figure 1 b"),
        // The words are the figure's number as it is now, and a question mark for a figure that is gone.
        ("fig# Zero\nfig# One\np a ⟦Figure 1⟧(b2)| b", "Backspace", "fig# Zero\nfig# One\np a Figure 2| b"),
        ("fig One\np a ⟦Figure 1⟧(b1)|", "Backspace", "fig One\np a Figure ?|"),
        ("p a |⟦Figure 1⟧(gone) b", "Delete", "p a |Figure ? b"),
    ];
    for (before, key, after) in cases {
        let mut editor = Harness::new(before);
        editor.key(key);
        assert_eq!(editor.state(), canon(after), "{key} in {before:?}");
        // The words do not ask to be a reference again.
        assert_eq!(editor.editor.referring(), None, "{key} in {before:?}");
        editor.type_text(" ");
        assert_eq!(targets(&editor), [], "a space typed after {key} in {before:?}");
    }
}

#[test]
fn a_pasted_reference_follows_its_figure_or_becomes_words() {
    // A copy of a figure is a figure of its own, and the copied words refer to the copy.
    let mut copied = Harness::new("fig# One\np see ⟦Figure 1⟧(b1)|");
    copied.play(&[K("Mod+a"), K("Mod+c"), K("Mod+End"), K("Mod+v")]);
    assert_eq!(figures(&copied).len(), 2, "in {:?}", copied.state());
    assert_eq!(targets(&copied), figures(&copied), "each line refers to the figure above it, in {:?}", copied.state());

    // Words that are cut and pasted still refer to the figure they did.
    let mut cut = Harness::new("fig# One\np ^see ⟦Figure 1⟧(b1)|\np end");
    cut.play(&[K("Mod+x"), K("Mod+End"), K("Mod+v")]);
    assert_eq!(targets(&cut), [BlockId::new("b1")], "in {:?}", cut.text());

    // Words pasted where their figure is not are only words.
    let mut alone = Harness::new("fig# One\np ^see ⟦Figure 1⟧(b1)|\np end");
    alone.key("Mod+c");
    alone.editor.select_picture(&BlockId::new("b1"));
    alone.play(&[K("Backspace"), K("Mod+End"), K("Mod+v")]);
    assert_eq!(alone.text(), canon("p see ⟦Figure 1⟧(b1)\np endsee Figure 1"));
}
