mod common;

use common::{Harness, canon};
use letters_editor::{Clip, Command};

/// Copies or cuts the selection in `from`, then pastes over the selection in `into`.
fn carry(from: &str, command: Command, into: &str) -> (Harness, Harness) {
    let mut source = Harness::new(from);
    source.run(command);
    let mut target = Harness::new(into);
    target.platform.clip = source.platform.clip.clone();
    target.key("Mod+v");
    (source, target)
}

#[test]
fn a_copy_pastes_as_it_was() {
    let cases = [
        // Part of a block is only text, and pours into the block at the caret.
        ("p one ^**two** th|ree", "h1 A|B", "h1 A**two** th|B"),
        ("h1 on^e t|wo", "p A^B|C", "p Ae t|C"),
        ("p [li^nk|](https://a.example)", "p A|B", "p A[nk|](https://a.example)B"),
        // A whole block brings its kind to an empty paragraph, and only its text anywhere else.
        ("h2 ^Title|", "p |", "h2 Title|"),
        ("h2 ^Title|", "p A|B", "p ATitle|B"),
        // Several blocks split the block at the caret and join it at both edges.
        ("p on^e\nul two\nh1 th|ree", "p A|B", "p Ae\nul two\nh1 th|B"),
        ("p on^e\nul two\nh1 th|ree", "p AB|", "p ABe\nul two\nh1 th|"),
        ("p ^one\nul two|", "p |AB", "p one\nul two|AB"),
        ("p ^one\nhr\n> p two|", "p A|B", "p Aone\nhr\n> p two\np |B"),
        ("p ^one\nhr\np |two", "p A|B", "p Aone\nhr\np |B"),
        // Text copied inside a quote does not bring the quote along.
        ("> p on^e\n> p t|wo", "p A|B", "p Ae\np t|B"),
        ("p one\n> ul t^wo\n> ul thr|ee", "> p A|B", "> p Awo\n> ul thr|B"),
        // Pasting replaces what is selected.
        ("p ^one|", "p A^B\np C|D", "p Aone|D"),
    ];
    for (from, into, after) in cases {
        let (source, target) = carry(from, Command::Copy, into);
        assert_eq!(target.state(), canon(after), "{from:?} into {into:?}");
        assert_eq!(source.state(), canon(from), "copying changed {from:?}");
    }
}

#[test]
fn a_cut_takes_the_selection_away_and_pastes_back_in_place() {
    let cases = [("p one ^**two**| three", "p one | three"), ("h1 on^e\nul two\np th|ree", "h1 on|ree")];
    for (before, after) in cases {
        let mut editor = Harness::new(before);
        let original = editor.text();
        editor.key("Mod+x");
        assert_eq!(editor.state(), canon(after), "cutting {before:?}");
        editor.key("Mod+v");
        assert_eq!(editor.text(), original, "pasting back into {before:?}");
    }
}

#[test]
fn other_applications_are_given_markdown() {
    let cases = [
        ("p on^e **two**\nhr\n> ul th|ree", vec!["e **two**", "---", "> - th"]),
        ("h2 ^A *title*|", vec!["## A *title*"]),
        // A phrase keeps the spaces around it as they were typed.
        ("p one^ **two** |three", vec![" **two** "]),
    ];
    for (from, lines) in cases {
        let (source, _) = carry(from, Command::Copy, "p |");
        let clip = source.platform.clip.expect("copying writes the clipboard");
        let written: Vec<&str> = clip.text.lines().filter(|line| !line.is_empty()).collect();
        assert_eq!(written, lines, "the text of {from:?}");
        assert!(clip.html.is_some_and(|html| html.contains("<p>")), "a copy of {from:?} offers HTML");
    }
}

#[test]
fn text_from_elsewhere_is_read_as_markdown() {
    let cases = [
        ("p A|B", "one", None, "p Aone|B"),
        ("p A|B", "**one** and *two*", None, "p A**one** and *two*|B"),
        ("p |", "# Title\r\n\r\n- one\r\n- two\r\n", None, "h1 Title\nul one\nul two|"),
        ("ul A|B", "one\n\ntwo", None, "ul Aone\np two|B"),
        // A phrase keeps the spaces around it, and takes the formatting it is pasted into as typing would.
        ("p A|B", " one ", None, "p A one |B"),
        ("p A|B", "    one\n", None, "p A    one|B"),
        ("p **A|B**", "one", None, "p **Aone|B**"),
        // Text that is only space has no Markdown to read.
        ("p A|B", "  ", None, "p A  |B"),
        // HTML that Letters did not write is not understood yet, so the text beside it is used.
        ("p A|B", "one", Some("<b>one</b>"), "p Aone|B"),
        // Code takes text as it is written.
        ("code A|B", "one\n  *two*", None, "code Aone⏎  \\*two\\*|B"),
    ];
    for (into, text, html, after) in cases {
        let mut editor = Harness::new(into);
        editor.platform.clip = Some(Clip { text: text.into(), html: html.map(str::to_owned) });
        editor.key("Mod+v");
        assert_eq!(editor.state(), canon(after), "{text:?} into {into:?}");
    }
}

#[test]
fn pasting_plainly_keeps_only_the_text() {
    let mut source = Harness::new("h1 ^**Title**\nul item|");
    source.key("Mod+c");
    let mut target = Harness::new("p A|B");
    target.platform.clip = source.platform.clip.clone();
    target.key("Mod+Shift+v");
    assert_eq!(target.state(), canon("p ATitle\np item|B"));

    // Text from elsewhere is kept as written, Markdown or not.
    let mut written = Harness::new("p A|B");
    written.platform.clip = Some(Clip { text: "# one\n*two*".into(), html: None });
    written.key("Mod+Shift+v");
    assert_eq!(written.state(), canon("p A# one\np \\*two\\*|B"));

    // Code is plain already, so an ordinary paste into it does the same.
    let mut code = Harness::new("code A|B");
    code.platform.clip = source.platform.clip.clone();
    code.key("Mod+v");
    assert_eq!(code.state(), canon("code ATitle⏎item|B"));
}

#[test]
fn an_address_pasted_over_a_selection_links_it() {
    let paste = |into: &str, text: &str| {
        let mut editor = Harness::new(into);
        editor.platform.clip = Some(Clip { text: text.into(), html: None });
        editor.key("Mod+v");
        editor.state()
    };
    let address = "https://example.com/page";
    assert_eq!(paste("p see ^this| page", address), canon("p see ^[this](https://example.com/page)| page"));
    // With nothing selected, or with text that is no address, it is pasted as text.
    assert_eq!(paste("p see |", address), canon("p see https://example.com/page|"));
    assert_eq!(paste("p see ^this| page", "that"), canon("p see that| page"));
}

#[test]
fn copy_and_cut_need_a_selection_and_paste_needs_a_clipboard() {
    let mut editor = Harness::new("p one|");
    assert!(!editor.key("Mod+c") && !editor.key("Mod+x"));
    assert_eq!(editor.platform.clip, None);
    editor.key("Mod+v");
    assert_eq!(editor.state(), "p one|");
}
