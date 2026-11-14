mod common;

use common::Do::{self, K, T};
use common::{Harness, canon};
use letters_editor::InputEvent;

fn play(before: &str, script: &[Do]) -> String {
    let mut editor = Harness::new(before);
    editor.play(script);
    editor.state()
}

#[test]
fn shorthand_at_the_start_of_a_block_sets_its_kind() {
    let cases = [
        ("p |", "# ", "h1 |"),
        ("p |Title", "### ", "h3 |Title"),
        ("p |", "- ", "ul |"),
        ("p |", "* ", "ul |"),
        ("p |", "12. ", "ol |"),
        ("p |", "[ ] ", "todo |"),
        ("p |", "[x] ", "done |"),
        ("p |", "- [ ] ", "todo |"),
        ("  ul |nested", "[x] ", "  done |nested"),
        ("p |quoted", "> ", "> p |quoted"),
        ("p before\np |", "---", "p before\nhr\np |"),
        ("> p |", "[!warn]", "warn> p |"),
        ("> p |body", "[!CAUTION]", "danger> p |body"),
        ("tip> p |", "[!INFO]", "info> p |"),
    ];
    for (before, typed, after) in cases {
        assert_eq!(play(before, &[T(typed)]), canon(after), "typing {typed:?} into {before:?}");
    }
}

#[test]
fn shorthand_means_nothing_where_it_does_not_start_a_block() {
    let cases = [
        ("p a |", "# ", "p a \\# |"),
        ("p |", "##### ", "p \\#\\#\\#\\#\\# |"),
        ("p |", "1.5 ", "p 1.5 |"),
        ("ul |", "- ", "ul - |"),
        ("code |", "# ", "code \\# |"),
        ("code |", "**a**", "code \\*\\*a\\*\\*|"),
        ("p |after", "---", "p ---|after"),
        ("> p |", "> ", "> p > |"),
        ("p |", "[!info]", "p \\[!info\\]|"),
        ("> p one\n> p |", "[!info]", "> p one\n> p \\[!info\\]|"),
        ("> p |", "[!what]", "> p \\[!what\\]|"),
    ];
    for (before, typed, after) in cases {
        assert_eq!(play(before, &[T(typed)]), canon(after), "typing {typed:?} into {before:?}");
    }
}

#[test]
fn a_closed_pair_of_markers_formats_the_text_between() {
    let cases = [
        ("p |", "*a*", "p *a|*"),
        ("p |", "**a b**", "p **a b|**"),
        ("p |", "***a***", "p ***a|***"),
        ("p |", "~~a~~", "p ~~a|~~"),
        ("p |", "`a*b`", "p `a\\*b|`"),
        ("h1 see | now", "**this**", "h1 see **this|** now"),
        ("p é |", "*ü*", "p é *ü|*"),
        ("p |", "[docs](https://example.com/a_b)", "p [docs|](https://example.com/a_b)"),
        ("ul a |", "[b](c)", "ul a [b|](c)"),
    ];
    for (before, typed, after) in cases {
        assert_eq!(play(before, &[T(typed)]), canon(after), "typing {typed:?} into {before:?}");
    }
}

#[test]
fn markers_that_are_not_a_pair_stay_as_typed() {
    let cases = [
        // Not yet closed.
        "**a*",
        "***a**",
        "~~a~",
        // Never a pair.
        "**",
        "a*",
        "* a*",
        "*a *",
        "~a~",
        "``",
        "2 * 3 * 4",
        // An image is not a link, and a link needs both of its parts.
        "![alt](image.png)",
        "[](b)",
        "[a]()",
        "[a](b c)",
    ];
    for typed in cases {
        let mut editor = Harness::new("p x |");
        editor.type_text(typed);
        let block = &editor.editor.doc().blocks[0];
        assert_eq!(block.plain_text(), format!("x {typed}"), "{typed:?}");
        assert!(block.body.runs().iter().all(|run| run.marks.is_empty() && run.link.is_none()), "{typed:?}");
    }
}

#[test]
fn typing_goes_on_outside_the_pair_it_closed() {
    assert_eq!(play("p |", &[T("**a** b")]), canon("p **a** b|"));
    assert_eq!(play("p |", &[T("[a](b)c")]), canon("p [a](b)c|"));
}

#[test]
fn backspace_straight_after_shorthand_gives_back_what_was_typed() {
    let cases = [
        ("p |", "# ", "p \\# |"),
        ("p |quoted", "> ", "p > |quoted"),
        ("p |", "---", "p ---|"),
        ("> p |", "[!tip]", "> p \\[!tip\\]|"),
        ("p |", "**a**", "p \\*\\*a\\*\\*|"),
        ("p |", "[a](b)", "p \\[a\\](b)|"),
    ];
    for (before, typed, after) in cases {
        assert_eq!(play(before, &[T(typed), K("Backspace")]), canon(after), "{typed:?} into {before:?}");
        // Undo takes the same single step back, and a soft keyboard's backspace does too.
        assert_eq!(play(before, &[T(typed), K("Mod+z")]), canon(after), "{typed:?} into {before:?}");
        let mut editor = Harness::new(before);
        editor.type_text(typed);
        editor.input(InputEvent::DeleteSurrounding { before: 1, after: 0 });
        assert_eq!(editor.state(), canon(after), "{typed:?} into {before:?}");
    }
}

#[test]
fn backspace_only_gives_shorthand_back_straight_away() {
    // After anything else, Backspace is Backspace again.
    assert_eq!(play("p |", &[T("# a"), K("Backspace"), K("Backspace")]), canon("p |"));
    assert_eq!(play("p |", &[T("# "), K("Right"), K("Backspace")]), canon("p |"));
    assert_eq!(play("p |", &[T("# "), K("Backspace"), K("Backspace")]), canon("p \\#|"));
}

#[test]
fn a_code_fence_becomes_a_code_block_on_enter() {
    let cases = [
        ("p |", "```", "code |"),
        ("p |", "```rust", "code:rust |"),
        ("p |", "```c++", "code:c++ |"),
        ("p text |", "```", "p text \\`\\`\\`\np |"),
        ("p |", "```a b", "p \\`\\`\\`a b\np |"),
    ];
    for (before, typed, after) in cases {
        assert_eq!(play(before, &[T(typed), K("Enter")]), canon(after), "{typed:?} into {before:?}");
    }
    assert_eq!(play("p |", &[T("```rust"), K("Enter"), K("Backspace")]), canon("p \\`\\`\\`rust|"));
}

#[test]
fn a_closing_fence_ends_the_code_block() {
    let cases = [
        ("code let a;⏎|", "```", "code let a;\np |"),
        ("code |", "```", "code\np |"),
        ("> code let a;⏎⏎|", "```", "> code let a;⏎\n> p |"),
        // Only a fence on a line of its own closes, at the end of the block.
        ("code let a; |", "```", "code let a; \\`\\`\\`⏎|"),
        ("code |⏎let a;", "```", "code \\`\\`\\`⏎|⏎let a;"),
        ("code let a;⏎|", "``", "code let a;⏎\\`\\`⏎|"),
    ];
    for (before, typed, after) in cases {
        assert_eq!(play(before, &[T(typed), K("Enter")]), canon(after), "{typed:?} into {before:?}");
    }
    let back = play("code let a;⏎|", &[T("```"), K("Enter"), K("Backspace")]);
    assert_eq!(back, canon("code let a;⏎\\`\\`\\`|"), "Backspace gives the fence back");
}

#[test]
fn shorthand_typed_through_an_input_method_counts_too() {
    let mut editor = Harness::new("p |");
    editor.input(InputEvent::SetComposition { text: "#".into(), selection: None });
    editor.input(InputEvent::Commit("# ".into()));
    assert_eq!(editor.state(), "h1 |");
}
