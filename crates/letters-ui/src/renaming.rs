//! Renaming a document where its name stands in the bar, and moving it from the flyout under the name.
//!
//! The window only asks: the shell renames and moves, and answers with the document's new place or with why
//! it could not.

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

/// The most room the name field takes in the bar.
const FIELD_WIDTH: f32 = 200.0;
const FLYOUT_WIDTH: f32 = 300.0;
/// A folder's row: its height, and how far it steps in for each level down the tree.
const ROW_HEIGHT: f32 = 32.0;
const INDENT: f32 = 18.0;
/// The most rows the tree shows before it scrolls.
const ROWS: f32 = 9.5;
/// The room a row keeps for the arrow that opens it.
const ARROW: f32 = 18.0;

/// What the name field and its flyout send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rename {
    /// Opens the name to be typed over.
    Open,
    /// What the name field holds.
    Name(String),
    /// Takes the name that was typed, and moves the document to the folder that was picked.
    Apply,
    /// Puts the field away. A name that was typed is taken, a folder that was picked is not.
    Close,
    /// Puts the field away and drops what was typed.
    Cancel,
    /// Picks the folder to move to, by its path.
    Pick(String),
    /// Shows or hides what is in a folder.
    Unfold(String, bool),
    /// Asks the shell to let the person choose a folder that is not in the tree.
    Elsewhere,
    /// What the new folder's name field holds, while it is asked for.
    Folder(Option<String>),
    /// Asks the shell for the new folder.
    MakeFolder,
}

/// How a message left the field.
pub(crate) enum Outcome {
    Open,
    /// Put away, with something asked of the shell that it may refuse.
    Asked,
    Closed,
}

/// The name being typed in the bar, and the folder picked under it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Renaming {
    name: String,
    /// The folder picked to move to, by its path.
    target: Option<String>,
    /// The folders that show what is in them.
    open: BTreeSet<String>,
    /// The name typed for a new folder, while it is asked for.
    folder: Option<String>,
    /// A folder the shell was asked to make, to be picked once it is listed: its parent's path and its name.
    made: Option<(String, String)>,
    /// Why the shell could not do what was last asked.
    pub notice: Option<String>,
}

impl Renaming {
    /// Starts from the document's own name. The tree opens as far as the folder the document is in.
    pub fn open(location: &Location) -> Self {
        let folders = &location.folders;
        let mut open: BTreeSet<String> = folders.iter().filter(|folder| folder.depth == 0).map(path).collect();
        if let Some(at) = folders.iter().position(|folder| folder.path == location.folder) {
            open.insert(location.folder.clone());
            open.extend(ancestors(folders, at).map(path));
        }
        Self { name: location.name.clone().unwrap_or_default(), open, ..Self::default() }
    }

    pub fn update(&mut self, message: Rename, location: &Location, requests: &mut Vec<Request>) -> Outcome {
        match message {
            Rename::Open => {}
            Rename::Name(name) => (self.name, self.notice) = (name, None),
            Rename::Apply | Rename::Close | Rename::Elsewhere => {
                let before = requests.len();
                let name = self.name.trim();
                if !name.is_empty() && location.name.as_deref() != Some(name) {
                    requests.push(Request::Rename(name.to_owned()));
                }
                match (&message, &self.target) {
                    (Rename::Apply, Some(target)) => requests.push(Request::Move(target.clone())),
                    (Rename::Elsewhere, _) => requests.push(Request::MoveElsewhere),
                    _ => {}
                }
                self.notice = None;
                return if requests.len() > before { Outcome::Asked } else { Outcome::Closed };
            }
            Rename::Cancel => return Outcome::Closed,
            Rename::Pick(path) => {
                (self.target, self.notice) = (Some(path).filter(|path| *path != location.folder), None)
            }
            Rename::Unfold(path, true) => _ = self.open.insert(path),
            Rename::Unfold(path, false) => _ = self.open.remove(&path),
            Rename::Folder(name) => self.folder = name,
            Rename::MakeFolder => {
                let name = self.folder.take().unwrap_or_default().trim().to_owned();
                if !name.is_empty() {
                    let parent = self.target.clone().unwrap_or_else(|| location.folder.clone());
                    requests.push(Request::NewFolder { parent: parent.clone(), name: name.clone() });
                    (self.made, self.notice) = (Some((parent, name)), None);
                }
            }
        }
        Outcome::Open
    }

    /// Takes in what the shell now says of the folders: one that was asked for is picked, and one that is
    /// gone or now holds the document no longer is.
    pub fn placed(&mut self, location: &Location) {
        let folders = &location.folders;
        if let Some((parent, name)) = self.made.take()
            && let Some(at) = folders.iter().position(|folder| folder.path == parent)
            && let Some(made) = children(folders, at).find(|folder| folder.name == name)
        {
            self.target = Some(made.path.clone());
            self.open.insert(parent);
        }
        let listed =
            |target: &String| *target != location.folder && folders.iter().any(|folder| folder.path == *target);
        self.target = self.target.take().filter(listed);
    }

    /// The field that stands where the name did, in no more than `room`. `unnamed` is what the bar calls a
    /// document that has no name of its own.
    pub fn field(&self, unnamed: &str, room: f32, location: &Location) -> Element<Message> {
        let send = |message: Rename| Message::Rename(message);
        let flyout = self.flyout(location);
        let mut field = TextField::new("Document name", &self.name)
            .placeholder(unnamed)
            .size(ControlSize::Small)
            .weight(600)
            .invalid(self.notice.is_some())
            .on_input(|name| Message::Rename(Rename::Name(name)))
            .on_submit(send(Rename::Apply))
            .on_cancel(send(Rename::Cancel));
        if flyout.is_none() {
            // With no flyout to close, leaving the field is what puts it away.
            field = field.on_leave(send(Rename::Close));
        }
        let field = Element::from(field).key("renaming").width(FIELD_WIDTH.min(room)).shrink().takes_focus();
        match flyout {
            Some(flyout) => field.popover(flyout),
            None => field,
        }
    }

    /// What hangs under the field: why the shell refused, and the folders the document can move to.
    fn flyout(&self, location: &Location) -> Option<Popover<Message>> {
        let folders = &location.folders;
        if folders.is_empty() && self.notice.is_none() {
            return None;
        }
        let quiet = TextStyle::control().size(tokens::TEXT_SM);
        let line = || Element::row().height(1.0).bg(tokens::LINE).decorative();
        let inset = Edges::xy(tokens::SPACE_SM, tokens::SPACE_XS);
        let mut flyout = Element::column().width(FLYOUT_WIDTH);
        if let Some(notice) = &self.notice {
            let said = Element::paragraph(notice, quiet).fg(tokens::DANGER).role(Role::Alert).label(notice);
            flyout = flyout.child(Element::column().padding(Edges::all(tokens::SPACE_MD)).child(said));
        }
        if !folders.is_empty() {
            if self.notice.is_some() {
                flyout = flyout.child(line());
            }
            // A document kept outside the space is listed first, beside the space, and then there are two.
            let roots: Vec<&Folder> = folders.iter().filter(|folder| folder.depth == 0).collect();
            let title = match roots[..] {
                [space] => format!("Folders in {}", space.name),
                _ => "Folders".into(),
            };
            let title = Element::text(title, quiet.weight(600)).fg(tokens::FG_MUTED);
            let tree = Element::column()
                .role(Role::Tree)
                .label("Move to")
                .arrows(Dir::Column)
                .one_stop()
                .max_height(ROWS * ROW_HEIGHT)
                .scroll()
                .children(self.rows(location));
            let other = row(0)
                .role(Role::Button)
                .on_press(Message::Rename(Rename::Elsewhere))
                .child(Element::row().width(ARROW))
                .child(Element::icon(icon::FOLDER_SEARCH, 16.0).fg(tokens::FG_MUTED))
                .child(Element::text("Other location\u{2026}", TextStyle::control()));
            flyout = flyout
                .child(Element::column().padding(Edges { bottom: 0.0, ..Edges::all(tokens::SPACE_MD) }).child(title))
                .child(Element::column().padding(inset).child(tree))
                .child(line())
                .child(Element::column().padding(inset).child(other))
                .child(line())
                .child(self.foot(location));
        }
        let flyout = Popover::new(flyout).label("Move this document").flush().max_width(FLYOUT_WIDTH + 2.0);
        Some(flyout.beside().open(true).on_close(Message::Rename(Rename::Close)))
    }

    /// The rows of the tree that show: a closed folder hides everything under it.
    fn rows(&self, location: &Location) -> Vec<Element<Message>> {
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
            rows.push(self.folder(folder, open, folder.path == location.folder));
        }
        rows
    }

    /// One folder's row. `open` says whether a folder that holds others shows them, and `here` whether the
    /// document is in it.
    fn folder(&self, folder: &Folder, open: Option<bool>, here: bool) -> Element<Message> {
        let unfold = |to: bool| Message::Rename(Rename::Unfold(folder.path.clone(), to));
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
        let picked = self.target.as_deref() == Some(folder.path.as_str());
        let mut row = row(folder.depth)
            .key(folder.path.as_str())
            .role(Role::TreeItem)
            .label(folder.name.as_str())
            .selected(picked)
            .on_press(Message::Rename(Rename::Pick(folder.path.clone())))
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

    /// The flyout's last row: a new folder, and the move itself.
    fn foot(&self, location: &Location) -> Element<Message> {
        let send = |message: Rename| Message::Rename(message);
        let named =
            |path: &String| location.folders.iter().find(|folder| folder.path == *path).map(|folder| &folder.name);
        // A new folder goes into the one that is picked, or else beside the document.
        let parent = named(self.target.as_ref().unwrap_or(&location.folder));
        let quiet = TextStyle::control().size(tokens::TEXT_SM);
        let naming = TextField::new("Folder name", self.folder.as_deref().unwrap_or_default())
            .placeholder("New folder")
            .size(ControlSize::Small)
            .on_input(|name| Message::Rename(Rename::Folder(Some(name))))
            .on_submit(send(Rename::MakeFolder));
        let mut asker = Element::column().gap(tokens::SPACE_SM).width(220.0);
        if let Some(parent) = parent {
            asker = asker.child(Element::text(format!("New folder in {parent}"), quiet).fg(tokens::FG_MUTED).shrink());
        }
        let asker = Popover::new(asker.child(naming))
            .label("New folder")
            .side(Side::Top)
            .open(self.folder.is_some())
            .on_close(send(Rename::Folder(None)));
        let folder = Button::new("New folder")
            .icon(icon::FOLDER_PLUS)
            .subtle()
            .small()
            .on_press(send(Rename::Folder(Some(String::new()))))
            .popover(asker);
        let moving = match self.target.as_ref().and_then(named) {
            Some(target) => Button::new(format!("Move to {target}")).on_press(send(Rename::Apply)),
            None => Button::new("Move").disabled(true),
        };
        Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .padding(Edges::xy(tokens::SPACE_SM, tokens::SPACE_SM))
            .child(folder)
            .child(Element::row().grow(1.0))
            .child(moving.primary().small())
    }
}

/// A row of the flyout, stepped in for its place in the tree.
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
