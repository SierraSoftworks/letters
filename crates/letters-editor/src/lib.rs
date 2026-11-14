//! The editor: turns input into edits of a [`Model`].
//!
//! - [`InputEvent`] is the one stream of text input every platform funnels into.
//! - [`KeyEvent`] carries key presses, which the keymap resolves to [`Command`]s.
//! - [`Editor`] owns the model and the selection, and is driven with nothing else.
//!
//! The editor never reads a clock or a clipboard itself. The shell injects a [`Platform`].

use std::ops::Range;

use letters_model::{Change, Document, Mark, Model, Op, Pos, Selection};

/// Text input, as every platform delivers it. Offsets are UTF-8 bytes relative to the caret's block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    /// Replaces the selection with text.
    Insert(String),
    /// Replaces a range of the caret's block, as autocorrect does.
    ReplaceRange { range: Range<usize>, text: String },
    /// Shows text an IME is still composing, replacing any earlier composition.
    SetComposition { text: String, selection: Option<Range<usize>> },
    /// Ends a composition with its final text.
    Commit(String),
    /// Deletes bytes around the caret, as a soft keyboard's backspace does.
    DeleteSurrounding { before: usize, after: usize },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    /// Ctrl on Linux and Windows, Cmd on macOS.
    pub primary: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A printable key by the character it produces unshifted, in lower case.
    Char(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub key: Key,
    pub mods: Modifiers,
}

/// Everything the editor can be asked to do. Keys, menus and the command palette all go through here.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    ToggleMark(Mark),
    Undo,
    Redo,
    SelectAll,
    Copy,
    Cut,
    Paste,
}

/// What the shell lends the editor.
pub trait Platform {
    /// Milliseconds since an arbitrary start. Only differences are meaningful.
    fn now_ms(&self) -> u64;
    fn clipboard_read(&mut self) -> Option<Clip>;
    fn clipboard_write(&mut self, clip: Clip);
    /// A new identifier, unique across devices.
    fn new_id(&mut self) -> String;
}

/// Clipboard content: plain text (Markdown when Letters wrote it) and, where offered, HTML.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clip {
    pub text: String,
    pub html: Option<String>,
}

/// A document being edited.
#[derive(Clone, Debug)]
pub struct Editor {
    model: Model,
    selection: Option<Selection>,
}

impl Editor {
    pub fn new(doc: Document) -> Self {
        let selection = doc.walk().find(|b| matches!(b.body, letters_model::Body::Text(_)));
        let selection = selection.map(|b| Selection::caret(Pos { block: b.id.clone(), offset: 0 }));
        Self { model: Model::new(doc), selection }
    }

    pub fn doc(&self) -> &Document {
        self.model.doc()
    }

    /// `None` when the document has nowhere to put a caret.
    pub fn selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    pub fn select(&mut self, selection: Selection) {
        self.selection = Some(selection);
    }

    /// Feeds text input. Returns what changed, for layout.
    pub fn input(&mut self, event: InputEvent, _platform: &mut dyn Platform) -> Change {
        match (event, self.selection.clone()) {
            (InputEvent::Insert(text), Some(selection)) if selection.is_caret() => {
                let at = selection.head;
                let moved = Pos { block: at.block.clone(), offset: at.offset + text.len() };
                match self.model.apply([Op::InsertText { at, text }]) {
                    Ok(change) => {
                        self.selection = Some(Selection::caret(moved));
                        change
                    }
                    Err(_) => Change::default(),
                }
            }
            _ => todo!("editor"),
        }
    }

    /// Feeds a key press. Returns `None` when the key means nothing here, so the shell may use it.
    pub fn key(&mut self, _event: KeyEvent, _platform: &mut dyn Platform) -> Option<Change> {
        todo!("editor")
    }

    pub fn run(&mut self, _command: Command, _platform: &mut dyn Platform) -> Change {
        todo!("editor")
    }
}
