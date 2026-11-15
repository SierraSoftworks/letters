//! The command palette: every action by its name, found by typing a few letters of it.
//!
//! A card under the top bar holds a field and a list of what can be done where the caret is. Focus stays in
//! the field: the arrow keys walk the list, Enter does what is picked, and Escape puts the card away. The
//! list is the table below. The window says which of it has something to act on now, and what each asks.

use accesskit::Role;
use letters_editor::{CalloutEdit, Command, Key, KeyEvent, Keymap, Modifiers, TableEdit};
use letters_model::{Align as ColumnAlign, BlockKind, ListKind, Mark, Tone};
use letters_render::{Rect, Size};

use crate::element::Element;
use crate::review::Reviewing;
use crate::style::{Align, Edges};
use crate::text::TextStyle;
use crate::tokens;
use crate::widgets::{Choice, Options, Popover, TextField};
use crate::window::{Message, Panel};

/// How wide the card is, where the window has the room.
const WIDTH: f32 = 520.0;
/// How high the list grows before it scrolls: some ten commands and the headings between them.
const LIST: f32 = 332.0;
/// The least of the list that shows in a window too low for all of it.
const LEAST: f32 = 96.0;
/// The room kept between the card and the window's edges, and between the bar and the card.
const MARGIN: f32 = 16.0;
/// The height of the field and the room round it.
const HEAD: f32 = 52.0;

/// What the keyboard and the pointer ask of the palette.
#[derive(Clone, Debug, PartialEq)]
pub enum Commanding {
    /// Shows the palette, or puts it away when it shows.
    Toggle,
    /// What the field holds.
    Type(String),
    /// Moves the pick one command down the list, or up, round its ends.
    Step { down: bool },
    /// A command was pressed, counted from the first that shows.
    Take(usize),
    /// Enter, which takes the command that is picked.
    Enter,
    /// Escape or a press elsewhere.
    Close,
}

/// What a command does. The window turns it into what it asks for, where there is something to act on.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Act {
    Run(Command),
    /// Moves the caret's row one place down its table, or up.
    MoveRow {
        down: bool,
    },
    /// Moves the caret's column one place to the right, or to the left.
    MoveColumn {
        right: bool,
    },
    /// Opens the link editor at the caret.
    Link,
    /// Starts a comment on the selection.
    Comment,
    /// Lets the person choose pictures to put in.
    Pictures,
    /// Turns the line under a picture to its alt text.
    AltText,
    /// Opens the card of the callout the caret is in.
    Callout,
    /// Shows the selected linked picture this once.
    Load,
    /// Shows the pictures of the selected one's host from now on.
    AlwaysLoad,
    /// Keeps the selected linked picture in the document.
    Embed,
    /// Keeps every linked picture in the document.
    EmbedAll,
    Panel(Panel),
    NewDocument,
    Open,
    Rename,
    /// Asks the shell to save the document, where it does so only when asked.
    Save,
    SaveCopy,
    Export,
    Share,
    Settings,
    /// Shows where the paper and the margins are set.
    PageSetup,
    TrustDocument,
    TrustFolder,
    /// Shows the document on the sheets it prints on.
    Print,
    /// Shows the document's earlier versions.
    History,
    /// Steps through the conflicts that wait for a choice, or settles the one that is open.
    Review(Reviewing),
    /// Goes to the next misspelled word and offers its corrections, or to the one before.
    Misspelling {
        back: bool,
    },
    /// Shows the bar that finds text, with the keyboard in its field.
    Find,
    /// Shows the bar that finds text with the line that replaces what is found.
    Replace,
    /// Goes to the next place the text was found at, or to the one before.
    FindStep {
        back: bool,
    },
    /// Shows the palette itself. It is not one of its own commands.
    Palette,
}

/// A command of the palette.
pub(crate) struct Entry {
    group: &'static str,
    pub name: &'static str,
    /// Other words the command is found by.
    also: &'static str,
    pub act: Act,
}

const fn entry(group: &'static str, name: &'static str, also: &'static str, act: Act) -> Entry {
    Entry { group, name, also, act }
}

const fn run(group: &'static str, name: &'static str, also: &'static str, command: Command) -> Entry {
    entry(group, name, also, Act::Run(command))
}

const fn kind(name: &'static str, also: &'static str, kind: BlockKind) -> Entry {
    run("Text", name, also, Command::SetBlock(kind))
}

const fn list(name: &'static str, also: &'static str, list: ListKind) -> Entry {
    kind(name, also, BlockKind::ListItem { list, depth: 0 })
}

const fn mark(name: &'static str, also: &'static str, mark: Mark) -> Entry {
    run("Format", name, also, Command::ToggleMark(mark))
}

const fn table(name: &'static str, also: &'static str, edit: TableEdit) -> Entry {
    run("Table", name, also, Command::Table(edit))
}

const fn callout(name: &'static str, also: &'static str, edit: CalloutEdit) -> Entry {
    run("Callout", name, also, Command::Callout(edit))
}

/// Every command, in the order the palette lists them: what belongs to the thing the caret is on comes
/// first. Names are told apart by the list, so no two are the same.
pub(crate) static ENTRIES: [Entry; 91] = [
    entry("Conflict", "Keep this version", "choose page later settle resolve", Act::Review(Reviewing::Keep)),
    entry("Conflict", "Use the other version", "choose held back earlier trade swap", Act::Review(Reviewing::Use)),
    entry("Conflict", "Close conflict", "leave undecided later", Act::Review(Reviewing::Close)),
    run("Image", "Edit caption", "picture", Command::EditCaption),
    entry("Image", "Edit alt text", "picture describe description", Act::AltText),
    run("Image", "Number this figure", "picture label caption", Command::SetNumbered(true)),
    run("Image", "Remove figure number", "picture label caption", Command::SetNumbered(false)),
    run("Image", "Move image left", "picture gallery", Command::ShiftFigure { right: false }),
    run("Image", "Move image right", "picture gallery", Command::ShiftFigure { right: true }),
    entry("Image", "Load image once", "picture linked remote", Act::Load),
    entry("Image", "Always load images from this host", "picture linked remote trust", Act::AlwaysLoad),
    entry("Image", "Embed image in the document", "picture linked remote keep", Act::Embed),
    entry("Image", "Embed all remote images", "pictures linked keep", Act::EmbedAll),
    table("Add row above", "insert", TableEdit::AddRow { below: false }),
    table("Add row below", "insert", TableEdit::AddRow { below: true }),
    table("Add column to the left", "insert before", TableEdit::AddColumn { after: false }),
    table("Add column to the right", "insert after", TableEdit::AddColumn { after: true }),
    table("Remove row", "delete", TableEdit::RemoveRow),
    table("Remove column", "delete", TableEdit::RemoveColumn),
    entry("Table", "Move row up", "", Act::MoveRow { down: false }),
    entry("Table", "Move row down", "", Act::MoveRow { down: true }),
    entry("Table", "Move column left", "", Act::MoveColumn { right: false }),
    entry("Table", "Move column right", "", Act::MoveColumn { right: true }),
    table("Align column left", "start", TableEdit::Align(ColumnAlign::Start)),
    table("Align column centre", "center middle", TableEdit::Align(ColumnAlign::Center)),
    table("Align column right", "end", TableEdit::Align(ColumnAlign::End)),
    table("Turn header row on", "heading first", TableEdit::Header(true)),
    table("Turn header row off", "heading first", TableEdit::Header(false)),
    entry("Callout", "Edit callout", "title icon colour color type", Act::Callout),
    callout("Info", "note blue", CalloutEdit::Tone(Tone::Info)),
    callout("Tip", "hint green", CalloutEdit::Tone(Tone::Tip)),
    callout("Warning", "caution amber", CalloutEdit::Tone(Tone::Warn)),
    callout("Danger", "error red", CalloutEdit::Tone(Tone::Danger)),
    callout("Use the type's defaults", "reset title icon colour color", CalloutEdit::Reset),
    callout("Turn into body text", "remove unwrap plain", CalloutEdit::Remove),
    kind("Body text", "paragraph normal plain", BlockKind::Paragraph),
    kind("Heading 1", "h1 title", BlockKind::Heading { level: 1 }),
    kind("Heading 2", "h2", BlockKind::Heading { level: 2 }),
    kind("Heading 3", "h3", BlockKind::Heading { level: 3 }),
    kind("Heading 4", "h4", BlockKind::Heading { level: 4 }),
    list("Bulleted list", "unordered", ListKind::Bullet),
    list("Numbered list", "ordered", ListKind::Ordered),
    list("Task list", "todo checklist checkbox", ListKind::Task { checked: false }),
    kind("Quote", "blockquote", BlockKind::Quote),
    kind("Code block", "pre", BlockKind::Code { language: None }),
    run("Text", "Indent", "nest deeper", Command::Indent),
    run("Text", "Outdent", "nest shallower unindent", Command::Outdent),
    run("Text", "Tick or untick task", "check uncheck done todo", Command::ToggleChecked),
    mark("Bold", "strong", Mark::Bold),
    mark("Italic", "emphasis", Mark::Italic),
    mark("Underline", "", Mark::Underline),
    mark("Strikethrough", "strike", Mark::Strike),
    mark("Highlight", "marker", Mark::Highlight),
    mark("Inline code", "monospace", Mark::Code),
    entry("Format", "Link", "url address", Act::Link),
    entry("Format", "Comment", "note", Act::Comment),
    run("Insert", "Table", "grid", Command::InsertTable { rows: 3, columns: 3 }),
    entry("Insert", "Image", "picture photo figure", Act::Pictures),
    run("Insert", "Callout", "box note", Command::SetBlock(BlockKind::callout(Tone::Info))),
    run("Insert", "Formula", "maths equation tex", Command::SetBlock(BlockKind::Formula)),
    run("Insert", "Divider", "rule line separator", Command::SetBlock(BlockKind::Divider)),
    run("Edit", "Undo", "", Command::Undo),
    run("Edit", "Redo", "", Command::Redo),
    run("Edit", "Cut", "", Command::Cut),
    run("Edit", "Copy", "", Command::Copy),
    run("Edit", "Paste", "", Command::Paste),
    run("Edit", "Paste as plain text", "without formatting", Command::PastePlain),
    run("Edit", "Select all", "", Command::SelectAll),
    entry("Edit", "Next spelling mistake", "misspelled word typo check", Act::Misspelling { back: false }),
    entry("Edit", "Previous spelling mistake", "misspelled word typo check", Act::Misspelling { back: true }),
    entry("Edit", "Find", "search look for text", Act::Find),
    entry("Edit", "Find and replace", "search substitute change text", Act::Replace),
    entry("Edit", "Find next", "search result", Act::FindStep { back: false }),
    entry("Edit", "Find previous", "search result", Act::FindStep { back: true }),
    entry("View", "Styles panel", "show hide fonts page", Act::Panel(Panel::Styles)),
    entry("View", "Notes panel", "show hide comments", Act::Panel(Panel::Notes)),
    entry("View", "History", "versions earlier before rewind restore", Act::History),
    entry("View", "Print view", "pages paper sheets preview", Act::Print),
    entry("View", "Next conflict", "changed two places choice", Act::Review(Reviewing::Step { back: false })),
    entry("View", "Previous conflict", "changed two places choice", Act::Review(Reviewing::Step { back: true })),
    entry("Document", "New document", "create", Act::NewDocument),
    entry("Document", "Open", "file", Act::Open),
    entry("Document", "Rename or move", "name folder", Act::Rename),
    entry("Document", "Save", "keep write file download", Act::Save),
    entry("Document", "Save a copy", "duplicate", Act::SaveCopy),
    entry("Document", "Export", "pdf word markdown html web", Act::Export),
    entry("Document", "Share", "", Act::Share),
    entry("Document", "Page setup", "paper margins orientation size landscape portrait", Act::PageSetup),
    entry("Document", "Settings", "preferences appearance accent", Act::Settings),
    entry("Document", "Trust this document", "safe mode", Act::TrustDocument),
    entry("Document", "Trust this folder", "safe mode", Act::TrustFolder),
];

const PRIMARY: Modifiers = Modifiers { primary: true, shift: false, alt: false };
const PRIMARY_SHIFT: Modifiers = Modifiers { primary: true, shift: true, alt: false };
const PRIMARY_ALT: Modifiers = Modifiers { primary: true, shift: false, alt: true };

/// The keys the window binds itself, for what is no command of the editor's. Where a command has two, the
/// second is the one a Mac's menus write: there the system keeps Cmd with H to hide the application.
static KEYS: [(KeyEvent, Act); 13] = [
    (
        KeyEvent { key: Key::F(7), mods: Modifiers { primary: false, shift: false, alt: false } },
        Act::Misspelling { back: false },
    ),
    (
        KeyEvent { key: Key::F(7), mods: Modifiers { primary: false, shift: true, alt: false } },
        Act::Misspelling { back: true },
    ),
    (KeyEvent { key: Key::Char('p'), mods: PRIMARY_SHIFT }, Act::Palette),
    (KeyEvent { key: Key::Char('p'), mods: PRIMARY }, Act::Print),
    (KeyEvent { key: Key::Char('k'), mods: PRIMARY }, Act::Link),
    (KeyEvent { key: Key::Char(','), mods: PRIMARY }, Act::Settings),
    (KeyEvent { key: Key::Char('s'), mods: PRIMARY }, Act::Save),
    (KeyEvent { key: Key::Char('s'), mods: PRIMARY_SHIFT }, Act::SaveCopy),
    (KeyEvent { key: Key::Char('f'), mods: PRIMARY }, Act::Find),
    (KeyEvent { key: Key::Char('h'), mods: PRIMARY }, Act::Replace),
    (KeyEvent { key: Key::Char('f'), mods: PRIMARY_ALT }, Act::Replace),
    (KeyEvent { key: Key::Char('g'), mods: PRIMARY }, Act::FindStep { back: false }),
    (KeyEvent { key: Key::Char('g'), mods: PRIMARY_SHIFT }, Act::FindStep { back: true }),
];

/// The command that does `act`, for its name where something other than the palette offers it.
pub(crate) fn entry_of(act: &Act) -> Option<&'static Entry> {
    ENTRIES.iter().find(|entry| entry.act == *act)
}

/// The commands of one group, in the palette's order.
pub(crate) fn group(group: &'static str) -> impl Iterator<Item = &'static Entry> {
    ENTRIES.iter().filter(move |entry| entry.group == group)
}

/// What a key does that the editor's keymap leaves to the window.
pub(crate) fn bound(event: &KeyEvent) -> Option<&'static Act> {
    KEYS.iter().find(|(key, _)| key == event).map(|(_, act)| act)
}

/// The key that does what a command does, as a menu writes it beside the command.
pub(crate) fn shortcut(act: &Act, keymap: &Keymap, mac: bool) -> Option<String> {
    let key = match act {
        Act::Run(command) => keymap.key(command),
        act => {
            let mut keys = KEYS.iter().filter(|(_, bound)| bound == act).map(|(key, _)| key);
            if mac { keys.next_back() } else { keys.next() }
        }
    };
    key.map(|key| chord(key, mac))
}

/// A key as menus write it: `Ctrl+Shift+P`, or the signs a Mac's menus use.
fn chord(event: &KeyEvent, mac: bool) -> String {
    let either = |standard: &str, sign: &str| (if mac { sign } else { standard }).to_owned();
    let key = match event.key {
        Key::Char(c) => c.to_uppercase().collect(),
        Key::Enter => either("Enter", "\u{21a9}"),
        Key::Tab => either("Tab", "\u{21e5}"),
        Key::Backspace => either("Backspace", "\u{232b}"),
        Key::Delete => either("Delete", "\u{2326}"),
        Key::Escape => "Esc".to_owned(),
        Key::Left => "\u{2190}".to_owned(),
        Key::Right => "\u{2192}".to_owned(),
        Key::Up => "\u{2191}".to_owned(),
        Key::Down => "\u{2193}".to_owned(),
        Key::Home => "Home".to_owned(),
        Key::End => "End".to_owned(),
        Key::PageUp => "Page Up".to_owned(),
        Key::PageDown => "Page Down".to_owned(),
        Key::F(number) => format!("F{number}"),
    };
    let Modifiers { primary, shift, alt } = event.mods;
    // Each system has its own order for the keys held down.
    let held = if mac {
        [(alt, "\u{2325}"), (shift, "\u{21e7}"), (primary, "\u{2318}")]
    } else {
        [(primary, "Ctrl+"), (alt, "Alt+"), (shift, "Shift+")]
    };
    let held: String = held.into_iter().filter(|(held, _)| *held).map(|(_, sign)| sign).collect();
    held + &key
}

/// Whether each word that is typed begins a word of the command's group, its name or its other words.
fn found(entry: &Entry, typed: &str) -> bool {
    let words = || [entry.group, entry.name, entry.also].into_iter().flat_map(|words| words.split(' '));
    typed.split_whitespace().all(|typed| words().any(|word| word.to_lowercase().starts_with(typed)))
}

/// The palette's own state: whether it shows, what is typed and which command is picked.
#[derive(Default)]
pub(crate) struct Commands {
    open: bool,
    typed: String,
    picked: usize,
}

impl Commands {
    pub fn open(&self) -> bool {
        self.open
    }

    /// Shows the palette with nothing typed, or puts it away.
    pub fn toggle(&mut self) {
        *self = Self { open: !self.open, ..Self::default() };
    }

    pub fn close(&mut self) {
        *self = Self::default();
    }

    /// Takes what the field holds now. The first command it leaves is the one picked.
    pub fn typed(&mut self, typed: String) {
        (self.typed, self.picked) = (typed, 0);
    }

    /// Moves the pick one command down or up a list of `shown`, round its ends.
    pub fn step(&mut self, down: bool, shown: usize) {
        self.picked = Options::<Message>::step(self.picked, down, shown);
    }

    /// The command that is picked among `shown`.
    pub fn picked(&self, shown: usize) -> usize {
        self.picked.min(shown.saturating_sub(1))
    }

    /// The commands that what is typed finds, in the palette's order: all of them while nothing is typed.
    pub fn matching(&self) -> impl Iterator<Item = &'static Entry> {
        let typed = self.typed.to_lowercase();
        ENTRIES.iter().filter(move |entry| found(entry, &typed))
    }

    /// The card over `offers`, the commands that match and have something to act on with what each asks.
    /// It hangs `top` down a window of the size `window`, from an element of no size that the window puts
    /// after everything else.
    pub fn view(
        &self,
        offers: &[(&'static Entry, Message)],
        keymap: &Keymap,
        mac: bool,
        top: f32,
        window: Size,
    ) -> Element<Message> {
        let send = Message::Commands;
        let quiet = TextStyle::control().size(tokens::TEXT_SM);
        let field = TextField::new("Command", self.typed.as_str())
            .placeholder("Type a command")
            .on_input(move |typed| send(Commanding::Type(typed)))
            .on_submit(send(Commanding::Enter))
            .on_cancel(send(Commanding::Close));
        let field = Element::from(field).key("commands").takes_focus();
        let head = Element::column().padding(Edges::all(tokens::SPACE_SM)).child(field);

        let list = if offers.is_empty() {
            let none = Element::text("No command matches.", quiet).fg(tokens::FG_MUTED).role(Role::Status);
            Element::row().padding(Edges::xy(tokens::SPACE_LG, tokens::SPACE_MD)).child(none)
        } else {
            let mut options = Options::new("Commands").picked(self.picked(offers.len()));
            let mut group = "";
            for (index, (entry, _)) in offers.iter().enumerate() {
                if entry.group != group {
                    group = entry.group;
                    options = options.heading(group);
                }
                let mut choice = Choice::new(entry.name).on_take(send(Commanding::Take(index)));
                if let Some(keys) = shortcut(&entry.act, keymap, mac) {
                    choice = choice.aside(keys, quiet);
                }
                options = options.choice(choice);
            }
            // A low window shows less of the list, and never none of it.
            let room = window.height - top - HEAD - 2.0 * MARGIN;
            // The rows end short of the bar that shows how far the list is scrolled, and under the field.
            let list = Element::from(options).padding(Edges::xy(tokens::SPACE_SM, tokens::SPACE_XS));
            list.max_height(LIST.min(room).max(LEAST)).scroll()
        };

        // The field has the keys: those that walk a list are no use to one line of text.
        let keys = move |event: &KeyEvent| match event.key {
            _ if event.mods != Modifiers::default() => None,
            Key::Down => Some(send(Commanding::Step { down: true })),
            Key::Up => Some(send(Commanding::Step { down: false })),
            _ => None,
        };
        let width = WIDTH.min(window.width - 2.0 * MARGIN).max(0.0);
        let line = Element::row().height(1.0).bg(tokens::LINE).decorative();
        let card = Element::column().width(width).child(head).child(line).child(list).on_key(keys);
        let card = Popover::new(card)
            .label("Commands")
            .flush()
            .max_width(width + 2.0)
            .at(Rect::new(window.width / 2.0, top + MARGIN / 2.0, 0.0, 0.0))
            .align(Align::Center)
            .gap(0.0)
            .open(true)
            .on_close(send(Commanding::Close));
        Element::row().overlay().popover(card)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::insert::{OFFERS, Put};

    fn names(typed: &str) -> Vec<&'static str> {
        let mut commands = Commands::default();
        commands.typed(typed.to_owned());
        commands.matching().map(|entry| entry.name).collect()
    }

    #[test]
    fn what_is_typed_finds_commands_by_the_words_of_their_names_groups_and_other_words() {
        assert_eq!(names("").len(), ENTRIES.len(), "nothing typed leaves every command");
        // What is typed, a command it must find, and one it must not.
        let cases = [
            ("head", "Heading 2", Some("Body text")),
            ("HEAD 2", "Heading 2", Some("Heading 1")),
            ("2 head", "Heading 2", Some("Heading 3")),
            ("h3", "Heading 3", Some("Heading 2")),
            ("table", "Add row above", Some("Bold")),
            ("ins tab", "Table", Some("Remove row")),
            ("picture", "Embed all remote images", Some("Paste")),
            ("center", "Align column centre", Some("Align column left")),
            ("prefer", "Settings", Some("Share")),
            ("   bold  ", "Bold", Some("Italic")),
            ("o", "Open", None),
        ];
        for (typed, finds, leaves) in cases {
            let found = names(typed);
            assert!(found.contains(&finds), "{typed:?} finds {finds:?} among {found:?}");
            assert!(leaves.is_none_or(|leaves| !found.contains(&leaves)), "{typed:?} leaves {leaves:?} out");
        }
        assert!(names("zzz").is_empty(), "words no command has find nothing");
        assert!(names("old").is_empty(), "the middle of a word finds nothing");
    }

    #[test]
    fn commands_are_told_apart_by_their_names() {
        let mut seen = HashSet::new();
        for entry in &ENTRIES {
            assert!(seen.insert(entry.name), "{:?} is listed twice", entry.name);
        }
    }

    #[test]
    fn what_the_insert_menu_offers_the_palette_offers_too() {
        for offer in &OFFERS {
            let act = match &offer.put {
                Put::Kind(kind) => Act::Run(Command::SetBlock(kind.clone())),
                Put::Table => Act::Run(Command::InsertTable { rows: 3, columns: 3 }),
                Put::Pictures => Act::Pictures,
            };
            assert!(ENTRIES.iter().any(|entry| entry.act == act), "{:?} is a command", offer.name);
        }
    }

    #[test]
    fn a_key_is_written_as_the_menus_of_its_system_write_it() {
        let key = |key, primary, shift, alt| KeyEvent { key, mods: Modifiers { primary, shift, alt } };
        // The key, as Linux and Windows write it, and as a Mac does.
        let cases = [
            (key(Key::Char('b'), true, false, false), "Ctrl+B", "\u{2318}B"),
            (key(Key::Char('p'), true, true, false), "Ctrl+Shift+P", "\u{21e7}\u{2318}P"),
            (key(Key::Char('1'), true, false, true), "Ctrl+Alt+1", "\u{2325}\u{2318}1"),
            (key(Key::Tab, false, true, false), "Shift+Tab", "\u{21e7}\u{21e5}"),
            (key(Key::Left, false, false, true), "Alt+\u{2190}", "\u{2325}\u{2190}"),
            (key(Key::Enter, true, false, false), "Ctrl+Enter", "\u{2318}\u{21a9}"),
        ];
        for (event, standard, mac) in cases {
            assert_eq!(chord(&event, false), standard);
            assert_eq!(chord(&event, true), mac);
        }
    }

    #[test]
    fn the_keys_the_window_binds_are_shown_beside_their_commands() {
        let keymap = Keymap::standard();
        // A command, and the key written beside it.
        let cases = [
            (Act::Run(Command::ToggleMark(Mark::Bold)), Some("Ctrl+B")),
            (Act::Link, Some("Ctrl+K")),
            (Act::Save, Some("Ctrl+S")),
            (Act::SaveCopy, Some("Ctrl+Shift+S")),
            (Act::Palette, Some("Ctrl+Shift+P")),
            (Act::Find, Some("Ctrl+F")),
            (Act::Replace, Some("Ctrl+H")),
            (Act::FindStep { back: true }, Some("Ctrl+Shift+G")),
            (Act::Share, None),
        ];
        for (act, key) in cases {
            assert_eq!(shortcut(&act, &keymap, false).as_deref(), key, "{act:?}");
        }
        // A Mac hides the application on Cmd with H, so its menus write the other key.
        assert_eq!(shortcut(&Act::Replace, &keymap, true).as_deref(), Some("\u{2325}\u{2318}F"));
        assert_eq!(shortcut(&Act::Find, &keymap, true).as_deref(), Some("\u{2318}F"));
        for (key, act) in &KEYS {
            assert_eq!(bound(key), Some(act));
            assert!(keymap.command(key).is_none(), "{key:?} is the window's to bind");
        }
    }
}
