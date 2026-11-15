//! A device's side of a session: a state machine the shell feeds and drains.
//!
//! The shell owns the socket, the timer and the document. It tells the client what happened, each time
//! with the time in milliseconds, and then drains [`Client::poll`] for what to do. In any order:
//!
//! - the socket opened, a frame arrived, or the socket closed: [`Client::opened`], [`Client::received`],
//!   [`Client::closed`];
//! - the document changed here, by the person or by a merge: [`Client::changed`];
//! - the style was set here, or the caret moved: [`Client::restyled`], [`Client::selected`];
//! - the time [`Client::wake_at`] named has come: [`Client::tick`].
//!
//! The client reads the document and never changes it: what arrives is handed out as [`Event::Merge`] and
//! [`Event::Notes`] for the shell to merge the way it merges a file. Nothing is queued for the server
//! while the socket is down. The document itself is the queue: on the next handshake the client sends
//! whatever the server lacks, so edits written offline, and edits a lost socket took with it, arrive
//! then. For work that must outlive the program, the shell keeps its own journal and lets go of it on
//! [`Event::Held`].

use std::collections::{BTreeMap, VecDeque};
use std::mem;

use letters_model::{Extent, Extra, Model, Note, NoteKind, Notes, Revision, Selection, Style};

use crate::access::{Refusal, Role};
use crate::presence::{Passage, Peer};
use crate::wire::{ClientMessage, Hello, MAX_FRAME, PROTOCOL, ServerMessage, SessionId, Welcome};
use crate::{PING_AFTER, SILENT_AFTER};

/// How long the client waits before it asks for a socket again, the first time. Each loss doubles it.
const RETRY_FIRST: u64 = 1_000;
const RETRY_LONGEST: u64 = 30_000;
/// A caret that keeps moving is told this often at most, in milliseconds.
const SELECTION_EVERY: u64 = 50;

/// The document as it is open on this device, lent to the client for the length of a call.
///
/// The model must be the one whose history the file's was attached to: see
/// [`Model::attach`](letters_model::Model::attach). A model that shows an older revision is fine. Its
/// replica id must be this run's own: two devices that write under one id cannot be merged, and neither
/// can a copy that was put back to an older state and writes on under the id it had.
#[derive(Clone, Copy, Debug)]
pub struct Local<'a> {
    /// `None` on a device that holds no copy yet. The first [`Event::Merge`] then carries the whole
    /// history, for [`Model::load`](letters_model::Model::load).
    pub model: Option<&'a Model>,
    pub notes: &'a Notes,
}

/// What the client asks of the shell, in the order it must be done.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Open the socket to the document, and answer with [`Client::opened`] or [`Client::closed`].
    Open,
    /// Send this frame as one binary message.
    Send(Vec<u8>),
    /// Close the socket, and tell the client nothing more of it: what [`Client::closed`] says is about
    /// the socket that is open or asked for now, and the client may ask for the next one at once.
    Close,
    /// Edits made elsewhere. Merge them into the document as they are, with
    /// [`Model::merge`](letters_model::Model::merge), and then call [`Client::changed`]. Where they met
    /// what this device wrote while it was away, the server has settled the two and sends that too.
    Merge(Vec<u8>),
    /// Notes that were added or changed elsewhere. Join them into the notes with
    /// [`Notes::merge`](letters_model::Notes::merge), move them to where their text is now with
    /// [`Notes::follow`](letters_model::Notes::follow), and then call [`Client::changed`].
    Notes(Notes),
    /// The server keeps on its disk everything this device held at the call that returned this number.
    /// The shell's own journal may let go of it.
    Held(u64),
    /// The server did not take work this device sent, and why. The work stays here, and
    /// [`Client::progress`] says [`Progress::Edited`] until a later session takes it.
    Refused(Refusal),
    /// [`Client::link`] changed.
    Link,
    /// [`Client::progress`] changed.
    Progress,
    /// [`Client::peers`] changed, and so the carets to draw.
    Peers,
}

/// How the device stands to the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// There is no socket. The client asks for one again at [`Client::wake_at`], unless it was stopped.
    Offline,
    /// A socket was asked for, or is open and the server has not answered the hello yet.
    Connecting,
    /// In session, as this role. A window lets the person do what the role may.
    Live(Role),
    /// The server ended the session for a reason that trying again does not mend. The client stays so
    /// until [`Client::start`] is called again.
    Refused(Refusal),
}

/// Where this device's own work is, for the window to say in a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// The server keeps everything written here, as far as this run of the program knows.
    Saved,
    /// Work is on its way to the server, or the handshake that tells what it lacks is.
    Saving,
    /// Work is held here that cannot be sent now: there is no socket, or the role does not allow it.
    Edited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Socket {
    None,
    /// Asked for at this time.
    Asked(u64),
    /// Open, with the hello sent.
    Greeting,
    Live(Role),
}

/// A device's side of the sync of one document.
pub struct Client {
    document: String,
    created: String,
    /// Whether the shell wants to be online: started, and not stopped since.
    wanted: bool,
    socket: Socket,
    link: Link,
    progress: Progress,
    events: VecDeque<Event>,

    retry_at: Option<u64>,
    /// Sockets lost in a row, which the wait before the next grows with.
    failures: u32,
    heard: u64,
    said: u64,

    session: Option<SessionId>,
    peers: Vec<Peer>,

    /// What the server holds, or was sent on this socket. `None` until a server first says.
    known: Option<Extent>,
    /// What the document held when it was first looked at, which stands in until a server says.
    baseline: Option<Extent>,
    /// The revision the document showed when it was last looked at, to tell cheaply that nothing changed.
    seen: Option<Revision>,
    /// What the document held when its work was last counted in `mark`.
    counted: Option<Extent>,
    /// Whether the document holds work that could not be sent when it was last looked at.
    document_waits: bool,
    /// Each note as the server holds it, as far as this device knows: as it arrived or was sent.
    told: BTreeMap<String, Note>,
    told_extra: Extra,
    /// Whether `told` was filled yet: by a server, or else from the notes as they were first looked at.
    told_any: bool,
    /// The notes that could not be sent when they were last looked at.
    notes_wait: Vec<Note>,
    /// A style set here that the server has not acknowledged, and the number it was sent under on this
    /// socket.
    style: Option<Style>,
    style_sent: Option<u64>,

    /// Counts what this device holds. It rises with every piece of work found or sent.
    mark: u64,
    /// The highest number the server has acknowledged.
    held: u64,
    /// Whether the server refused work on this socket, after which nothing more counts as held.
    refused: bool,
    /// Whether an earlier run left work the server may lack. See [`Client::owes`].
    owed: bool,
    /// Whether the socket was given up once already because the server could not read an edit, with no
    /// edit held since. A second time, the client stays and says so.
    gapped: bool,

    /// The caret as the shell last told it, and as the server was last told.
    selection: Option<Passage>,
    selection_told: Option<Passage>,
    selection_at: u64,
}

impl Client {
    /// A client for the document with this id. An empty id is for a device that holds no copy yet. It
    /// does nothing until [`Client::start`].
    pub fn new(document: impl Into<String>) -> Self {
        Self {
            document: document.into(),
            created: String::new(),
            wanted: false,
            socket: Socket::None,
            link: Link::Offline,
            progress: Progress::Saved,
            events: VecDeque::new(),
            retry_at: None,
            failures: 0,
            heard: 0,
            said: 0,
            session: None,
            peers: Vec::new(),
            known: None,
            baseline: None,
            seen: None,
            counted: None,
            document_waits: false,
            told: BTreeMap::new(),
            told_extra: Extra::new(),
            told_any: false,
            notes_wait: Vec::new(),
            style: None,
            style_sent: None,
            mark: 0,
            held: 0,
            refused: false,
            owed: false,
            gapped: false,
            selection: None,
            selection_told: None,
            selection_at: 0,
        }
    }

    /// Says that the shell's own journal holds work from an earlier run, which the server may lack.
    /// [`Client::progress`] then says so until a handshake has told.
    pub fn owes(&mut self) {
        self.owed = true;
        self.judge();
    }

    /// Goes online: asks for a socket now, and again after each loss, waiting longer each time.
    pub fn start(&mut self, now: u64) {
        self.wanted = true;
        if self.socket == Socket::None {
            self.ask(now);
        }
    }

    /// Goes offline and stays there, as when the window closes or the person signs out.
    pub fn stop(&mut self) {
        self.wanted = false;
        self.retry_at = None;
        if self.socket != Socket::None {
            self.events.push_back(Event::Close);
        }
        self.drop_socket(Link::Offline);
    }

    /// The socket [`Event::Open`] asked for is open. The client says hello.
    pub fn opened(&mut self, local: Local, now: u64) {
        if !matches!(self.socket, Socket::Asked(_)) {
            // A socket nobody asked for: the shell closes it.
            return self.events.push_back(Event::Close);
        }
        self.socket = Socket::Greeting;
        (self.heard, self.said) = (now, now);
        let hello = Hello {
            protocol: PROTOCOL,
            document: self.document.clone(),
            extent: local.model.map(Model::extent).unwrap_or_default(),
            // An older revision on show is not where this copy stands.
            revision: local.model.filter(|model| model.is_latest()).map(Model::revision),
        };
        self.send(&ClientMessage::Hello(hello), now);
    }

    /// A binary message arrived. One that cannot be read ends the socket, and the client tries again.
    pub fn received(&mut self, frame: &[u8], local: Local, now: u64) {
        if !matches!(self.socket, Socket::Greeting | Socket::Live(_)) {
            return;
        }
        self.heard = now;
        match ServerMessage::decode(frame) {
            Ok(Some(message)) => self.take(message, local, now),
            // A kind of message from a later build.
            Ok(None) => {}
            Err(_) => {
                self.events.push_back(Event::Close);
                self.lost(now);
            }
        }
    }

    /// The socket closed or could not be opened. Nothing is lost: the client tries again later.
    pub fn closed(&mut self, now: u64) {
        if self.socket != Socket::None {
            self.lost(now);
        }
    }

    /// The document or its notes may have changed here: by the person, or by a merge of what
    /// [`Client::poll`] handed out. Costs next to nothing when neither did, so call it freely.
    ///
    /// Returns a number for what the device holds now. [`Event::Held`] with that number, or a later
    /// one, says the server keeps all of it.
    pub fn changed(&mut self, local: Local, now: u64) -> u64 {
        self.flush(local, now);
        self.mark
    }

    /// The person set the document's style outside the editor. Returns a number as [`Client::changed`]
    /// does. The server writes the style into the history under no one's name.
    ///
    /// A style the editor set itself is in the history already, under the person's name, and travels
    /// with the edits: there is nothing to call. Either way the style comes to every device in the
    /// history: read [`Model::style`](letters_model::Model::style) when a merge's `Change::style` says
    /// it changed.
    pub fn restyled(&mut self, style: &Style, now: u64) -> u64 {
        self.style = Some(style.clone());
        self.style_sent = None;
        self.mark += 1;
        self.send_style(now);
        self.judge();
        self.mark
    }

    /// The person's caret or selection moved. `None` when the window has none to show.
    pub fn selected(&mut self, model: &Model, selection: Option<&Selection>, now: u64) {
        self.selection = selection.and_then(|selection| Passage::of(model, selection));
        self.tell_selection(now);
    }

    /// The time [`Client::wake_at`] named has come.
    pub fn tick(&mut self, now: u64) {
        match self.socket {
            Socket::None => {
                if self.wanted && self.retry_at.is_some_and(|at| at <= now) {
                    self.ask(now);
                }
            }
            Socket::Asked(at) => {
                if now.saturating_sub(at) >= SILENT_AFTER {
                    self.events.push_back(Event::Close);
                    self.lost(now);
                }
            }
            Socket::Greeting | Socket::Live(_) => {
                if now.saturating_sub(self.heard) >= SILENT_AFTER {
                    self.events.push_back(Event::Close);
                    return self.lost(now);
                }
                self.tell_selection(now);
                if now.saturating_sub(self.said) >= PING_AFTER {
                    self.send(&ClientMessage::Ping, now);
                }
            }
        }
    }

    /// When to call [`Client::tick`] next, whatever else happens before. `None` when nothing is due.
    pub fn wake_at(&self) -> Option<u64> {
        match self.socket {
            Socket::None => self.retry_at.filter(|_| self.wanted),
            Socket::Asked(at) => Some(at + SILENT_AFTER),
            Socket::Greeting => Some(self.heard + SILENT_AFTER),
            Socket::Live(_) => {
                let caret = (self.selection != self.selection_told).then_some(self.selection_at + SELECTION_EVERY);
                let quiet = (self.said + PING_AFTER).min(self.heard + SILENT_AFTER);
                Some(caret.map_or(quiet, |caret| caret.min(quiet)))
            }
        }
    }

    /// The next thing for the shell to do. Drain it after every call.
    pub fn poll(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    pub fn link(&self) -> Link {
        self.link
    }

    pub fn progress(&self) -> Progress {
        self.progress
    }

    /// This session as the others know it, while in session.
    pub fn session(&self) -> Option<SessionId> {
        self.session
    }

    /// The document's id and when it was created, as RFC 3339, once the server has said. A device that
    /// starts its copy from the first [`Event::Merge`] keeps both with it.
    pub fn document(&self) -> Option<(&str, &str)> {
        self.known.is_some().then_some((&self.document, &self.created))
    }

    /// Everyone else who has the document open, in the order they came. Empty while not in session.
    pub fn peers(&self) -> &[Peer] {
        &self.peers
    }
}

impl Client {
    fn ask(&mut self, now: u64) {
        self.retry_at = None;
        self.socket = Socket::Asked(now);
        self.events.push_back(Event::Open);
        self.set_link(Link::Connecting);
        self.judge();
    }

    fn send(&mut self, message: &ClientMessage, now: u64) {
        self.said = now;
        self.events.push_back(Event::Send(message.encode()));
    }

    fn set_link(&mut self, link: Link) {
        if self.link != link {
            self.link = link;
            self.events.push_back(Event::Link);
        }
    }

    /// Forgets the socket and everything that lasted only as long as it did.
    fn drop_socket(&mut self, link: Link) {
        self.socket = Socket::None;
        self.session = None;
        self.style_sent = None;
        self.selection_told = None;
        // What was on its way may not have arrived, so the next handshake has to tell.
        self.seen = None;
        if !self.peers.is_empty() {
            self.peers.clear();
            self.events.push_back(Event::Peers);
        }
        self.set_link(link);
        self.judge();
    }

    /// The socket is gone without the server having said why. The client asks again after a wait.
    fn lost(&mut self, now: u64) {
        self.drop_socket(Link::Offline);
        if self.wanted {
            let wait = RETRY_FIRST.saturating_mul(1 << self.failures.min(5)).min(RETRY_LONGEST);
            // Devices that lost one server together do not all come back in the same instant. The core
            // has no randomness, and each device's clock started somewhere else.
            self.retry_at = Some(now + wait + now % (wait / 4));
            self.failures += 1;
        }
    }

    fn take(&mut self, message: ServerMessage, local: Local, now: u64) {
        match message {
            ServerMessage::Welcome(welcome) => self.welcomed(welcome, local, now),
            ServerMessage::Goodbye { why } => {
                self.events.push_back(Event::Close);
                if why.is_final() {
                    self.retry_at = None;
                    self.drop_socket(Link::Refused(why));
                } else if why == Refusal::AccessChanged {
                    // The server expects this session back at once, in its new role.
                    self.drop_socket(Link::Offline);
                    if self.wanted {
                        self.ask(now);
                    }
                } else {
                    self.lost(now);
                }
            }
            // Everything else belongs to a session.
            _ if !matches!(self.socket, Socket::Live(_)) => {}
            ServerMessage::Held { seq } => {
                if self.style_sent.is_some_and(|sent| sent <= seq) {
                    (self.style, self.style_sent) = (None, None);
                }
                self.gapped = false;
                let held = seq.min(self.mark);
                if !self.refused && held > self.held {
                    self.held = held;
                    self.events.push_back(Event::Held(held));
                }
                self.judge();
            }
            ServerMessage::Refused { seq, why } => {
                if self.style_sent == Some(seq) {
                    // The style is one the server cannot take, however often it is sent.
                    (self.style, self.style_sent) = (None, None);
                }
                self.refused = true;
                self.events.push_back(Event::Refused(why));
                if why == Refusal::Unreadable && !mem::replace(&mut self.gapped, true) {
                    // The edits built on some the server never took in. A new handshake tells which.
                    self.events.push_back(Event::Close);
                    return self.lost(now);
                }
                self.judge();
            }
            ServerMessage::Edits { updates } => {
                // What the server sends, it holds. An update it wrote wrongly is the shell's to refuse.
                if let (Some(known), Ok(reach)) = (&mut self.known, Extent::of(&updates)) {
                    known.merge(&reach);
                }
                self.events.push_back(Event::Merge(updates));
            }
            ServerMessage::Notes { notes } => {
                self.told.extend(notes.notes.iter().map(|note| (note.id.clone(), note.clone())));
                self.told_extra.extend(notes.extra.clone());
                self.events.push_back(Event::Notes(notes));
            }
            ServerMessage::Joined(peer) => {
                self.peers.retain(|held| held.session != peer.session);
                self.peers.push(peer);
                self.events.push_back(Event::Peers);
            }
            ServerMessage::Selection { session, selection } => {
                if let Some(peer) = self.peers.iter_mut().find(|peer| peer.session == session) {
                    peer.selection = selection;
                    self.events.push_back(Event::Peers);
                }
            }
            ServerMessage::Left { session } => {
                self.peers.retain(|peer| peer.session != session);
                self.events.push_back(Event::Peers);
            }
            ServerMessage::Pong => {}
        }
    }

    fn welcomed(&mut self, welcome: Welcome, local: Local, now: u64) {
        if self.socket != Socket::Greeting {
            return;
        }
        if !self.document.is_empty() && welcome.document != self.document {
            // The server should have said so itself. Its document is not merged into this one.
            self.events.push_back(Event::Close);
            self.retry_at = None;
            return self.drop_socket(Link::Refused(Refusal::OtherDocument));
        }
        (self.document, self.created) = (welcome.document, welcome.created);
        self.socket = Socket::Live(welcome.role);
        self.session = Some(welcome.session);
        self.failures = 0;
        self.refused = false;
        self.set_link(Link::Live(welcome.role));
        self.peers = welcome.peers;
        self.events.push_back(Event::Peers);

        self.known = Some(welcome.extent);
        if !welcome.updates.is_empty() {
            self.events.push_back(Event::Merge(welcome.updates));
        }
        self.told = welcome.notes.notes.iter().map(|note| (note.id.clone(), note.clone())).collect();
        self.told_extra = welcome.notes.extra.clone();
        self.told_any = true;
        if local.notes.clone().merge(&welcome.notes) {
            self.events.push_back(Event::Notes(welcome.notes));
        }

        // Whatever the server lacks goes now: what was written offline, or lost with the last socket.
        let before = self.mark;
        self.flush(local, now);
        self.send_style(now);
        if self.selection.is_some() {
            self.tell_selection(now);
        }
        self.owed = false;
        if self.mark == before && !self.withheld() && self.style.is_none() && self.held < self.mark {
            // The server lacked nothing, so it holds all that was ever counted.
            self.held = self.mark;
            self.events.push_back(Event::Held(self.held));
        }
        self.judge();
    }

    /// Whether the session may send what `needs` asks for right now.
    fn may(&self, needs: fn(Role) -> bool) -> bool {
        matches!(self.socket, Socket::Live(role) if needs(role))
    }

    /// The next number to send under.
    fn next(&mut self) -> u64 {
        self.mark += 1;
        self.mark
    }

    /// Sends what the server lacks of the document and its notes, or counts it as waiting.
    fn flush(&mut self, local: Local, now: u64) {
        if let Some(model) = local.model {
            self.flush_document(model, now);
        }
        self.flush_notes(local.notes, now);
        self.judge();
    }

    fn flush_document(&mut self, model: &Model, now: u64) {
        let revision = model.revision();
        let can = self.may(Role::edits);
        if self.seen.as_ref() == Some(&revision) && !(self.document_waits && can) {
            return;
        }
        self.seen = Some(revision);
        let extent = model.extent();
        // Until a server says what it holds, only what is written from here on counts as work.
        let reference = self.known.as_ref().unwrap_or_else(|| self.baseline.get_or_insert_with(|| extent.clone()));
        if extent <= *reference {
            return self.document_waits = false;
        }
        match &mut self.known {
            Some(known) if can => {
                let updates = model.updates_for(known);
                self.counted = Some(extent.clone());
                self.document_waits = false;
                let seq = self.next();
                let message = ClientMessage::Edits { seq, updates };
                if message.encode().len() > MAX_FRAME {
                    // No frame can carry it. The shell hands the server the file whole instead.
                    self.refused = true;
                    return self.events.push_back(Event::Refused(Refusal::TooLarge));
                }
                if let Some(known) = &mut self.known {
                    known.merge(&extent);
                }
                self.send(&message, now);
            }
            _ => {
                if self.counted.as_ref() != Some(&extent) {
                    self.counted = Some(extent);
                    self.mark += 1;
                }
                self.document_waits = true;
            }
        }
    }

    fn flush_notes(&mut self, notes: &Notes, now: u64) {
        if !self.told_any {
            // As with the document: until a server says, only what changes from here on counts.
            self.told = notes.notes.iter().map(|note| (note.id.clone(), note.clone())).collect();
            self.told_extra = notes.extra.clone();
            self.told_any = true;
        }
        // A commenter's session sends comments alone. Any other note that differs here is not its to send.
        let commenter = matches!(self.socket, Socket::Live(Role::Commenter));
        let mut news = Vec::new();
        for note in notes.notes.iter().filter(|note| !commenter || note.kind == NoteKind::Comment) {
            let told = self.told.get(&note.id);
            if told.is_some_and(|told| same(told, note)) {
                continue;
            }
            // What the server makes of the two is what it holds afterwards.
            let joined = match told {
                Some(told) => join(told, note),
                None => note.clone(),
            };
            if told.is_none_or(|told| !same(told, &joined)) {
                news.push((note.clone(), joined));
            }
        }
        let extra = !commenter && notes.extra.iter().any(|(key, value)| self.told_extra.get(key) != Some(value));
        if news.is_empty() && !extra {
            return self.notes_wait.clear();
        }
        if self.may(Role::comments) {
            let mut sent = Notes::default();
            for (note, joined) in news {
                self.told.insert(note.id.clone(), joined);
                sent.notes.push(note);
            }
            if extra {
                sent.extra = notes.extra.clone();
                self.told_extra.extend(notes.extra.clone());
            }
            self.notes_wait.clear();
            let seq = self.next();
            self.send(&ClientMessage::Notes { seq, notes: sent }, now);
        } else {
            let waiting: Vec<Note> = news.into_iter().map(|(note, _)| note).collect();
            if waiting != self.notes_wait {
                self.notes_wait = waiting;
                self.mark += 1;
            }
        }
    }

    fn send_style(&mut self, now: u64) {
        if let (Some(style), None, true) = (self.style.clone(), self.style_sent, self.may(Role::edits)) {
            self.style_sent = Some(self.mark);
            self.send(&ClientMessage::Style { seq: self.mark, style }, now);
        }
    }

    fn tell_selection(&mut self, now: u64) {
        let due = now.saturating_sub(self.selection_at) >= SELECTION_EVERY;
        if self.selection != self.selection_told && due && matches!(self.socket, Socket::Live(_)) {
            self.selection_told = self.selection.clone();
            self.selection_at = now;
            self.send(&ClientMessage::Selection(self.selection.clone()), now);
        }
    }

    /// Whether work is held here that the session could not send.
    fn withheld(&self) -> bool {
        self.document_waits || !self.notes_wait.is_empty() || (self.style.is_some() && self.style_sent.is_none())
    }

    /// Works out [`Progress`] anew.
    fn judge(&mut self) {
        let waiting = self.held < self.mark || self.owed;
        let progress = match self.socket {
            _ if !waiting => Progress::Saved,
            Socket::None => Progress::Edited,
            Socket::Asked(_) | Socket::Greeting => Progress::Saving,
            Socket::Live(_) if self.refused || self.withheld() => Progress::Edited,
            Socket::Live(_) => Progress::Saving,
        };
        if self.progress != progress {
            self.progress = progress;
            self.events.push_back(Event::Progress);
        }
    }
}

/// Whether two copies of a note say the same. Where its text is right now is left out when both are tied
/// to their text: each replica works that out from its own document, and they differ while edits travel.
fn same(a: &Note, b: &Note) -> bool {
    if a.anchor.stable.is_none() || a.anchor.stable != b.anchor.stable {
        return a == b;
    }
    a == b || {
        let mut b = b.clone();
        (b.anchor.block, b.anchor.range) = (a.anchor.block.clone(), a.anchor.range.clone());
        *a == b
    }
}

/// One note from two copies of it, as [`Notes::merge`] joins them.
fn join(a: &Note, b: &Note) -> Note {
    let mut joined = Notes { notes: vec![a.clone()], extra: Extra::new() };
    joined.merge(&Notes { notes: vec![b.clone()], extra: Extra::new() });
    joined.notes.pop().expect("two copies of a note join into one")
}
