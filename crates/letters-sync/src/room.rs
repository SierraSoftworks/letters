//! The server's side of one open document: a room that its sessions meet in.
//!
//! The host owns the sockets, the folders and the clock, and lends them call by call. The times it tells
//! are milliseconds since 1970. For each open document the host:
//!
//! - takes the file's stamp, reads the file into a [`WorkingCopy`], opens a room over it with
//!   [`Room::open`] and hands it the stamp with [`Room::read_as`];
//! - admits each socket it has checked with [`Room::join`], feeds its frames to [`Room::received`], and
//!   says when it closed with [`Room::leave`];
//! - drains [`Room::poll`] after every call, for the frames to send and the sockets to close;
//! - at [`Room::wake_at`], calls [`Room::tick`] and then [`Room::exchange`];
//! - drops the room once [`Room::is_idle`], and calls [`Room::stop`] on each room before it stops itself.
//!
//! Every change is in the room's journal before it is acknowledged, so a server that stops loses nothing
//! it said it held. The file is rewritten when the edits pause, the way every writer of a shared folder
//! saves: read, merge, write beside, rename. What another writer put in the file reaches the sessions
//! through that merge.

use std::cmp::Ordering;
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Seek};
use std::mem;

use letters_format::{ASSETS, Error, Files, Journal, Reading, Save, Saved, Stamp, WorkingCopy, hash};
use letters_model::{Extent, Notes, Revision, Style};

use crate::access::{Access, Refusal, Role, comments};
use crate::presence::{Passage, Peer};
use crate::wire::{ClientMessage, Hello, MAX_FRAME, PROTOCOL, ServerMessage, SessionId, Welcome};
use crate::{PING_AFTER, SILENT_AFTER};

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

struct Session {
    access: Access,
    colour: u8,
    /// Whether its hello came. Until then it is sent nothing and is told to no one.
    greeted: bool,
    heard: u64,
    selection: Option<Passage>,
    /// Where the device's copy stood at its hello, while that copy and the room each hold edits the
    /// other lacks: the device wrote while it was away. Its first edits settle against it.
    apart: Option<Revision>,
    /// How many of its messages were refused as unreadable.
    unreadable: u32,
}

/// One open document on the server.
pub struct Room {
    copy: WorkingCopy,
    journal: Journal,
    sessions: BTreeMap<SessionId, Session>,
    /// The last session id handed out. One is never used twice.
    last: u32,
    out: VecDeque<Outbound>,
    /// When the first change came that no save has taken to the file, and when the latest did.
    unsaved: Option<(u64, u64)>,
    /// When to read back a file that was written, which is what lets the journal go.
    read_back: Option<u64>,
    /// When a save that failed is tried again. Nothing else is due before.
    retry_at: Option<u64>,
    /// When to look next whether the file changed, and how it looked when it was last read or written.
    look_at: u64,
    stamp: Option<Stamp>,
    /// Whether the last save went to a conflict copy. Nothing is saved until the host says where to.
    parked: bool,
    /// Pictures that were handed over and that the document does not name yet.
    offered: BTreeMap<String, Vec<u8>>,
    /// Acknowledgements that wait for a save, since the journal could not take what they are for.
    owed: Vec<(SessionId, u64)>,
}

/// A host keeps rooms behind a lock and calls them from any thread.
const _: fn() = || {
    fn sent<T: Send>() {}
    sent::<Room>();
};

impl Room {
    /// Opens a room over a document as it was read from its file. A new document is written to its file
    /// before a room is opened over it.
    ///
    /// `state` is a folder on the host's local disk for this document's journal, and `home` says where
    /// the document is saved, in the host's own words. What a run that stopped early left in the journal
    /// is taken in, and is then owed to the file. Fails when the journal cannot be read or does not fit
    /// the document.
    ///
    /// The copy is [wary](WorkingCopy::wary) from here on: what devices send and what other writers leave
    /// in the file is tried apart from the document before it is merged. A host opens the file itself
    /// with [`WorkingCopy::open_wary`], so that the history it starts from is checked as well.
    pub fn open(mut copy: WorkingCopy, home: &str, state: &mut impl Files, now: u64) -> Result<Self, Error> {
        copy.wary = true;
        let (journal, left) = Journal::open(state, &copy.id, &copy.created, Some(home))?;
        if let Some(left) = &left {
            copy.replay(left)?;
        }
        Ok(Self {
            copy,
            // Whatever the journal holds leaves it only through a save.
            unsaved: (!journal.is_empty()).then_some((now, now)),
            journal,
            sessions: BTreeMap::new(),
            last: 0,
            out: VecDeque::new(),
            read_back: None,
            retry_at: None,
            look_at: now + LOOK_EVERY,
            stamp: None,
            parked: false,
            offered: BTreeMap::new(),
            owed: Vec::new(),
        })
    }

    /// How the document's file looked just before the host read the copy this room was opened over. The
    /// room then leaves the file alone until a writer changes it. A host that does not say costs the
    /// first look a read of the whole file, and nothing else.
    pub fn read_as(&mut self, stamp: Stamp) {
        self.stamp = Some(stamp);
    }

    /// The document as the room holds it, for the host to render or export.
    pub fn copy(&self) -> &WorkingCopy {
        &self.copy
    }

    /// Admits a socket that the host has checked. The room waits for its hello.
    pub fn join(&mut self, access: Access, now: u64) -> SessionId {
        self.last += 1;
        let taken = |colour: &u8| self.sessions.values().any(|session| session.colour == *colour);
        // The first colour nobody here has, so that few people look as different as the window can make them.
        let colour = (0..=u8::MAX).find(|colour| !taken(colour)).unwrap_or(self.last as u8);
        let session =
            Session { access, colour, greeted: false, heard: now, selection: None, apart: None, unreadable: 0 };
        self.sessions.insert(SessionId(self.last), session);
        SessionId(self.last)
    }

    /// A binary message arrived from a session. What it changes is journalled in `state`, then
    /// acknowledged and passed on. What its role does not allow is refused. Returns whether the document
    /// or its notes changed.
    ///
    /// Fails when the journal could not be written. The change is in memory all the same and goes to the
    /// file with the next save, and it is acknowledged only then.
    pub fn received(
        &mut self,
        session: SessionId,
        frame: &[u8],
        state: &mut impl Files,
        now: u64,
    ) -> Result<bool, Error> {
        let Some(from) = self.sessions.get_mut(&session) else { return Ok(false) };
        from.heard = now;
        let (greeted, role) = (from.greeted, from.access.role);
        let message = match ClientMessage::decode(frame) {
            Ok(Some(message)) => message,
            // A kind of message from a later build.
            Ok(None) => return Ok(false),
            Err(_) => return Ok(self.end(session, Refusal::Unreadable)),
        };
        match message {
            ClientMessage::Hello(hello) if !greeted => Ok(self.greet(session, hello)),
            // A second hello, or anything else before the first.
            ClientMessage::Hello(_) => Ok(self.end(session, Refusal::Unreadable)),
            _ if !greeted => Ok(self.end(session, Refusal::Unreadable)),
            ClientMessage::Ping => Ok(self.to(session, &ServerMessage::Pong)),
            ClientMessage::Selection(selection) => {
                self.sessions.get_mut(&session).expect("it was just found").selection = selection.clone();
                Ok(self.pass_on(Some(session), &ServerMessage::Selection { session, selection }))
            }
            ClientMessage::Edits { seq, .. } | ClientMessage::Style { seq, .. } if !role.edits() => {
                Ok(self.refuse(session, seq, Refusal::ReadOnly))
            }
            ClientMessage::Notes { seq, .. } if !role.comments() => Ok(self.refuse(session, seq, Refusal::NoComments)),
            ClientMessage::Edits { seq, updates } => self.edits(session, seq, &updates, state, now),
            ClientMessage::Style { seq, style } => self.style(session, seq, style, state, now),
            ClientMessage::Notes { seq, notes } => self.notes(session, role, seq, notes, state, now),
        }
    }

    /// A session's socket closed.
    pub fn leave(&mut self, session: SessionId) {
        if self.sessions.remove(&session).is_some_and(|session| session.greeted) {
            self.pass_on(None, &ServerMessage::Left { session });
        }
        self.owed.retain(|(owed, _)| *owed != session);
    }

    /// Ends a session from the host's side, as when the person signs out or their access is withdrawn.
    /// A session whose role changed is ended with [`Refusal::AccessChanged`] and joins again.
    pub fn dismiss(&mut self, session: SessionId, why: Refusal) {
        self.end(session, why);
    }

    /// Takes a picture that a device handed to the host, for the edit that names it. The name is
    /// `assets/<sha256>.<extension>` and must be the hash of the bytes.
    ///
    /// The picture goes to the journal and the file once the document names it. One that no edit ever
    /// names is forgotten with the room, and does not keep the room from being idle.
    pub fn add_asset(&mut self, name: &str, bytes: Vec<u8>, state: &mut impl Files, now: u64) -> Result<(), Error> {
        let stem = name.strip_prefix(ASSETS).and_then(|file| file.split('.').next());
        if stem.is_none_or(|stem| hash(&bytes).strip_prefix("sha256:") != Some(stem)) {
            return Err(Error::HashMismatch(name.into()));
        }
        self.offered.insert(name.into(), bytes);
        self.adopt_assets(state, now)
    }

    /// A picture the file may not hold yet. The host reads every other one from the file.
    pub fn asset(&self, name: &str) -> Option<&[u8]> {
        self.copy.entries.get(name).or_else(|| self.offered.get(name)).map(Vec::as_slice)
    }

    /// The next thing for the host to do. Drain it after every call.
    pub fn poll(&mut self) -> Option<Outbound> {
        self.out.pop_front()
    }

    /// When to call [`Room::tick`] and [`Room::exchange`] next. `None` when nothing is due.
    pub fn wake_at(&self) -> Option<u64> {
        let silent = self.sessions.values().map(|session| session.heard + SILENT_AFTER).min();
        [silent, self.file_at()].into_iter().flatten().min()
    }

    /// Ends the sessions that have been silent for [`SILENT_AFTER`](crate::SILENT_AFTER): a device says
    /// something every [`PING_AFTER`](crate::PING_AFTER) at least.
    pub fn tick(&mut self, now: u64) {
        const _: () = assert!(PING_AFTER < SILENT_AFTER);
        let silent = |(id, session): (&SessionId, &Session)| (now >= session.heard + SILENT_AFTER).then_some(*id);
        let silent: Vec<SessionId> = self.sessions.iter().filter_map(silent).collect();
        for session in silent {
            // Nothing is said to a socket that seems dead.
            self.out.push_back(Outbound::Close(session));
            self.leave(session);
        }
    }

    /// Whether [`Room::exchange`] has anything to do at `now`.
    pub fn due(&self, now: u64) -> bool {
        self.file_at().is_some_and(|at| at <= now)
    }

    /// Does what is due between the room and the document's file in `folder`: a look at whether another
    /// writer changed it, or a save. `None` when neither was due or the look found nothing new.
    ///
    /// A save merges what the file holds first, and what that brings goes to every session. A file that
    /// was written is read back once more when things are quiet, and only then does the journal let go.
    ///
    /// After [`Saved::ConflictCopy`] the room saves nothing more, since a file that keeps refusing would
    /// be given a new copy with every save. The sessions go on, and the journal keeps their edits and
    /// those the copy holds. The host decides: it ends the sessions with [`Refusal::Gone`] and drops the
    /// room, or says where the document lives from now on with [`Room::moved`], which may be where it
    /// lived before. A room that is opened over the file later tries the file once more.
    pub fn exchange(
        &mut self,
        folder: &mut impl Files,
        state: &mut impl Files,
        save: &Save,
        now: u64,
    ) -> Result<Option<Saved>, Error> {
        if !self.due(now) {
            return Ok(None);
        }
        let looking = self.retry_at.is_none() && !self.owes(now);
        if looking {
            self.look_at = now + LOOK_EVERY;
            // A stamp is only a hint, and one that cannot be read is no reason to leave the file alone.
            if folder.stamp(save.name).is_ok_and(|stamp| stamp == self.stamp) {
                return Ok(None);
            }
        }
        self.flush(folder, state, save, now).map(Some)
    }

    /// Merges another copy of the document into the room's own: a device's whole file, or a conflict copy
    /// beside the document. What it brought goes to every session and is owed to the file like an edit,
    /// with nothing of it in the journal: the host calls [`Room::flush`] at once, and says only then that
    /// the copy was merged.
    ///
    /// The copy is read once, and before the host holds the room: sessions write on while a long history
    /// is read. A host that merged it into the file instead would have the room read the whole file back.
    /// Fails as [`WorkingCopy::merge_read`] does.
    pub fn merge_read<R: Read + Seek>(
        &mut self,
        other: Reading<'_, R>,
        state: &mut impl Files,
        now: u64,
    ) -> Result<(), Error> {
        let before = (self.copy.model().extent(), self.copy.notes.clone());
        let merged = self.copy.merge_read(other);
        // A copy that broke off half merged may have brought something in all the same.
        if merged.is_ok() || self.copy.model().extent() != before.0 {
            self.touch(now);
        }
        let told = self.tell_news(&before, state, now);
        merged.and(told)
    }

    /// Saves now, whatever is due: before the host renders the file or moves it to the trash. Like
    /// [`Room::exchange`] in everything else. A host that stops calls [`Room::stop`].
    pub fn flush(
        &mut self,
        folder: &mut impl Files,
        state: &mut impl Files,
        save: &Save,
        now: u64,
    ) -> Result<Saved, Error> {
        let mark = self.journal.mark();
        let before = (self.copy.model().extent(), self.copy.notes.clone());
        let saved = match letters_format::save(folder, &mut self.copy, save) {
            Ok(saved) => saved,
            Err(error) => {
                // The merge may have brought something in before the write failed.
                self.tell_news(&before, state, now)?;
                self.unsaved.get_or_insert((now, now));
                self.retry_at = Some(now + LOOK_EVERY);
                return Err(error);
            }
        };
        (self.unsaved, self.read_back, self.retry_at) = (None, None, None);
        self.look_at = now + LOOK_EVERY;
        let held = match &saved {
            Saved::Unchanged => self.journal.saved(state, mark, &saved),
            Saved::Written | Saved::Restored { .. } => {
                self.read_back = Some(now + SAVE_AFTER);
                Ok(())
            }
            Saved::ConflictCopy { .. } => {
                // The journal keeps what the file lacks, so a room that is opened over the file later
                // holds it too, whatever becomes of the copy.
                self.parked = true;
                Ok(())
            }
        };
        self.stamp = folder.stamp(save.name).ok().flatten();
        // The document is safe on disk however the save ended.
        for (session, seq) in mem::take(&mut self.owed) {
            self.to(session, &ServerMessage::Held { seq });
        }
        self.tell_news(&before, state, now)?;
        held?;
        Ok(saved)
    }

    /// The host is stopping, or drops the room with people in it. Writes to the file whatever it lacks,
    /// and closes every session once it was sent what the save let the room say. Drain [`Room::poll`]
    /// afterwards.
    ///
    /// `None` when the file lacked nothing, and for a room that waits for [`Room::moved`], whose file
    /// would only be given another copy. What the file does not hold then stays in the journal, as it
    /// does when the save fails, and the room that is opened over the file next takes it in.
    ///
    /// The devices are told no reason, so each asks again and is back once the server is.
    pub fn stop(
        &mut self,
        folder: &mut impl Files,
        state: &mut impl Files,
        save: &Save,
        now: u64,
    ) -> Result<Option<Saved>, Error> {
        let saved = match self.parked || self.unsaved.is_none() {
            true => Ok(None),
            false => self.flush(folder, state, save, now).map(Some),
        };
        for session in mem::take(&mut self.sessions).into_keys() {
            self.out.push_back(Outbound::Close(session));
        }
        self.owed.clear();
        saved
    }

    /// The document is saved at `home` from now on, in the host's own words: after a move or a rename,
    /// or to save again after a conflict copy. The next [`Room::exchange`] saves there.
    pub fn moved(&mut self, home: &str, state: &mut impl Files) -> Result<(), Error> {
        self.parked = false;
        self.stamp = None;
        // Due at once.
        self.retry_at = Some(0);
        self.journal.moved(state, home)
    }

    /// How many sessions are in the room.
    pub fn sessions(&self) -> usize {
        self.sessions.len()
    }

    /// The names of the sessions that have said hello, one for each: who has the document open now.
    pub fn people(&self) -> impl Iterator<Item = &str> {
        self.sessions.values().filter(|session| session.greeted).map(|session| session.access.name.as_str())
    }

    /// How many of a session's messages the room refused as [`Refusal::Unreadable`] and went on. A sound
    /// device hangs up at the first, so a host may end a session that keeps sending them.
    pub fn unreadable(&self, session: SessionId) -> u32 {
        self.sessions.get(&session).map_or(0, |session| session.unreadable)
    }

    /// Whether the room has no sessions and nothing that is due to the file, so that the host may drop
    /// it. What the journal still holds is taken in by the room that is opened next.
    pub fn is_idle(&self) -> bool {
        self.sessions.is_empty() && self.file_at().is_none() && self.owed.is_empty()
    }
}

impl Room {
    /// Sends a message to one session. Always `false`, for the callers that changed nothing.
    fn to(&mut self, session: SessionId, message: &ServerMessage) -> bool {
        self.out.push_back(Outbound::Send(session, message.encode()));
        false
    }

    /// Tells a session that its message numbered `seq` was not taken. Always `false`.
    fn refuse(&mut self, session: SessionId, seq: u64, why: Refusal) -> bool {
        if let Some(from) = self.sessions.get_mut(&session).filter(|_| why == Refusal::Unreadable) {
            from.unreadable = from.unreadable.saturating_add(1);
        }
        self.to(session, &ServerMessage::Refused { seq, why })
    }

    /// Sends a message to every session that said hello, but for one.
    fn pass_on(&mut self, but: Option<SessionId>, message: &ServerMessage) -> bool {
        let frame = message.encode();
        let greeted = |(id, session): (&SessionId, &Session)| (session.greeted && Some(*id) != but).then_some(*id);
        let to: Vec<SessionId> = self.sessions.iter().filter_map(greeted).collect();
        for session in to {
            if frame.len() > MAX_FRAME {
                // It lacks what no frame can bring it, and takes the file whole.
                self.end(session, Refusal::TooLarge);
            } else {
                self.out.push_back(Outbound::Send(session, frame.clone()));
            }
        }
        false
    }

    /// Says goodbye to a session and closes it.
    fn end(&mut self, session: SessionId, why: Refusal) -> bool {
        if self.sessions.contains_key(&session) {
            self.to(session, &ServerMessage::Goodbye { why });
            self.out.push_back(Outbound::Close(session));
            self.leave(session);
        }
        false
    }

    fn greet(&mut self, session: SessionId, hello: Hello) -> bool {
        let held = self.copy.model().extent();
        let why = match hello.protocol.cmp(&PROTOCOL) {
            Ordering::Less => Some(Refusal::TooOld),
            Ordering::Greater => Some(Refusal::TooNew),
            // A copy of another document would be merged into this one whole. Its id tells, and so does a
            // history with no edit in common. A device with no copy has neither.
            Ordering::Equal if hello.extent.is_empty() => None,
            Ordering::Equal if hello.document != self.copy.id || !hello.extent.shares(&held) => {
                Some(Refusal::OtherDocument)
            }
            Ordering::Equal => None,
        };
        if let Some(why) = why {
            return self.end(session, why);
        }
        let holds = matches!(hello.extent.partial_cmp(&held), Some(Ordering::Greater | Ordering::Equal));
        let welcomed = held.clone();
        let peer = |(id, session): (&SessionId, &Session)| {
            let Session { access, colour, selection, .. } = session;
            Peer { session: *id, name: access.name.clone(), colour: *colour, selection: selection.clone() }
        };
        let others = self.sessions.iter().filter(|(_, session)| session.greeted).map(peer).collect();
        let joining = &self.sessions[&session];
        let joined = ServerMessage::Joined(peer((&session, joining)));
        let welcome = ServerMessage::Welcome(Welcome {
            protocol: PROTOCOL,
            session,
            role: joining.access.role,
            document: self.copy.id.clone(),
            created: self.copy.created.clone(),
            updates: if holds { Vec::new() } else { self.copy.model().updates_for(&hello.extent) },
            extent: held,
            notes: self.copy.notes.clone(),
            peers: others,
        });
        let frame = welcome.encode();
        if frame.len() > MAX_FRAME {
            return self.end(session, Refusal::TooLarge);
        }
        self.pass_on(None, &joined);
        let greeted = self.sessions.get_mut(&session).expect("it was just read");
        greeted.greeted = true;
        greeted.apart = hello.revision.filter(|_| hello.extent.partial_cmp(&welcomed).is_none());
        self.out.push_back(Outbound::Send(session, frame));
        false
    }

    fn edits(
        &mut self,
        session: SessionId,
        seq: u64,
        updates: &[u8],
        state: &mut impl Files,
        now: u64,
    ) -> Result<bool, Error> {
        let before = (self.copy.model().extent(), self.copy.model().revision());
        // The copy is wary, and tries a device's update apart from the document first. One that the
        // history library panics on, or cannot save and read again, would cost everyone the document,
        // and one that builds on edits the room never saw would wait in it unseen.
        if self.copy.edit(updates).is_err() {
            return Ok(self.refuse(session, seq, Refusal::Unreadable));
        }
        // Held is a promise, so the room sees for itself that the document now reaches all of the update.
        let reaches = |reach: Extent| self.copy.model().extent().partial_cmp(&reach);
        let whole = matches!(Extent::of(updates).map(reaches), Ok(Some(Ordering::Greater | Ordering::Equal)));
        let apart = self.sessions.get_mut(&session).and_then(|session| session.apart.take()).filter(|_| whole);
        let met = (self.copy.model().extent(), apart.is_some().then(|| self.copy.notes.clone()));
        if let Some(theirs) = &apart {
            // What the device wrote while it was away meets what was written here as two files meet. A
            // revision the edits did not bring is one the device made up, and settles nothing.
            let _ = self.copy.settle(&before.1, theirs);
        }
        // What the document gained is what is kept and passed on, whatever was sent.
        let mut kept = self.keep_edits(Some(session), &before.0, state, now);
        if self.copy.model().extent() != met.0 {
            // The room wrote the settling, so the device that sent the edits lacks it too.
            let settled = self.copy.model().updates_for(&met.0);
            self.to(session, &ServerMessage::Edits { updates: settled });
        }
        if let Some(notes) = &met.1 {
            let raised = self.keep_notes(None, notes, state, now);
            kept = kept.and_then(|edits| Ok(edits | raised?));
        }
        if whole && kept.is_ok() {
            self.to(session, &ServerMessage::Held { seq });
        } else if whole {
            self.owed.push((session, seq));
        } else {
            self.refuse(session, seq, Refusal::Unreadable);
        }
        kept
    }

    fn style(
        &mut self,
        session: SessionId,
        seq: u64,
        style: Style,
        state: &mut impl Files,
        now: u64,
    ) -> Result<bool, Error> {
        let before = self.copy.model().extent();
        if self.copy.set_style(style).is_err() {
            return Ok(self.refuse(session, seq, Refusal::Unreadable));
        }
        // The room wrote the change, so the session that asked for it lacks it too.
        let kept = self.keep_edits(None, &before, state, now);
        match kept {
            Ok(_) => _ = self.to(session, &ServerMessage::Held { seq }),
            Err(_) => self.owed.push((session, seq)),
        }
        kept
    }

    /// Journals what the document gained since it held `before`, and passes it on to every session but
    /// one. Returns whether it gained anything.
    fn keep_edits(
        &mut self,
        but: Option<SessionId>,
        before: &Extent,
        state: &mut impl Files,
        now: u64,
    ) -> Result<bool, Error> {
        if self.copy.model().extent() == *before {
            return Ok(false);
        }
        let gained = self.copy.model().updates_for(before);
        let journalled = self.journal.edits(state, &gained);
        self.pass_on(but, &ServerMessage::Edits { updates: gained });
        self.touch(now);
        journalled.and(self.adopt_assets(state, now)).map(|()| true)
    }

    fn notes(
        &mut self,
        session: SessionId,
        role: Role,
        seq: u64,
        sent: Notes,
        state: &mut impl Files,
        now: u64,
    ) -> Result<bool, Error> {
        let mut taken = match role.edits() {
            true => sent.clone(),
            false => match comments(&self.copy.notes, &sent) {
                Ok(allowed) => allowed,
                Err(why) => return Ok(self.refuse(session, seq, why)),
            },
        };
        let before = self.copy.notes.clone();
        let mut notes = mem::take(&mut self.copy.notes);
        taken.pin(self.copy.model());
        notes.merge(&taken);
        // A conflict that came in may be newer than one held for its block.
        notes.raise([]);
        notes.follow(self.copy.model());
        self.copy.notes = notes;
        let kept = self.keep_notes(Some((session, &sent)), &before, state, now);
        match kept {
            Ok(_) => _ = self.to(session, &ServerMessage::Held { seq }),
            Err(_) => self.owed.push((session, seq)),
        }
        kept
    }

    /// Journals the notes if they differ from `before`, and passes on the ones that do. The session that
    /// sent some is told only those that the room holds differently than it sent them. Returns whether
    /// any differ.
    fn keep_notes(
        &mut self,
        from: Option<(SessionId, &Notes)>,
        before: &Notes,
        state: &mut impl Files,
        now: u64,
    ) -> Result<bool, Error> {
        let held = &self.copy.notes;
        let differs = |notes: &Notes| {
            let news = held.notes.iter().filter(|note| !notes.notes.contains(note)).cloned().collect();
            let extra = held.extra.iter().filter(|(key, value)| notes.extra.get(*key) != Some(value));
            Notes { notes: news, extra: extra.map(|(key, value)| (key.clone(), value.clone())).collect() }
        };
        let empty = |notes: &Notes| notes.notes.is_empty() && notes.extra.is_empty();
        let news = differs(before);
        if empty(&news) {
            return Ok(false);
        }
        let back = from.map(|(session, sent)| (session, differs(sent)));
        let journalled = self.journal.notes(state, &self.copy.notes);
        self.pass_on(from.map(|(session, _)| session), &ServerMessage::Notes { notes: news });
        if let Some((session, back)) = back.filter(|(_, back)| !empty(back)) {
            self.to(session, &ServerMessage::Notes { notes: back });
        }
        self.touch(now);
        journalled.map(|()| true)
    }

    /// Tells every session what a save's merge brought in from the file since the room held `before`.
    /// What came from the file is in the file, so none of it goes to the journal but the notes, which
    /// the merge may have added to.
    fn tell_news(&mut self, before: &(Extent, Notes), state: &mut impl Files, now: u64) -> Result<(), Error> {
        let unsaved = self.unsaved;
        if self.copy.model().extent() != before.0 {
            let gained = self.copy.model().updates_for(&before.0);
            self.pass_on(None, &ServerMessage::Edits { updates: gained });
        }
        let kept = self.keep_notes(None, &before.1, state, now);
        // News from the file is no work that is owed to the file.
        self.unsaved = unsaved;
        kept.and(self.adopt_assets(state, now))
    }

    /// Moves the pictures that the document has come to name from the ones offered to the ones a save
    /// writes, by way of the journal.
    fn adopt_assets(&mut self, state: &mut impl Files, now: u64) -> Result<(), Error> {
        if self.offered.is_empty() {
            return Ok(());
        }
        let named: Vec<String> = {
            let document = self.copy.model().doc().assets();
            self.offered.keys().filter(|name| document.contains(name.as_str())).cloned().collect()
        };
        let mut journalled = Ok(());
        for name in named {
            let bytes = self.offered.remove(&name).expect("it was just listed");
            journalled = journalled.and(self.journal.asset(state, &name, &bytes));
            self.copy.entries.insert(name, bytes);
            self.touch(now);
        }
        journalled
    }

    /// Something changed that the file does not hold yet.
    fn touch(&mut self, now: u64) {
        let first = self.unsaved.map_or(now, |(first, _)| first);
        self.unsaved = Some((first, now));
    }

    /// When the file is next due a save.
    fn save_at(&self) -> Option<u64> {
        match self.unsaved {
            Some((first, latest)) => Some((latest + SAVE_AFTER).min(first + SAVE_WITHIN)),
            // A file that was written is read back when things are quiet. While they are not, the next
            // save reads it anyway.
            None => self.read_back,
        }
    }

    /// Whether a save is due at `now`, and not only a look.
    fn owes(&self, now: u64) -> bool {
        self.save_at().is_some_and(|at| at <= now)
    }

    /// When the file is next due anything.
    fn file_at(&self) -> Option<u64> {
        if self.parked {
            return None;
        }
        if self.retry_at.is_some() {
            return self.retry_at;
        }
        let look = (!self.sessions.is_empty()).then_some(self.look_at);
        [self.save_at(), look].into_iter().flatten().min()
    }
}
