//! A document that lives on a Letters server: its copy on this device, and its line to the server.
//!
//! The copy is a `.let` file like any other, kept under `.letters/servers` in the person's home folder
//! and saved, journalled and merged as a file the person opened is. So the document opens at once, and
//! can be written in with no server in reach. The line trades what each side lacks while there is one:
//! `letters-sync` says what to send and what to merge, and this carries it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use letters_api::{Code, Opened, paths};
use letters_format::Archive;
use letters_model::{Model, Notes, Selection};
use letters_sync::{Client as Syncing, Event, Link, Local, MAX_FRAME, Peer, Progress, Refusal};
use sha2::{Digest, Sha256};
use ureq::http::Uri;

use crate::keychain::Keychain;
use crate::remote::Roots;
use crate::server::{Client, Refused};
use crate::session::Session;
use crate::signin::{SignIn, Unsigned};
use crate::socket::{Closed, Heard, Socket};

/// The server and the document that the address of a document's page names, as a browser shows it:
/// `https://letters.example/d/ID`. `None` for any other address.
pub fn address(typed: &str) -> Option<(String, String)> {
    let uri: Uri = typed.trim().parse().ok()?;
    let authority = uri.authority().filter(|_| uri.scheme_str() == Some("https"))?;
    let mut parts = uri.path().strip_prefix("/d/")?.split('/');
    let id = unescaped(parts.next()?).filter(|id| !id.is_empty())?;
    // The page to read the document on names it as well.
    matches!((parts.next(), parts.next()), (None | Some("" | "read"), None))
        .then(|| (format!("https://{authority}"), id))
}

/// A part of an address as it was before its bytes were escaped.
fn unescaped(part: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(part.len());
    let mut rest = part.bytes();
    while let Some(byte) = rest.next() {
        bytes.push(match byte {
            b'%' => {
                let digits = [rest.next()?, rest.next()?];
                u8::from_str_radix(std::str::from_utf8(&digits).ok()?, 16).ok()?
            }
            byte => byte,
        });
    }
    String::from_utf8(bytes).ok()
}

/// Where the copy of a server's document is kept on this device: `.letters/servers/HOST/ID.let` in the
/// person's home folder.
pub fn cached(home: &Path, server: &Client, id: &str) -> PathBuf {
    let host = server.origin().trim_start_matches("https://");
    home.join(".letters").join("servers").join(named(host)).join(format!("{}.let", named(id)))
}

/// `text` as one name in a folder. A server chooses its ids, so nothing of one is taken as written but
/// letters, digits, `-`, `_` and a `.` that is not first: no name leads out of its folder.
fn named(text: &str) -> String {
    // An empty name would leave a file that is all ending. No other name is written so.
    let mut out = String::from(if text.is_empty() { "%" } else { "" });
    for (at, byte) in text.bytes().enumerate() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' => out.push(char::from(byte)),
            b'.' if at > 0 => out.push('.'),
            byte => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Why a server's document could not be opened.
#[derive(Debug)]
pub enum Unreached {
    /// The server did not hand it over, and there is no copy of it on this device.
    Refused(Refused),
    /// The copy could not be kept on this device.
    Unkept(io::Error),
    /// Nobody signed in to the server.
    Unsigned(Unsigned),
    /// The system opened no browser to sign in at.
    NoBrowser(io::Error),
}

impl fmt::Display for Unreached {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refused) => refused.fmt(f),
            Self::Unkept(error) => write!(f, "its copy could not be kept on this device ({error})"),
            Self::Unsigned(unsigned) => unsigned.fmt(f),
            Self::NoBrowser(_) => write!(f, "no browser could be opened to sign in at"),
        }
    }
}

impl std::error::Error for Unreached {}

/// A server's document, ready to open from its copy on this device.
#[derive(Clone, Debug, PartialEq)]
pub struct Reached {
    /// What the server said of the document: now, or when it was last asked if it is out of reach.
    pub opened: Opened,
    /// The copy's file.
    pub path: PathBuf,
    /// Whether the server answered just now.
    pub online: bool,
}

/// Asks the server for the document and brings its file to this device if no copy is here yet. A
/// document with a copy opens from it whether or not the server is in reach. This waits on the network.
pub fn reach(session: &Session, id: &str, home: &Path) -> Result<Reached, Unreached> {
    let path = cached(home, session.server(), id);
    let said = path.with_extension("json");
    let opened = match session.open(id) {
        Ok(opened) => opened,
        // No word from the server is no word against the copy that is here.
        Err(Refused::Unreachable(_) | Refused::Slow) if path.exists() => {
            let opened = fs::read(&said).ok().and_then(|json| serde_json::from_slice(&json).ok());
            return Ok(Reached { opened: opened.unwrap_or_else(|| unsaid(id)), path, online: false });
        }
        Err(refused) => {
            // What the person may no longer read is not kept for them.
            if matches!(refused.code(), Some(Code::Forbidden | Code::NotFound)) {
                let _ = (fs::remove_file(&path), fs::remove_file(&said));
            }
            return Err(Unreached::Refused(refused));
        }
    };
    let keep = |to: &Path, bytes: &[u8]| {
        // Written beside and renamed, so that a crash leaves a whole file or none.
        let beside = to.with_extension("new");
        fs::write(&beside, bytes).and_then(|()| fs::rename(&beside, to))
    };
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(Unreached::Unkept)?;
    }
    if !path.exists() {
        let most = session.limits().map_err(Unreached::Refused)?.document;
        let file = session.file(id, most).map_err(Unreached::Refused)?;
        keep(&path, &file).map_err(Unreached::Unkept)?;
    }
    // Kept for the day the document is opened with no server in reach.
    let _ = serde_json::to_vec(&opened).map(|json| keep(&said, &json));
    Ok(Reached { opened, path, online: true })
}

/// One picture of a server's document that the copy here lacks, by the name the document has for it:
/// `assets/HASH.png`. `None` when the server has none of that name, and when what it sent is not the
/// picture: a name is the hash of what it names. This waits on the network.
pub fn picture(session: &Session, id: &str, name: &str) -> Option<Vec<u8>> {
    let short = name.strip_prefix("assets/")?;
    let bytes = session.asset(id, short, session.limits().ok()?.asset).ok()?;
    let hash: String = Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    (short.split('.').next() == Some(hash.as_str())).then_some(bytes)
}

/// How long a person has at the browser to sign in.
const SIGN_IN_WITHIN: Duration = Duration::from_secs(300);

/// Opens the way to a server's document: a session at its server, and the document's copy on this device.
///
/// The session is `held` if the app has one there, else the one the keychain kept from an earlier run,
/// else a new one from a sign-in at the system's browser, which `browse` is handed the page of. The wait
/// for the browser ends when `given_up` is set. All of this waits on the network, and on the person.
pub fn enter(
    server: &Client,
    id: &str,
    held: Option<Arc<Session>>,
    keychain: Option<Keychain>,
    home: &Path,
    browse: &dyn Fn(&str) -> io::Result<()>,
    given_up: &AtomicBool,
) -> Result<(Arc<Session>, Reached), Unreached> {
    let kept = || Session::kept(server.clone(), keychain?).ok().flatten().map(Arc::new);
    if let Some(session) = held.or_else(kept).filter(|session| !session.ended()) {
        match reach(&session, id, home) {
            // A session the server has ended is followed by a new sign-in.
            Err(Unreached::Refused(refused)) if refused.code() == Some(Code::SignedOut) => {}
            reached => return reached.map(|reached| (session, reached)),
        }
    }
    // Nobody is sent to sign in at what is no Letters server.
    server.discover().map_err(Unreached::Refused)?;
    let sign_in = SignIn::begin().map_err(Unreached::Unsigned)?;
    browse(&sign_in.page(server).map_err(Unreached::Unsigned)?).map_err(Unreached::NoBrowser)?;
    let token = sign_in.finish(server, SIGN_IN_WITHIN, given_up).map_err(Unreached::Unsigned)?;
    let session = Arc::new(Session::begin(server.clone(), keychain, token));
    reach(&session, id, home).map(|reached| (session, reached))
}

/// What is known of a document whose server never said anything this device kept: no more than its id.
/// The server says the rest once it is back.
fn unsaid(id: &str) -> Opened {
    Opened {
        id: id.to_owned(),
        created: String::new(),
        name: id.to_owned(),
        space: None,
        path: String::new(),
        role: letters_api::Role::Reader,
        author: String::new(),
        socket: paths::socket(id),
        trust: letters_api::Trust::Safe,
        allows: letters_api::Allows::default(),
        style: None,
        notes: None,
    }
}

/// What a line leaves for the window that holds the document to do, in the order it must be done.
#[derive(Clone, Debug, PartialEq)]
pub enum Told {
    /// Edits made elsewhere. Merge them into the document, and then call [`Line::changed`].
    Merge(Vec<u8>),
    /// Notes added or changed elsewhere. Join them into the notes, move them to where their text is now,
    /// and then call [`Line::changed`].
    Notes(Notes),
    /// The server did not take work that was sent, and why. The work stays in the copy here.
    Refused(Refusal),
    /// The server did not take the picture of this name, so the others will not see it.
    Unkept(String),
    /// [`Line::link`] changed.
    Link,
    /// [`Line::progress`] changed.
    Progress,
    /// [`Line::peers`] changed.
    Peers,
}

/// What a line's window hands back to [`Line::heard`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Said {
    /// The socket of this number had something to say.
    Socket(u64, Heard),
    /// The pictures the server was owed were handed over, or could not be for now.
    Handed,
}

/// What tells a line's window that there is something for [`Line::heard`]. It is called on another thread.
pub type Tell = Arc<dyn Fn(Said) + Send + Sync>;

/// How long the line waits before it tries again to hand over pictures the server could not be given, in
/// milliseconds.
const HAND_AGAIN: u64 = 5_000;

/// The pictures the server is to hold before it is sent the edits that name them.
#[derive(Default)]
struct Owed {
    /// Pictures that came into the document here, by their names in it.
    pictures: BTreeMap<String, Vec<u8>>,
    /// Whether the pictures in the copy's file were held against what the server has: the copy may hold
    /// some from a run that ended before they were handed over.
    compared: bool,
    /// The pictures the server would not take, until the window is told.
    unkept: Vec<String>,
}

/// What a line and the threads that hand over its pictures share.
struct Handing {
    session: Arc<Session>,
    /// The document's id, and where its copy is on this device.
    id: String,
    copy: PathBuf,
    owed: Mutex<Owed>,
    /// Held by whoever is handing pictures over, so that no picture goes twice at once.
    turn: Mutex<()>,
}

impl Handing {
    fn owed(&self) -> MutexGuard<'_, Owed> {
        self.owed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Hands the server every picture it is owed. Fails when the server cannot be asked now, with the rest
    /// still owed. A picture the server will not take is given up, and told. This waits on the network.
    fn hand(&self) -> Result<(), Refused> {
        let _turn = self.turn.lock().unwrap_or_else(PoisonError::into_inner);
        if !self.owed().compared {
            match self.session.assets(&self.id) {
                Ok(there) => self.compare(&there.into_iter().collect()),
                Err(refused) if passes(&refused) => return Err(refused),
                // A server that lists nothing for this session takes nothing from it either.
                Err(_) => {}
            }
            self.owed().compared = true;
        }
        loop {
            let next = self.owed().pictures.iter().next().map(|(name, bytes)| (name.clone(), bytes.clone()));
            let Some((name, bytes)) = next else { return Ok(()) };
            let short = name.strip_prefix(ASSETS).unwrap_or(&name);
            let kept = self.session.keep_asset(&self.id, short, &bytes);
            match kept {
                Err(refused) if passes(&refused) => return Err(refused),
                kept => {
                    let mut owed = self.owed();
                    owed.pictures.remove(&name);
                    if kept.is_err() {
                        owed.unkept.push(name);
                    }
                }
            }
        }
    }

    /// Owes the server the pictures in the copy's file that it does not list.
    fn compare(&self, there: &BTreeSet<String>) {
        let Ok(file) = fs::File::open(&self.copy) else { return };
        let Ok(mut file) = Archive::open(io::BufReader::new(file)) else { return };
        let lacks = |name: &&str| name.strip_prefix(ASSETS).is_some_and(|short| !there.contains(short));
        let lacking: Vec<String> = file.names().filter(lacks).map(str::to_owned).collect();
        for name in lacking {
            if let Ok(Some(bytes)) = file.read(&name) {
                self.owed().pictures.entry(name).or_insert(bytes);
            }
        }
    }
}

/// Where a document keeps its pictures, as its blocks name them.
const ASSETS: &str = "assets/";

/// Whether a refusal may not last: the network is down, the server is busy, or the sign-in is over, which
/// the socket's own refusal then says.
fn passes(refused: &Refused) -> bool {
    match refused {
        Refused::Unreachable(_) | Refused::Slow => true,
        Refused::Status(status) => *status >= 500,
        refused => refused.code() == Some(Code::SignedOut),
    }
}

/// One open document's line to its server. The window feeds it whatever happens, each time with the time
/// in milliseconds on its own clock, and does what comes back.
pub struct Line {
    session: Arc<Session>,
    /// The whole address of the document's socket.
    address: String,
    roots: Roots,
    sync: Syncing,
    socket: Option<Socket>,
    /// How many sockets were opened. Only the last is heard: an earlier one may still be saying how it
    /// ended.
    sockets: u64,
    tell: Tell,
    /// The server would not take the session's token for the last socket.
    denied: bool,
    /// Why the server will open no socket for this document, however often it is asked.
    ended: Option<Refusal>,
    handing: Arc<Handing>,
    /// Whether a thread is handing pictures over while the line is live.
    hands: bool,
    /// When to try that again, after the server could not be given them.
    hand_at: Option<u64>,
}

impl Line {
    /// The line of the document `opened`, whose copy on this device is the file at `copy`. It does nothing
    /// until [`Line::start`]. `tell` is called when there is something for [`Line::heard`].
    pub fn new(session: Arc<Session>, opened: &Opened, copy: &Path, roots: Roots, tell: Tell) -> Self {
        let address = session.server().socket(&opened.socket);
        let sync = Syncing::new(opened.id.clone());
        let handing = Arc::new(Handing {
            session: session.clone(),
            id: opened.id.clone(),
            copy: copy.to_owned(),
            owed: Mutex::default(),
            turn: Mutex::default(),
        });
        Self {
            session,
            address,
            roots,
            sync,
            socket: None,
            sockets: 0,
            tell,
            denied: false,
            ended: None,
            handing,
            hands: false,
            hand_at: None,
        }
    }

    /// The session the line asks its server with.
    pub fn session(&self) -> &Arc<Session> {
        &self.session
    }

    /// Goes online. Only once the file's history is attached to the editor's model. `owes` says that the
    /// journal holds work of an earlier run, which the server may lack until the first handshake has told.
    pub fn start(&mut self, owes: bool, now: u64) -> Vec<Told> {
        self.ended = None;
        if owes {
            self.sync.owes();
        }
        self.sync.start(now);
        self.drain()
    }

    /// Goes offline and stays there, as when the window closes.
    pub fn stop(&mut self) {
        self.sync.stop();
        self.socket = None;
    }

    /// A picture came into the document here, under this name. Say so before [`Line::changed`] hears of
    /// the edit that names it: the server is handed the picture first.
    pub fn keep(&mut self, name: &str, bytes: Vec<u8>) {
        self.handing.owed().pictures.insert(name.to_owned(), bytes);
    }

    /// What `tell` was called with.
    pub fn heard(&mut self, said: Said, local: Local<'_>, now: u64) -> Vec<Told> {
        let (socket, heard) = match said {
            Said::Socket(socket, heard) => (socket, heard),
            Said::Handed => {
                self.hands = false;
                // What is still owed could not be handed over now.
                self.hand_at = self.owes().then_some(now + HAND_AGAIN);
                return self.changed(local, now);
            }
        };
        if socket != self.sockets {
            return Vec::new();
        }
        match heard {
            Heard::Open => {
                self.denied = false;
                self.sync.opened(local, now);
            }
            // A picture that came in since the socket was asked for is not with the server, and the
            // handshake would send the edit that names it: the next socket hands it over first.
            Heard::Frame(_) if self.sync.link() == Link::Connecting && self.owes() => {
                self.socket = None;
                self.sockets += 1;
                self.sync.closed(now);
            }
            Heard::Frame(frame) => self.sync.received(&frame, local, now),
            Heard::Closed(closed) => {
                self.socket = None;
                self.ended = match closed {
                    // A token the server will not take is renewed for the next socket, if it can be.
                    Closed::Refused(401) if self.session.ended() => Some(Refusal::SignedOut),
                    Closed::Refused(403) => Some(Refusal::NoAccess),
                    Closed::Refused(404) => Some(Refusal::Gone),
                    _ => None,
                };
                self.denied = closed == Closed::Refused(401);
                if self.ended.is_some() {
                    self.sync.stop();
                    return [vec![Told::Link], self.drain()].concat();
                }
                self.sync.closed(now);
            }
        }
        self.drain()
    }

    /// The document or its notes may have changed here, by the person or by a merge.
    pub fn changed(&mut self, local: Local<'_>, now: u64) -> Vec<Told> {
        // Edits wait for the pictures they may name.
        if !self.hand(now) {
            self.sync.changed(local, now);
        }
        self.drain()
    }

    /// Whether the server is owed pictures.
    fn owes(&self) -> bool {
        !self.handing.owed().pictures.is_empty()
    }

    /// Whether pictures are still to be handed over, which a live line starts on here. One with no socket
    /// hands them over before it opens the next.
    fn hand(&mut self, now: u64) -> bool {
        if self.hands {
            return true;
        }
        if !self.owes() {
            self.hand_at = None;
            return false;
        }
        if matches!(self.sync.link(), Link::Live(_)) && self.hand_at.is_none_or(|at| at <= now) {
            (self.hands, self.hand_at) = (true, None);
            let (handing, tell) = (self.handing.clone(), self.tell.clone());
            thread::spawn(move || {
                let _ = handing.hand();
                tell(Said::Handed);
            });
        }
        true
    }

    /// The person's caret or selection moved.
    pub fn selected(&mut self, model: &Model, selection: Option<&Selection>, now: u64) -> Vec<Told> {
        self.sync.selected(model, selection, now);
        self.drain()
    }

    /// The time [`Line::wake_at`] named has come.
    pub fn tick(&mut self, now: u64) -> Vec<Told> {
        self.sync.tick(now);
        self.hand(now);
        self.drain()
    }

    /// When to call [`Line::tick`] next. `None` when nothing is due.
    pub fn wake_at(&self) -> Option<u64> {
        let (sync, hand) = (self.sync.wake_at(), self.hand_at);
        sync.zip(hand).map(|(sync, hand)| sync.min(hand)).or(sync).or(hand)
    }

    /// How the document stands to its server.
    pub fn link(&self) -> Link {
        self.ended.map_or_else(|| self.sync.link(), Link::Refused)
    }

    /// Where the work written here is. Edits that wait for their pictures are on their way.
    pub fn progress(&self) -> Progress {
        match self.sync.progress() {
            Progress::Saved if self.hands || self.owes() => Progress::Saving,
            progress => progress,
        }
    }

    /// Everyone else who has the document open.
    pub fn peers(&self) -> &[Peer] {
        self.sync.peers()
    }

    /// Does what the sync asks that is the line's own to do, and gives the rest.
    fn drain(&mut self) -> Vec<Told> {
        let mut told: Vec<Told> =
            std::mem::take(&mut self.handing.owed().unkept).into_iter().map(Told::Unkept).collect();
        while let Some(event) = self.sync.poll() {
            match event {
                Event::Open => self.open(),
                Event::Send(frame) => {
                    if let Some(socket) = &self.socket {
                        socket.send(frame);
                    }
                }
                Event::Close => {
                    // What the socket still says of its end is for nobody: the next one has another number.
                    self.socket = None;
                    self.sockets += 1;
                }
                Event::Merge(edits) => told.push(Told::Merge(edits)),
                Event::Notes(notes) => told.push(Told::Notes(notes)),
                // The copy on this device and its journal keep the work whether or not the server has it.
                Event::Held(_) => {}
                Event::Refused(refusal) => told.push(Told::Refused(refusal)),
                Event::Link => told.push(Told::Link),
                Event::Progress => told.push(Told::Progress),
                Event::Peers => told.push(Told::Peers),
            }
        }
        told
    }

    fn open(&mut self) {
        self.sockets += 1;
        let (session, denied, number, tell) = (self.session.clone(), self.denied, self.sockets, self.tell.clone());
        let handing = self.handing.clone();
        let token = move || {
            // The pictures first: the handshake sends the edits that name them.
            let token = handing.hand().and_then(|()| if denied { session.renewed() } else { session.token() });
            token.map_err(|refused| match refused.code() {
                Some(Code::SignedOut) => Closed::Refused(401),
                _ => Closed::Lost(refused.to_string()),
            })
        };
        let heard = move |heard| tell(Said::Socket(number, heard));
        self.socket = Some(Socket::open(&self.address, token, &self.roots, MAX_FRAME, heard));
    }
}
