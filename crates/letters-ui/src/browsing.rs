//! Opening a document a server keeps: one card lists the places of a server the person is signed in to and
//! what each holds, and the document that is pressed is asked of the shell. So is what else the card offers
//! to have done there: a document started or imported, conflict copies merged, and what was thrown away put
//! back.
//!
//! The window only asks: the shell talks to the server, and answers with what a place holds or with why it
//! could not be read.

use accesskit::Role;
use letters_api::{Entry, EntryKind, Role as Grade, Space, SpaceKind, Tag, Trashed};

use crate::color::fade;
use crate::element::Element;
use crate::icon::{self, Icon};
use crate::layout::MARGIN;
use crate::opening::{Request, day, day_called, seconds};
use crate::servers::{Server, host};
use crate::style::{Align, Dir, Edges, Justify, Look, Ring};
use crate::text::TextStyle;
use crate::threads::{FACE, face, lettered};
use crate::tokens::{self, Color as Ink};
use crate::widgets::{Button, Popover, Segmented, Side};
use crate::window::Message;

/// How wide the card is where the window has the room.
const WIDTH: f32 = 880.0;
/// How wide the places are, in a tight card too, and how tall they and what they hold stand, whatever is
/// in them.
const PLACES: f32 = 200.0;
const NARROW: f32 = 120.0;
const HEIGHT: f32 = 340.0;
const ROW: f32 = 36.0;
/// How many of a document's writers a row shows the faces of, and how wide its columns after the name are:
/// those faces, the day it was made and the day it was last written.
const FACES: usize = 3;
const WRITERS: f32 = FACES as f32 * (FACE + tokens::SPACE_2XS);
const DAY: f32 = 88.0;
/// How wide the end of a row of the trash is, where what was thrown away is put back.
const BACK: f32 = 104.0;

/// How much a card has room for, by how wide it is.
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Room {
    /// Names, and what is done with them by buttons that show their icons alone.
    Tight,
    /// And the day a thing was last written or thrown away, with the buttons of a row named.
    Plain,
    /// And who wrote a document and when it was made, with the buttons over the list named.
    Roomy,
}

impl Room {
    fn of(width: f32) -> Self {
        match width {
            width if width >= 760.0 => Self::Roomy,
            width if width >= 540.0 => Self::Plain,
            _ => Self::Tight,
        }
    }
}

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
    /// List what the place holds by this, or the other way round if it is listed so already.
    Sort(Order),
    /// Merge its conflict copies into the document with this id.
    Merge(String),
    /// Make a document in the folder that is looked at out of a file of this device.
    Import,
    /// Look at what was thrown away on the server.
    Trash,
    /// Put back what `file` names in the trash of this space.
    PutBack {
        space: String,
        file: String,
    },
}

/// What the documents of a place are listed by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// From A to Z.
    Name,
    /// The last written first.
    Modified,
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
    /// The trash is looked at in place of `place`, and what is in it once the server has said.
    binned: bool,
    thrown: Option<Vec<Trashed>>,
    /// Why the last place could not be read.
    notice: Option<String>,
    /// What the person chose to list a place by, and whether that is turned round.
    chosen: Option<(Order, bool)>,
    /// What the place holds has just come, and its first row takes the keyboard.
    fresh: bool,
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
            if self.place.is_none() && !self.binned {
                let own = listing.spaces.iter().find(|space| space.kind == SpaceKind::Personal);
                let first = own.or(listing.spaces.first()).map(|space| within(space, ""));
                self.go(first.unwrap_or(Place::Shared), requests);
            }
            self.spaces = Some(listing.spaces);
        } else if !self.binned && self.place.as_ref() == Some(&listing.place) {
            (self.entries, self.notice, self.fresh) = (Some(listing.entries), None, true);
        }
    }

    /// Takes in what is in a server's trash. An answer that comes when that trash is not looked at is late,
    /// and is let go.
    pub fn trashed(&mut self, server: &str, thrown: Vec<Trashed>) {
        if self.open && self.binned && server == self.server {
            (self.thrown, self.notice) = (Some(thrown), None);
        }
    }

    /// Takes in why the place that was asked for could not be read. One that comes when nothing is asked
    /// is about a place that was left.
    pub fn refused(&mut self, why: &str) {
        let asked = if self.binned { self.thrown.is_none() } else { self.entries.is_none() };
        if self.open && asked {
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
            Browsing::New | Browsing::Import => {
                if let Some((space, path)) = self.written() {
                    let (server, space, path) = (self.server.clone(), space.to_owned(), path.to_owned());
                    requests.push(match message {
                        Browsing::Import => Request::ImportAt { server, space, path },
                        _ => Request::NewAt { server, space, path },
                    });
                    self.close();
                }
            }
            Browsing::Sort(by) => {
                let turned = self.order().is_some_and(|(at, turned)| at == by && !turned);
                // The keyboard stays on the heading, to turn the order round with.
                (self.chosen, self.fresh) = (Some((by, turned)), false);
            }
            Browsing::Trash => {
                (self.binned, self.thrown, self.notice) = (true, None, None);
                requests.push(Request::BrowseTrash { server: self.server.clone() });
            }
            Browsing::PutBack { space, file } => {
                requests.push(Request::PutBack { server: self.server.clone(), space, file });
                // The answer is the trash without it, so it is waited for as the trash was.
                self.thrown = None;
            }
            Browsing::Merge(id) => {
                if let Some(place) = self.place.clone() {
                    requests.push(Request::MergeConflictsOf { server: self.server.clone(), id, place });
                    // The answer is what the place holds without the copies, so it is waited for as the
                    // place was.
                    self.entries = None;
                }
            }
        }
    }

    /// What the place is listed by, and whether that is turned round: by name, as a server lists it, until
    /// the person chooses. What was open lately stays in the order it was opened in.
    fn order(&self) -> Option<(Order, bool)> {
        match self.place {
            Some(Place::Recent) | None => None,
            Some(_) => Some(self.chosen.unwrap_or((Order::Name, false))),
        }
    }

    /// What the place holds in the order it is listed in.
    fn sorted<'a>(&self, entries: &'a [Entry]) -> Vec<&'a Entry> {
        let mut rows: Vec<&Entry> = entries.iter().collect();
        let Some((by, turned)) = self.order() else { return rows };
        let folder = |entry: &Entry| entry.kind == EntryKind::Folder;
        rows.sort_by(|a, b| {
            let names = a.name.to_lowercase().cmp(&b.name.to_lowercase());
            let within = match by {
                Order::Name => names,
                Order::Modified if !folder(a) => b.modified.cmp(&a.modified),
                // Folders keep to their names: when one was last written says little of what is in it.
                Order::Modified => return folder(b).cmp(&folder(a)).then(names),
            };
            folder(b).cmp(&folder(a)).then(if turned { within.reverse() } else { within })
        });
        rows
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
        (self.spaces, self.place, self.entries, self.notice, self.binned) = (None, None, None, None, false);
        requests.push(Request::Browse { server: server.clone(), place: Place::Spaces });
        self.server = server;
    }

    /// Turns to a place of the server, and asks what it holds.
    fn go(&mut self, place: Place, requests: &mut Vec<Request>) {
        (self.entries, self.notice, self.binned) = (None, None, false);
        requests.push(Request::Browse { server: self.server.clone(), place: place.clone() });
        self.place = Some(place);
    }

    /// The card over the middle of a window this wide, while it is open: see `settings::Dialog::view`.
    pub fn view(&self, servers: &[Server], window: f32) -> Option<Element<Message>> {
        if !self.open {
            return None;
        }
        let width = WIDTH.min(window - 2.0 * MARGIN);
        let room = Room::of(width);
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
            .child(self.places(room))
            .child(line())
            .child(self.held(room));
        let content = Element::column()
            .width(width)
            .gap(tokens::SPACE_MD)
            .padding(Edges { top: tokens::SPACE_LG, ..Edges::all(tokens::SPACE_XL) })
            .child(head)
            .child(sources)
            .child(body);
        let dialog = Popover::new(content)
            .flush()
            .label("Open")
            .max_width(width)
            .modal()
            .open(true)
            .on_close(send(Browsing::Close));
        Some(Element::row().overlay().popover(dialog))
    }

    /// The places of the server down the card's side: the person's spaces, then what reaches them from
    /// other people's, what they had open lately, and what was thrown away.
    fn places(&self, room: Room) -> Element<Message> {
        let spaces = self.spaces.as_deref().unwrap_or_default().iter().map(|space| {
            let mark = if space.kind == SpaceKind::Personal { icon::FOLDER } else { icon::USERS };
            (mark, space.name.as_str(), within(space, ""))
        });
        let others = [(icon::LINK, "Shared with me", Place::Shared), (icon::HISTORY, "Recent", Place::Recent)];
        let rows = spaces.chain(others).map(|(mark, name, place)| {
            // A folder of a space is in that space.
            let here = match (&self.place, &place) {
                _ if self.binned => false,
                (Some(Place::Folder { space: at, .. }), Place::Folder { space, .. }) => at == space,
                (at, place) => at.as_ref() == Some(place),
            };
            option(mark, name, here).selected(here).on_press(send(Browsing::Go(place)))
        });
        let trash = option(icon::TRASH_2, "Trash", self.binned).selected(self.binned).on_press(send(Browsing::Trash));
        Element::column()
            .width(if room == Room::Tight { NARROW } else { PLACES })
            .padding(Edges::all(tokens::SPACE_XS))
            .role(Role::ListBox)
            .label("Places")
            .arrows(Dir::Column)
            .one_stop()
            .scroll()
            .children(rows)
            .child(trash)
    }

    /// What the place that is looked at holds, under the way that leads to it.
    fn held(&self, room: Room) -> Element<Message> {
        // What is too long for a row is cut short there: the pane is as wide as the card leaves it.
        let pane = Element::column()
            .grow(1.0)
            .shrink()
            .padding(Edges::all(tokens::SPACE_XS))
            .gap(tokens::SPACE_XS)
            .children(self.way(room));
        let said = |sentence: &str, role: Role, ink: Ink| {
            // Set in as far as the rows it stands for.
            let sentence = Element::paragraph(sentence, quiet()).fg(ink).role(role).label(sentence);
            Element::column().padding(Edges::all(tokens::SPACE_SM)).child(sentence)
        };
        let quietly = |sentence: &str| said(sentence, Role::Status, tokens::FG_MUTED);
        let loading = format!("Loading documents from {}.", host(&self.server));
        if let Some(why) = &self.notice {
            return pane.child(said(why, Role::Alert, tokens::DANGER));
        }
        let today = self.now.map(|now| day(now, self.zone));
        if self.binned {
            return match self.thrown.as_deref() {
                None => pane.child(quietly(&loading)),
                Some([]) => pane.child(quietly("The trash is empty.")),
                Some(thrown) => pane.child(gone_heads(room)).child(self.gone(thrown, today, room)),
            };
        }
        let entries = match self.entries.as_deref() {
            None => return pane.child(quietly(&loading)),
            Some([]) => {
                let none = match self.place {
                    Some(Place::Shared) => "No documents have been shared with you yet.",
                    Some(Place::Recent) => "You have not opened any documents from this server yet.",
                    _ => "This folder is empty.",
                };
                return pane.child(quietly(none));
            }
            Some(entries) => entries,
        };
        // The first of them takes the keyboard as they come, so that a folder gone into with Enter is
        // walked on with the arrow keys.
        let rows = self.sorted(entries).into_iter().enumerate().map(|(at, entry)| {
            let row = self.entry(entry, today, room);
            if at == 0 && self.fresh { row.takes_focus() } else { row }
        });
        let list = Element::column()
            .grow(1.0)
            .shrink()
            .role(Role::ListBox)
            .label("Documents")
            .arrows(Dir::Column)
            .one_stop()
            .scroll()
            .children(rows);
        pane.child(self.heads(room)).child(list)
    }

    /// What stands in each column of the list, over it. The two a place is listed by are pressed for it.
    fn heads(&self, room: Room) -> Element<Message> {
        let plain = |name: &str, width: f32| {
            Element::row().width(width).child(Element::text(name, small()).fg(tokens::FG_MUTED))
        };
        let more = [plain("Contributors", WRITERS), plain("Created", DAY)];
        let day = Element::row().width(DAY).child(self.head("Modified", Order::Modified));
        Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .padding(Edges::xy(tokens::SPACE_SM, 0.0))
            .child(Element::row().grow(1.0).child(self.head("Name", Order::Name)))
            .children(more.into_iter().filter(|_| room == Room::Roomy))
            .children((room > Room::Tight).then_some(day))
    }

    /// The heading of a column the place is listed by, with which way it runs while it is.
    fn head(&self, name: &str, by: Order) -> Element<Message> {
        let said = Element::text(name, small());
        let Some((at, turned)) = self.order() else { return said.fg(tokens::FG_MUTED) };
        let head = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_2XS)
            .radius(tokens::RADIUS_CONTROL)
            .fg(tokens::FG_MUTED)
            .hover(Look::NONE.with_fg(tokens::FG))
            .ring(Ring::Outline(2.0))
            .role(Role::Button)
            .label(name)
            .focusable()
            .on_press(send(Browsing::Sort(by)))
            .child(said);
        if at != by {
            return head;
        }
        // Names run up from A and days run back from today, until the order is turned round.
        let (mark, sorted) = match (by, turned) {
            (Order::Name, false) => (icon::CHEVRON_UP, "Sorted from A to Z"),
            (Order::Name, true) => (icon::CHEVRON_DOWN, "Sorted from Z to A"),
            (Order::Modified, false) => (icon::CHEVRON_DOWN, "Sorted newest first"),
            (Order::Modified, true) => (icon::CHEVRON_UP, "Sorted oldest first"),
        };
        head.description(sorted).child(Element::icon(mark, 12.0))
    }

    /// What was thrown away on the server, the last first: each with where it was and the day it went. Whoever
    /// edits in its space puts it back from its row.
    fn gone(&self, thrown: &[Trashed], today: Option<i64>, room: Room) -> Element<Message> {
        let rows = thrown.iter().map(|gone| {
            let folder = gone.kind == EntryKind::Folder;
            // Where it was: its space, and its folders there.
            let space = self.spaces.as_deref().unwrap_or_default().iter().find(|space| space.id == gone.space);
            let folders = gone.folder.split('/').filter(|folder| !folder.is_empty());
            let was: Vec<&str> = space.map(|space| space.name.as_str()).into_iter().chain(folders).collect();
            let was = (!was.is_empty()).then(|| was.join(" / "));
            let when = day_called(day(gone.thrown, self.zone), today);
            let kind = if folder { "Folder" } else { "Document" };
            let told = [
                Some(kind.to_owned()),
                was.as_ref().map(|was| format!("Was in {was}")),
                Some(format!("Thrown away {when}")),
            ];
            let about: Vec<String> = told.into_iter().flatten().collect();
            let back = (gone.role >= Grade::Editor).then(|| {
                let asked = Browsing::PutBack { space: gone.space.clone(), file: gone.file.clone() };
                act("Put back", icon::UNDO_2, asked, gone.name.as_str(), room == Room::Tight)
            });
            let muted = |said: String| Element::text(said, quiet()).fg(tokens::FG_MUTED);
            // A tight card has room for the name alone, and no column to keep the button under.
            let (was, when, end) = match room {
                Room::Tight => (None, None, Element::row()),
                _ => (was, Some(Element::row().width(DAY).child(muted(when))), Element::row().width(BACK)),
            };
            option(if folder { icon::FOLDER } else { icon::FILE_TEXT }, &gone.name, false)
                .key(format!("{}/{}", gone.space, gone.file).as_str())
                .description(about.join(", "))
                .child(Element::row().width(0.0).grow(1.0).clip().children(was.map(|was| muted(was).shrink())))
                .children(when)
                .child(end.justify(Justify::End).children(back))
        });
        Element::column()
            .grow(1.0)
            .shrink()
            .role(Role::ListBox)
            .label("Trash")
            .arrows(Dir::Column)
            .one_stop()
            .scroll()
            .children(rows)
    }

    /// The way to the folder that is looked at, from its space down. Each step but the last leads back. At
    /// its end a document is started in the folder or made there out of a file, where the person may write.
    /// A tight card has room for the folder's own name alone, with one step that leads up from it.
    fn way(&self, room: Room) -> Option<Element<Message>> {
        let Some(Place::Folder { space, path }) = self.place.as_ref().filter(|_| !self.binned) else { return None };
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
        let tight = room == Room::Tight;
        for (at, step) in steps.skip(if tight { folders.len().saturating_sub(1) } else { 0 }) {
            if at > 0 && !tight {
                way = way.child(Element::icon(icon::CHEVRON_RIGHT, 14.0).fg(tokens::FG_MUTED).decorative());
            }
            if at == folders.len() {
                // Set in as far as a step that is pressed, and as the rows under it.
                let here = Element::text(step, quiet().weight(600)).shrink();
                way = way.child(Element::row().padding(Edges::xy(tokens::SPACE_SM, 0.0)).shrink().child(here));
            } else {
                let back = Place::Folder { space: space.clone(), path: folders[..at].join("/") };
                let up = match tight {
                    true => Element::from(Button::icon_only(icon::ARROW_LEFT, step).subtle().small())
                        .tooltip(step, Side::Top),
                    // A step gives up the end of its name before the way outgrows the card.
                    false => Button::new(step).subtle().small().shrink().into(),
                };
                way = way.child(up.on_press(send(Browsing::Go(back))));
            }
        }
        if self.written().is_some() {
            let here = folders.last().copied().or(name).unwrap_or_default();
            // Named, they would leave the steps too little of a card that is not roomy.
            let brief = room < Room::Roomy;
            let made = Element::row()
                .gap(tokens::SPACE_XS)
                .child(act("Import", icon::FILE_UP, Browsing::Import, format!("Into {here}"), brief))
                .child(act("New document", icon::PLUS, Browsing::New, format!("In {here}"), brief));
            way = way.child(Element::row().grow(1.0)).child(made);
        }
        Some(way.shrink().height(tokens::CONTROL_HEIGHT_SM))
    }

    /// One folder or document of the place. A folder is gone into, a document is opened, and a file that
    /// could not be read as a document is only listed. So is a conflict copy, which goes by the id of the
    /// document it is a copy of: whoever edits there merges it into that document from its row.
    fn entry(&self, entry: &Entry, today: Option<i64>, room: Room) -> Element<Message> {
        let folder = entry.kind == EntryKind::Folder;
        let copy = entry.tags.iter().find_map(|tag| match tag {
            Tag::ConflictCopy { of } => Some(of.as_str()),
            _ => None,
        });
        let merge = copy.zip(entry.id.as_ref()).filter(|_| entry.role >= Grade::Editor).map(|(of, id)| {
            let original = of.rsplit_once('/').map_or(of, |(_, name)| name);
            let original = original.strip_suffix(".let").unwrap_or(original);
            let (does, into) = (Browsing::Merge(id.clone()), format!("Into {original}"));
            act("Merge conflicts", icon::GIT_MERGE, does, into, room == Room::Tight)
        });
        let merged = merge.is_some();
        let does = match (&entry.space, &entry.id) {
            _ if copy.is_some() => None,
            (Some(space), _) if folder => {
                Some(Browsing::Go(Place::Folder { space: space.clone(), path: entry.path.clone() }))
            }
            (_, Some(id)) if !folder => Some(Browsing::Open(id.clone())),
            _ => None,
        };
        // The button says what the note would.
        let noted = notes(&entry.tags, !merged);
        let called = |seconds: u64| day_called(day(seconds, self.zone), today);
        let made = entry.created.as_deref().and_then(seconds).map(called);
        let when = (!folder).then(|| called(entry.modified));
        // A screen reader is told the row whole, in the order it is read across.
        let kind = if folder { "Folder" } else { "Document" };
        let by = (!entry.contributors.is_empty()).then(|| format!("By {}", entry.contributors.join(", ")));
        let told = [
            Some(kind.to_owned()),
            noted.clone(),
            by,
            made.as_ref().map(|made| format!("Created {made}")),
            when.as_ref().map(|when| format!("Modified {when}")),
        ];
        let about: Vec<String> = told.into_iter().flatten().collect();
        let mark = if folder { icon::FOLDER } else { icon::FILE_TEXT };
        let key = format!("{}/{}/{}", entry.space.as_deref().unwrap_or_default(), entry.path, entry.name);
        // The note has the room the name and the day leave, and is cut short in it: the name is what a
        // document is found by.
        let noted = noted.as_deref().map(|noted| Element::text(noted, quiet()).fg(tokens::FG_MUTED).shrink());
        let on = |said: Option<String>| {
            Element::row().width(DAY).children(said.map(|said| Element::text(said, quiet()).fg(tokens::FG_MUTED)))
        };
        let row = option(mark, &entry.name, false)
            .key(key.as_str())
            .description(about.join(", "))
            .children(merge)
            .child(Element::row().width(0.0).grow(1.0).clip().children(noted))
            .children([writers(&entry.contributors), on(made)].into_iter().filter(|_| room == Room::Roomy))
            .children((room > Room::Tight).then(|| on(when)));
        match does {
            Some(does) => row.on_press(send(does)),
            // The row of a copy is there for its button.
            None if merged => row,
            None => row.disabled(true),
        }
    }
}

/// What stands in each column of the trash, over it.
fn gone_heads(room: Room) -> Element<Message> {
    let plain = |name: &str| Element::text(name, small()).fg(tokens::FG_MUTED);
    let more = [Element::row().width(DAY).child(plain("Thrown away")), Element::row().width(BACK)];
    Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_SM)
        .padding(Edges::xy(tokens::SPACE_SM, 0.0))
        .child(Element::row().grow(1.0).child(plain("Name")))
        .children(more.into_iter().filter(|_| room > Room::Tight))
}

/// A small button of the card, with what it is done to said after its name. A `brief` one shows its icon
/// alone, and is named under the pointer.
fn act(name: &str, mark: Icon, does: Browsing, to: impl Into<String>, brief: bool) -> Element<Message> {
    let button = match brief {
        true => Element::from(Button::icon_only(mark, name).small()).tooltip(name, Side::Top),
        false => Button::new(name).icon(mark).small().into(),
    };
    button.on_press(send(does)).description(to)
}

/// A row of any list: an icon and a name, lit under the pointer and ringed while it has the keyboard.
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

/// What a file table marks an entry with, in a few words: who writes in it now, who else reaches it, and
/// what is wrong with it. `copies` is whether a conflict copy is said to be one.
fn notes(tags: &[Tag], copies: bool) -> Option<String> {
    let counted = |count: u32, one: &str, many: &str| match count {
        0 => None,
        1 => Some(format!("1 {one}")),
        _ => Some(format!("{count} {many}")),
    };
    let said: Vec<String> = tags
        .iter()
        .filter_map(|tag| match tag {
            Tag::Editing { people } if !people.is_empty() => Some(format!("{} editing", people.join(", "))),
            Tag::ConflictCopy { .. } if copies => Some("Conflict copy".to_owned()),
            Tag::Damaged => Some("Could not be read".to_owned()),
            Tag::ImportNotes { count } => counted(*count, "import note", "import notes"),
            Tag::Shared { with } => counted(*with, "other", "others").map(|others| format!("Shared with {others}")),
            Tag::Links { count } => counted(*count, "link", "links"),
            _ => None,
        })
        .collect();
    (!said.is_empty()).then(|| said.join(", "))
}

/// Who wrote a document: the faces of the first of them, each named under the pointer, and how many more
/// there are.
fn writers(names: &[String]) -> Element<Message> {
    // The count takes the last face's place.
    let shown = if names.len() > FACES { FACES - 1 } else { names.len() };
    let faces = names[..shown].iter().map(|name| face(name, false).tooltip(name.as_str(), Side::Top));
    let rest = names[shown..].join(", ");
    let more = (names.len() > shown).then(|| lettered(&format!("+{}", names.len() - shown)).tooltip(rest, Side::Top));
    Element::row().width(WRITERS).gap(tokens::SPACE_2XS).children(faces).children(more)
}

/// A folder of a space, by its path there.
fn within(space: &Space, path: &str) -> Place {
    Place::Folder { space: space.id.clone(), path: path.to_owned() }
}

fn quiet() -> TextStyle {
    TextStyle::control().size(tokens::TEXT_SM)
}

fn small() -> TextStyle {
    TextStyle::control().size(tokens::TEXT_XS)
}

fn send(message: Browsing) -> Message {
    Message::Browse(message)
}
