//! The server's side of one open document: a room that its sessions meet in.
//!
//! The host owns the sockets, the folders and the clock, and lends them call by call. The times it tells
//! are milliseconds since 1970, since the room dates what it writes. For each open document the host:
//!
//! - reads the file into a [`WorkingCopy`] and opens a room over it with [`Room::open`];
//! - admits each socket it has checked with [`Room::join`], feeds its frames to [`Room::received`], and
//!   says when it closed with [`Room::leave`];
//! - drains [`Room::poll`] after every call, for the frames to send and the sockets to close;
//! - at [`Room::wake_at`], calls [`Room::tick`] and then [`Room::exchange`];
//! - drops the room once [`Room::is_idle`].
//!
//! Every change is in the room's journal before it is acknowledged, so a server that stops loses nothing
//! it said it held. The file is rewritten when the edits pause, the way every writer of a shared folder
//! saves: read, merge, write beside, rename. What another writer put in the file reaches the sessions
//! through that merge.

use letters_format::{Error, Files, Save, Saved, WorkingCopy};

use crate::access::{Access, Refusal};
use crate::wire::SessionId;

/// The edits must pause for this long before the file is rewritten, in milliseconds.
pub const SAVE_AFTER: u64 = 2_000;
/// The file is rewritten this long after an edit at the latest, however the edits keep coming.
pub const SAVE_WITHIN: u64 = 30_000;
/// How often the room looks whether another writer changed the file, while it has sessions.
pub const LOOK_EVERY: u64 = 10_000;

/// What the room asks of the host, in the order it must be done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outbound {
    /// Send this frame to a session as one binary message.
    Send(SessionId, Vec<u8>),
    /// Close a session's socket, once what was to be sent to it is sent. [`Room::leave`] need not follow.
    Close(SessionId),
}

/// One open document on the server.
pub struct Room {
    _built: (),
}

impl Room {
    /// Opens a room over a document as it was read from its file, or as it was made for a new one.
    ///
    /// `state` is the host's journal folder on local disk, and `home` says where the document is saved,
    /// in the host's own words. What a run that stopped early left in the journal is taken in, and is then
    /// owed to the file. Fails when the journal cannot be read or does not fit the document.
    pub fn open(copy: WorkingCopy, home: &str, state: &mut impl Files, now: u64) -> Result<Self, Error> {
        let _ = (copy, home, state, now);
        todo!()
    }

    /// The document as the room holds it, for the host to render or export.
    pub fn copy(&self) -> &WorkingCopy {
        todo!()
    }

    /// Admits a socket that the host has checked. The room waits for its hello.
    pub fn join(&mut self, access: Access, now: u64) -> SessionId {
        let _ = (access, now);
        todo!()
    }

    /// A binary message arrived from a session. What it changes is journalled in `state`, then
    /// acknowledged and passed on. What its role does not allow is refused.
    ///
    /// Fails when the journal could not be written. The change is in memory all the same and goes to the
    /// file with the next save, and it is acknowledged only then.
    pub fn received(
        &mut self,
        session: SessionId,
        frame: &[u8],
        state: &mut impl Files,
        now: u64,
    ) -> Result<(), Error> {
        let _ = (session, frame, state, now);
        todo!()
    }

    /// A session's socket closed.
    pub fn leave(&mut self, session: SessionId) {
        let _ = session;
        todo!()
    }

    /// Ends a session from the host's side, as when the person signs out or their access is withdrawn or
    /// changed. A session whose role changed is ended with [`Refusal::Other`] and joins again.
    pub fn dismiss(&mut self, session: SessionId, why: Refusal) {
        let _ = (session, why);
        todo!()
    }

    /// Keeps a picture that a device handed to the host, until the file is seen to hold it. The name is
    /// `assets/<sha256>.<extension>` and must be the hash of the bytes.
    pub fn add_asset(&mut self, name: &str, bytes: Vec<u8>, state: &mut impl Files, now: u64) -> Result<(), Error> {
        let _ = (name, bytes, state, now);
        todo!()
    }

    /// A picture the file does not hold yet. The host reads every other one from the file.
    pub fn asset(&self, name: &str) -> Option<&[u8]> {
        let _ = name;
        todo!()
    }

    /// The next thing for the host to do. Drain it after every call.
    pub fn poll(&mut self) -> Option<Outbound> {
        todo!()
    }

    /// When to call [`Room::tick`] and [`Room::exchange`] next. `None` when nothing is due.
    pub fn wake_at(&self) -> Option<u64> {
        todo!()
    }

    /// Ends the sessions that have been silent for [`SILENT_AFTER`](crate::SILENT_AFTER).
    pub fn tick(&mut self, now: u64) {
        let _ = now;
        todo!()
    }

    /// Whether [`Room::exchange`] has anything to do at `now`.
    pub fn due(&self, now: u64) -> bool {
        let _ = now;
        todo!()
    }

    /// Does what is due between the room and the document's file in `folder`: a look at whether another
    /// writer changed it, or a save. `None` when neither was due or the look found nothing new.
    ///
    /// A save merges what the file holds first, and what that brings goes to every session. A file that
    /// was written is read back once more when things are quiet, and only then does the journal let go.
    /// After [`Saved::ConflictCopy`] the host decides where the document lives from now on, and says so
    /// with [`Room::moved`] before the next save.
    pub fn exchange(
        &mut self,
        folder: &mut impl Files,
        state: &mut impl Files,
        save: &Save,
        now: u64,
    ) -> Result<Option<Saved>, Error> {
        let _ = (folder, state, save, now);
        todo!()
    }

    /// The document is saved somewhere else from now on.
    pub fn moved(&mut self, home: &str, state: &mut impl Files) -> Result<(), Error> {
        let _ = (home, state);
        todo!()
    }

    /// How many sessions are in the room.
    pub fn sessions(&self) -> usize {
        todo!()
    }

    /// Whether the room has no sessions and owes the file nothing, so that the host may drop it.
    pub fn is_idle(&self) -> bool {
        todo!()
    }
}
