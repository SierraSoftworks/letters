use letters::input::{Composer, Press, Translated, translate};
use letters_editor::{InputEvent, Key, KeyEvent, Modifiers};
use winit::event::Ime;
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};

const NONE: ModifiersState = ModifiersState::empty();
const CTRL: ModifiersState = ModifiersState::CONTROL;
const SHIFT: ModifiersState = ModifiersState::SHIFT;
const ALT: ModifiersState = ModifiersState::ALT;
const CMD: ModifiersState = ModifiersState::SUPER;

fn character(text: &str) -> WinitKey {
    WinitKey::Character(text.into())
}

fn named(key: NamedKey) -> WinitKey {
    WinitKey::Named(key)
}

fn press(key: WinitKey, unmodified: WinitKey, text: Option<&str>, mods: ModifiersState, mac: bool) -> Translated {
    translate(Press { key: &key, unmodified: &unmodified, text, mods }, mac)
}

fn key(key: Key, primary: bool, shift: bool, alt: bool) -> Option<KeyEvent> {
    Some(KeyEvent { key, mods: Modifiers { primary, shift, alt } })
}

#[test]
fn typing_is_text_and_not_a_key() {
    let cases = [
        (character("a"), character("a"), "a", NONE),
        (character("A"), character("a"), "A", SHIFT),
        (character("é"), character("e"), "é", NONE),
        (named(NamedKey::Space), named(NamedKey::Space), " ", NONE),
        (character("語"), character("語"), "語", NONE),
    ];
    for (logical, unmodified, text, mods) in cases {
        let translated = press(logical, unmodified, Some(text), mods, false);
        assert_eq!(translated, Translated { key: None, text: Some(text.into()) }, "{text:?}");
    }
}

#[test]
fn editing_keys_are_keys_and_never_type_their_control_characters() {
    let cases = [
        (NamedKey::Enter, Some("\r"), Key::Enter),
        (NamedKey::Tab, Some("\t"), Key::Tab),
        (NamedKey::Backspace, Some("\u{8}"), Key::Backspace),
        (NamedKey::Delete, Some("\u{7f}"), Key::Delete),
        (NamedKey::Escape, Some("\u{1b}"), Key::Escape),
        (NamedKey::ArrowLeft, None, Key::Left),
        (NamedKey::ArrowDown, None, Key::Down),
        (NamedKey::Home, None, Key::Home),
        (NamedKey::PageUp, None, Key::PageUp),
    ];
    for (winit, text, expected) in cases {
        let translated = press(named(winit), named(winit), text, SHIFT, false);
        assert_eq!(translated, Translated { key: key(expected, false, true, false), text: None }, "{winit:?}");
    }
}

#[test]
fn shortcuts_match_the_unshifted_key_under_each_platforms_primary_modifier() {
    let cases = [
        // (logical, unmodified, text, mods, mac, expected key)
        (character("b"), character("b"), Some("\u{2}"), CTRL, false, key(Key::Char('b'), true, false, false)),
        (character("Z"), character("z"), None, CTRL | SHIFT, false, key(Key::Char('z'), true, true, false)),
        (character("b"), character("b"), Some("b"), CMD, true, key(Key::Char('b'), true, false, false)),
        (
            named(NamedKey::Space),
            named(NamedKey::Space),
            Some(" "),
            CTRL,
            false,
            key(Key::Char(' '), true, false, false),
        ),
        // Ctrl is not the primary modifier on macOS, and Cmd is not elsewhere.
        (character("b"), character("b"), None, CTRL, true, None),
        (character("b"), character("b"), None, CMD, false, None),
    ];
    for (logical, unmodified, text, mods, mac, expected) in cases {
        let translated = press(logical.clone(), unmodified, text, mods, mac);
        assert_eq!(translated.key, expected, "{logical:?} with {mods:?}, mac: {mac}");
        if expected.is_some() {
            assert_eq!(translated.text, None, "a shortcut must not also type");
        }
    }
}

#[test]
fn alt_types_only_on_macos() {
    let on_mac = press(character("å"), character("a"), Some("å"), ALT, true);
    assert_eq!(on_mac.text.as_deref(), Some("å"));
    let elsewhere = press(character("a"), character("a"), Some("a"), ALT, false);
    assert_eq!(elsewhere, Translated { key: key(Key::Char('a'), false, false, true), text: None });
}

#[test]
fn unmapped_keys_mean_nothing() {
    for winit in [NamedKey::F5, NamedKey::Shift, NamedKey::CapsLock, NamedKey::AudioVolumeUp] {
        assert_eq!(press(named(winit), named(winit), None, NONE, false), Translated::default(), "{winit:?}");
    }
}

fn compose(events: &[Ime]) -> Vec<InputEvent> {
    let mut composer = Composer::default();
    events.iter().filter_map(|event| composer.event(event)).collect()
}

fn composition(text: &str, selection: Option<(usize, usize)>) -> InputEvent {
    InputEvent::SetComposition { text: text.into(), selection: selection.map(|(start, end)| start..end) }
}

#[test]
fn a_composition_is_shown_then_committed() {
    let preedit = |text: &str, at| Ime::Preedit(text.into(), Some((at, at)));
    let cases: [(&[Ime], &[InputEvent]); 4] = [
        // Japanese: preedit grows, then commits.
        (
            &[Ime::Enabled, preedit("に", 3), preedit("にほん", 9), Ime::Commit("日本".into())],
            &[composition("に", Some((3, 3))), composition("にほん", Some((9, 9))), InputEvent::Commit("日本".into())],
        ),
        // X11 and Wayland clear the preedit before committing: the text still arrives exactly once.
        (
            &[preedit("ni", 2), Ime::Preedit(String::new(), None), Ime::Commit("你".into())],
            &[composition("ni", Some((2, 2))), composition("", None), InputEvent::Insert("你".into())],
        ),
        // An input method that commits with no composition is plain typing.
        (&[Ime::Enabled, Ime::Commit("a".into())], &[InputEvent::Insert("a".into())]),
        // Losing the input method mid-composition drops the composed text.
        (&[preedit("ni", 2), Ime::Disabled], &[composition("ni", Some((2, 2))), composition("", None)]),
    ];
    for (events, expected) in cases {
        assert_eq!(compose(events), expected, "{events:?}");
    }
}

#[test]
fn keys_belong_to_the_input_method_while_composing() {
    let mut composer = Composer::default();
    assert!(!composer.active());
    composer.event(&Ime::Preedit("ni".into(), Some((2, 2))));
    assert!(composer.active());
    composer.event(&Ime::Commit("你".into()));
    assert!(!composer.active());
}
