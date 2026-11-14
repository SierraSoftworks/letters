//! Turns what winit reports into the editor's input: [`KeyEvent`]s for keys and [`InputEvent`]s for text.
//!
//! These adapters are pure, so the rules can be tested without a window.

use letters_editor::{InputEvent, Key, KeyEvent, Modifiers};
use winit::event::Ime;
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};

/// One key press, as winit describes it.
#[derive(Clone, Copy, Debug)]
pub struct Press<'a> {
    pub key: &'a WinitKey,
    /// The key with no modifiers applied, which is what shortcuts are matched on.
    pub unmodified: &'a WinitKey,
    /// The text the press types, if any.
    pub text: Option<&'a str>,
    pub mods: ModifiersState,
}

/// What a key press means. The editor is offered `key` first, and `text` is typed only if it declines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Translated {
    pub key: Option<KeyEvent>,
    pub text: Option<String>,
}

/// `mac` selects Cmd as the primary modifier and lets Option type characters.
pub fn modifiers(state: ModifiersState, mac: bool) -> Modifiers {
    Modifiers {
        primary: if mac { state.super_key() } else { state.control_key() },
        shift: state.shift_key(),
        alt: state.alt_key(),
    }
}

pub fn translate(press: Press<'_>, mac: bool) -> Translated {
    let mods = modifiers(press.mods, mac);
    let key = match press.key {
        WinitKey::Named(named) => named_key(*named),
        _ => None,
    };
    // A character is a key only as part of a shortcut. Typed on its own it is text.
    let shortcut = || match press.unmodified {
        WinitKey::Character(text) => text.chars().next(),
        WinitKey::Named(NamedKey::Space) => Some(' '),
        _ => None,
    };
    let key = key.or_else(|| (mods.primary || mods.alt).then(shortcut).flatten().map(lower).map(Key::Char));

    // Alt types characters only on macOS. Elsewhere AltGr does, and winit does not report that as Alt.
    let types = !mods.primary && (!mods.alt || mac);
    let text = press.text.filter(|text| types && !text.is_empty() && !text.chars().any(char::is_control));
    Translated { key: key.map(|key| KeyEvent { key, mods }), text: text.map(str::to_owned) }
}

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn named_key(key: NamedKey) -> Option<Key> {
    Some(match key {
        NamedKey::Enter => Key::Enter,
        NamedKey::Tab => Key::Tab,
        NamedKey::Backspace => Key::Backspace,
        NamedKey::Delete => Key::Delete,
        NamedKey::Escape => Key::Escape,
        NamedKey::ArrowLeft => Key::Left,
        NamedKey::ArrowRight => Key::Right,
        NamedKey::ArrowUp => Key::Up,
        NamedKey::ArrowDown => Key::Down,
        NamedKey::Home => Key::Home,
        NamedKey::End => Key::End,
        NamedKey::PageUp => Key::PageUp,
        NamedKey::PageDown => Key::PageDown,
        _ => return None,
    })
}

/// Follows an input method through a composition.
#[derive(Clone, Debug, Default)]
pub struct Composer {
    active: bool,
}

impl Composer {
    /// Whether text is being composed. Key presses belong to the input method until it ends.
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn event(&mut self, ime: &Ime) -> Option<InputEvent> {
        match ime {
            Ime::Preedit(text, selection) => {
                let was_active = std::mem::replace(&mut self.active, !text.is_empty());
                let selection = selection.map(|(start, end)| start..end);
                (was_active || self.active).then(|| InputEvent::SetComposition { text: text.clone(), selection })
            }
            // Text committed outside a composition is plain typing.
            Ime::Commit(text) if std::mem::take(&mut self.active) => Some(InputEvent::Commit(text.clone())),
            Ime::Commit(text) => Some(InputEvent::Insert(text.clone())),
            Ime::Disabled if std::mem::take(&mut self.active) => {
                Some(InputEvent::SetComposition { text: String::new(), selection: None })
            }
            Ime::Enabled | Ime::Disabled => None,
        }
    }
}
