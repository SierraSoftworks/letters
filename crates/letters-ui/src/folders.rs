//! The folders a document can go to, as a tree to pick one from: under the name in the bar when a document
//! is moved, and on the card that saves a copy of it.

use std::collections::BTreeSet;

use accesskit::Role;
use letters_editor::{Key, KeyEvent};

use crate::color::fade;
use crate::element::Element;
use crate::icon;
use crate::opening::{Folder, Location, Request};
use crate::style::{Align, Dir, Edges, Look};
use crate::text::TextStyle;
use crate::theme::accent;
use crate::tokens;
use crate::widgets::{Button, ControlSize, Popover, Side, TextField};
use crate::window::Message;

/// A folder's row: its height, and how far it steps in for each level down the tree.
pub(crate) const ROW_HEIGHT: f32 = 32.0;
const INDENT: f32 = 18.0;
/// The room a row keeps for the arrow that opens it.
const ARROW: f32 = 18.0;

/// What the rows of a tree send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Browse {
    /// Picks a folder, by its path.
    Pick(String),
    /// Shows or hides what is in a folder.
    Unfold(String, bool),
    /// What the new folder's name field holds, while it is asked for.
    Folder(Option<String>),
    /// Asks the shell for the new folder.
    MakeFolder,
}

/// Which folder of a tree is picked, and which show what is in them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Tree {
    /// The folder that is picked, by its path.
    pub picked: Option<String>,
    open: BTreeSet<String>,
    /// The name typed for a new folder, while it is asked for.
    naming: Option<String>,
    /// A folder the shell was asked to make, to be picked once it is listed: its parent's path and its name.
    made: Option<(String, String)>,
}

impl Tree {
    /// A tree open as far as the folder the document is in, with nothing picked.
    pub fn open(location: &Location) -> Self {
        let folders = &location.folders;
        let mut open: BTreeSet<String> = folders.iter().filter(|folder| folder.depth == 0).map(path).collect();
        if let Some(at) = folders.iter().position(|folder| folder.path == location.folder) {
            open.insert(location.folder.clone());
            open.extend(ancestors(folders, at).map(path));
        }
        Self { open, ..Self::default() }
    }

    /// Does what a row or the new folder's field asked. A new folder is asked of the shell.
    pub fn update(&mut self, message: Browse, location: &Location, requests: &mut Vec<Request>) {
        match message {
            Browse::Pick(path) => self.picked = Some(path),
            Browse::Unfold(path, true) => _ = self.open.insert(path),
            Browse::Unfold(path, false) => _ = self.open.remove(&path),
            Browse::Folder(name) => self.naming = name,
            Browse::MakeFolder => {
                let name = self.naming.take().unwrap_or_default().trim().to_owned();
                if !name.is_empty() {
                    let parent = self.picked.clone().unwrap_or_else(|| location.folder.clone());
                    requests.push(Request::NewFolder { parent: parent.clone(), name: name.clone() });
                    self.made = Some((parent, name));
                }
            }
        }
    }

    /// Takes in what the shell now says of the folders: one that was asked for is picked, and one that is
    /// gone no longer is.
    pub fn placed(&mut self, location: &Location) {
        let folders = &location.folders;
        if let Some((parent, name)) = self.made.take()
            && let Some(at) = folders.iter().position(|folder| folder.path == parent)
            && let Some(made) = children(folders, at).find(|folder| folder.name == name)
        {
            self.picked = Some(made.path.clone());
            self.open.insert(parent);
        }
        self.picked = self.picked.take().filter(|picked| folders.iter().any(|folder| folder.path == *picked));
    }

    /// The name of the folder that is picked.
    pub fn picked_name<'a>(&self, location: &'a Location) -> Option<&'a str> {
        named(location, self.picked.as_ref()?)
    }

    /// The tree itself, which scrolls once it has more than `most` rows.
    pub fn list(&self, location: &Location, label: &str, most: f32, send: fn(Browse) -> Message) -> Element<Message> {
        Element::column()
            .role(Role::Tree)
            .label(label)
            .arrows(Dir::Column)
            .one_stop()
            .max_height(most * ROW_HEIGHT)
            .scroll()
            .children(self.rows(location, send))
    }

    /// The button that asks for a new folder's name. The folder goes into the one that is picked, or else
    /// beside the document.
    pub fn new_folder(&self, location: &Location, send: fn(Browse) -> Message) -> Button<Message> {
        let parent = named(location, self.picked.as_ref().unwrap_or(&location.folder));
        let quiet = TextStyle::control().size(tokens::TEXT_SM);
        let naming = TextField::new("Folder name", self.naming.as_deref().unwrap_or_default())
            .placeholder("New folder")
            .size(ControlSize::Small)
            .on_input(move |name| send(Browse::Folder(Some(name))))
            .on_submit(send(Browse::MakeFolder));
        let mut asker = Element::column().gap(tokens::SPACE_SM).width(220.0);
        if let Some(parent) = parent {
            asker = asker.child(Element::text(format!("New folder in {parent}"), quiet).fg(tokens::FG_MUTED).shrink());
        }
        let asker = Popover::new(asker.child(naming))
            .label("New folder")
            .side(Side::Top)
            .open(self.naming.is_some())
            .on_close(send(Browse::Folder(None)));
        Button::new("New folder")
            .icon(icon::FOLDER_PLUS)
            .subtle()
            .small()
            .on_press(send(Browse::Folder(Some(String::new()))))
            .popover(asker)
    }

    /// The rows of the tree that show: a closed folder hides everything under it. `send` makes the window's
    /// message of what a row asks for.
    fn rows(&self, location: &Location, send: fn(Browse) -> Message) -> Vec<Element<Message>> {
        let folders = &location.folders;
        let mut rows = Vec::new();
        let mut hidden: Option<u8> = None;
        for (at, folder) in folders.iter().enumerate() {
            if hidden.is_some_and(|depth| folder.depth > depth) {
                continue;
            }
            let parent = folders.get(at + 1).is_some_and(|next| next.depth > folder.depth);
            let open = parent.then(|| self.open.contains(&folder.path));
            hidden = (open == Some(false)).then_some(folder.depth);
            rows.push(self.folder(folder, open, folder.path == location.folder, send));
        }
        rows
    }

    /// One folder's row. `open` says whether a folder that holds others shows them, and `here` whether the
    /// document is in it.
    fn folder(&self, folder: &Folder, open: Option<bool>, here: bool, send: fn(Browse) -> Message) -> Element<Message> {
        let unfold = |to: bool| send(Browse::Unfold(folder.path.clone(), to));
        let arrow = match open {
            Some(open) => {
                let arrow = if open { icon::CHEVRON_DOWN } else { icon::CHEVRON_RIGHT };
                Element::row().child(Element::icon(arrow, 14.0).fg(tokens::FG_MUTED)).on_click(unfold(!open))
            }
            None => Element::row(),
        };
        let glyph = match (folder.depth, open) {
            (0, _) => icon::USERS,
            (_, Some(true)) => icon::FOLDER_OPEN,
            _ => icon::FOLDER,
        };
        // The arrow keys along the tree's other axis open and close a folder.
        let (opens, closes) = (unfold(true), unfold(false));
        let keys = move |event: &KeyEvent| match (&event.key, open) {
            _ if event.mods.primary || event.mods.alt => None,
            (Key::Right, Some(false)) => Some(opens.clone()),
            (Key::Left, Some(true)) => Some(closes.clone()),
            _ => None,
        };
        let picked = self.picked.as_deref() == Some(folder.path.as_str());
        let mut row = row(folder.depth)
            .key(folder.path.as_str())
            .role(Role::TreeItem)
            .label(folder.name.as_str())
            .selected(picked)
            .on_press(send(Browse::Pick(folder.path.clone())))
            .on_key(keys)
            .child(arrow.width(ARROW).height(ROW_HEIGHT).align(Align::Center).decorative())
            .child(Element::icon(glyph, 16.0).fg(tokens::FG_MUTED))
            .child(Element::text(folder.name.as_str(), TextStyle::control()).shrink());
        if let Some(open) = open {
            row = row.expanded(open);
        }
        if picked {
            let over = Look::bg(fade(accent().base, 0.18));
            row = row.bg(accent().soft).hover(over).focused(over);
        }
        if here {
            let quiet = TextStyle::control().size(tokens::TEXT_SM);
            let mark = Element::row()
                .align(Align::Center)
                .gap(tokens::SPACE_XS)
                .fg(tokens::FG_MUTED)
                .child(Element::icon(icon::CHECK, 14.0))
                .child(Element::text("It is here", quiet));
            row = row.description("It is here").child(Element::row().grow(1.0)).child(mark);
        }
        row
    }
}

/// What stands over a tree: the space its folders are in. A document kept outside the space is listed
/// first, beside the space, and then there are two.
pub(crate) fn title(location: &Location) -> String {
    let mut roots = location.folders.iter().filter(|folder| folder.depth == 0);
    match (roots.next(), roots.next()) {
        (Some(space), None) => format!("Folders in {}", space.name),
        _ => "Folders".into(),
    }
}

/// The row under a tree that reaches the folders it does not list.
pub(crate) fn elsewhere(message: Message) -> Element<Message> {
    row(0)
        .role(Role::Button)
        .on_press(message)
        .child(Element::row().width(ARROW))
        .child(Element::icon(icon::FOLDER_SEARCH, 16.0).fg(tokens::FG_MUTED))
        .child(Element::text("Other location\u{2026}", TextStyle::control()))
}

fn named<'a>(location: &'a Location, path: &String) -> Option<&'a str> {
    location.folders.iter().find(|folder| folder.path == *path).map(|folder| folder.name.as_str())
}

/// A row among folders, stepped in for its place in the tree.
fn row(depth: u8) -> Element<Message> {
    Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_SM)
        .height(ROW_HEIGHT)
        .padding(Edges { left: f32::from(depth) * INDENT, right: tokens::SPACE_SM, ..Edges::ZERO })
        .radius(tokens::RADIUS_CONTROL)
        .hover(Look::bg(tokens::NEUTRAL_SOFT))
        .focused(Look::bg(tokens::NEUTRAL_SOFT))
        .focusable()
}

fn path(folder: &Folder) -> String {
    folder.path.clone()
}

/// The folders that hold the one at `at`, from the nearest: each is the last before it that is less deep.
fn ancestors(folders: &[Folder], at: usize) -> impl Iterator<Item = &Folder> {
    let mut depth = folders[at].depth;
    folders[..at].iter().rev().filter(move |folder| {
        let above = folder.depth < depth;
        depth = depth.min(folder.depth);
        above
    })
}

/// The folders directly in the one at `at`.
fn children(folders: &[Folder], at: usize) -> impl Iterator<Item = &Folder> {
    let depth = folders[at].depth;
    folders[at + 1..]
        .iter()
        .take_while(move |folder| folder.depth > depth)
        .filter(move |folder| folder.depth == depth + 1)
}
