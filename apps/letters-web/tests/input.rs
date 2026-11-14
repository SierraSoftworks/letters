use letters_editor::{InputEvent, Key, KeyEvent, Modifiers};
use letters_web::input::{Clipboard, Dom, Held, Reply, TextInput};

const NONE: Held = Held { ctrl: false, meta: false, shift: false, alt: false, alt_graph: false };
const CTRL: Held = Held { ctrl: true, ..NONE };
const CMD: Held = Held { meta: true, ..NONE };
const SHIFT: Held = Held { shift: true, ..NONE };
const ALT: Held = Held { alt: true, ..NONE };

fn with(a: Held, b: Held) -> Held {
    Held { ctrl: a.ctrl || b.ctrl, meta: a.meta || b.meta, shift: a.shift || b.shift, alt: a.alt || b.alt, ..NONE }
}

fn press<'a>(key: &'a str, code: &'a str, held: Held) -> Dom<'a> {
    Dom::KeyDown { key, code, held, ime: false }
}

fn key(key: Key, primary: bool, shift: bool, alt: bool) -> KeyEvent {
    KeyEvent { key, mods: Modifiers { primary, shift, alt } }
}

fn typed(kind: &'static str, data: &'static str) -> Dom<'static> {
    Dom::BeforeInput { kind, data: Some(data) }
}

/// The hidden input holding `value`, with the caret after it.
fn holds(value: &str) -> Dom<'_> {
    let caret = value.encode_utf16().count() as u32;
    Dom::Input { value, selection: caret..caret }
}

/// A composition showing `text`, with the caret after it.
fn composition(text: &str) -> InputEvent {
    InputEvent::SetComposition { text: text.into(), selection: Some(text.len()..text.len()) }
}

/// The text events a run of DOM events sends the editor.
fn text_of(input: &mut TextInput, events: &[Dom<'_>]) -> Vec<InputEvent> {
    let replies = events.iter().map(|event| input.event(event.clone()));
    replies.filter_map(|reply| if let Reply::Text(text) = reply { Some(text) } else { None }).collect()
}

#[test]
fn typing_is_left_to_the_browser_and_arrives_as_text() {
    let alt_gr = Held { ctrl: true, alt: true, alt_graph: true, ..NONE };
    let cases =
        [("a", "KeyA", NONE), ("A", "KeyA", SHIFT), ("é", "KeyE", NONE), (" ", "Space", NONE), ("@", "KeyQ", alt_gr)];
    for (text, code, held) in cases {
        let mut input = TextInput::new(false);
        assert_eq!(input.event(press(text, code, held)), Reply::Pass, "{text:?}");
        let reply = input.event(Dom::BeforeInput { kind: "insertText", data: Some(text) });
        assert_eq!(reply, Reply::Text(InputEvent::Insert(text.into())), "{text:?}");
    }
}

#[test]
fn editing_keys_are_keys_the_browser_must_not_act_on() {
    let cases = [
        ("Enter", Key::Enter),
        ("Backspace", Key::Backspace),
        ("Delete", Key::Delete),
        ("ArrowLeft", Key::Left),
        ("ArrowDown", Key::Down),
        ("Home", Key::Home),
        ("PageUp", Key::PageUp),
    ];
    for (name, expected) in cases {
        let reply = TextInput::new(false).event(press(name, name, SHIFT));
        assert_eq!(reply, Reply::Key { event: key(expected, false, true, false), claim: true }, "{name}");
    }
}

#[test]
fn tab_and_escape_stay_the_browsers_unless_the_editor_uses_them() {
    for (name, expected) in [("Tab", Key::Tab), ("Escape", Key::Escape)] {
        let reply = TextInput::new(false).event(press(name, name, NONE));
        assert_eq!(reply, Reply::Key { event: key(expected, false, false, false), claim: false }, "{name}");
    }
}

#[test]
fn shortcuts_are_matched_on_the_character_the_key_types_unmodified() {
    let cases = [
        // (what the browser reports, held, macOS, the key the editor is offered)
        (("b", "KeyB"), CTRL, false, key(Key::Char('b'), true, false, false)),
        (("X", "KeyX"), with(CTRL, SHIFT), false, key(Key::Char('x'), true, true, false)),
        // A layout that moves the letters is followed.
        (("b", "KeyN"), CTRL, false, key(Key::Char('b'), true, false, false)),
        // Shift and Option change what a key types. The shortcut is still the key's own character.
        (("&", "Digit7"), with(CTRL, SHIFT), false, key(Key::Char('7'), true, true, false)),
        ((">", "Period"), with(CMD, SHIFT), true, key(Key::Char('.'), true, true, false)),
        (("¡", "Digit1"), with(CMD, ALT), true, key(Key::Char('1'), true, false, true)),
        (("ç", "KeyC"), with(CMD, ALT), true, key(Key::Char('c'), true, false, true)),
        ((" ", "Space"), CTRL, false, key(Key::Char(' '), true, false, false)),
    ];
    for ((typed, code), held, mac, expected) in cases {
        let reply = TextInput::new(mac).event(press(typed, code, held));
        assert_eq!(reply, Reply::Key { event: expected, claim: false }, "{typed:?} on {code}");
    }
}

#[test]
fn the_primary_modifier_is_cmd_on_macos_and_ctrl_elsewhere() {
    let cases = [(CTRL, false, true), (CMD, false, false), (CMD, true, true), (CTRL, true, false)];
    for (held, mac, is_shortcut) in cases {
        let reply = TextInput::new(mac).event(press("b", "KeyB", held));
        assert_eq!(matches!(reply, Reply::Key { .. }), is_shortcut, "{held:?}, macOS: {mac}");
    }
}

#[test]
fn clipboard_shortcuts_are_left_to_the_browser_to_raise_as_events() {
    let mut input = TextInput::new(false);
    for (typed, code, held) in [("c", "KeyC", CTRL), ("x", "KeyX", CTRL), ("v", "KeyV", CTRL)] {
        assert_eq!(input.event(press(typed, code, held)), Reply::Pass, "{typed}");
    }
    assert_eq!(input.clipboard_key(Clipboard::Copy), key(Key::Char('c'), true, false, false));
    assert_eq!(input.clipboard_key(Clipboard::Cut), key(Key::Char('x'), true, false, false));
    assert_eq!(input.clipboard_key(Clipboard::Paste), key(Key::Char('v'), true, false, false));
}

#[test]
fn shift_asks_for_a_paste_without_formatting_until_the_next_plain_paste() {
    let mut input = TextInput::new(false);
    assert_eq!(input.event(press("V", "KeyV", with(CTRL, SHIFT))), Reply::Pass);
    assert_eq!(input.clipboard_key(Clipboard::Paste), key(Key::Char('v'), true, true, false));
    assert_eq!(input.event(press("v", "KeyV", CTRL)), Reply::Pass);
    assert_eq!(input.clipboard_key(Clipboard::Paste), key(Key::Char('v'), true, false, false));
}

#[test]
fn a_composition_is_followed_from_its_first_key_to_its_commit() {
    let ime = |key| Dom::KeyDown { key, code: "", held: NONE, ime: true };
    let commit = |text: &str| InputEvent::Commit(text.into());
    let traces: [(&str, &[Dom<'_>], &[InputEvent]); 4] = [
        (
            // Firefox ends the composition and then reports its text once more.
            "Firefox, pinyin",
            &[
                ime("Process"),
                Dom::CompositionStart,
                holds("n"),
                ime("Process"),
                holds("ni"),
                Dom::CompositionEnd { data: "你" },
                holds("你"),
            ],
            &[composition("n"), composition("ni"), commit("你")],
        ),
        (
            // Chromium reports the final text in an `input` event before the composition ends.
            "Chromium, kana",
            &[
                ime("Process"),
                Dom::CompositionStart,
                typed("insertCompositionText", "に"),
                holds("に"),
                typed("insertCompositionText", "日"),
                holds("日"),
                Dom::CompositionEnd { data: "日" },
            ],
            &[composition("に"), composition("日"), commit("日")],
        ),
        (
            // Safari sends the Enter that picked the candidate after the composition has ended.
            "Safari, late Enter",
            &[Dom::CompositionStart, holds("ㅎ"), Dom::CompositionEnd { data: "한" }, ime("Enter")],
            &[composition("ㅎ"), commit("한")],
        ),
        (
            "given up",
            &[Dom::CompositionStart, holds("n"), Dom::CompositionEnd { data: "" }],
            &[composition("n"), InputEvent::SetComposition { text: String::new(), selection: None }],
        ),
    ];
    for (name, events, expected) in traces {
        let mut input = TextInput::new(false);
        assert_eq!(text_of(&mut input, events), expected, "{name}");
        assert!(!input.composing(), "{name}");
    }
}

#[test]
fn keys_pressed_during_a_composition_belong_to_the_input_method() {
    let mut input = TextInput::new(false);
    input.event(Dom::CompositionStart);
    for name in ["Enter", "Backspace", "ArrowLeft", "Escape"] {
        assert_eq!(input.event(press(name, name, NONE)), Reply::Pass, "{name}");
    }
    assert!(input.composing());
}

#[test]
fn the_caret_inside_a_composition_is_counted_in_bytes() {
    let cases = [("日本語", 1..2, 3..6), ("😀a", 2..3, 4..5), ("abc", 0..3, 0..3), ("é", 5..9, 2..2)];
    for (value, utf16, bytes) in cases {
        let mut input = TextInput::new(false);
        input.event(Dom::CompositionStart);
        let reply = input.event(Dom::Input { value, selection: utf16 });
        let expected = InputEvent::SetComposition { text: value.into(), selection: Some(bytes) };
        assert_eq!(reply, Reply::Text(expected), "{value:?}");
    }
}

#[test]
fn edits_made_without_a_key_press_become_the_keys_that_make_them() {
    let cases = [
        // (inputType, macOS, the key)
        ("insertParagraph", false, key(Key::Enter, false, false, false)),
        ("insertLineBreak", false, key(Key::Enter, false, true, false)),
        ("deleteContentBackward", false, key(Key::Backspace, false, false, false)),
        ("deleteContentForward", true, key(Key::Delete, false, false, false)),
        ("deleteWordBackward", false, key(Key::Backspace, true, false, false)),
        ("deleteWordBackward", true, key(Key::Backspace, false, false, true)),
        ("deleteWordForward", true, key(Key::Delete, false, false, true)),
        ("historyUndo", false, key(Key::Char('z'), true, false, false)),
        ("historyRedo", true, key(Key::Char('z'), true, true, false)),
    ];
    for (kind, mac, expected) in cases {
        let reply = TextInput::new(mac).event(Dom::BeforeInput { kind, data: None });
        assert_eq!(reply, Reply::Key { event: expected, claim: true }, "{kind}, macOS: {mac}");
    }
}

#[test]
fn edits_the_shell_has_no_meaning_for_never_reach_the_hidden_input() {
    for kind in ["insertFromPaste", "insertFromDrop", "formatBold", "deleteByCut"] {
        let reply = TextInput::new(false).event(Dom::BeforeInput { kind, data: None });
        assert_eq!(reply, Reply::Drop, "{kind}");
    }
}

#[test]
fn text_from_dictation_or_a_replacement_is_typed() {
    for kind in ["insertReplacementText", "insertFromYank", "insertText"] {
        let reply = TextInput::new(false).event(typed(kind, "word"));
        assert_eq!(reply, Reply::Text(InputEvent::Insert("word".into())), "{kind}");
    }
}

#[test]
fn text_the_browser_inserted_without_asking_is_still_typed() {
    let reply = TextInput::new(false).event(holds("x"));
    assert_eq!(reply, Reply::Text(InputEvent::Insert("x".into())));
}
