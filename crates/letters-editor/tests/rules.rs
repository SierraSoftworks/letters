mod common;

use common::Do::{self, K, T};
use common::{Harness, canon, clip};
use letters_editor::{Command, InputEvent};
use letters_model::{BlockKind, Mark};

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

/// An editor that writes a stretch of text on its own, such as a comment.
fn inline(before: &str) -> Harness {
    let mut editor = Harness::new(before);
    editor.editor.set_inline_only(true);
    editor
}

#[test]
fn a_stretch_of_text_on_its_own_stays_the_block_it_is() {
    // What on a page makes or changes a block, starts maths, or shows a page as a card.
    let cases = [
        "# ",
        "- ",
        "1. ",
        "[ ] ",
        "> ",
        "---",
        "[!info]",
        "```rust\n",
        "$$\n",
        "$x$",
        "$$x$$",
        "| a | b |\n",
        "https://a.example\n",
        "one\ntwo",
    ];
    for typed in cases {
        let mut editor = inline("p |");
        editor.type_text(typed);

        let blocks = &editor.editor.doc().blocks;
        assert_eq!(blocks.len(), 1, "typing {typed:?}");
        assert_eq!(blocks[0].kind, BlockKind::Paragraph, "typing {typed:?}");
        assert_eq!(blocks[0].plain_text(), typed);
        assert!(blocks[0].body.runs().iter().all(|run| run.marks.is_empty()), "typing {typed:?} marks nothing");
    }
    let mut editor = inline("p one|");
    editor.play(&[K("Enter"), T("two")]);
    assert_eq!(editor.editor.doc().blocks.len(), 1, "Enter breaks the line inside the block");

    // Lines pasted as plain text stay in the block too, whatever came with them.
    let mut editor = inline("p one|");
    editor.platform.clip = Some(clip(" two\n\nthree\r\nfour", Some("<h1>two</h1><p>three</p><p>four</p>")));
    editor.run(Command::PastePlain);
    let blocks = &editor.editor.doc().blocks;
    assert_eq!((blocks.len(), blocks[0].plain_text().as_str()), (1, "one two\n\nthree\nfour"));
}

#[test]
fn a_stretch_of_text_on_its_own_takes_the_shorthand_that_marks_text() {
    let cases = [
        ("**a**", "p **a|**"),
        ("*a*", "p *a|*"),
        ("~~a~~", "p ~~a|~~"),
        ("`a`", "p `a|`"),
        ("[a](https://a.example)", "p [a](https://a.example)|"),
        ("https://a.example ", "p [https://a.example](https://a.example) |"),
    ];
    for (typed, after) in cases {
        let mut editor = inline("p |");
        editor.type_text(typed);
        assert_eq!(editor.state(), canon(after), "typing {typed:?}");
        assert!(editor.editor.take_unfetched().is_empty(), "{typed:?}: no page is fetched for a link there");

        // Backspace straight after gives back what was typed, as it does on a page.
        editor.key("Backspace");
        assert_eq!(editor.state(), play("p |", &[T(typed), K("Backspace")]), "{typed:?} given back");
        assert_eq!(editor.editor.doc().blocks[0].plain_text(), typed, "{typed:?} given back");
    }
    // A dollar sign is no start of maths there, so it holds no shorthand back.
    let mut editor = inline("p |");
    editor.type_text("$a *b*");
    let runs = editor.editor.doc().blocks[0].body.runs();
    assert!(runs.iter().any(|run| run.text == "b" && run.marks.contains(&Mark::Italic)), "{}", editor.state());
}
