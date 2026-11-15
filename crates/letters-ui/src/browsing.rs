//! Opening a document a server keeps: one card lists the places of a server the person is signed in to and
//! what each holds, and the document that is pressed is asked of the shell.
//!
//! The window only asks: the shell talks to the server, and answers with what a place holds or with why it
//! could not be read.

use accesskit::Role;
use letters_api::{Entry, EntryKind, Role as Grade, Space, SpaceKind, Tag};

use crate::color::fade;
use crate::element::Element;
use crate::icon::{self, Icon};
use crate::opening::{Request, day, day_called};
use crate::servers::{Server, host};
use crate::style::{Align, Dir, Edges, Justify, Look, Ring};
use crate::text::TextStyle;
use crate::tokens::{self, Color as Ink};
use crate::widgets::{Button, Popover, Segmented};
use crate::window::Message;

const WIDTH: f32 = 760.0;
/// How wide the places are, and how tall they and what they hold stand, whatever is in them.
const PLACES: f32 = 200.0;
const HEIGHT: f32 = 340.0;
const ROW: f32 = 36.0;

/// A place among the documents a server keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// The server itself: the spaces the person has there.
    Spaces,
    /// A folder of a space, by the space's id and the folder's path in it as
    /// [`Entry::path`](letters_api::Entry::path) writes it. The space itself has an empty path.
    Folder { space: String, path: String },
    /// The documents others shared with the person.
    Shared,
    /// The documents the person had open lately.
    Recent,
}

/// What a place holds, as its server answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Browsed {
    /// The [`Server::address`] of the server that answered.
    pub server: String,
    pub place: Place,
    /// The person's spaces, for [`Place::Spaces`].
    pub spaces: Vec<Space>,
    /// What every other place holds: its folders first, then its documents.
    pub entries: Vec<Entry>,
}

/// What the card asks of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Browsing {
    Show,
    Close,
    /// Look at the server at this address.
    Server(String),
    /// Look at a place of the server.
    Go(Place),
    /// Open the document with this id.
    Open(String),
    /// Choose a file of this device instead.
    Device,
    /// Start a document in the folder that is looked at.
    New,
}

#[derive(Default)]
pub(crate) struct Browser {
    open: bool,
    /// The address of the server that is looked at.
    server: String,
    /// Its spaces, once it has said.
    spaces: Option<Vec<Space>>,
    /// The place that is looked at, and what it holds once the server has said.
    place: Option<Place>,
    entries: Option<Vec<Entry>>,
    /// Why the last place could not be read.
    notice: Option<String>,
    /// The time when the card opened, in seconds since 1970, and the person's zone in minutes.
    now: Option<u64>,
    zone: i32,
}

impl Browser {
    pub fn open(&self) -> bool {
        self.open
    }

    pub fn close(&mut self) {
        (self.open, self.notice) = (false, None);
    }

    /// Takes in what a place holds. An answer about a place that is no longer looked at is late, and is
    /// let go.
    pub fn told(&mut self, listing: Browsed, requests: &mut Vec<Request>) {
        if !self.open || listing.server != self.server {
            return;
        }
        if listing.place == Place::Spaces {
            // The card opens on the person's own space, or on what is shared with them if they have none.
            if self.place.is_none() {
                let own = listing.spaces.iter().find(|space| space.kind == SpaceKind::Personal);
                let first = own.or(listing.spaces.first()).map(|space| within(space, ""));
                self.go(first.unwrap_or(Place::Shared), requests);
            }
            self.spaces = Some(listing.spaces);
        } else if self.place.as_ref() == Some(&listing.place) {
            (self.entries, self.notice) = (Some(listing.entries), None);
        }
    }

    /// Takes in why the place that was asked for could not be read. One that comes when nothing is asked
    /// is about a place that was left.
    pub fn refused(&mut self, why: &str) {
        if self.open && self.entries.is_none() {
            self.notice = Some(why.to_owned());
        }
    }

    /// Takes in where the person is signed in. The card of a server they are signed out of goes.
    pub fn signed(&mut self, servers: &[Server]) {
        if !servers.iter().any(|server| server.address == self.server) {
            self.close();
        }
    }

    /// Does what the card asked. `servers` are those the person is signed in to, and `clock` the time now
    /// with the person's zone.
    pub fn update(
        &mut self,
        message: Browsing,
        servers: &[Server],
        clock: (Option<u64>, i32),
        requests: &mut Vec<Request>,
    ) {
        match message {
            Browsing::Show => {
                // The server that was looked at last, while the person is still signed in to it.
                let known = servers.iter().find(|server| server.address == self.server);
                let Some(server) = known.or(servers.first()) else { return };
                (self.open, (self.now, self.zone)) = (true, clock);
                self.look(server.address.clone(), requests);
            }
            Browsing::Close => self.close(),
            Browsing::Server(server) => {
                if server != self.server && servers.iter().any(|known| known.address == server) {
                    self.look(server, requests);
                }
            }
            Browsing::Go(place) => self.go(place, requests),
            Browsing::Open(id) => {
                requests.push(Request::OpenFrom { server: self.server.clone(), id });
                self.close();
            }
            Browsing::Device => {
                requests.push(Request::OpenDialog);
                self.close();
            }
            Browsing::New => {
                if let Some((space, path)) = self.written() {
                    let (server, space, path) = (self.server.clone(), space.to_owned(), path.to_owned());
                    requests.push(Request::NewAt { server, space, path });
                    self.close();
                }
            }
        }
    }

    /// The folder that is looked at, by its space and its path there, if the person may write in it: a
    /// document is started only in a space they edit.
    fn written(&self) -> Option<(&str, &str)> {
        let Some(Place::Folder { space, path }) = &self.place else { return None };
        let known = self.spaces.as_deref()?.iter().find(|known| known.id == *space)?;
        (known.role >= Grade::Editor).then_some((space.as_str(), path.as_str()))
    }

    /// Turns to a server, and asks what spaces the person has there.
    fn look(&mut self, server: String, requests: &mut Vec<Request>) {
        (self.spaces, self.place, self.entries, self.notice) = (None, None, None, None);
        requests.push(Request::Browse { server: server.clone(), place: Place::Spaces });
        self.server = server;
    }

    /// Turns to a place of the server, and asks what it holds.
    fn go(&mut self, place: Place, requests: &mut Vec<Request>) {
        (self.entries, self.notice) = (None, None);
        requests.push(Request::Browse { server: self.server.clone(), place: place.clone() });
        self.place = Some(place);
    }

    /// The card over the middle of the window, while it is open: see `settings::Dialog::view`.
    pub fn view(&self, servers: &[Server]) -> Option<Element<Message>> {
        if !self.open {
            return None;
        }
        let head = Element::row()
            .align(Align::Center)
            .justify(Justify::Between)
            .child(Element::text("Open", TextStyle::title().size(tokens::TEXT_XL)))
            .child(Button::icon_only(icon::X, "Close").subtle().small().on_press(send(Browsing::Close)));
        // One server is named. Of several, one is chosen.
        let from: Element<Message> = match servers {
            [only] => Element::row()
                .align(Align::Center)
                .gap(tokens::SPACE_XS)
                .fg(tokens::FG_MUTED)
                .child(Element::icon(icon::CLOUD, 16.0))
                .child(Element::text(host(&only.address), TextStyle::control()).shrink()),
            several => {
                let mut choice = Segmented::new("Server");
                for server in several {
                    let chosen = server.address == self.server;
                    choice =
                        choice.segment(host(&server.address), chosen, send(Browsing::Server(server.address.clone())));
                }
                choice.into()
            }
        };
        let device = Button::new("This device").icon(icon::FOLDER_OPEN).on_press(send(Browsing::Device));
        let sources = Element::row()
            .align(Align::Center)
            .justify(Justify::Between)
            .gap(tokens::SPACE_MD)
            .child(from)
            .child(device);
        let line = || Element::row().width(1.0).bg(tokens::LINE).decorative();
        let body = Element::row()
            .height(HEIGHT)
            .border(Edges::all(1.0), tokens::LINE)
            .radius(tokens::RADIUS_CONTROL)
            .child(self.places())
            .child(line())
            .child(self.held());
        let content = Element::column()
            .width(WIDTH)
            .gap(tokens::SPACE_MD)
            .padding(Edges { top: tokens::SPACE_LG, ..Edges::all(tokens::SPACE_XL) })
            .child(head)
            .child(sources)
            .child(body);
        let dialog = Popover::new(content)
            .flush()
            .label("Open")
            .max_width(WIDTH)
            .modal()
            .open(true)
            .on_close(send(Browsing::Close));
        Some(Element::row().overlay().popover(dialog))
    }

    /// The places of the server down the card's side: the person's spaces, then what reaches them from
    /// other people's, and what they had open lately.
    fn places(&self) -> Element<Message> {
        let spaces = self.spaces.as_deref().unwrap_or_default().iter().map(|space| {
            let mark = if space.kind == SpaceKind::Personal { icon::FOLDER } else { icon::USERS };
            (mark, space.name.as_str(), within(space, ""))
        });
        let others = [(icon::LINK, "Shared with me", Place::Shared), (icon::HISTORY, "Recent", Place::Recent)];
        let rows = spaces.chain(others).map(|(mark, name, place)| {
            // A folder of a space is in that space.
            let here = match (&self.place, &place) {
                (Some(Place::Folder { space: at, .. }), Place::Folder { space, .. }) => at == space,
                (at, place) => at.as_ref() == Some(place),
            };
            option(mark, name, here).selected(here).on_press(send(Browsing::Go(place)))
        });
        Element::column()
            .width(PLACES)
            .padding(Edges::all(tokens::SPACE_XS))
            .role(Role::ListBox)
            .label("Places")
            .arrows(Dir::Column)
            .one_stop()
            .scroll()
            .children(rows)
    }

    /// What the place that is looked at holds, under the way that leads to it.
    fn held(&self) -> Element<Message> {
        let pane = Element::column()
            .grow(1.0)
            .padding(Edges::all(tokens::SPACE_XS))
            .gap(tokens::SPACE_XS)
            .children(self.way());
        let said = |sentence: &str, role: Role, ink: Ink| {
            // Set in as far as the rows it stands for.
            let sentence = Element::paragraph(sentence, quiet()).fg(ink).role(role).label(sentence);
            Element::column().padding(Edges::all(tokens::SPACE_SM)).child(sentence)
        };
        let entries = match (&self.notice, &self.entries) {
            (Some(why), _) => return pane.child(said(why, Role::Alert, tokens::DANGER)),
            (None, None) => {
                let loading = format!("Loading documents from {}.", host(&self.server));
                return pane.child(said(&loading, Role::Status, tokens::FG_MUTED));
            }
            (None, Some(entries)) if entries.is_empty() => {
                let none = match self.place {
                    Some(Place::Shared) => "No documents have been shared with you yet.",
                    Some(Place::Recent) => "You have not opened any documents from this server yet.",
                    _ => "This folder is empty.",
                };
                return pane.child(said(none, Role::Status, tokens::FG_MUTED));
            }
            (None, Some(entries)) => entries,
        };
        let today = self.now.map(|now| day(now, self.zone));
        // The first of them takes the keyboard as they come, so that a folder gone into with Enter is
        // walked on with the arrow keys.
        let rows = entries.iter().enumerate().map(|(at, entry)| {
            let row = self.entry(entry, today);
            if at == 0 { row.takes_focus() } else { row }
        });
        let list = Element::column()
            .grow(1.0)
            .role(Role::ListBox)
            .label("Documents")
            .arrows(Dir::Column)
            .one_stop()
            .scroll()
            .children(rows);
        pane.child(list)
    }

    /// The way to the folder that is looked at, from its space down. Each step but the last leads back. At
    /// its end a document is started in the folder, where the person may write.
    fn way(&self) -> Option<Element<Message>> {
        let Some(Place::Folder { space, path }) = &self.place else { return None };
        let name = self
            .spaces
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|known| known.id == *space)
            .map(|space| space.name.as_str());
        let folders: Vec<&str> = path.split('/').filter(|folder| !folder.is_empty()).collect();
        let mut way = Element::row().align(Align::Center).gap(tokens::SPACE_2XS).role(Role::Group).label("Folder");
        let steps = [name.unwrap_or("Space")].into_iter().chain(folders.iter().copied()).enumerate();
        for (at, step) in steps {
            if at > 0 {
                way = way.child(Element::icon(icon::CHEVRON_RIGHT, 14.0).fg(tokens::FG_MUTED).decorative());
            }
            if at == folders.len() {
                // Set in as far as a step that is pressed, and as the rows under it.
                let here = Element::text(step, quiet().weight(600)).shrink();
                way = way.child(Element::row().padding(Edges::xy(tokens::SPACE_SM, 0.0)).shrink().child(here));
            } else {
                let back = Place::Folder { space: space.clone(), path: folders[..at].join("/") };
                way = way.child(Button::new(step).subtle().small().on_press(send(Browsing::Go(back))));
            }
        }
        if self.written().is_some() {
            let here = folders.last().copied().or(name).unwrap_or_default();
            let new = Button::new("New document").icon(icon::PLUS).small().on_press(send(Browsing::New));
            way = way.child(Element::row().grow(1.0)).child(Element::from(new).description(format!("In {here}")));
        }
        Some(way.height(tokens::CONTROL_HEIGHT_SM))
    }

    /// One folder or document of the place. A folder is gone into, a document is opened, and a file that
    /// could not be read as a document is only listed.
    fn entry(&self, entry: &Entry, today: Option<i64>) -> Element<Message> {
        let folder = entry.kind == EntryKind::Folder;
        let does = match (&entry.space, &entry.id) {
            (Some(space), _) if folder => {
                Some(Browsing::Go(Place::Folder { space: space.clone(), path: entry.path.clone() }))
            }
            (_, Some(id)) if !folder => Some(Browsing::Open(id.clone())),
            _ => None,
        };
        let noted = notes(&entry.tags);
        let when = (!folder).then(|| day_called(day(entry.modified, self.zone), today));
        // A screen reader is told the row whole, in the order it is read across.
        let kind = if folder { "Folder" } else { "Document" };
        let about: Vec<&str> = [Some(kind), noted.as_deref(), when.as_deref()].into_iter().flatten().collect();
        let mark = if folder { icon::FOLDER } else { icon::FILE_TEXT };
        let key = format!("{}/{}/{}", entry.space.as_deref().unwrap_or_default(), entry.path, entry.name);
        let row = option(mark, &entry.name, false)
            .key(key.as_str())
            .description(about.join(", "))
            .children(noted.as_deref().map(|noted| Element::text(noted, quiet()).fg(tokens::FG_MUTED).shrink()))
            .child(Element::row().grow(1.0))
            .children(when.as_deref().map(|when| Element::text(when, quiet()).fg(tokens::FG_MUTED)));
        match does {
            Some(does) => row.on_press(send(does)),
            None => row.disabled(true),
        }
    }
}

/// A row of either list: an icon and a name, lit under the pointer and ringed while it has the keyboard.
fn option(mark: Icon, name: &str, chosen: bool) -> Element<Message> {
    let weight = if chosen { 600 } else { 500 };
    Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_SM)
        .height(ROW)
        .padding(Edges::xy(tokens::SPACE_SM, 0.0))
        .radius(tokens::RADIUS_CONTROL)
        .bg(if chosen { tokens::NEUTRAL_SOFT } else { fade(tokens::NEUTRAL_SOFT, 0.0) })
        .hover(Look::bg(tokens::NEUTRAL_SOFT))
        .focused(Look::bg(tokens::NEUTRAL_SOFT))
        // Inside the row, where the list that scrolls does not cut it off.
        .ring(Ring::Outline(-3.0))
        .role(Role::ListBoxOption)
        .label(name)
        .focusable()
        .child(Element::icon(mark, 16.0).fg(tokens::FG_MUTED))
        .child(Element::text(name, TextStyle::control().weight(weight)).shrink())
}

/// What a file table marks an entry with, in a few words: who writes in it now, and what is wrong with it.
fn notes(tags: &[Tag]) -> Option<String> {
    let said: Vec<String> = tags
        .iter()
        .filter_map(|tag| match tag {
            Tag::Editing { people } if !people.is_empty() => Some(format!("{} editing", people.join(", "))),
            Tag::ConflictCopy { .. } => Some("Conflict copy".to_owned()),
            Tag::Damaged => Some("Could not be read".to_owned()),
            _ => None,
        })
        .collect();
    (!said.is_empty()).then(|| said.join(", "))
}

/// A folder of a space, by its path there.
fn within(space: &Space, path: &str) -> Place {
    Place::Folder { space: space.id.clone(), path: path.to_owned() }
}

fn quiet() -> TextStyle {
    TextStyle::control().size(tokens::TEXT_SM)
}

fn send(message: Browsing) -> Message {
    Message::Browse(message)
}
