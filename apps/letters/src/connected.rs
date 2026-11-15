//! A document that lives on a Letters server: its copy on this device, and its line to the server.
//!
//! The copy is a `.let` file like any other, kept under `.letters/servers` in the person's home folder
//! and saved, journalled and merged as a file the person opened is. So the document opens at once, and
//! can be written in with no server in reach. The line trades what each side lacks while there is one:
//! `letters-sync` says what to send and what to merge, and this carries it.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use letters_api::{Code, Opened, paths};
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
    /// [`Line::link`] changed.
    Link,
    /// [`Line::progress`] changed.
    Progress,
    /// [`Line::peers`] changed.
    Peers,
}

/// What tells a line's window that its socket has something to say. It is called on another thread, with
/// the socket's number.
pub type Tell = Arc<dyn Fn(u64, Heard) + Send + Sync>;

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
}

impl Line {
    /// The line of the document `opened`, which does nothing until [`Line::start`]. `tell` is called
    /// when a socket has something to say, which is then handed to [`Line::heard`].
    pub fn new(session: Arc<Session>, opened: &Opened, roots: Roots, tell: Tell) -> Self {
        let address = session.server().socket(&opened.socket);
        let sync = Syncing::new(opened.id.clone());
        Self { session, address, roots, sync, socket: None, sockets: 0, tell, denied: false, ended: None }
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

    /// The socket of this number had something to say.
    pub fn heard(&mut self, socket: u64, heard: Heard, local: Local<'_>, now: u64) -> Vec<Told> {
        if socket != self.sockets {
            return Vec::new();
        }
        match heard {
            Heard::Open => {
                self.denied = false;
                self.sync.opened(local, now);
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
        self.sync.changed(local, now);
        self.drain()
    }

    /// The person's caret or selection moved.
    pub fn selected(&mut self, model: &Model, selection: Option<&Selection>, now: u64) -> Vec<Told> {
        self.sync.selected(model, selection, now);
        self.drain()
    }

    /// The time [`Line::wake_at`] named has come.
    pub fn tick(&mut self, now: u64) -> Vec<Told> {
        self.sync.tick(now);
        self.drain()
    }

    /// When to call [`Line::tick`] next. `None` when nothing is due.
    pub fn wake_at(&self) -> Option<u64> {
        self.sync.wake_at()
    }

    /// How the document stands to its server.
    pub fn link(&self) -> Link {
        self.ended.map_or_else(|| self.sync.link(), Link::Refused)
    }

    pub fn progress(&self) -> Progress {
        self.sync.progress()
    }

    /// Everyone else who has the document open.
    pub fn peers(&self) -> &[Peer] {
        self.sync.peers()
    }

    /// Does what the sync asks that is the line's own to do, and gives the rest.
    fn drain(&mut self) -> Vec<Told> {
        let mut told = Vec::new();
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
        let token = move || {
            let token = if denied { session.renewed() } else { session.token() };
            token.map_err(|refused| match refused.code() {
                Some(Code::SignedOut) => Closed::Refused(401),
                _ => Closed::Lost(refused.to_string()),
            })
        };
        let heard = move |heard| tell(number, heard);
        self.socket = Some(Socket::open(&self.address, token, &self.roots, MAX_FRAME, heard));
    }
}
