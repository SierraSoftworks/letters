//! Turns what the hidden input reports into the editor's input: [`KeyEvent`]s for keys and [`InputEvent`]s
//! for text.
//!
//! Every browser takes the same path. Keys are read from `keydown`, text from `beforeinput`, and a
//! composition from the `input` events between `compositionstart` and `compositionend`. A soft keyboard
//! reports little of that, so the hidden input rests on one space: what the keyboard does to it shows in
//! the `input` event, when nothing said so beforehand. [`Touch`] reads fingers the same way: scrolls, flicks
//! and taps from pointer events. The rules are pure, so a recorded run of events can be replayed without a
//! browser.

use std::ops::Range;

use letters_editor::{InputEvent, Key, KeyEvent, Modifiers};
use letters_render::Point;

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
    /// `beforeinput`, with its `inputType` and `data`. `cancelable` says whether the edit can be stopped,
    /// which a soft keyboard often does not allow.
    BeforeInput {
        kind: &'a str,
        data: Option<&'a str>,
        cancelable: bool,
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
/// Outside a composition the hidden input holds [`TextInput::REST`] alone, so whatever follows that during
/// one is the composition.
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
    /// What the hidden input holds between edits, with the caret after it. A soft keyboard reports a
    /// backspace only when there is something to delete, and a space is not a word it would try to correct.
    pub const REST: &'static str = " ";

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
            // An edit that cannot be stopped is read from the `input` event that follows it.
            Dom::BeforeInput { cancelable, .. } if self.composing || !cancelable => Reply::Pass,
            Dom::BeforeInput { kind, data, .. } => self.before_input(kind, data),
            Dom::Input { value, selection } if self.composing => {
                let text = value.strip_prefix(Self::REST).unwrap_or(value);
                let lead = (value.len() - text.len()) as u32;
                let offset = |at: u32| byte_offset(text, at.saturating_sub(lead));
                let caret = offset(selection.start)..offset(selection.end);
                Reply::Text(InputEvent::SetComposition { text: text.into(), selection: Some(caret) })
            }
            Dom::Input { .. } if ended => Reply::Pass,
            Dom::Input { value, .. } => unasked(value),
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
            // A soft keyboard's Enter. One from a real keyboard was a key press, with or without Shift.
            "insertLineBreak" | "insertParagraph" => press(Key::Enter, false, false, false),
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

/// What the browser did to the resting hidden input without asking first, read from what it holds now.
fn unasked(value: &str) -> Reply {
    let press = |key| Reply::Key { event: KeyEvent { key, mods: Modifiers::default() }, claim: true };
    match value.strip_prefix(TextInput::REST) {
        Some("") => Reply::Pass,
        Some("\n") => press(Key::Enter),
        Some(added) => Reply::Text(InputEvent::Insert(added.into())),
        // The space it rested on is gone, which is a backspace.
        None if value.is_empty() => press(Key::Backspace),
        None => Reply::Text(InputEvent::Insert(value.into())),
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

/// How far a finger may wander and still be tapping, in logical pixels.
const SLOP: f32 = 10.0;
/// How long a finger may stay down and still be tapping, in milliseconds. The browser makes a longer press
/// a request for a menu.
const LONG_MS: f64 = 500.0;
/// A finger that rested this long before lifting was placing the page, not flicking it.
const REST_MS: f64 = 100.0;
/// The slowest flick that carries the page on, and the speed at which a glide is over, in pixels a
/// millisecond.
const GLIDE_FROM: f32 = 0.1;
const GLIDE_UNTIL: f32 = 0.05;
/// How quickly a glide slows: the time in which it loses about two thirds of its speed, in milliseconds.
const GLIDE_MS: f64 = 325.0;

/// One finger on the canvas, as its pointer events report it. Times are in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Finger {
    Down {
        id: i32,
        at: Point,
        ms: f64,
    },
    Move {
        id: i32,
        at: Point,
        ms: f64,
    },
    Up {
        id: i32,
        ms: f64,
    },
    /// The browser took the touch for itself, as it does for a pinch.
    Cancel {
        id: i32,
    },
}

/// What fingers ask of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    None,
    /// Scroll what is under `at` by `by`, as a wheel would.
    Scroll {
        at: Point,
        by: Point,
    },
    /// A press and release at one place.
    Tap(Point),
}

/// Follows one finger: a drag scrolls, a flick leaves the page gliding, and a touch that stays put is a tap.
#[derive(Clone, Debug, Default)]
pub struct Touch {
    finger: Option<Track>,
    glide: Option<Glide>,
}

#[derive(Clone, Debug)]
struct Track {
    id: i32,
    start: Point,
    down_ms: f64,
    last: Point,
    last_ms: f64,
    dragging: bool,
    /// Cleared when the touch stopped a glide, which is all it was for.
    tap: bool,
    /// How fast the page is being scrolled, in pixels a millisecond.
    speed: Point,
}

#[derive(Clone, Debug)]
struct Glide {
    at: Point,
    speed: Point,
    ms: f64,
}

impl Touch {
    pub fn event(&mut self, finger: Finger) -> Gesture {
        match finger {
            Finger::Down { id, at, ms } => {
                let stopped = self.glide.take().is_some();
                // A second finger makes it a pinch, which is the browser's.
                self.finger = self.finger.is_none().then_some(Track {
                    id,
                    start: at,
                    down_ms: ms,
                    last: at,
                    last_ms: ms,
                    dragging: false,
                    tap: !stopped,
                    speed: Point::default(),
                });
                Gesture::None
            }
            Finger::Move { id, at, ms } => {
                let Some(track) = self.finger.as_mut().filter(|track| track.id == id) else { return Gesture::None };
                let wandered = (at.x - track.start.x).hypot(at.y - track.start.y);
                if !track.dragging && wandered < SLOP {
                    return Gesture::None;
                }
                track.dragging = true;
                // The page follows the finger, so it scrolls the other way.
                let by = Point { x: track.last.x - at.x, y: track.last.y - at.y };
                let elapsed = (ms - track.last_ms) as f32;
                if elapsed > 0.0 {
                    // The latest movement counts most: a flick is judged by how the finger left.
                    let blend = |before: f32, now: f32| before * 0.2 + now / elapsed * 0.8;
                    track.speed = Point { x: blend(track.speed.x, by.x), y: blend(track.speed.y, by.y) };
                }
                (track.last, track.last_ms) = (at, ms);
                Gesture::Scroll { at: track.start, by }
            }
            Finger::Up { id, ms } => {
                let Some(track) = self.finger.take_if(|track| track.id == id) else { return Gesture::None };
                if !track.dragging {
                    let tapped = track.tap && ms - track.down_ms < LONG_MS;
                    return if tapped { Gesture::Tap(track.start) } else { Gesture::None };
                }
                if ms - track.last_ms < REST_MS && track.speed.x.hypot(track.speed.y) >= GLIDE_FROM {
                    self.glide = Some(Glide { at: track.start, speed: track.speed, ms });
                }
                Gesture::None
            }
            Finger::Cancel { id } => {
                self.finger.take_if(|track| track.id == id);
                Gesture::None
            }
        }
    }

    /// Whether a flick is still carrying the page, so that frames should keep coming.
    pub fn gliding(&self) -> bool {
        self.glide.is_some()
    }

    /// How far the glide carries the page by the frame at `ms`, and what it scrolls.
    pub fn glide(&mut self, ms: f64) -> Option<(Point, Point)> {
        let glide = self.glide.as_mut()?;
        let kept = (-(ms - glide.ms).max(0.0) / GLIDE_MS).exp() as f32;
        // The distance covered while the speed fell from what it was to what is kept of it.
        let reach = GLIDE_MS as f32 * (1.0 - kept);
        let step = (glide.at, Point { x: glide.speed.x * reach, y: glide.speed.y * reach });
        glide.speed = Point { x: glide.speed.x * kept, y: glide.speed.y * kept };
        glide.ms = ms;
        if glide.speed.x.hypot(glide.speed.y) < GLIDE_UNTIL {
            self.glide = None;
        }
        Some(step)
    }

    /// Ends a glide, as reaching the end of the page does.
    pub fn stop(&mut self) {
        self.glide = None;
    }
}
