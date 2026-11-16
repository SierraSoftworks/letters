//! The keys that do things, on one sheet.
//!
//! A dialog lists every command that has a key, under the groups the palette lists them in, and after
//! them the keys that are no command: those that move the caret, and what the history and Print view
//! answer. It reads the tables the keys are bound in, so it cannot say a key that does something else.

use accesskit::Role;
use letters_editor::{Command, Key, KeyEvent, Keymap, Modifiers, Motion};
use letters_render::Size;

use crate::commands::{self, Act};
use crate::element::Element;
use crate::icon;
use crate::settings::quiet;
use crate::style::{Align, Edges, Justify};
use crate::text::TextStyle;
use crate::tokens;
use crate::widgets::{Button, Popover, SidePanel};
use crate::window::Message;

/// How wide the dialog is, where the window has the room, and the least it is with two columns.
const WIDTH: f32 = 720.0;
const TWO: f32 = 600.0;
/// The room kept between the dialog and the window's edges.
const MARGIN: f32 = 16.0;
/// What the dialog takes of the window's height besides its lines, with the room it keeps clear of the
/// window's edges, and the least the lines are given before they scroll.
const AROUND: f32 = 170.0;
const SHORTEST: f32 = 120.0;
/// The height of a line.
const LINE: f32 = 30.0;

/// The palette's groups in the order the sheet has them: what is typed most comes first. A group that is
/// not named here follows them.
const ORDER: [&str; 7] = ["Edit", "Format", "Text", "Insert", "Image", "View", "Document"];

/// One thing that is done, and the keys that do it.
#[derive(Debug, PartialEq)]
pub(crate) struct Line {
    pub name: String,
    pub keys: Vec<String>,
}

/// The lines under one heading.
#[derive(Debug, PartialEq)]
pub(crate) struct Group {
    pub title: &'static str,
    pub lines: Vec<Line>,
}

const NONE: Modifiers = Modifiers { primary: false, shift: false, alt: false };

fn plain(key: Key) -> KeyEvent {
    KeyEvent { key, mods: NONE }
}

/// Everything the sheet lists, as `keymap` and the window's own table bind it. The keys that open and
/// close windows are listed for an `application`: a browser keeps them.
pub(crate) fn groups(keymap: &Keymap, mac: bool, application: bool) -> Vec<Group> {
    let written = |key: &KeyEvent| commands::chord(key, mac);
    let line = |name: &str, keys: Vec<String>| Line { name: name.to_owned(), keys };
    let mut groups: Vec<Group> = ORDER.iter().map(|title| Group { title, lines: Vec::new() }).collect();
    let mut add = |title: &'static str, line: Line| match groups.iter_mut().find(|group| group.title == title) {
        Some(group) => group.lines.push(line),
        None => groups.push(Group { title, lines: vec![line] }),
    };
    // The palette is no command of its own, and is where everything without a key is found.
    if let Some(key) = commands::shortcut(&Act::Palette, keymap, mac) {
        add("View", line("Commands", vec![key]));
    }
    for (group, name, act) in commands::listed().filter(|(.., act)| application || !commands::of_application(act)) {
        let keys: Vec<String> = match act {
            // Every key an editor's command has: redo has two outside a Mac.
            Act::Run(command) => {
                keymap.bindings().filter(|(_, bound)| *bound == command).map(|(key, _)| written(key)).collect()
            }
            // Of the window's keys the one its menus write: the other may be the system's on this machine.
            act => commands::shortcut(act, keymap, mac).into_iter().collect(),
        };
        if !keys.is_empty() {
            add(group, line(name, keys));
        }
    }
    if let Some(key) = keymap.key(&Command::LineBreak) {
        add("Text", line("Line break", vec![written(key)]));
    }

    // The caret's keys differ between systems, so they are asked of the keymap as well.
    let to = |to: Motion| keymap.key(&Command::Move { to, extend: false }).map(&written);
    let pair = |name: &str, back: Motion, on: Motion| line(name, to(back).into_iter().chain(to(on)).collect());
    // Shift alone, as each system's menus write it.
    let shift = if mac { "\u{21e7}" } else { "Shift" }.to_owned();
    let mut caret = vec![
        pair("Word left, right", Motion::WordLeft, Motion::WordRight),
        pair("Start, end of line", Motion::LineStart, Motion::LineEnd),
        pair("Start, end of paragraph", Motion::BlockStart, Motion::BlockEnd),
        pair("Start, end of document", Motion::DocStart, Motion::DocEnd),
        line("Select while moving", vec![shift]),
    ];
    let word = keymap.key(&Command::Delete(Motion::WordLeft)).map(&written);
    caret.extend(word.map(|key| line("Delete a word", vec![key])));
    groups.push(Group { title: "Caret", lines: caret });

    let back = || line("Back to the document", vec![written(&plain(Key::Escape))]);
    let steps = vec![written(&plain(Key::Left)), written(&plain(Key::Right))];
    groups.push(Group { title: "History", lines: vec![line("Earlier, later version", steps), back()] });
    let tab = |shift| written(&KeyEvent { key: Key::Tab, mods: Modifiers { shift, ..NONE } });
    let mut print = vec![
        line("Next, previous place for a page break", vec![tab(false), tab(true)]),
        line("Add or remove the page break", vec![written(&plain(Key::Enter))]),
    ];
    print.extend(commands::shortcut(&Act::Print, keymap, mac).map(|key| line("Print", vec![key])));
    print.push(back());
    groups.push(Group { title: "Print view", lines: print });

    groups.retain(|group| !group.lines.is_empty());
    groups
}

/// Where the second column starts: the place that leaves the two nearest to one height.
fn halved(groups: &[Group]) -> usize {
    // A heading takes the room of some two lines.
    let tall = |groups: &[Group]| groups.iter().map(|group| group.lines.len() + 2).sum::<usize>();
    (0..=groups.len()).min_by_key(|at| tall(&groups[..*at]).abs_diff(tall(&groups[*at..]))).unwrap_or(0)
}

/// A key as a cap: the look of a key on the board.
fn cap(keys: &str) -> Element<Message> {
    let style = TextStyle::control().size(tokens::TEXT_SM);
    Element::row()
        .align(Align::Center)
        .padding(Edges::xy(6.0, 2.0))
        .bg(tokens::TONE_1)
        .border(Edges::all(1.0), tokens::LINE_STRONG)
        .radius(tokens::RADIUS_SM)
        .child(Element::text(keys, style).fg(tokens::FG_MUTED))
}

fn column(groups: &[Group]) -> Element<Message> {
    let mut column = Element::column().grow(1.0);
    for group in groups {
        column = column.child(SidePanel::section(group.title));
        for line in &group.lines {
            let name = Element::row().shrink().clip().child(Element::text(line.name.as_str(), quiet()));
            let keys = Element::row().gap(tokens::SPACE_XS).children(line.keys.iter().map(|keys| cap(keys)));
            let row = Element::row().align(Align::Center).justify(Justify::Between).gap(tokens::SPACE_MD);
            column = column.child(row.min_height(LINE).child(name).child(keys));
        }
    }
    column
}

/// The dialog over the middle of a window of the size `window`. It hangs from an element of no size, as
/// the settings do.
pub(crate) fn view(keymap: &Keymap, (mac, application): (bool, bool), window: Size) -> Element<Message> {
    let groups = groups(keymap, mac, application);
    let width = WIDTH.min(window.width - 2.0 * MARGIN).max(0.0);
    let head = Element::row()
        .align(Align::Center)
        .justify(Justify::Between)
        .child(Element::text("Keyboard shortcuts", TextStyle::title().size(tokens::TEXT_XL)))
        .child(Button::icon_only(icon::X, "Close").subtle().small().on_press(Message::Shortcuts(false)));
    let note = match commands::shortcut(&Act::Palette, keymap, mac) {
        Some(key) => format!("Everything else is done by name in Commands ({key})."),
        None => "Everything else is done by name in Commands.".to_owned(),
    };
    let note = Element::paragraph(note, quiet()).fg(tokens::FG_MUTED).padding(Edges::top(tokens::SPACE_XS));
    let side = Edges::xy(tokens::SPACE_XL, 0.0);
    // A narrow window has one column, which reads in the order of the groups.
    let at = if width < TWO { groups.len() } else { halved(&groups) };
    let (first, second) = groups.split_at(at);
    let columns = Element::row().align(Align::Start).gap(tokens::SPACE_2XL).child(column(first));
    let columns = if second.is_empty() { columns } else { columns.child(column(second)) };
    // The lines scroll under the title in a window too low for them, and run to the dialog's edges so
    // that the thumb stands clear of them.
    let lines = Element::column()
        .padding(Edges { bottom: tokens::SPACE_XL, ..side })
        .child(columns)
        .max_height((window.height - AROUND).max(SHORTEST))
        .scroll()
        .role(Role::Group)
        .label("Shortcuts");
    let content = Element::column()
        .width(width)
        .padding(Edges::top(tokens::SPACE_LG))
        .child(Element::column().padding(side).child(head).child(note))
        .child(lines);
    let dialog = Popover::new(content)
        .flush()
        .label("Keyboard shortcuts")
        .max_width(width)
        .modal()
        .open(true)
        .on_close(Message::Shortcuts(false));
    Element::row().overlay().popover(dialog)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys_of<'a>(groups: &'a [Group], title: &str, name: &str) -> Option<&'a [String]> {
        let group = groups.iter().find(|group| group.title == title)?;
        group.lines.iter().find(|line| line.name == name).map(|line| line.keys.as_slice())
    }

    #[test]
    fn the_sheet_writes_the_keys_of_each_system_as_they_are_bound() {
        // Whether the keys are a Mac's, the group, the line, and the keys written on it.
        let cases: [(bool, &str, &str, &[&str]); 12] = [
            (false, "Format", "Bold", &["Ctrl+B"]),
            (false, "Edit", "Redo", &["Ctrl+Shift+Z", "Ctrl+Y"]),
            (true, "Edit", "Redo", &["\u{21e7}\u{2318}Z"]),
            (false, "Edit", "Find and replace", &["Ctrl+H"]),
            (true, "Edit", "Find and replace", &["\u{2325}\u{2318}F"]),
            (false, "View", "Commands", &["Ctrl+Shift+P"]),
            (false, "View", "Keyboard shortcuts", &["Ctrl+/"]),
            (false, "Caret", "Word left, right", &["Ctrl+\u{2190}", "Ctrl+\u{2192}"]),
            (true, "Caret", "Word left, right", &["\u{2325}\u{2190}", "\u{2325}\u{2192}"]),
            (false, "Caret", "Select while moving", &["Shift"]),
            (true, "Caret", "Select while moving", &["\u{21e7}"]),
            (false, "Print view", "Print", &["Ctrl+P"]),
        ];
        for (mac, title, name, keys) in cases {
            let keymap = if mac { Keymap::mac() } else { Keymap::standard() };
            let groups = groups(&keymap, mac, true);
            let written: Option<Vec<&str>> =
                keys_of(&groups, title, name).map(|keys| keys.iter().map(String::as_str).collect());
            assert_eq!(written.as_deref(), Some(keys), "{title}, {name}");
        }
    }

    #[test]
    fn a_command_without_a_key_is_left_to_the_palette() {
        let groups = groups(&Keymap::standard(), false, true);
        assert_eq!(keys_of(&groups, "Document", "Share"), None);
        assert!(groups.iter().all(|group| group.title != "Table"), "no command of a table has a key");
        assert!(groups.iter().all(|group| group.lines.iter().all(|line| !line.keys.is_empty())));
    }

    #[test]
    fn two_columns_are_near_one_height() {
        let groups = groups(&Keymap::standard(), false, true);
        let at = halved(&groups);
        let tall = |groups: &[Group]| groups.iter().map(|group| group.lines.len() + 2).sum::<usize>();
        let (first, second) = (tall(&groups[..at]), tall(&groups[at..]));
        let longest = groups.iter().map(|group| group.lines.len() + 2).max().unwrap_or(0);
        assert!(first.abs_diff(second) <= longest, "{first} and {second} lines");
    }
}
