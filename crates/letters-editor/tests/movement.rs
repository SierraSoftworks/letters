mod common;

use common::{Harness, canon};
use letters_editor::Command;
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
    ];
    for (before, keys, after) in cases {
        let mut editor = Harness::wrapped(before, 5);
        keys.iter().for_each(|key| _ = editor.key(key));
        assert_eq!(editor.state(), canon(after), "{keys:?} from {before:?}");
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
