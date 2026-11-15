mod common;

use common::Do::{self, K, T};
use common::{Harness, canon};
use letters_editor::Command;
use letters_model::{BlockKind, ListKind, Tone};

fn play(before: &str, script: &[Do]) -> String {
    let mut editor = Harness::new(before);
    editor.play(script);
    editor.state()
}

#[test]
fn enter_breaks_the_block_at_the_caret() {
    let cases = [
        ("p Hello| world", "p Hello\np | world"),
        ("p Hello|", "p Hello\np |"),
        ("p |Hello", "p\np |Hello"),
        ("p He^llo wor|ld", "p He\np |ld"),
        ("p **bo|ld**", "p **bo**\np **|ld**"),
        // Body text follows a heading, and a heading pushed down leaves body text behind.
        ("h2 Title|", "h2 Title\np |"),
        ("h2 Ti|tle", "h2 Ti\nh2 |tle"),
        ("h2 |Title", "p\nh2 |Title"),
        // A list goes on, and an empty item steps out of it one level at a time.
        ("  ol one|", "  ol one\n  ol |"),
        ("ul one\n  ul |", "ul one\nul |"),
        ("ul one\nul |", "ul one\np |"),
        // The tick stays with the text it was given to.
        ("done one| two", "done one\ntodo | two"),
        ("done |one", "todo\ndone |one"),
        // A quote goes on, and its empty last line steps out of it.
        ("> p one|", "> p one\n> p |"),
        ("> p one\n> p |", "> p one\np |"),
        ("> p |", "p |"),
        ("> p |\n> p two", "> p\n> p |\n> p two"),
        // Code keeps its lines, the empty ones too.
        ("code let a;|", "code let a;⏎|"),
        ("code let a;⏎|", "code let a;⏎⏎|"),
        ("code let a;⏎|⏎let b;", "code let a;⏎⏎|⏎let b;"),
    ];
    for (before, after) in cases {
        assert_eq!(play(before, &[K("Enter")]), canon(after), "from {before:?}");
    }
}

#[test]
fn shift_enter_breaks_the_line_inside_the_block() {
    assert_eq!(play("ul one| two", &[K("Shift+Enter")]), canon("ul one⏎| two"));
}

#[test]
fn backspace_and_delete_remove_characters_words_and_selections() {
    let cases = [
        ("p ab👩‍👧|c", "Backspace", "p ab|c"),
        ("p a|é👩‍👧", "Delete", "p a|👩‍👧"),
        ("p one two| three", "Mod+Backspace", "p one | three"),
        ("p one| two three", "Mod+Delete", "p one| three"),
        ("p o^ne tw|o", "Backspace", "p o|o"),
        ("p o^ne tw|o", "Delete", "p o|o"),
        ("p o^ne tw|o", "Mod+Backspace", "p o|o"),
        ("h1 On^e\nhr\n> p two\n> ul th|ree\np four", "Backspace", "h1 On|ree\np four"),
        // Text left after the first block was emptied keeps its own block.
        ("h1 ^One\np two\nul th|ree", "Delete", "ul |ree"),
        ("h1 ^One\np two|", "Delete", "h1 |"),
        ("p |", "Backspace", "p |"),
        ("p one|", "Delete", "p one|"),
    ];
    for (before, key, after) in cases {
        assert_eq!(play(before, &[K(key)]), canon(after), "{key} from {before:?}");
    }
}

#[test]
fn backspace_at_the_start_of_a_block_undoes_the_block_before_joining_it() {
    let cases = [
        // One `#` at a time, and then body text.
        ("h3 |Title", "h2 |Title"),
        ("h1 |Title", "p |Title"),
        ("  ul |item", "ul |item"),
        ("todo |item", "p |item"),
        ("code:rust |let a;", "p |let a;"),
        // A paragraph joins the text before it, wherever that is.
        ("p one\np |two", "p one|two"),
        ("h1 one\np |two", "h1 one|two"),
        ("> p one\np |two", "> p one|two"),
        ("> p one\n> p |two", "> p one|two"),
        ("p **one**\np |*two*", "p **one|***two*"),
        // An empty block before it gives way instead.
        ("h1\np |two", "p |two"),
        // What holds no text is removed whole.
        ("p one\nhr\np |two", "p one\np |two"),
        // The tag of a callout goes first, then the block leaves the quote.
        ("tip> p |one\ntip> p two", "> p |one\n> p two"),
        ("> p |one\n> p two", "p |one\n> p two"),
        ("> p |one", "p |one"),
        ("p |one", "p |one"),
    ];
    for (before, after) in cases {
        assert_eq!(play(before, &[K("Backspace")]), canon(after), "from {before:?}");
    }
}

#[test]
fn delete_at_the_end_of_a_block_joins_the_next_one() {
    let cases = [
        ("p one|\nh1 two", "p one|two"),
        ("p one|\n> ul two\n> p three", "p one|two\n> p three"),
        ("> p one|\np two", "> p one|two"),
        ("p |\nh1 two", "h1 |two"),
        ("p one|\nhr\np two", "p one|\np two"),
        ("p one|\n> p two", "p one|two"),
    ];
    for (before, after) in cases {
        assert_eq!(play(before, &[K("Delete")]), canon(after), "from {before:?}");
    }
}

#[test]
fn tab_nests_list_items_and_steps_code() {
    let cases = [
        ("ul one\nul |two", "Tab", "ul one\n  ul |two"),
        // An item nests at most one level below the one before it.
        ("ul one\n  ul |two", "Tab", "ul one\n  ul |two"),
        ("ul |one", "Tab", "ul |one"),
        ("ul one\nul ^two\nul th|ree", "Tab", "ul one\n  ul ^two\n  ul th|ree"),
        ("ul one\n  ul |two", "Shift+Tab", "ul one\nul |two"),
        ("ul |one", "Shift+Tab", "ul |one"),
        ("code if a {⏎|b", "Tab", "code if a {⏎    |b"),
        ("code if a {⏎    b|", "Shift+Tab", "code if a {⏎b|"),
    ];
    for (before, key, after) in cases {
        assert_eq!(play(before, &[K(key)]), canon(after), "{key} from {before:?}");
    }
}

#[test]
fn blocks_change_kind_on_command() {
    let bullet = BlockKind::ListItem { list: ListKind::Bullet, depth: 0 };
    let task = BlockKind::ListItem { list: ListKind::Task { checked: false }, depth: 0 };
    let cases = [
        ("p one|", BlockKind::Heading { level: 2 }, "h2 one|"),
        ("p o^ne\nh1 two\nul th|ree", bullet.clone(), "ul o^ne\nul two\nul th|ree"),
        // Asking for the kind a block already has gives a paragraph back.
        ("h2 one|", BlockKind::Heading { level: 2 }, "p one|"),
        ("  ul one|", bullet, "p one|"),
        // A list keeps its nesting when it changes kind.
        ("  ul one|", task, "  todo one|"),
        ("p let a;|", BlockKind::Code { language: Some("rust".into()) }, "code:rust let a;|"),
        // Quotes and callouts wrap, change and unwrap.
        ("p o^ne\np tw|o\np three", BlockKind::Quote, "> p o^ne\n> p tw|o\np three"),
        ("> p one|\n> p two", BlockKind::callout(Tone::Warn), "warn> p one|\nwarn> p two"),
        ("p zero\n> p one|\n> p two", BlockKind::Quote, "p zero\np one|\np two"),
        // A divider goes below the block, or takes the place of an empty paragraph.
        ("p one|\np two", BlockKind::Divider, "p one\nhr\np |two"),
        ("p one|", BlockKind::Divider, "p one\nhr\np |"),
        ("p one\np |", BlockKind::Divider, "p one\nhr\np |"),
    ];
    for (before, kind, after) in cases {
        let mut editor = Harness::new(before);
        editor.run(Command::SetBlock(kind.clone()));
        assert_eq!(editor.state(), canon(after), "{kind:?} on {before:?}");
    }
}

#[test]
fn a_task_is_ticked_from_the_keyboard() {
    assert_eq!(play("todo one|", &[K("Mod+Enter")]), canon("done one|"));
    assert_eq!(play("done one|", &[K("Mod+Enter"), T("!")]), canon("todo one!|"));
}
