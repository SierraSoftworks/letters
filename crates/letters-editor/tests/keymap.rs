mod common;

use common::{Harness, canon, key};
use letters_editor::{Command, KeyEvent, Keymap, Modifiers, Motion};
use letters_model::Mark;

fn systems() -> [(&'static str, Keymap); 2] {
    [("standard", Keymap::standard()), ("mac", Keymap::mac())]
}

#[test]
fn each_system_moves_by_its_own_keys() {
    let to = |to| Command::Move { to, extend: false };
    let cases = [
        (Keymap::standard(), "Mod+Left", to(Motion::WordLeft)),
        (Keymap::standard(), "End", to(Motion::LineEnd)),
        (Keymap::standard(), "Mod+Home", to(Motion::DocStart)),
        (Keymap::standard(), "Mod+Backspace", Command::Delete(Motion::WordLeft)),
        (Keymap::standard(), "Mod+y", Command::Redo),
        (Keymap::mac(), "Alt+Left", to(Motion::WordLeft)),
        (Keymap::mac(), "Mod+Right", to(Motion::LineEnd)),
        (Keymap::mac(), "Mod+Up", to(Motion::DocStart)),
        (Keymap::mac(), "Alt+Backspace", Command::Delete(Motion::WordLeft)),
        (Keymap::mac(), "Mod+Backspace", Command::Delete(Motion::LineStart)),
    ];
    for (keymap, name, command) in cases {
        assert_eq!(keymap.command(&key(name)), Some(&command), "{name}");
    }
}

#[test]
fn every_system_shares_the_common_shortcuts() {
    let cases = [
        ("Left", Command::Move { to: Motion::Left, extend: false }),
        ("Backspace", Command::Delete(Motion::Left)),
        ("Enter", Command::Split),
        ("Shift+Enter", Command::LineBreak),
        ("Tab", Command::Indent),
        ("Mod+b", Command::ToggleMark(Mark::Bold)),
        ("Mod+z", Command::Undo),
        ("Mod+Shift+z", Command::Redo),
        ("Mod+v", Command::Paste),
        ("Mod+Shift+v", Command::PastePlain),
    ];
    for (system, keymap) in systems() {
        for (name, command) in &cases {
            assert_eq!(keymap.command(&key(name)), Some(command), "{name} on {system}");
        }
    }
}

#[test]
fn shift_extends_the_selection_with_every_motion() {
    for (system, keymap) in systems() {
        for (event, command) in keymap.bindings() {
            if let Command::Move { to, extend: false } = command {
                let shifted = KeyEvent { key: event.key.clone(), mods: Modifiers { shift: true, ..event.mods } };
                let extends = Command::Move { to: *to, extend: true };
                assert_eq!(keymap.command(&shifted), Some(&extends), "{event:?} on {system}");
            }
        }
    }
}

#[test]
fn a_menu_can_show_the_key_beside_a_command() {
    let keymap = Keymap::standard();
    assert_eq!(keymap.key(&Command::ToggleMark(Mark::Bold)), Some(&key("Mod+b")));
    assert_eq!(keymap.key(&Command::SelectWord), None, "a command with no key shows none");
}

#[test]
fn keys_can_be_bound_again() {
    let mut keymap = Keymap::standard();
    let (word, enter) = (key("Mod+d"), key("Enter"));
    keymap.bind(word.key, word.mods, Command::SelectWord);
    keymap.bind(enter.key, enter.mods, Command::LineBreak);
    keymap.unbind(&key("Mod+b"));

    let mut editor = Harness::new("p one tw|o");
    editor.editor.set_keymap(keymap);
    assert!(editor.key("Mod+d"));
    assert_eq!(editor.state(), canon("p one ^two|"));
    assert!(editor.key("Enter"));
    assert_eq!(editor.state(), canon("p one ⏎|"), "the key runs its new command and not its old one");
    assert!(!editor.key("Mod+b"), "a key that is bound to nothing is left to the shell");
}

#[test]
fn a_key_with_nothing_to_do_is_left_to_the_shell() {
    let cases = [
        // (a place where the key means nothing, a place where it does, the key)
        ("p one|", "ul one|", "Tab"),
        ("p o^ne\np tw|o", "p o^ne\nul tw|o", "Tab"),
        ("p o^ne\np tw|o", "ul o^ne\np tw|o", "Shift+Tab"),
        ("p one|", "todo one|", "Mod+Enter"),
        ("p one|", "p ^one|", "Mod+c"),
        ("code one|", "p one|", "Mod+b"),
    ];
    for (idle, used, name) in cases {
        let mut editor = Harness::new(idle);
        assert!(!editor.key(name), "{name} in {idle:?}");
        assert_eq!(editor.state(), canon(idle), "{name} in {idle:?}");
        assert!(Harness::new(used).key(name), "{name} in {used:?}");
    }
    // Text arrives as input, so a plain character is no key of the editor's.
    assert!(!Harness::new("p one|").key("q") && !Harness::new("p one|").key("Escape"));
}
