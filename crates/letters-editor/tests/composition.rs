mod common;

use common::{Harness, canon};
use letters_editor::InputEvent;

fn compose(text: &str) -> InputEvent {
    InputEvent::SetComposition { text: text.into(), selection: None }
}

fn commit(text: &str) -> InputEvent {
    InputEvent::Commit(text.into())
}

/// Feeds `events` and returns each state passed through.
fn states(before: &str, events: &[InputEvent]) -> Vec<String> {
    let mut editor = Harness::new(before);
    events.iter().map(|event| (editor.input(event.clone()), editor.state()).1).collect()
}

#[test]
fn a_composition_shows_in_place_until_it_is_committed() {
    let events = [compose("k"), compose("ka"), compose("か"), compose("かな"), commit("仮名")];
    let expected = ["p あk|い", "p あka|い", "p あか|い", "p あかな|い", "p あ仮名|い"];
    assert_eq!(states("p あ|い", &events), expected.map(canon));
}

#[test]
fn the_composing_text_is_reported_for_drawing() {
    let mut editor = Harness::new("p ab|c");
    assert_eq!(editor.editor.composition(), None);
    editor.input(compose("xyz"));
    let (block, range) = editor.editor.composition().expect("a composition is under way");
    assert_eq!((block.as_str(), range), ("b1", 2..5));
    editor.input(commit("xyz"));
    assert_eq!(editor.editor.composition(), None);
}

#[test]
fn the_input_method_places_the_caret_inside_the_composition() {
    let event = |selection| InputEvent::SetComposition { text: "かな".into(), selection: Some(selection) };
    let cases = [(0..0, "p a|かなb"), (3..3, "p aか|なb"), (0..6, "p a^かな|b"), (4..99, "p aか^な|b")];
    for (selection, after) in cases {
        assert_eq!(states("p a|b", &[event(selection.clone())]), [canon(after)], "{selection:?}");
    }
}

#[test]
fn a_composition_replaces_the_selection_and_takes_the_marks_at_the_caret() {
    assert_eq!(states("p o^ne tw|o", &[compose("x"), commit("xy")]).last(), Some(&canon("p oxy|o")));
    assert_eq!(states("p **bold|** plain", &[compose("x"), commit("xy")]).last(), Some(&canon("p **boldxy|** plain")));
}

#[test]
fn giving_a_composition_up_leaves_the_document_as_it_was() {
    let cases = [
        ("p a|b", vec![compose("x"), compose("xy"), compose("")]),
        ("p o^ne tw|o", vec![compose("x"), compose("")]),
        ("p a|b", vec![compose("x"), commit("")]),
        ("", vec![compose("x"), compose("")]),
    ];
    for (before, events) in cases {
        let mut editor = Harness::new(before);
        events.into_iter().for_each(|event| _ = editor.input(event));
        assert_eq!(editor.state(), canon(before));
        assert!(!editor.editor.can_undo(), "nothing happened to {before:?}, so nothing is there to undo");
    }
    // Some systems clear the composition before they commit it.
    assert_eq!(states("p a|b", &[compose("x"), compose(""), commit("語")]).last(), Some(&canon("p a語|b")));
}

#[test]
fn a_whole_composition_is_one_undo_step() {
    let mut editor = Harness::new("p a|b");
    [compose("k"), compose("か"), commit("仮")].into_iter().for_each(|event| _ = editor.input(event));
    editor.key("Mod+z");
    assert_eq!(editor.state(), "p a|b");
    editor.key("Mod+Shift+z");
    assert_eq!(editor.state(), "p a仮|b");
}

#[test]
fn anything_else_ends_a_composition_where_it_stands() {
    let mut editor = Harness::new("p a|b");
    editor.input(compose("x"));
    editor.key("Left");
    assert_eq!((editor.state(), editor.editor.composition()), (canon("p a|xb"), None));
    editor.type_text("y");
    editor.key("Mod+z");
    editor.key("Mod+z");
    assert_eq!(editor.state(), "p a|b");
}

#[test]
fn text_committed_with_no_composition_is_typed() {
    assert_eq!(states("p a|b", &[commit("語")]), [canon("p a語|b")]);
}

#[test]
fn autocorrect_replaces_a_range_and_keeps_its_formatting() {
    let replace = |range, text: &str| InputEvent::ReplaceRange { range, text: text.into() };
    let cases = [
        // The caret keeps its place after, inside and before the replacement.
        ("p teh cat|", replace(0..3, "the"), "p the cat|"),
        ("p teh| cat", replace(0..3, "there"), "p there| cat"),
        ("p |a teh", replace(2..5, "the"), "p |a the"),
        ("p **teh** cat|", replace(0..3, "the"), "p **the** cat|"),
        ("p [teh](https://a.example) cat|", replace(0..3, "the"), "p [the](https://a.example) cat|"),
        // An empty range is typing at that place, and a range past the end stops there.
        ("p ab|", replace(1..1, "X"), "p aXb|"),
        ("p ab|", replace(1..99, "X"), "p aX|"),
    ];
    for (before, event, after) in cases {
        assert_eq!(states(before, std::slice::from_ref(&event)), [canon(after)], "{event:?} on {before:?}");
    }
}

#[test]
fn a_soft_keyboard_deletes_around_the_caret() {
    let delete = |before, after| InputEvent::DeleteSurrounding { before, after };
    let cases = [
        ("p abc|def", delete(2, 0), "p a|def"),
        ("p abc|def", delete(0, 2), "p abc|f"),
        ("p abc|def", delete(1, 1), "p ab|ef"),
        // The selection itself stays, and what is asked for past either end is not there to delete.
        ("p a^bcd|ef", delete(1, 1), "p ^bcd|f"),
        ("p abc|def", delete(99, 99), "p |"),
        ("p one\np |two", delete(1, 0), "p one\np |two"),
        // A count that ends inside a character takes the whole of it.
        ("p aé|b", delete(1, 0), "p a|b"),
        ("p a|éb", delete(0, 1), "p a|b"),
    ];
    for (before, event, after) in cases {
        assert_eq!(states(before, std::slice::from_ref(&event)), [canon(after)], "{event:?} on {before:?}");
    }
}
