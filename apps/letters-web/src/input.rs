//! Turns what the hidden input reports into the editor's input: [`KeyEvent`]s for keys and [`InputEvent`]s
//! for text.
//!
//! Every browser takes the same path. Keys are read from `keydown`, text from `beforeinput`, and a
//! composition from the `input` events between `compositionstart` and `compositionend`. The rules are pure,
//! so a recorded run of events can be replayed without a browser.

use std::ops::Range;

use letters_editor::{InputEvent, Key, KeyEvent, Modifiers};

/// The modifier keys held during an event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub ctrl: bool,
    pub meta: bool,
    pub shift: bool,
    pub alt: bool,
    /// AltGr, which types characters. Windows reports it as Ctrl and Alt together.
    pub alt_graph: bool,
}

/// `mac` selects Cmd as the primary modifier and lets Option type characters.
pub fn modifiers(held: Held, mac: bool) -> Modifiers {
    Modifiers {
        primary: !held.alt_graph && if mac { held.meta } else { held.ctrl },
        shift: held.shift,
        alt: !held.alt_graph && held.alt,
    }
}

/// One event on the hidden input, reduced to what the rules read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dom<'a> {
    /// `keydown`. `ime` says the press belongs to an input method: `isComposing`, or key code 229.
    KeyDown {
        key: &'a str,
        code: &'a str,
        held: Held,
        ime: bool,
    },
    /// `beforeinput`, with its `inputType` and `data`.
    BeforeInput {
        kind: &'a str,
        data: Option<&'a str>,
    },
    /// `input`: what the hidden input now holds, and the selection in it in UTF-16 units.
    Input {
        value: &'a str,
        selection: Range<u32>,
    },
    CompositionStart,
    CompositionEnd {
        data: &'a str,
    },
}

/// What an event means for the editor and for the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Nothing for the editor. The browser carries on, which is how a key becomes text, reaches an input
    /// method or raises a clipboard event.
    Pass,
    /// Nothing for the editor, and the browser must not act on it either.
    Drop,
    /// A key to offer the editor. The browser's own action is stopped if the editor uses the key, or
    /// whether it does or not when `claim` is set.
    Key { event: KeyEvent, claim: bool },
    /// Text for the editor. The browser's own edit is stopped where it can be.
    Text(InputEvent),
}

/// A clipboard event on the hidden input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clipboard {
    Copy,
    Cut,
    Paste,
}

/// Follows the hidden input through keys, typing and composition.
///
/// Outside a composition the hidden input is kept empty, so whatever it holds during one is the composition.
#[derive(Clone, Debug)]
pub struct TextInput {
    mac: bool,
    composing: bool,
    /// Whether a composition has just ended. Firefox reports its text once more in an `input` event.
    ended: bool,
    /// Whether the last clipboard shortcut held Shift, which asks for a paste without formatting.
    plain: bool,
}

impl TextInput {
    /// `mac` follows macOS conventions: Cmd is the primary modifier, and Option moves by word.
    pub fn new(mac: bool) -> Self {
        Self { mac, composing: false, ended: false, plain: false }
    }

    /// Whether text is being composed. Until it ends, the hidden input holds the composition.
    pub fn composing(&self) -> bool {
        self.composing
    }

    pub fn event(&mut self, event: Dom<'_>) -> Reply {
        let ended = std::mem::take(&mut self.ended);
        match event {
            Dom::KeyDown { ime, .. } if ime || self.composing => Reply::Pass,
            Dom::KeyDown { key, code, held, .. } => self.key_down(key, code, held),
            Dom::BeforeInput { .. } if self.composing => Reply::Pass,
            Dom::BeforeInput { kind, data } => self.before_input(kind, data),
            Dom::Input { value, selection } if self.composing => {
                let caret = byte_offset(value, selection.start)..byte_offset(value, selection.end);
                Reply::Text(InputEvent::SetComposition { text: value.into(), selection: Some(caret) })
            }
            // Text the browser put in the input without asking first.
            Dom::Input { value, .. } if !ended && !value.is_empty() => Reply::Text(InputEvent::Insert(value.into())),
            Dom::Input { .. } => Reply::Pass,
            Dom::CompositionStart => {
                self.composing = true;
                Reply::Pass
            }
            Dom::CompositionEnd { data } => {
                self.ended = true;
                // An input method may end a composition it never showed, which is plain typing.
                match std::mem::take(&mut self.composing) {
                    true if data.is_empty() => {
                        Reply::Text(InputEvent::SetComposition { text: String::new(), selection: None })
                    }
                    true => Reply::Text(InputEvent::Commit(data.into())),
                    false => Reply::Pass,
                }
            }
        }
    }

    /// The key that asks the editor for what a clipboard event carries out.
    pub fn clipboard_key(&self, action: Clipboard) -> KeyEvent {
        let (key, shift) = match action {
            Clipboard::Copy => ('c', false),
            Clipboard::Cut => ('x', false),
            Clipboard::Paste => ('v', self.plain),
        };
        KeyEvent { key: Key::Char(key), mods: Modifiers { primary: true, shift, alt: false } }
    }

    fn key_down(&mut self, key: &str, code: &str, held: Held) -> Reply {
        let mods = modifiers(held, self.mac);
        if let Some(key) = named_key(key) {
            // Tab and Escape stay the browser's when the editor has no use for them, so focus can leave.
            let claim = !matches!(key, Key::Tab | Key::Escape);
            return Reply::Key { event: KeyEvent { key, mods }, claim };
        }
        // A character is a key only as part of a shortcut. Typed on its own it is text.
        let Some(key) = (mods.primary || mods.alt).then(|| shortcut(key, code, mods)).flatten() else {
            return Reply::Pass;
        };
        // The browser turns these into clipboard events, the only place the clipboard can be reached.
        if mods.primary && !mods.alt && (key == 'v' || (!mods.shift && matches!(key, 'c' | 'x'))) {
            self.plain = mods.shift;
            return Reply::Pass;
        }
        Reply::Key { event: KeyEvent { key: Key::Char(key), mods }, claim: false }
    }

    /// An edit the browser is about to make for something other than a key press this shell has handled,
    /// such as a soft keyboard, a menu or dictation.
    fn before_input(&self, kind: &str, data: Option<&str>) -> Reply {
        let press = |key, primary, shift, alt| Reply::Key {
            event: KeyEvent { key, mods: Modifiers { primary, shift, alt } },
            claim: true,
        };
        // Deleting a word takes Option on macOS and Ctrl elsewhere.
        let word = |key| press(key, !self.mac, false, self.mac);
        match kind {
            "insertLineBreak" => press(Key::Enter, false, true, false),
            "insertParagraph" => press(Key::Enter, false, false, false),
            "deleteContentBackward" => press(Key::Backspace, false, false, false),
            "deleteContentForward" => press(Key::Delete, false, false, false),
            "deleteWordBackward" => word(Key::Backspace),
            "deleteWordForward" => word(Key::Delete),
            "historyUndo" => press(Key::Char('z'), true, false, false),
            "historyRedo" => press(Key::Char('z'), true, true, false),
            // These cannot be stopped. The composition's own events say what they did.
            _ if kind.contains("Composition") => Reply::Pass,
            _ if kind.starts_with("insert") => match data.filter(|data| !data.is_empty()) {
                Some(data) => Reply::Text(InputEvent::Insert(data.into())),
                None => Reply::Drop,
            },
            _ => Reply::Drop,
        }
    }
}

fn named_key(key: &str) -> Option<Key> {
    Some(match key {
        "Enter" => Key::Enter,
        "Tab" => Key::Tab,
        "Backspace" => Key::Backspace,
        "Delete" => Key::Delete,
        "Escape" => Key::Escape,
        "ArrowLeft" => Key::Left,
        "ArrowRight" => Key::Right,
        "ArrowUp" => Key::Up,
        "ArrowDown" => Key::Down,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        _ => return None,
    })
}

/// The character a shortcut is matched on: the one its key types with nothing held.
///
/// The browser reports only what the key types with the modifiers held. A letter is still that letter, in
/// either case. Anything else is read from where the key sits, as Shift and Option turn `7` into `&` or `¶`.
fn shortcut(key: &str, code: &str, mods: Modifiers) -> Option<char> {
    let mut typed = key.chars();
    let typed = typed.next().filter(|_| typed.next().is_none());
    let letter = typed.filter(|c| c.is_alphabetic() && !mods.alt);
    let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
    letter.or_else(|| position(code)).or(typed).map(lower)
}

/// What a key types unshifted on a US keyboard, for the keys shortcuts use.
fn position(code: &str) -> Option<char> {
    let last = code.chars().next_back()?;
    match code {
        _ if code.len() == 4 && code.starts_with("Key") => Some(last.to_ascii_lowercase()),
        _ if code.len() == 6 && code.starts_with("Digit") => Some(last),
        "Minus" => Some('-'),
        "Period" => Some('.'),
        "Comma" => Some(','),
        "Slash" => Some('/'),
        "Space" => Some(' '),
        _ => None,
    }
}

/// The UTF-8 offset in `text` of an offset in UTF-16 units, which is how the browser counts.
fn byte_offset(text: &str, utf16: u32) -> usize {
    let mut units = 0;
    for (at, c) in text.char_indices() {
        if units >= utf16 {
            return at;
        }
        units += c.len_utf16() as u32;
    }
    text.len()
}
