//! The documents that are open now. Each has a room of `letters-sync`, which is lent the sockets, the
//! folders and the clock here.
//!
//! A room is opened when the first device joins its document, and dropped once nobody is in it and its
//! file holds everything. A call into a room reads or writes a folder, so each runs where blocking is
//! allowed, behind the room's own lock. What a room journals is in a folder of its own under
//! `<state>/rooms/`, and a journal that a run of the server left behind is taken to its file as the next
//! run starts: what a device was told the server holds is not lost with the server.
//!
//! A room is of a file and not of a document, since a copy made by hand carries the id of its original.

use std::collections::HashMap;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use letters_api::{Code, Role};
use letters_format::{ASSETS, Archive, Error, Save, Saved, Unsaved, WorkingCopy, is_journal};
use letters_model::ReplicaId;
use letters_sync::{Access, LOOK_EVERY, Outbound, Refusal as Ended, Room, SessionId};
use tokio::sync::{Notify, mpsc};
use tokio::time::Instant;

use crate::api::{Found, blocking};
use crate::error::{Answer, Refusal};
use crate::ids;
use crate::session::Signed;
use crate::spaces::{Folder, Place, Root};
use crate::{App, Shared, log};

/// How many frames may wait for one socket. A device that falls this far behind is let go, and joins
/// again for what it lacks.
const BACKLOG: usize = 1024;
/// How long a room with nobody in it is kept once a picture was handed to it, for the device that is
/// about to send the edit that names the picture. In milliseconds, as every time here.
const LINGER: u64 = 60_000;
/// The least time between two turns of a room's keeper, so that a room that stays due cannot spin.
const REST: u64 = 50;
/// How often a save is tried before the document goes to a conflict copy.
const ATTEMPTS: u32 = 3;

/// What a socket is told to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    /// Send this as one binary message.
    Frame(Vec<u8>),
    /// Close the socket: the room has said why in the frame before.
    Close,
}

/// What a session may do in a room, by its role in the space. An owner edits: who may share is the
/// server's business and not the room's.
fn may(role: Role) -> letters_sync::Role {
    match role {
        Role::Reader => letters_sync::Role::Reader,
        Role::Commenter => letters_sync::Role::Commenter,
        Role::Editor | Role::Owner => letters_sync::Role::Editor,
    }
}

/// A lock that is taken even after a panic under it: what it guards is only ever replaced whole.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Names a file among the rooms: its root and its place.
fn key(root: &Root, place: &Place) -> String {
    format!("{}\0{}", root.key, place.as_str())
}

/// Where a document is saved, as its journal records it: the root's folder and the place in it.
fn home(root: &Root, place: &Place) -> String {
    format!("{}/{}", root.key, place.as_str())
}

/// A failure of the room's own, as a refusal. The journal and the folders are the server's to keep
/// working, so none of it is the device's fault.
fn failed(error: Error) -> Refusal {
    log(&format!("a room failed: {error}"));
    Refusal::new(Code::Unavailable, "The server cannot keep what is written just now.")
}

pub struct Rooms {
    /// Where the journals are: `<state>/rooms`.
    folder: PathBuf,
    /// By [`key`]. This lock is never taken by whoever holds a room's.
    open: Mutex<HashMap<String, Arc<Open>>>,
    /// The journals no room has: the id of the document each is of, and its folder's name. The room
    /// that is opened next for the document takes one over, wherever the document's file is by then.
    left: Mutex<Vec<(String, String)>>,
    /// The server is stopping, and no room is opened any more. Set and read under the lock on `open`.
    closed: AtomicBool,
    /// The time when the clock was first read, and that moment on the runtime's clock.
    started: OnceLock<(u64, Instant)>,
}

struct Open {
    state: Mutex<State>,
    /// When the room is next due, for its keeper. `u64::MAX` when nothing is.
    due: AtomicU64,
    /// Tells the keeper that `due` may have changed.
    wake: Notify,
    /// Who has the document open, for whoever lists it: read without waiting for a save to end.
    people: Mutex<Vec<String>>,
}

enum State {
    /// Whoever holds the lock is opening the room.
    Opening,
    Open(Box<Hosted>),
    /// The room was dropped. Whoever finds it asks for the file's room again.
    Gone,
}

/// A room and what it is lent.
struct Hosted {
    room: Room,
    root: Root,
    place: Place,
    /// The room's journal folder, and its name under [`Rooms::folder`].
    state: Folder,
    journal: String,
    seats: HashMap<SessionId, Seated>,
    /// The room is kept until then, though nobody may be in it.
    linger: u64,
    /// No save is tried before then, after the folder could not be reached.
    hold: u64,
    /// The last save went to a conflict copy, and the journal holds what the file lacks. Nothing more is
    /// saved, so that a file that keeps refusing is not given a copy with every save.
    parked: bool,
    /// The file was not there at the last look.
    missing: bool,
    /// The file is gone from its place. The room is dropped at its keeper's next turn.
    lost: bool,
}

struct Seated {
    to: mpsc::Sender<Out>,
    /// The sign-in the socket was opened with, to end the one with the other.
    signed: String,
}

/// One device's place in a room. Dropping it leaves the room.
pub struct Seat {
    app: Shared,
    open: Arc<Open>,
    session: SessionId,
    left: bool,
}

impl Rooms {
    pub fn new(state: &Path) -> Self {
        Self {
            folder: state.join("rooms"),
            open: Mutex::new(HashMap::new()),
            left: Mutex::new(Vec::new()),
            closed: AtomicBool::new(false),
            started: OnceLock::new(),
        }
    }

    /// The time in milliseconds since 1970, as the rooms are told it. It never runs backwards.
    pub fn now(&self) -> u64 {
        let since = |time: SystemTime| time.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis() as u64);
        let (started, at) = self.started.get_or_init(|| (since(SystemTime::now()), Instant::now()));
        started + at.elapsed().as_millis() as u64
    }

    /// Takes a room out of the list, if the list still holds this one for the file.
    fn forget(&self, key: &str, open: &Arc<Open>) {
        let mut rooms = lock(&self.open);
        if rooms.get(key).is_some_and(|listed| Arc::ptr_eq(listed, open)) {
            rooms.remove(key);
        }
    }

    /// Does `work` in the room of the file at `place`, which is opened if it is not open.
    fn within<T>(
        &self,
        app: &Shared,
        root: &Root,
        place: &Place,
        work: impl FnOnce(&mut Hosted, u64) -> Answer<T>,
    ) -> Answer<(Arc<Open>, T)> {
        let key = key(root, place);
        loop {
            let open = {
                let mut rooms = lock(&self.open);
                if self.closed.load(Ordering::Acquire) {
                    return Err(Refusal::new(Code::Unavailable, "The server is stopping."));
                }
                rooms.entry(key.clone()).or_insert_with(|| Arc::new(Open::new())).clone()
            };
            let mut state = lock(&open.state);
            let now = self.now();
            if matches!(*state, State::Opening) {
                match Hosted::open(self, root, place, None, now) {
                    Ok(hosted) => {
                        *state = State::Open(Box::new(hosted));
                        tokio::spawn(keep(app.clone(), open.clone()));
                    }
                    Err(refusal) => {
                        *state = State::Gone;
                        drop(state);
                        self.forget(&key, &open);
                        return Err(refusal);
                    }
                }
            }
            let State::Open(hosted) = &mut *state else {
                // Dropped as this came for it.
                drop(state);
                self.forget(&key, &open);
                continue;
            };
            let done = work(hosted, now);
            hosted.settle(&open, now);
            drop(state);
            return done.map(|done| (open, done));
        }
    }

    /// Does `work` in the room of a file, if one is open.
    fn if_open<T>(&self, key: &str, work: impl FnOnce(&mut Hosted, u64) -> T) -> Option<T> {
        let open = lock(&self.open).get(key).cloned()?;
        let mut state = lock(&open.state);
        let State::Open(hosted) = &mut *state else { return None };
        let now = self.now();
        let done = work(hosted, now);
        hosted.settle(&open, now);
        Some(done)
    }

    /// Does `work` in every open room.
    fn each(&self, mut work: impl FnMut(&mut Hosted, u64)) {
        let rooms: Vec<Arc<Open>> = lock(&self.open).values().cloned().collect();
        for open in rooms {
            if let State::Open(hosted) = &mut *lock(&open.state) {
                let now = self.now();
                work(hosted, now);
                hosted.settle(&open, now);
            }
        }
    }

    /// The names of who has the file open now, each once.
    pub fn people(&self, root: &Root, place: &Place) -> Vec<String> {
        let open = lock(&self.open).get(&key(root, place)).cloned();
        open.map(|open| lock(&open.people).clone()).unwrap_or_default()
    }

    /// Brings the file up to its room, before it is read whole: to be sent, or shown to read.
    pub fn save(&self, root: &Root, place: &Place) {
        self.if_open(&key(root, place), |hosted, now| hosted.save(now, true));
    }

    /// A picture the room of a file holds, which its file may not hold yet. `name` is the entry's.
    pub fn asset(&self, root: &Root, place: &Place, name: &str) -> Option<Vec<u8>> {
        let held = |hosted: &mut Hosted, _| hosted.room.asset(name).map(<[u8]>::to_vec);
        self.if_open(&key(root, place), held).flatten()
    }

    /// The pictures the room of a file holds beyond its file's, as their entries are named.
    pub fn assets(&self, root: &Root, place: &Place) -> Vec<String> {
        let held = |hosted: &mut Hosted, _| {
            let entries = hosted.room.copy().entries.keys();
            entries.filter(|name| name.starts_with(ASSETS)).cloned().collect()
        };
        self.if_open(&key(root, place), held).unwrap_or_default()
    }

    /// Hands the room of a file a picture, for the edit that is about to name it. `name` is the entry's,
    /// and says the hash of the bytes.
    pub fn add_asset(&self, app: &Shared, root: &Root, place: &Place, name: &str, bytes: Vec<u8>) -> Answer<()> {
        let handed = |hosted: &mut Hosted, now: u64| {
            let taken = hosted.room.add_asset(name, bytes, &mut hosted.state, now);
            taken.map_err(|error| match error {
                Error::HashMismatch(_) => {
                    Refusal::invalid("A picture goes by the hash of its bytes, and this is not it.")
                }
                error => failed(error),
            })?;
            hosted.linger = now + LINGER;
            Ok(())
        };
        self.within(app, root, place, handed).map(|_| ())
    }

    /// Does `work`, which moves or renames the file at `from` to `to`, or throws it away when there is no
    /// `to`. A room that is open for the file saves first and follows the file, or ends with it.
    ///
    /// No room is opened while a file moves, so that none writes to where the file was.
    pub fn moving<T>(
        &self,
        from: (&Root, &Place),
        to: Option<(Root, Place)>,
        work: impl FnOnce() -> Answer<T>,
    ) -> Answer<T> {
        let mut rooms = lock(&self.open);
        let from = key(from.0, from.1);
        let Some(open) = rooms.get(&from).cloned() else { return work() };
        let mut state = lock(&open.state);
        let State::Open(hosted) = &mut *state else { return work() };
        let now = self.now();
        // What cannot be saved now stays in the journal, and goes to the file where it is next.
        hosted.save_through(now);
        let done = match work() {
            Ok(done) => done,
            Err(refusal) => {
                hosted.settle(&open, now);
                return Err(refusal);
            }
        };
        rooms.remove(&from);
        match to {
            Some((root, place)) => {
                if let Err(error) = hosted.room.moved(&home(&root, &place), &mut hosted.state) {
                    log(&format!("a room's journal could not be told of a move: {error}"));
                }
                rooms.insert(key(&root, &place), open.clone());
                (hosted.root, hosted.place) = (root, place);
                (hosted.parked, hosted.missing, hosted.hold) = (false, false, 0);
                hosted.settle(&open, now);
            }
            None => {
                hosted.end(Ended::Gone);
                hosted.settle(&open, now);
                hosted.discard(self);
                *state = State::Gone;
                open.gone();
            }
        }
        Ok(done)
    }

    /// Ends every socket that was opened with the sign-in `signed`.
    pub fn signed_out(&self, signed: &str) {
        self.each(|hosted, _| {
            let theirs = |(session, seated): (&SessionId, &Seated)| (seated.signed == signed).then_some(*session);
            let theirs: Vec<SessionId> = hosted.seats.iter().filter_map(theirs).collect();
            theirs.into_iter().for_each(|session| hosted.room.dismiss(session, Ended::SignedOut));
        });
    }

    /// Does `work`, which writes to the file at `place` as any other writer of its folder would. A room
    /// that is open for the file saves first and reads the file again at once, so that its sessions are
    /// told what was written.
    pub fn around<T>(&self, root: &Root, place: &Place, work: impl FnOnce() -> Answer<T>) -> Answer<T> {
        let open = lock(&self.open).get(&key(root, place)).cloned();
        let Some(open) = open else { return work() };
        let mut state = lock(&open.state);
        let State::Open(hosted) = &mut *state else { return work() };
        let now = self.now();
        hosted.save(now, true);
        let done = work();
        hosted.save(now, true);
        hosted.settle(&open, now);
        done
    }

    /// Brings every file up to its room and ends every session, as the server stops. Each file is
    /// written and read back, which is when its journal lets go, so that nothing is left for the next
    /// start to recover. No room is opened afterwards.
    ///
    /// The devices are given no reason: each takes its socket for lost, and comes back when the server
    /// does, with whatever it was not told the server holds.
    pub fn close(&self) {
        let rooms: Vec<Arc<Open>> = {
            let mut rooms = lock(&self.open);
            self.closed.store(true, Ordering::Release);
            rooms.drain().map(|(_, open)| open).collect()
        };
        for open in rooms {
            let mut state = lock(&open.state);
            if let State::Open(hosted) = &mut *state {
                let now = self.now();
                hosted.save_through(now);
                // Each device hears what the save let the server say it holds, and then its socket closes.
                hosted.settle(&open, now);
                for (session, seated) in hosted.seats.drain() {
                    let _ = seated.to.try_send(Out::Close);
                    hosted.room.leave(session);
                }
                hosted.discard(self);
            }
            *state = State::Gone;
            open.gone();
        }
    }

    /// One turn of a room's keeper: what is due is done. Says whether the room is gone.
    fn turn(&self, open: &Arc<Open>) -> bool {
        let mut state = lock(&open.state);
        let State::Open(hosted) = &mut *state else { return true };
        let now = self.now();
        hosted.room.tick(now);
        if hosted.room.due(now) && now >= hosted.hold {
            hosted.save(now, false);
        }
        hosted.settle(open, now);
        if !hosted.lost && (!hosted.room.is_idle() || now < hosted.linger) {
            return false;
        }
        let key = key(&hosted.root, &hosted.place);
        hosted.discard(self);
        *state = State::Gone;
        drop(state);
        self.forget(&key, open);
        true
    }

    /// Takes the journals that an earlier run left to their files: each to where it says its document
    /// is saved, or to wherever the document is now. One whose document is not found waits for whoever
    /// opens the document next.
    pub fn recover(&self, app: &Shared) {
        let Ok(folders) = std::fs::read_dir(&self.folder) else { return };
        let roots = app.roots();
        let all: Vec<&Root> = roots.iter().collect();
        for folder in folders.flatten() {
            let Ok(name) = folder.file_name().into_string() else { continue };
            let Some(mut state) = self.journals(&name) else { continue };
            let Some(left) = left_in(&folder.path(), &mut state) else {
                // Nothing is owed to any file.
                let _ = std::fs::remove_dir_all(folder.path());
                continue;
            };
            let home = left.home.as_deref().unwrap_or_default();
            let said = all.iter().filter_map(|root| Some((*root, placed(root, home)?)));
            let said = said.max_by_key(|(root, _)| root.key.len());
            let found = said.filter(|(root, place)| holds(root, place, &left.id)).or_else(|| {
                let (at, place) = app.library.find(&app.store, &all, &left.id, app.now()).ok()??;
                Some((all[at], place))
            });
            let taken = found.is_some_and(|(root, place)| self.resume(app, root, &place, &name));
            if !taken {
                log(&format!("the journal {name} waits for its document, which was not found"));
                lock(&self.left).push((left.id, name));
            }
        }
    }

    /// Opens the room of a file over the journal folder `journal`, for the journal to reach the file.
    fn resume(&self, app: &Shared, root: &Root, place: &Place, journal: &str) -> bool {
        let (key, now) = (key(root, place), self.now());
        if lock(&self.open).contains_key(&key) {
            return false;
        }
        let mut hosted = match Hosted::open(self, root, place, Some(journal.to_owned()), now) {
            Ok(hosted) => hosted,
            Err(refusal) => {
                log(&format!("the journal {journal} could not be taken to its file: {}", refusal.0.message));
                return false;
            }
        };
        let open = Arc::new(Open::new());
        hosted.settle(&open, now);
        *lock(&open.state) = State::Open(Box::new(hosted));
        lock(&self.open).insert(key, open.clone());
        tokio::spawn(keep(app.clone(), open));
        true
    }

    /// The journal folder `name`, made if it is not there.
    fn journals(&self, name: &str) -> Option<Folder> {
        let path = self.folder.join(name);
        Root::open(&path.to_string_lossy(), true).and_then(|root| root.folder(&Place::default())).ok()
    }

    /// A journal that was left for the document `id`, for the room that is opened for it now.
    fn adopt(&self, id: &str) -> Option<String> {
        let mut left = lock(&self.left);
        let at = left.iter().position(|(document, _)| document == id)?;
        Some(left.swap_remove(at).1)
    }
}

/// The place in `root` that a journal's `home` names, if it names one there.
fn placed(root: &Root, home: &str) -> Option<Place> {
    Place::parse(home.strip_prefix(root.key.as_str())?.strip_prefix('/')?).ok()
}

/// What the journal in a room's folder still holds for its file, if anything.
fn left_in(path: &Path, state: &mut Folder) -> Option<Unsaved> {
    let names = std::fs::read_dir(path).ok()?.flatten().filter_map(|entry| entry.file_name().into_string().ok());
    let journals: Vec<String> = names.filter(|name| is_journal(name)).collect();
    journals.iter().find_map(|name| Unsaved::read(state, name).ok().flatten())
}

/// Whether the file at `place` holds the document `id`.
fn holds(root: &Root, place: &Place, id: &str) -> bool {
    let (folder, name) = place.split();
    let file = root.folder(&folder).and_then(|folder| folder.open(name));
    file.ok().and_then(|file| Archive::open(BufReader::new(file)).ok()).is_some_and(|file| file.manifest().id == id)
}

impl Open {
    fn new() -> Self {
        Self {
            state: Mutex::new(State::Opening),
            due: AtomicU64::new(u64::MAX),
            wake: Notify::new(),
            people: Mutex::new(Vec::new()),
        }
    }

    /// Tells the keeper that its room is gone, and that nobody has it open.
    fn gone(&self) {
        lock(&self.people).clear();
        self.due.store(0, Ordering::Release);
        self.wake.notify_one();
    }
}

impl Hosted {
    /// Opens a room over the file at `place`, with the journal folder `journal`, or one that was left
    /// for the document, or a new one.
    fn open(rooms: &Rooms, root: &Root, place: &Place, journal: Option<String>, now: u64) -> Answer<Self> {
        let (folder, name) = place.split();
        let file = root.folder(&folder)?.open(name)?;
        let mut archive = Archive::open(BufReader::new(file))?;
        // A replica of this room's own: what it settles between two devices is written under it.
        let copy = WorkingCopy::open(&mut archive, ReplicaId::new(&ids::block_id()))?;
        let journal = journal.or_else(|| rooms.adopt(&copy.id)).unwrap_or_else(ids::block_id);
        let unavailable = || Refusal::new(Code::Unavailable, "The server cannot keep what is written just now.");
        let mut state = rooms.journals(&journal).ok_or_else(unavailable)?;
        let room = Room::open(copy, &home(root, place), &mut state, now).map_err(failed)?;
        Ok(Self {
            room,
            root: root.again()?,
            place: place.clone(),
            state,
            journal,
            seats: HashMap::new(),
            linger: 0,
            hold: 0,
            parked: false,
            missing: false,
            lost: false,
        })
    }

    /// Does what the room asks: frames go to their sockets, and the keeper is told when the room is due.
    fn settle(&mut self, open: &Open, now: u64) {
        while let Some(out) = self.room.poll() {
            let (session, out) = match out {
                Outbound::Send(session, frame) => (session, Out::Frame(frame)),
                Outbound::Close(session) => (session, Out::Close),
            };
            let closes = out == Out::Close;
            let sent = self.seats.get(&session).map(|seated| seated.to.try_send(out).is_ok());
            if closes {
                self.seats.remove(&session);
            } else if sent == Some(false) {
                // Its socket takes nothing more: gone, or too far behind to be caught up frame by frame.
                self.seats.remove(&session);
                self.room.leave(session);
            }
        }
        let mut people: Vec<String> = self.room.people().map(str::to_owned).collect();
        people.sort();
        people.dedup();
        let mut shown = lock(&open.people);
        if *shown != people {
            *shown = people;
        }
        drop(shown);
        let lingers = (now < self.linger).then_some(self.linger);
        let due = [self.room.wake_at().map(|at| at.max(self.hold)), lingers].into_iter().flatten().min();
        // A room with nothing left to wait for is looked at once more, to be dropped.
        let over = self.lost || (due.is_none() && self.room.is_idle());
        let due = if over { Some(now) } else { due };
        open.due.store(due.unwrap_or(u64::MAX), Ordering::Release);
        open.wake.notify_one();
    }

    /// Saves everything the room has, and reads a file that was written back at once. That is when the
    /// journal lets go, which otherwise waits for a quiet moment.
    fn save_through(&mut self, now: u64) {
        if self.save(now, true) {
            self.save(now, true);
        }
    }

    /// Saves what is due between the room and its file, or everything the room has when `whatever` says
    /// so. A save that fails is tried again by the room, and its edits stay in the journal. Says whether
    /// the file was written.
    fn save(&mut self, now: u64, whatever: bool) -> bool {
        if self.parked || self.lost {
            return false;
        }
        let place = self.place.clone();
        let (folder, name) = place.split();
        let mut folder = match self.root.folder(&folder) {
            Ok(folder) if folder.has(name) => folder,
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                log(&format!("\"{name}\" cannot be saved while its folder cannot be reached: {error}"));
                self.hold = now + LOOK_EVERY;
                return false;
            }
            _ => {
                self.not_there(name, now);
                return false;
            }
        };
        (self.missing, self.hold) = (false, 0);
        let (token, day) = (ids::block_id(), ids::rfc3339(now / 1000));
        let label = format!("{} server", day.get(..10).unwrap_or(&day));
        let save = Save { name, token: &token, label: &label, attempts: ATTEMPTS };
        let saved = match whatever {
            true => self.room.flush(&mut folder, &mut self.state, &save, now).map(Some),
            false => self.room.exchange(&mut folder, &mut self.state, &save, now),
        };
        match saved {
            Ok(Some(Saved::ConflictCopy { name: copy, cause })) => {
                log(&format!("\"{name}\" could not be saved ({cause}), and what its room held went to \"{copy}\""));
                self.parked = true;
                // A file that is another document now, or one this build cannot write, is not the room's.
                if matches!(cause, Error::OtherDocument(_) | Error::NewerFormat(_) | Error::Unmerged) {
                    self.end(Ended::Gone);
                }
                false
            }
            Ok(saved) => matches!(saved, Some(Saved::Written | Saved::Restored { .. })),
            Err(error) => {
                log(&format!("\"{name}\" could not be saved, and is tried again: {error}"));
                false
            }
        }
    }

    /// The file is not where the room saves it. A writer that replaces a file in two steps leaves a
    /// moment like that, so one look is forgiven. At the second the document was moved or removed behind
    /// the server's back: writing it again here would make two of it, so the room ends, and its journal
    /// waits for whoever opens the document where it is now.
    fn not_there(&mut self, name: &str, now: u64) {
        if !std::mem::replace(&mut self.missing, true) {
            self.hold = now + LOOK_EVERY;
            return;
        }
        log(&format!("\"{name}\" is gone from its folder, and its room ends"));
        self.lost = true;
        self.end(Ended::Gone);
    }

    /// Ends every session, saying why.
    fn end(&mut self, why: Ended) {
        let sessions: Vec<SessionId> = self.seats.keys().copied().collect();
        sessions.into_iter().for_each(|session| self.room.dismiss(session, why));
    }

    /// Lets go of the room's journal folder: removed once its file holds everything, and left for the
    /// room that is opened next for the document otherwise.
    fn discard(&mut self, rooms: &Rooms) {
        let path = rooms.folder.join(&self.journal);
        match left_in(&path, &mut self.state) {
            Some(left) => lock(&rooms.left).push((left.id, self.journal.clone())),
            None => drop(std::fs::remove_dir_all(path)),
        }
    }
}

/// Wakes a room when it is due, until it is dropped.
async fn keep(app: Shared, open: Arc<Open>) {
    let mut rested = 0;
    loop {
        let (due, now) = (open.due.load(Ordering::Acquire).max(rested), app.rooms.now());
        if due > now {
            let wait = Duration::from_millis((due - now).min(24 * 60 * 60 * 1000));
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = open.wake.notified() => {}
            }
            continue;
        }
        let (turning, room) = (app.clone(), open.clone());
        let gone = tokio::task::spawn_blocking(move || turning.rooms.turn(&room)).await.unwrap_or(true);
        if gone {
            break;
        }
        rested = app.rooms.now() + REST;
    }
}

impl App {
    /// Seats `who` in the room of the document `id`: their place, and what to send down their socket.
    pub async fn join(self: &Arc<Self>, who: &Signed, id: &str) -> Answer<(Seat, mpsc::Receiver<Out>)> {
        let (app, who, id) = (self.clone(), who.clone(), id.to_owned());
        blocking(move || {
            let found = app.locate(&who, &id)?;
            app.seat(&who, &found)
        })
        .await
    }

    /// Seats `who` in the room of a document they were found to reach.
    fn seat(self: &Arc<Self>, who: &Signed, found: &Found) -> Answer<(Seat, mpsc::Receiver<Out>)> {
        let access = Access { role: may(found.space.role), name: self.signs_as(who)? };
        let (to, from) = mpsc::channel(BACKLOG);
        let signed = who.session.clone();
        let (open, session) = self.rooms.within(self, &found.root, &found.place, |hosted, now| {
            if hosted.lost {
                return Err(Refusal::not_found());
            }
            let session = hosted.room.join(access, now);
            hosted.seats.insert(session, Seated { to, signed });
            Ok(session)
        })?;
        Ok((Seat { app: self.clone(), open, session, left: false }, from))
    }
}

impl Seat {
    /// Does `work` in the seat's room, where blocking is allowed. Nothing is done in a room that is gone.
    fn inside(&self, work: impl FnOnce(&mut Hosted, SessionId, u64) + Send + 'static) -> tokio::task::JoinHandle<()> {
        let (app, open, session) = (self.app.clone(), self.open.clone(), self.session);
        tokio::task::spawn_blocking(move || {
            let mut state = lock(&open.state);
            let State::Open(hosted) = &mut *state else { return };
            let now = app.rooms.now();
            work(hosted, session, now);
            hosted.settle(&open, now);
        })
    }

    /// A binary message came from the device.
    pub async fn received(&self, frame: Vec<u8>) {
        let heard = self.inside(move |hosted, session, now| {
            if let Err(error) = hosted.room.received(session, &frame, &mut hosted.state, now) {
                log(&format!("a room could not journal an edit, which waits for the next save: {error}"));
            }
        });
        let _ = heard.await;
    }

    /// The sign-in the socket was opened with has run out.
    pub async fn signed_out(&self) {
        let _ = self.inside(|hosted, session, _| hosted.room.dismiss(session, Ended::SignedOut)).await;
    }

    /// The device's socket closed.
    pub async fn leave(mut self) {
        if let Some(left) = self.go() {
            let _ = left.await;
        }
    }

    fn go(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        if std::mem::replace(&mut self.left, true) || tokio::runtime::Handle::try_current().is_err() {
            return None;
        }
        Some(self.inside(|hosted, session, now| {
            hosted.seats.remove(&session);
            hosted.room.leave(session);
            if hosted.seats.is_empty() {
                // Nobody is left to wait for a pause: the file is brought up to the room at once.
                hosted.save(now, true);
            }
        }))
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        self.go();
    }
}
