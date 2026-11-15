//! Renaming a document where its name stands in the bar, and moving it from the flyout under the name.
//!
//! The window only asks: the shell renames and moves, and answers with the document's new place or with why
//! it could not.

use accesskit::Role;

use crate::element::Element;
use crate::folders::{self, Browse, Tree};
use crate::opening::{Location, Request};
use crate::style::{Align, Edges};
use crate::text::TextStyle;
use crate::tokens;
use crate::widgets::{Button, ControlSize, Popover, TextField};
use crate::window::Message;

/// The most room the name field takes in the bar.
const FIELD_WIDTH: f32 = 200.0;
const FLYOUT_WIDTH: f32 = 300.0;
/// The most rows the tree shows before it scrolls.
const ROWS: f32 = 9.5;

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
    /// What is done among the folders: one is picked to move to, opened or closed, or a new one asked for.
    Browse(Browse),
    /// Asks the shell to let the person choose a folder that is not in the tree.
    Elsewhere,
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
    /// The folders, with the one picked to move to.
    tree: Tree,
    /// Why the shell could not do what was last asked.
    pub notice: Option<String>,
}

impl Renaming {
    /// Starts from the document's own name. The tree opens as far as the folder the document is in.
    pub fn open(location: &Location) -> Self {
        Self { name: location.name.clone().unwrap_or_default(), tree: Tree::open(location), ..Self::default() }
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
                match (&message, &self.tree.picked) {
                    (Rename::Apply, Some(target)) => requests.push(Request::Move(target.clone())),
                    (Rename::Elsewhere, _) => requests.push(Request::MoveElsewhere),
                    _ => {}
                }
                self.notice = None;
                return if requests.len() > before { Outcome::Asked } else { Outcome::Closed };
            }
            Rename::Cancel => return Outcome::Closed,
            Rename::Browse(message) => {
                if matches!(message, Browse::Pick(_) | Browse::MakeFolder) {
                    self.notice = None;
                }
                self.tree.update(message, location, requests);
                self.stay(location);
            }
        }
        Outcome::Open
    }

    /// Takes in what the shell now says of the folders: one that was asked for is picked, and one that is
    /// gone or now holds the document no longer is.
    pub fn placed(&mut self, location: &Location) {
        self.tree.placed(location);
        self.stay(location);
    }

    /// The folder the document is in is nowhere to move it to.
    fn stay(&mut self, location: &Location) {
        self.tree.picked = self.tree.picked.take().filter(|picked| *picked != location.folder);
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
            let title = Element::text(folders::title(location), quiet.weight(600)).fg(tokens::FG_MUTED);
            let tree = self.tree.list(location, "Move to", ROWS, browse);
            // The row that reaches the folders the tree does not list, with the line over it.
            let other =
                || Element::column().padding(inset).child(folders::elsewhere(Message::Rename(Rename::Elsewhere)));
            let other = (!location.closed).then(|| [line(), other()]).into_iter().flatten();
            flyout = flyout
                .child(Element::column().padding(Edges { bottom: 0.0, ..Edges::all(tokens::SPACE_MD) }).child(title))
                .child(Element::column().padding(inset).child(tree))
                .children(other)
                .child(line())
                .child(self.foot(location));
        }
        let flyout = Popover::new(flyout).label("Move this document").flush().max_width(FLYOUT_WIDTH + 2.0);
        Some(flyout.beside().open(true).on_close(Message::Rename(Rename::Close)))
    }

    /// The flyout's last row: a new folder, and the move itself.
    fn foot(&self, location: &Location) -> Element<Message> {
        let folder = self.tree.new_folder(location, browse);
        let moving = match self.tree.picked_name(location) {
            Some(target) => Button::new(format!("Move to {target}")).on_press(Message::Rename(Rename::Apply)),
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

fn browse(message: Browse) -> Message {
    Message::Rename(Rename::Browse(message))
}
