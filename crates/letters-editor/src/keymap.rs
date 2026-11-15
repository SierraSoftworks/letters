//! Which key runs which command.

use letters_model::{BlockKind, ListKind, Mark};

use crate::{Command, Key, KeyEvent, Modifiers, Motion};

/// The keys bound to commands. Menus and the command palette read their shortcuts from here too.
#[derive(Clone, Debug)]
pub struct Keymap {
    bindings: Vec<(KeyEvent, Command)>,
}

impl Default for Keymap {
    /// The conventions of the system this build is for. A browser shell picks at run time instead.
    fn default() -> Self {
        if cfg!(target_os = "macos") { Self::mac() } else { Self::standard() }
    }
}

const NONE: Modifiers = Modifiers { primary: false, shift: false, alt: false };
const PRIMARY: Modifiers = Modifiers { primary: true, ..NONE };
const SHIFT: Modifiers = Modifiers { shift: true, ..NONE };
const ALT: Modifiers = Modifiers { alt: true, ..NONE };
const PRIMARY_SHIFT: Modifiers = Modifiers { shift: true, ..PRIMARY };
const PRIMARY_ALT: Modifiers = Modifiers { alt: true, ..PRIMARY };

impl Keymap {
    /// Linux and Windows conventions: Ctrl moves by word, and Home and End by line.
    pub fn standard() -> Self {
        let mut keymap = Self::shared();
        keymap.motions(&[
            (Key::Left, PRIMARY, Motion::WordLeft),
            (Key::Right, PRIMARY, Motion::WordRight),
            (Key::Up, PRIMARY, Motion::BlockStart),
            (Key::Down, PRIMARY, Motion::BlockEnd),
            (Key::Home, NONE, Motion::LineStart),
            (Key::End, NONE, Motion::LineEnd),
            (Key::Home, PRIMARY, Motion::DocStart),
            (Key::End, PRIMARY, Motion::DocEnd),
        ]);
        keymap.bind(Key::Backspace, PRIMARY, Command::Delete(Motion::WordLeft));
        keymap.bind(Key::Delete, PRIMARY, Command::Delete(Motion::WordRight));
        keymap.bind(Key::Char('y'), PRIMARY, Command::Redo);
        keymap.figures();
        keymap
    }

    /// macOS conventions: Option moves by word, and Cmd by line and to the ends of the document.
    pub fn mac() -> Self {
        let mut keymap = Self::shared();
        keymap.motions(&[
            (Key::Left, ALT, Motion::WordLeft),
            (Key::Right, ALT, Motion::WordRight),
            (Key::Up, ALT, Motion::BlockStart),
            (Key::Down, ALT, Motion::BlockEnd),
            (Key::Left, PRIMARY, Motion::LineStart),
            (Key::Right, PRIMARY, Motion::LineEnd),
            (Key::Up, PRIMARY, Motion::DocStart),
            (Key::Down, PRIMARY, Motion::DocEnd),
            (Key::Home, NONE, Motion::DocStart),
            (Key::End, NONE, Motion::DocEnd),
        ]);
        keymap.bind(Key::Backspace, ALT, Command::Delete(Motion::WordLeft));
        keymap.bind(Key::Delete, ALT, Command::Delete(Motion::WordRight));
        keymap.bind(Key::Backspace, PRIMARY, Command::Delete(Motion::LineStart));
        keymap.bind(Key::Delete, PRIMARY, Command::Delete(Motion::LineEnd));
        keymap.figures();
        keymap
    }

    /// What every system agrees on.
    fn shared() -> Self {
        let mut keymap = Self { bindings: Vec::new() };
        keymap.motions(&[
            (Key::Left, NONE, Motion::Left),
            (Key::Right, NONE, Motion::Right),
            (Key::Up, NONE, Motion::Up),
            (Key::Down, NONE, Motion::Down),
            (Key::PageUp, NONE, Motion::PageUp),
            (Key::PageDown, NONE, Motion::PageDown),
        ]);
        let heading = |level| Command::SetBlock(BlockKind::Heading { level });
        let list = |list| Command::SetBlock(BlockKind::ListItem { list, depth: 0 });
        let commands = [
            (Key::Backspace, NONE, Command::Delete(Motion::Left)),
            (Key::Backspace, SHIFT, Command::Delete(Motion::Left)),
            (Key::Delete, NONE, Command::Delete(Motion::Right)),
            (Key::Enter, NONE, Command::Split),
            (Key::Enter, SHIFT, Command::LineBreak),
            (Key::Enter, PRIMARY, Command::ToggleChecked),
            (Key::Escape, NONE, Command::LeaveFormula),
            (Key::Tab, NONE, Command::Indent),
            (Key::Tab, SHIFT, Command::Outdent),
            (Key::Char('b'), PRIMARY, Command::ToggleMark(Mark::Bold)),
            (Key::Char('i'), PRIMARY, Command::ToggleMark(Mark::Italic)),
            (Key::Char('u'), PRIMARY, Command::ToggleMark(Mark::Underline)),
            (Key::Char('e'), PRIMARY, Command::ToggleMark(Mark::Code)),
            (Key::Char('x'), PRIMARY_SHIFT, Command::ToggleMark(Mark::Strike)),
            (Key::Char('h'), PRIMARY_SHIFT, Command::ToggleMark(Mark::Highlight)),
            (Key::Char('0'), PRIMARY_ALT, Command::SetBlock(BlockKind::Paragraph)),
            (Key::Char('1'), PRIMARY_ALT, heading(1)),
            (Key::Char('2'), PRIMARY_ALT, heading(2)),
            (Key::Char('3'), PRIMARY_ALT, heading(3)),
            (Key::Char('4'), PRIMARY_ALT, heading(4)),
            (Key::Char('7'), PRIMARY_SHIFT, list(ListKind::Ordered)),
            (Key::Char('8'), PRIMARY_SHIFT, list(ListKind::Bullet)),
            (Key::Char('9'), PRIMARY_SHIFT, list(ListKind::Task { checked: false })),
            (Key::Char('.'), PRIMARY_SHIFT, Command::SetBlock(BlockKind::Quote)),
            (Key::Char('c'), PRIMARY_ALT, Command::SetBlock(BlockKind::Code { language: None })),
            (Key::Char('-'), PRIMARY_ALT, Command::SetBlock(BlockKind::Divider)),
            (Key::Char('z'), PRIMARY, Command::Undo),
            (Key::Char('z'), PRIMARY_SHIFT, Command::Redo),
            (Key::Char('a'), PRIMARY, Command::SelectAll),
            (Key::Char('c'), PRIMARY, Command::Copy),
            (Key::Char('x'), PRIMARY, Command::Cut),
            (Key::Char('v'), PRIMARY, Command::Paste),
            (Key::Char('v'), PRIMARY_SHIFT, Command::PastePlain),
        ];
        for (key, mods, command) in commands {
            keymap.bind(key, mods, command);
        }
        keymap
    }

    /// The keys that mean something else on a picture, in a caption or after the words for a reference than
    /// they do in other text.
    fn figures(&mut self) {
        self.also(Key::Escape, NONE, Command::LeaveCaption);
        self.also(Key::Escape, NONE, Command::DismissReference);
        self.also(Key::Left, ALT, Command::ShiftFigure { right: false });
        self.also(Key::Right, ALT, Command::ShiftFigure { right: true });
    }

    /// Binds each motion to its key, and to the same key with Shift to extend the selection.
    fn motions(&mut self, motions: &[(Key, Modifiers, Motion)]) {
        for (key, mods, to) in motions {
            for extend in [false, true] {
                let mods = Modifiers { shift: extend, ..*mods };
                self.bind(key.clone(), mods, Command::Move { to: *to, extend });
            }
        }
    }

    /// Binds a key, in place of whatever it ran before.
    pub fn bind(&mut self, key: Key, mods: Modifiers, command: Command) {
        let event = KeyEvent { key, mods };
        self.unbind(&event);
        self.bindings.push((event, command));
    }

    /// Binds a key to one command more, which is tried before what the key runs already. This is for a
    /// command that has something to act on only in some places, such as on a picture: everywhere else
    /// the key runs what it ran before.
    pub fn also(&mut self, key: Key, mods: Modifiers, command: Command) {
        self.bindings.insert(0, (KeyEvent { key, mods }, command));
    }

    pub fn unbind(&mut self, event: &KeyEvent) {
        self.bindings.retain(|(bound, _)| bound != event);
    }

    /// The command a key runs wherever nothing bound with [`Keymap::also`] has something to act on.
    pub fn command(&self, event: &KeyEvent) -> Option<&Command> {
        self.commands(event).last()
    }

    /// Every command a key is bound to, in the order to try them. The editor runs the first that has
    /// something to act on.
    pub fn commands(&self, event: &KeyEvent) -> impl Iterator<Item = &Command> {
        self.bindings.iter().filter(move |(bound, _)| bound == event).map(|(_, command)| command)
    }

    /// The key that runs a command, to show beside it in a menu.
    pub fn key(&self, command: &Command) -> Option<&KeyEvent> {
        self.bindings.iter().find(|(_, bound)| bound == command).map(|(event, _)| event)
    }

    /// Every binding, in the order they were made.
    pub fn bindings(&self) -> impl Iterator<Item = (&KeyEvent, &Command)> {
        self.bindings.iter().map(|(event, command)| (event, command))
    }
}
