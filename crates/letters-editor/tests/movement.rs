mod common;

use common::Do::{K, T};
use common::{Do, Harness, canon};
use letters_editor::{Command, Motion};
use letters_model::{Pos, Selection};

/// Presses `keys` from `before` and returns each state passed through.
fn walk(before: &str, keys: &[&str]) -> Vec<String> {
    let mut editor = Harness::new(before);
    keys.iter().map(|key| (editor.key(key), editor.state()).1).collect()
}

#[test]
fn arrows_move_by_what_a_reader_sees_as_one_character() {
    let cases = [
        ("p a|é👩‍👧z", "Right", "p aé|👩‍👧z"),
        ("p aé|👩‍👧z", "Right", "p aé👩‍👧|z"),
        ("p aé👩‍👧|z", "Left", "p aé|👩‍👧z"),
        // Blocks follow one another, and what holds no text is stepped over.
        ("p one|\nhr\n> p two", "Right", "p one\nhr\n> p |two"),
        ("p one\nhr\n> p |two", "Left", "p one|\nhr\n> p two"),
        // The ends of the document are a wall.
        ("p |one", "Left", "p |one"),
        ("p one|", "Right", "p one|"),
    ];
    for (before, key, after) in cases {
        assert_eq!(walk(before, &[key]), [canon(after)], "{key} from {before:?}");
    }
}

#[test]
fn words_lines_blocks_and_the_document_have_their_own_keys() {
    let cases = [
        ("p one, two| three", "Mod+Left", "p one, |two three"),
        ("p one, |two three", "Mod+Left", "p |one, two three"),
        ("p one|, two three", "Mod+Right", "p one, two| three"),
        ("p one\np |two", "Mod+Left", "p one|\np two"),
        ("p one|\np two", "Mod+Right", "p one\np |two"),
        ("p one\np tw|o\np three", "Home", "p one\np |two\np three"),
        ("p one\np tw|o\np three", "End", "p one\np two|\np three"),
        ("code one⏎tw|o⏎three", "Home", "code one⏎|two⏎three"),
        ("code one⏎tw|o⏎three", "End", "code one⏎two|⏎three"),
        ("p one\np tw|o", "Mod+Up", "p one\np |two"),
        ("p one\np |two", "Mod+Up", "p |one\np two"),
        ("p o|ne\np two", "Mod+Down", "p one|\np two"),
        ("p one\nhr\np tw|o", "Mod+Home", "p |one\nhr\np two"),
        ("p o|ne\nhr\np two", "Mod+End", "p one\nhr\np two|"),
    ];
    for (before, key, after) in cases {
        assert_eq!(walk(before, &[key]), [canon(after)], "{key} from {before:?}");
    }
}

#[test]
fn without_a_layout_up_and_down_follow_line_breaks_and_keep_their_column() {
    let cases = [
        ("p one\np two th|ree\np 12", &["Up"][..], "p one|\np two three\np 12"),
        // The column is remembered across a line too short to hold it.
        ("p one long line\np 12\np three wor|ds", &["Up", "Up"], "p one long |line\np 12\np three words"),
        ("code ab|c⏎d⏎efg", &["Down", "Down"], "code abc⏎d⏎ef|g"),
        // Past the first and last line the caret goes to the edge of its line.
        ("p on|e\np two", &["Up"], "p |one\np two"),
        ("p one\np t|wo", &["Down"], "p one\np two|"),
        ("p a\np b\np c\np |d", &["PageUp"], "p |a\np b\np c\np d"),
    ];
    for (before, keys, after) in cases {
        assert_eq!(walk(before, keys).last(), Some(&canon(after)), "{keys:?} from {before:?}");
    }
}

#[test]
fn with_a_layout_movement_follows_its_lines() {
    // Every line is five characters wide.
    let cases = [
        ("p abcdefg|hijklmn", &["Up"][..], "p ab|cdefghijklmn"),
        ("p abcdefg|hijklmn", &["Down"], "p abcdefghijkl|mn"),
        ("p abcdefg|hijklmn", &["Home"], "p abcde|fghijklmn"),
        ("p abcdefg|hijklmn", &["End"], "p abcdefghij|klmn"),
        ("p abcdefg|hijklmn", &["Shift+End"], "p abcdefg^hij|klmn"),
        // The column holds through the short last line of the first block.
        ("p abcdefg\np abcd|efghi", &["Up", "Up"], "p abcd|efg\np abcdefghi"),
        ("p abcd|efg\np abcdefghi", &["Down", "Down", "Down"], "p abcdefg\np abcdefghi|"),
        ("p abcdefghijklmnopqrstuvw|xyz", &["PageUp"], "p abcdefgh|ijklmnopqrstuvwxyz"),
        // A wrapped line of code is not its last, and the one after it is.
        ("code ab|cdefghi", &["Down"], "code abcdefg|hi"),
        ("code ab|cdefghi", &["Down", "Down"], "code abcdefghi\np |"),
    ];
    for (before, keys, after) in cases {
        let mut editor = Harness::wrapped(before, 5);
        keys.iter().for_each(|key| _ = editor.key(key));
        assert_eq!(editor.state(), canon(after), "{keys:?} from {before:?}");
    }
}

#[test]
fn an_arrow_off_the_end_of_the_document_makes_a_line_to_leave_by() {
    let cases = [
        // Code is left by its last line, and an empty last line is not left behind.
        ("code one⏎tw|o", "Down", "code one⏎two\np |"),
        ("code one⏎two|", "Right", "code one⏎two\np |"),
        ("code one⏎|", "Down", "code one\np |"),
        ("code on|e⏎two", "Up", "p |\ncode one⏎two"),
        ("code |one", "Left", "p |\ncode one"),
        // A quote or a callout is left from the last line in it.
        ("> p one\n> p tw|o", "Down", "> p one\n> p two\np |"),
        ("tip> ul one|", "Right", "tip> ul one\np |"),
        ("> p o|ne\n> p two", "Up", "p |\n> p one\n> p two"),
        // Code in a quote is left first, and then the quote. An empty line moves out itself.
        ("> code on|e", "Down", "> code one\n> p |"),
        ("> code one\n> p |", "Down", "> code one\np |"),
        ("> p |", "Down", "p |"),
        // What holds no text is no place to stop, so the line is made past it.
        ("p on|e\nhr", "Down", "p one\nhr\np |"),
        ("hr\np on|e", "Up", "p |\nhr\np one"),
        ("p |\nhr", "Down", "hr\np |"),
    ];
    for (before, key, after) in cases {
        let mut editor = Harness::new(before);
        editor.key(key);
        assert_eq!(editor.state(), canon(after), "{key} from {before:?}");
        editor.key("Mod+z");
        assert_eq!(editor.state(), canon(before), "undoing {key} from {before:?}");
    }
}

#[test]
fn an_arrow_makes_no_line_where_there_is_one_or_nothing_to_leave() {
    let cases = [
        // Body text, headings and lists end the document as they are.
        ("p on|e", "Down", "p one|"),
        ("h1 one|", "Right", "h1 one|"),
        ("ul on|e", "Up", "ul |one"),
        // With text beyond, the caret goes to it.
        ("code on|e⏎two\np three", "Down", "code one⏎tw|o\np three"),
        ("code one⏎two|\n> p three", "Right", "code one⏎two\n> p |three"),
        ("> p one|\np", "Down", "> p one\np |"),
        // A selection is extended or collapsed, never led out.
        ("> p o^n|e", "Shift+Down", "> p o^ne|"),
        ("code o^n|e", "Right", "code on|e"),
    ];
    for (before, key, after) in cases {
        let mut editor = Harness::new(before);
        editor.key(key);
        assert_eq!(editor.state(), canon(after), "{key} from {before:?}");
        assert!(!editor.editor.can_undo(), "{key} from {before:?} edited the document");
    }
}

#[test]
fn a_click_below_everything_gets_a_line_to_type_on() {
    // The document, where the caret ends up, and whether a line had to be made.
    let cases = [
        ("code o|ne", "code one\np |", true),
        ("> p o|ne", "> p one\np |", true),
        ("> code o|ne", "> code one\np |", true),
        ("p o|ne\nhr", "p one\nhr\np |", true),
        ("hr", "hr\np |", true),
        ("", "p |", true),
        // Text the caret can simply be at the end of needs none.
        ("p o|ne\nul two", "p one\nul two|", false),
        ("h1 o^ne|\np", "h1 one\np |", false),
    ];
    for (before, after, made) in cases {
        let mut editor = Harness::new(before);
        editor.run(Command::LineBelow);
        assert_eq!(editor.state(), canon(after), "below {before:?}");
        assert_eq!(editor.editor.can_undo(), made, "below {before:?}");
        editor.key("Mod+z");
        assert_eq!(editor.text(), canon(before).replace(['^', '|'], ""), "undoing the line below {before:?}");
    }
}

#[test]
fn shift_extends_the_selection_and_a_plain_arrow_collapses_it() {
    let cases = [
        ("p one |two", &["Shift+Right", "Shift+Right"][..], "p one ^tw|o"),
        ("p one |two", &["Shift+Left", "Shift+Mod+Left"], "p |one ^two"),
        ("p on|e\np two", &["Shift+Down"], "p on^e\np tw|o"),
        ("p one\np tw|o", &["Shift+Mod+Home"], "p |one\np tw^o"),
        // Shrinking back to nothing leaves a caret.
        ("p one |two", &["Shift+Right", "Shift+Left"], "p one |two"),
        ("p o^ne tw|o", &["Left"], "p o|ne two"),
        ("p o|ne tw^o", &["Left"], "p o|ne two"),
        ("p o^ne tw|o", &["Right"], "p one tw|o"),
        ("p o^ne\np tw|o\np three", &["Down"], "p one\np two\np th|ree"),
        ("p one\np t^wo\np thr|ee", &["Up"], "p o|ne\np two\np three"),
    ];
    for (before, keys, after) in cases {
        assert_eq!(walk(before, keys).last(), Some(&canon(after)), "{keys:?} from {before:?}");
    }
}

#[test]
fn commands_select_words_blocks_and_everything() {
    let cases = [
        ("p one tw|o three", Command::SelectWord, "p one ^two| three"),
        ("p one two|", Command::SelectWord, "p one ^two|"),
        ("p |", Command::SelectWord, "p |"),
        ("p one\nul tw|o", Command::SelectBlock, "p one\nul ^two|"),
        ("p one\nhr\n> p tw|o", Command::SelectAll, "p ^one\nhr\n> p two|"),
    ];
    for (before, command, after) in cases {
        let mut editor = Harness::new(before);
        editor.run(command.clone());
        assert_eq!(editor.state(), canon(after), "{command:?} from {before:?}");
    }
}

#[test]
fn a_selection_from_outside_is_brought_onto_text() {
    let mut editor = Harness::new("p |héllo\nhr");
    let select = |editor: &mut Harness, block: &str, offset| {
        editor.editor.select(Selection::caret(Pos::new(block, offset)));
        editor.state()
    };
    // Inside a character, past the end, on a divider and nowhere at all.
    assert_eq!(select(&mut editor, "b1", 2), canon("p h|éllo\nhr"));
    assert_eq!(select(&mut editor, "b1", 99), canon("p héllo|\nhr"));
    assert_eq!(select(&mut editor, "b2", 0), canon("p héllo|\nhr"));
    assert_eq!(select(&mut editor, "nope", 0), canon("p héllo|\nhr"));
}

/// Plays `script` on a page whose lines are five characters wide and wrap with no space.
fn narrow(before: &str, script: &[Do]) -> Harness {
    let mut editor = Harness::wrapped(before, 5);
    editor.play(script);
    editor
}

#[test]
fn the_end_of_a_line_that_wraps_with_no_space_is_a_place_of_its_own() {
    // One offset ends a line and starts the next. The flag says the caret is at the end of the first.
    let cases = [
        ("p ab|cdefghijklmn", &[K("End")][..], "p abcde|fghijklmn", true),
        // End stays where it is, and Home is the start of the line the caret is shown on.
        ("p ab|cdefghijklmn", &[K("End"), K("End")], "p abcde|fghijklmn", true),
        ("p ab|cdefghijklmn", &[K("End"), K("Home")], "p |abcdefghijklmn", false),
        ("p ab|cdefghijklmn", &[K("Shift+End")], "p ab^cde|fghijklmn", true),
        ("p ab|cdefghijklmn", &[K("Shift+End"), K("Shift+End")], "p ab^cde|fghijklmn", true),
        ("p ab|cdefghijklmn", &[K("Shift+End"), K("Shift+Home")], "p |ab^cdefghijklmn", false),
        // The same offset come to from the next line is that line's start.
        ("p abcdef|ghijklmn", &[K("Left")], "p abcde|fghijklmn", false),
        ("p abcdefg|hijklmn", &[K("Home"), K("Home")], "p abcde|fghijklmn", false),
        ("p abcdefg|hijklmn", &[K("Home"), K("End")], "p abcdefghij|klmn", true),
        // Up and down keep to the ends of lines, and the last line ends where its text does.
        ("p ab|cdefghijklmn", &[K("End"), K("Down")], "p abcdefghij|klmn", true),
        ("p abcdefg|hijklmn", &[K("End"), K("Up")], "p abcde|fghijklmn", true),
        ("p ab|cdefghijklmn", &[K("End"), K("Down"), K("Down")], "p abcdefghijklmn|", false),
        // A page is three lines.
        ("p ab|cdefghijklmnopqrstuvwxy", &[K("End"), K("PageDown")], "p abcdefghijklmnopqrst|uvwxy", true),
        ("p ab|cdefghijklmnopqrstuvwxy", &[K("End"), K("Shift+PageDown")], "p abcde^fghijklmnopqrst|uvwxy", true),
        // A step sideways leaves the end of a line as it leaves any place.
        ("p ab|cdefghijklmn", &[K("End"), K("Left")], "p abcd|efghijklmn", false),
        ("p ab|cdefghijklmn", &[K("End"), K("Right")], "p abcdef|ghijklmn", false),
        // A selection that ends there collapses to where its end is shown.
        ("p ab|cdefghijklmn", &[K("Shift+End"), K("Right")], "p abcde|fghijklmn", true),
        ("p ab|cdefghijklmn", &[K("Shift+End"), K("Left")], "p ab|cdefghijklmn", false),
        // What is typed or deleted there is at the end of the line, and undoing it leads back there.
        ("p ab|cdefghijklmn", &[K("End"), T("x")], "p abcdex|fghijklmn", false),
        ("p ab|cdefghijklmn", &[K("End"), K("Backspace")], "p abcd|fghijklmn", false),
        ("p ab|cdefghijklmn", &[K("End"), T("x"), K("Mod+z")], "p abcde|fghijklmn", true),
        // The caret stays on its line for what moves it nowhere.
        ("p ab|cdefghijklmn", &[K("End"), K("Mod+b")], "p abcde|fghijklmn", true),
        ("p ab|cdefghijklmn", &[K("End"), K("Delete")], "p abcde|ghijklmn", true),
    ];
    for (before, script, after, before_wrap) in cases {
        let editor = narrow(before, script);
        assert_eq!(editor.state(), canon(after), "{script:?} from {before:?}");
        assert_eq!(editor.editor.before_wrap(), before_wrap, "the line after {script:?} from {before:?}");
    }
}

#[test]
fn a_line_is_deleted_up_to_the_edge_the_caret_is_shown_at() {
    let cases = [
        ("p ab|cdefghijklmn", &[][..], Motion::LineEnd, "p ab|fghijklmn"),
        ("p ab|cdefghijklmn", &[K("End")], Motion::LineStart, "p |fghijklmn"),
        ("p ab|cdefghijklmn", &[K("End")], Motion::LineEnd, "p abcde|fghijklmn"),
        ("p abcdefg|hijklmn", &[K("Home")], Motion::LineStart, "p abcde|fghijklmn"),
    ];
    for (before, script, motion, after) in cases {
        let mut editor = narrow(before, script);
        editor.run(Command::Delete(motion));
        assert_eq!(editor.state(), canon(after), "deleting to {motion:?} after {script:?} from {before:?}");
    }
}

#[test]
fn a_press_at_a_wrap_is_on_the_line_that_layout_says() {
    // The same offset, as a press past the end of the first line gives it and as one before the second.
    let cases = [(true, "p |abcdefghijklmn", "p abcde|fghijklmn"), (false, "p abcde|fghijklmn", "p abcdefghij|klmn")];
    for (before_wrap, home, end) in cases {
        let pressed = |key: &str| {
            let mut editor = narrow("p |abcdefghijklmn", &[]);
            editor.editor.select_at(Selection::caret(Pos::new("b1", 5)), before_wrap);
            assert_eq!(editor.editor.before_wrap(), before_wrap);
            editor.key(key);
            editor.state()
        };
        assert_eq!(pressed("Home"), canon(home), "Home, before the wrap: {before_wrap}");
        assert_eq!(pressed("End"), canon(end), "End, before the wrap: {before_wrap}");
    }
    // A drag that ends past the line's end is on that line too, and a plain press takes the caret off it.
    let mut editor = narrow("p |abcdefghijklmn", &[]);
    editor.editor.select_at(Selection { anchor: Pos::new("b1", 2), head: Pos::new("b1", 5) }, true);
    editor.key("Shift+Home");
    assert_eq!(editor.state(), canon("p |ab^cdefghijklmn"));
    editor.editor.select_at(Selection::caret(Pos::new("b1", 5)), true);
    editor.editor.select(Selection::caret(Pos::new("b1", 5)));
    assert!(!editor.editor.before_wrap(), "a press at the start of the next line");
}

#[test]
fn a_word_is_picked_on_the_line_the_caret_is_shown_on() {
    // Lines of five characters: `well-`, `known`, `-word`, and `abcde`, `fghij`. The caret is put where the
    // first line wraps, on one side of it. The last flag says whether the word picked ends on that line.
    let cases = [
        ("p |well-known-word", true, "p well^-|known-word", true),
        ("p |well-known-word", false, "p well-^known|-word", false),
        // A word that the wrap runs through is picked whole from either side.
        ("p |abcdefghij", true, "p ^abcdefghij|", false),
        ("p |abcdefghij", false, "p ^abcdefghij|", false),
    ];
    for (before, before_wrap, after, ends_before_wrap) in cases {
        let mut editor = narrow(before, &[]);
        editor.editor.select_at(Selection::caret(Pos::new("b1", 5)), before_wrap);
        editor.run(Command::SelectWord);
        assert_eq!(editor.state(), canon(after), "{before:?}, before the wrap: {before_wrap}");
        assert_eq!(editor.editor.before_wrap(), ends_before_wrap, "{before:?}, before the wrap: {before_wrap}");
    }
}
