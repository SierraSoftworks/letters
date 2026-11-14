mod common;

use common::Do::{self, K, T};
use common::{Harness, canon};
use letters_editor::Command;
use letters_model::{BlockId, Mark};

fn play(before: &str, script: &[Do]) -> String {
    let mut editor = Harness::new(before);
    editor.play(script);
    editor.state()
}

#[test]
fn a_mark_is_switched_on_unless_all_of_the_selection_has_it() {
    let cases = [
        ("p one ^two| three", "Mod+b", "p one ^**two**| three"),
        ("p one ^**two**| three", "Mod+b", "p one ^two| three"),
        ("p ^one **two**| three", "Mod+b", "p ^**one two**| three"),
        ("p o^ne\nhr\n> h2 **two**\n> ul th|ree", "Mod+i", "p o^*ne*\nhr\n> h2 ***two***\n> ul *th*|ree"),
        ("p ^one| two", "Mod+u", "p ^__one__| two"),
        ("p ^one| two", "Mod+e", "p ^`one`| two"),
        ("p ^one| two", "Mod+Shift+x", "p ^~~one~~| two"),
        ("p ^one| two", "Mod+Shift+h", "p ^==one==| two"),
        // Code takes no marks, so a selection that runs through some leaves it be.
        ("p ^one\ncode two\np three|", "Mod+b", "p ^**one**\ncode two\np **three**|"),
    ];
    for (before, key, after) in cases {
        assert_eq!(play(before, &[K(key)]), canon(after), "{key} on {before:?}");
    }
}

#[test]
fn the_marks_in_force_follow_the_selection() {
    let cases = [
        ("p plain **bo|ld**", vec![Mark::Bold]),
        ("p plain **bold|**", vec![Mark::Bold]),
        ("p plain| **bold**", vec![]),
        ("p |**bold**", vec![]),
        ("p ^**bold *both***|", vec![Mark::Bold]),
        ("p **bo^ld** pl|ain", vec![]),
        ("p ^**one**\np **two**|", vec![Mark::Bold]),
    ];
    for (state, marks) in cases {
        assert_eq!(Harness::new(state).marks(), marks.into_iter().collect(), "{state:?}");
    }
}

#[test]
fn markers_show_around_the_spans_the_caret_touches() {
    let cases = [
        ("p plain **bo|ld** plain", "plain **bo|ld** plain"),
        ("p plain **bold|** plain", "plain **bold|** plain"),
        ("p plain |**bold** plain", "plain |**bold** plain"),
        ("p a ***bo|th*** b", "a ***bo|th*** b"),
        ("p **bold *bo|th* bold**", "**bold *bo|th* bold**"),
        ("p ~~a|~~`b`", "~~a|~~`b`"),
        // Out of touch, a span shows as it reads.
        ("p plain| **bold** plain", "plain| bold plain"),
        // A selection shows no markers, so that the text under it holds still.
        ("p ^**bold**|", "bold|"),
        // Code shows its text as written.
        ("code **bo|ld**", "bo|ld"),
    ];
    for (state, drawn) in cases {
        assert_eq!(Harness::new(state).revealed(), drawn, "{state:?}");
    }
}

#[test]
fn each_marker_is_a_stop_for_the_caret() {
    // Left to right through a bold word, and back again.
    let stops = ["a|**b**c", "a**|b**c", "a**b|**c", "a**b**|c", "abc|"];
    let mut editor = Harness::new("p |a**b**c");
    for drawn in stops {
        editor.key("Right");
        assert_eq!(editor.revealed(), drawn);
    }
    for drawn in stops.iter().rev().skip(1) {
        editor.key("Left");
        assert_eq!(editor.revealed(), *drawn);
    }
}

#[test]
fn typing_at_a_marker_lands_on_the_side_the_caret_is_on() {
    // The caret starts before `a` in "a**b**c" and takes one stop for each press.
    let cases = [(1, "p ax|**b**c"), (2, "p a**x|b**c"), (3, "p a**bx|**c"), (4, "p a**b**x|c")];
    for (presses, after) in cases {
        let mut editor = Harness::new("p |a**b**c");
        (0..presses).for_each(|_| _ = editor.key("Right"));
        editor.type_text("x");
        assert_eq!(editor.state(), canon(after), "after {presses} presses");
    }
}

#[test]
fn deleting_either_marker_takes_the_mark_off_the_span() {
    // The caret starts before `a` in "a**b**c" and takes one stop for each press.
    let cases = [
        // Backspace from just inside the opening marker, and from just outside the closing one.
        (2, "Backspace", "a|bc"),
        (4, "Backspace", "ab|c"),
        // Delete from just outside the opening marker, and from just inside the closing one.
        (1, "Delete", "a|bc"),
        (3, "Delete", "ab|c"),
    ];
    for (presses, key, after) in cases {
        let mut editor = Harness::new("p |a**b**c");
        (0..presses).for_each(|_| _ = editor.key("Right"));
        editor.key(key);
        assert_eq!(editor.revealed(), after, "{key} after {presses} presses");
        assert_eq!(editor.text(), "p abc", "{key} after {presses} presses");
    }
}

#[test]
fn an_empty_pair_waits_for_text_and_goes_when_its_marker_is_deleted() {
    let mut editor = Harness::new("p a|c");
    editor.key("Mod+b");
    assert_eq!(editor.revealed(), "a**|**c");
    editor.key("Backspace");
    assert_eq!((editor.revealed().as_str(), editor.state()), ("a|c", canon("p a|c")));

    // Deleting the last character of a span leaves its pair to type into.
    let mut editor = Harness::new("p a**b|**c");
    editor.key("Backspace");
    assert_eq!(editor.revealed(), "a**|**c");
    editor.type_text("d");
    assert_eq!(editor.state(), canon("p a**d|**c"));
}

#[test]
fn a_star_typed_beside_a_revealed_marker_changes_the_pair() {
    let cases = [
        ("p a *b|* c", "p a **b|** c"),
        ("p a |*b* c", "p a |**b** c"),
        ("p a **b|** c", "p a ***b|*** c"),
        // Away from a marker a star is only a star.
        ("p a *b* c|", "p a *b* c\\*|"),
        ("p a ***b|*** c", "p a ***b\\*|*** c"),
    ];
    for (before, after) in cases {
        assert_eq!(play(before, &[T("*")]), canon(after), "from {before:?}");
    }
    // Both sides change at once, wherever the caret was.
    let mut editor = Harness::new("p a *b|* c");
    editor.type_text("*");
    assert_eq!(editor.revealed(), "a **b|** c");
}

#[test]
fn a_heading_shows_its_marker_and_takes_edits_to_it() {
    let marker = |state: &str| Harness::new(state).editor.markers().block;
    assert_eq!(marker("h3 Ti|tle"), Some((BlockId::new("b1"), "###".to_owned())));
    assert_eq!(marker("p Ti|tle"), None);
    assert_eq!(marker("p one\nwarn> ul tw|o"), Some((BlockId::new("b3"), "[!WARN]".to_owned())));

    let cases = [
        ("h1 |Title", "#", "h2 |Title"),
        ("h4 |Title", "#", "h4 |Title"),
        ("h1 Ti|tle", "#", "h1 Ti\\#|tle"),
        ("p |Title", "#", "p \\#|Title"),
    ];
    for (before, typed, after) in cases {
        assert_eq!(play(before, &[T(typed)]), canon(after), "typing {typed:?} into {before:?}");
    }
}

#[test]
fn links_are_set_changed_and_removed() {
    let link = |address: &str| Command::SetLink(Some(address.to_owned()));
    let cases = [
        ("p see ^this| page", link("https://a.example"), "p see ^[this](https://a.example)| page"),
        ("p ^one\np two|", link("https://a.example"), "p ^[one](https://a.example)\np [two](https://a.example)|"),
        // A caret stands for the whole of the link it is in.
        ("p see [th|is](https://a.example) page", link("https://b.example"), "p see [th|is](https://b.example) page"),
        ("p see [this|](https://a.example) page", Command::SetLink(None), "p see this| page"),
        (
            "p see [th^is pa|ge](https://a.example)",
            Command::SetLink(None),
            "p see [th](https://a.example)^is pa|[ge](https://a.example)",
        ),
    ];
    for (before, command, after) in cases {
        let mut editor = Harness::new(before);
        editor.run(command.clone());
        assert_eq!(editor.state(), canon(after), "{command:?} on {before:?}");
    }
    assert_eq!(Harness::new("p see [th|is](https://a.example)").editor.link(), Some("https://a.example"));
    assert_eq!(Harness::new("p se|e [this](https://a.example)").editor.link(), None);
}
