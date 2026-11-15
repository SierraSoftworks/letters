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
//! whatever the server lacks, so edits written offline, and edits a lost socket took with it,
//! arrive then. For work that must outlive the program, the shell keeps its own journal and lets go of it
//! on [`Event::Held`].

use letters_model::{Model, Notes, Selection, Style};

use crate::access::{Refusal, Role};
use crate::presence::Peer;
use crate::wire::SessionId;

/// The document as it is open on this device, lent to the client for the length of a call.
///
/// The model must be the one whose history the file's was attached to: see
/// [`Model::attach`](letters_model::Model::attach). A model that shows an older revision is fine.
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
    /// Close the socket. [`Client::closed`] may follow, and need not.
    Close,
    /// Edits made elsewhere. Merge them into the document as a file's are merged, and then call
    /// [`Client::changed`].
    Merge(Vec<u8>),
    /// Notes that were added or changed elsewhere. Join them into the notes with
    /// [`Notes::merge`](letters_model::Notes::merge), and then call [`Client::changed`].
    Notes(Notes),
    /// The server keeps on its disk everything this device held at the call that returned this number.
    /// The shell's own journal may let go of it.
    Held(u64),
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
    /// The server keeps everything written here.
    Saved,
    /// Work is on its way to the server, or the handshake that tells what it lacks is.
    Saving,
    /// Work is held here that cannot be sent now: there is no socket, or the role does not allow it.
    Edited,
}

/// A device's side of the sync of one document.
pub struct Client {
    _built: (),
}

impl Client {
    /// A client for the document with this id. An empty id is for a device that holds no copy yet. It
    /// does nothing until [`Client::start`].
    pub fn new(document: impl Into<String>) -> Self {
        let _ = document.into();
        todo!()
    }

    /// Goes online: asks for a socket now, and again after each loss, waiting longer each time.
    pub fn start(&mut self, now: u64) {
        let _ = now;
        todo!()
    }

    /// Goes offline and stays there, as when the window closes or the person signs out.
    pub fn stop(&mut self) {
        todo!()
    }

    /// The socket [`Event::Open`] asked for is open. The client says hello.
    pub fn opened(&mut self, local: Local, now: u64) {
        let _ = (local, now);
        todo!()
    }

    /// A binary message arrived. One that cannot be read ends the socket, and the client tries again.
    pub fn received(&mut self, frame: &[u8], local: Local, now: u64) {
        let _ = (frame, local, now);
        todo!()
    }

    /// The socket closed or could not be opened. Nothing is lost: the client tries again later.
    pub fn closed(&mut self, now: u64) {
        let _ = now;
        todo!()
    }

    /// The document or its notes may have changed here: by the person, or by a merge of what
    /// [`Client::poll`] handed out. Costs next to nothing when neither did, so call it freely.
    ///
    /// Returns a number for what the device holds now. [`Event::Held`] with that number, or a later
    /// one, says the server keeps all of it.
    pub fn changed(&mut self, local: Local, now: u64) -> u64 {
        let _ = (local, now);
        todo!()
    }

    /// The person set the document's style in the window. Returns a number as [`Client::changed`] does.
    ///
    /// The style comes back to every device in the history: read
    /// [`Model::style`](letters_model::Model::style) when a merge's `Change::style` says it changed.
    pub fn restyled(&mut self, style: &Style, now: u64) -> u64 {
        let _ = (style, now);
        todo!()
    }

    /// The person's caret or selection moved. `None` when the window has none to show.
    pub fn selected(&mut self, model: &Model, selection: Option<&Selection>, now: u64) {
        let _ = (model, selection, now);
        todo!()
    }

    /// The time [`Client::wake_at`] named has come.
    pub fn tick(&mut self, now: u64) {
        let _ = now;
        todo!()
    }

    /// When to call [`Client::tick`] next, whatever else happens before. `None` when nothing is due.
    pub fn wake_at(&self) -> Option<u64> {
        todo!()
    }

    /// The next thing for the shell to do. Drain it after every call.
    pub fn poll(&mut self) -> Option<Event> {
        todo!()
    }

    pub fn link(&self) -> Link {
        todo!()
    }

    pub fn progress(&self) -> Progress {
        todo!()
    }

    /// This session as the others know it, while in session.
    pub fn session(&self) -> Option<SessionId> {
        todo!()
    }

    /// The document's id and when it was created, as RFC 3339, once the server has said. A device that
    /// starts its copy from the first [`Event::Merge`] keeps both with it.
    pub fn document(&self) -> Option<(&str, &str)> {
        todo!()
    }

    /// Everyone else who has the document open, in the order they came. Empty while not in session.
    pub fn peers(&self) -> &[Peer] {
        todo!()
    }
}
