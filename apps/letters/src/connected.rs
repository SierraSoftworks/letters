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

use letters_api::{Access, Code, CreatedLink, EntryKind, Grant, NewLink, Opened, Shares, paths};
use letters_format::{Archive, OPLOG, WorkingCopy};
use letters_model::{Model, Notes, ReplicaId, Selection};
use letters_sync::{Client as Syncing, Event, Link, Local, MAX_FRAME, Peer, Progress, Refusal};
use sha2::{Digest, Sha256};
use ureq::http::Uri;

use crate::files;
use crate::keychain::Keychain;
use crate::remote::Roots;
use crate::server::{Client, Refused};
use crate::servers;
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
}

impl Unreached {
    /// Why a sign-in did not work, as the person is to read it. `None` if they gave it up themselves.
    pub fn sign_in(&self) -> Option<String> {
        match self {
            Self::Unsigned(Unsigned::GivenUp) => None,
            Self::Refused(Refused::Said(said)) | Self::Unsigned(Unsigned::Refused(Refused::Said(said))) => {
                Some(said.message.clone())
            }
            other => Some(format!("Signing in did not work: {other}.")),
        }
    }
}

impl fmt::Display for Unreached {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refused) => refused.fmt(f),
            Self::Unkept(error) => write!(f, "its copy could not be kept on this device{}", files::because(error)),
            Self::Unsigned(unsigned) => unsigned.fmt(f),
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
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(Unreached::Unkept)?;
    }
    if !path.exists() {
        let most = session.limits().map_err(Unreached::Refused)?.document;
        let file = session.file(id, most).map_err(Unreached::Refused)?;
        keep(&path, &file).map_err(Unreached::Unkept)?;
    }
    remember(&path, &opened);
    Ok(Reached { opened, path, online: true })
}

/// Writes a file beside its place and renames it there, so that a crash leaves a whole file or none.
fn keep(to: &Path, bytes: &[u8]) -> io::Result<()> {
    let beside = to.with_extension("new");
    fs::write(&beside, bytes).and_then(|()| fs::rename(&beside, to))
}

/// Keeps what the server said of the document whose copy is at `copy`, for the day the document is
/// opened with no server in reach.
fn remember(copy: &Path, opened: &Opened) {
    let _ = serde_json::to_vec(opened).map(|json| keep(&copy.with_extension("json"), &json));
}

/// A folder of the space a server keeps a document in, as a tree to move the document in lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shelf {
    /// Where it is in its space, as [`Move::folder`](letters_api::Move::folder) names it. Empty for the
    /// space itself.
    pub path: String,
    pub name: String,
    /// How far down the tree it is. The space is at 0.
    pub depth: u8,
}

/// The most folders of a space that are listed, each of which is asked for.
const SHELVES: usize = 200;
/// The last number a first heading's name is tried with, where others have the name.
const NUMBERED: u32 = 9;

/// What is asked of a server about what a document is called and where it is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placing {
    /// Only how it stands now: nothing tells a device that someone renamed or moved the document.
    Look,
    /// The name its first heading gives a document that has none of its own. It takes one once, with a
    /// number after a name that is taken, and a server that will not have it is not asked twice.
    Name(String),
    Rename(String),
    /// Another folder of `space`, by its [`Shelf::path`].
    Move {
        space: String,
        folder: String,
    },
    /// A folder to make in `parent`, for the document to move to.
    NewFolder {
        space: String,
        parent: String,
        name: String,
    },
}

/// How a document stands at its server once that was asked.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// What the server says of the document now, if it said.
    pub opened: Option<Opened>,
    /// The folders of the document's space in tree order, the space itself first, if the server said.
    /// A document that reaches the person only through a share has none.
    pub shelves: Option<Vec<Shelf>>,
    /// Why something that was asked was not done, as the person is to read it. What was asked after it
    /// was left alone.
    pub refused: Option<String>,
}

/// Asks the server of the document whose copy is at `copy` for each of `asks` in turn, up to the first
/// it refuses, and then how the document stands. This waits on the network.
pub fn place(session: &Session, id: &str, copy: &Path, asks: &[Placing]) -> Placed {
    let ask = |ask: &Placing| match ask {
        Placing::Look => Ok(()),
        // What its first heading asks for is nothing the person asked for, and is refused in silence.
        Placing::Name(name) => {
            let _ = name_once(session, id, name);
            Ok(())
        }
        Placing::Rename(name) => session.rename(id, name).map(drop),
        Placing::Move { space, folder } => session.relocate(id, space, folder).map(drop),
        Placing::NewFolder { space, parent, name } => session.new_folder(space, parent, name).map(drop),
    };
    let refused = asks.iter().try_for_each(ask).err().map(|refused| sentence(&refused));
    let opened = session.open(id).ok();
    if let Some(opened) = &opened {
        remember(copy, opened);
    }
    let space = opened.as_ref().and_then(|opened| opened.space.as_deref());
    Placed { shelves: space.and_then(|space| shelves(session, space).ok()), opened, refused }
}

/// Why a server did not do what the person asked, as they are to read it: the server's own sentence where
/// it gave one.
pub fn sentence(refused: &Refused) -> String {
    match refused {
        Refused::Said(said) => said.to_string(),
        other => format!("That could not be done: {other}."),
    }
}

/// What the Share dialog asks of a document's server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sharing {
    /// Who the document is shared with.
    Look,
    /// All its people from here on.
    People(Vec<Grant>),
    /// A link to make.
    Link(NewLink),
    /// A link to revoke, by its id.
    Revoke(String),
    /// Who reached the document through a share.
    Log,
    /// Who did before this time, which is that of the oldest row the dialog holds.
    Earlier(u64),
}

/// What a document's server answered the Share dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shared {
    /// Who the document is shared with now, and whether the server makes links.
    Shares(Shares, bool),
    /// The link that was asked for, with its address.
    Made(CreatedLink),
    Revoked,
    /// Who reached the document through a share, newest first, and how many days the server keeps such
    /// a row: 0 if it does not say.
    Log(Vec<Access>, u32),
    /// The page of that log that [`Sharing::Earlier`] asked for.
    Earlier(Vec<Access>),
    /// Why it was not done, as the person is to read it.
    Refused(String),
}

/// Asks a document's server what the Share dialog asked. This waits on the network.
pub fn share(session: &Session, id: &str, ask: &Sharing) -> Shared {
    let told = |shares| Shared::Shares(shares, session.links().unwrap_or(false));
    let answered = match ask {
        Sharing::Look => session.shares(id).map(told),
        Sharing::People(people) => session.share(id, people).map(told),
        Sharing::Link(new) => session.make_link(id, new).map(Shared::Made),
        Sharing::Revoke(link) => session.revoke_link(id, link).map(|()| Shared::Revoked),
        Sharing::Log => session.access_log(id, None).map(|log| Shared::Log(log, session.log_days().unwrap_or(0))),
        Sharing::Earlier(time) => session.access_log(id, Some(*time)).map(Shared::Earlier),
    };
    answered.unwrap_or_else(|refused| {
        Shared::Refused(match refused {
            // No word of why: the server is one from before documents were shared.
            Refused::Status(404 | 405) => "This server does not share documents yet.".to_owned(),
            refused => sentence(&refused),
        })
    })
}

/// Gives a document the name of its first heading, unless it has a name of its own by now: another
/// device may have named it since this one last asked.
fn name_once(session: &Session, id: &str, name: &str) -> Result<(), Refused> {
    if session.open(id)?.named {
        return Ok(());
    }
    let mut tried = session.rename(id, name);
    for number in 2..=NUMBERED {
        if tried.as_ref().err().and_then(Refused::code) != Some(Code::NameTaken) {
            break;
        }
        tried = session.rename(id, &format!("{name} {number}"));
    }
    tried.map(drop)
}

/// The folders of a space in tree order, the space itself first.
fn shelves(session: &Session, space: &str) -> Result<Vec<Shelf>, Refused> {
    let inside = |folder: &str, depth: u8| {
        let listing = session.entries(space, folder)?;
        let folders = listing.entries.into_iter().filter(|entry| entry.kind == EntryKind::Folder);
        // Each is taken from the end, so the first of a folder is the next listed.
        let mut folders: Vec<_> = folders.map(|entry| (entry, depth)).collect();
        folders.reverse();
        Ok((listing.space.name, folders))
    };
    let (name, mut left) = inside("", 1)?;
    let mut out = vec![Shelf { path: String::new(), name, depth: 0 }];
    while let Some((folder, depth)) = left.pop().filter(|_| out.len() < SHELVES) {
        if let Some(deeper) = depth.checked_add(1) {
            left.extend(inside(&folder.path, deeper)?.1);
        }
        out.push(Shelf { path: folder.path, name: folder.name, depth });
    }
    Ok(out)
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
/// else a new one from a [`sign_in`]. All of this waits on the network, and on the person.
#[allow(clippy::too_many_arguments)]
pub fn enter(
    server: &Client,
    id: &str,
    held: Option<Arc<Session>>,
    keychain: Option<Keychain>,
    home: &Path,
    browse: &dyn Fn(&str) -> io::Result<()>,
    waiting: &dyn Fn(&str, &str),
    given_up: &AtomicBool,
) -> Result<(Arc<Session>, Reached), Unreached> {
    let kept = || Session::kept(server.clone(), keychain?).ok().flatten().map(Arc::new);
    if let Some(session) = held.or_else(kept).filter(|session| !session.ended()) {
        match reach(&session, id, home) {
            // A session the server has ended is followed by a new sign-in.
            Err(Unreached::Refused(refused)) if refused.code() == Some(Code::SignedOut) => {}
            reached => {
                // One the keychain kept since before servers were listed is listed from here on.
                if reached.is_ok() {
                    list(&session, home);
                }
                return reached.map(|reached| (session, reached));
            }
        }
    }
    let session = sign_in(server, keychain, home, browse, waiting, given_up)?;
    reach(&session, id, home).map(|reached| (session, reached))
}

/// Signs the person in to `server` at the system's browser, which `browse` is handed the sign-in page of.
///
/// `waiting` hears the server's name and the page's address as the browser is sent there. A
/// browser that did not open is no end: the person can open the page themselves. The wait ends when
/// `given_up` is set. The server is listed in `home` if the keychain keeps the sign-in. This waits on the
/// network, and on the person.
pub fn sign_in(
    server: &Client,
    keychain: Option<Keychain>,
    home: &Path,
    browse: &dyn Fn(&str) -> io::Result<()>,
    waiting: &dyn Fn(&str, &str),
    given_up: &AtomicBool,
) -> Result<Arc<Session>, Unreached> {
    // Nobody is sent to sign in at what is no Letters server.
    let said = server.discover().map_err(Unreached::Refused)?;
    let sign_in = SignIn::begin().map_err(Unreached::Unsigned)?;
    let page = sign_in.page(server).map_err(Unreached::Unsigned)?;
    waiting(&said.name, &page);
    let _ = browse(&page);
    let token = sign_in.finish(server, SIGN_IN_WITHIN, given_up).map_err(Unreached::Unsigned)?;
    let session = Arc::new(Session::begin(server.clone(), keychain, token));
    list(&session, home);
    Ok(session)
}

/// Lists the session's server among those the person is signed in to, if the sign-in outlasts the app.
fn list(session: &Session, home: &Path) {
    if session.unkept().is_none() {
        // A list that cannot be written costs a row in a dialog, and no sign-in.
        let _ = servers::remember(home, session.server().origin());
    }
}

/// What is known of a document whose server never said anything this device kept: no more than its id.
/// The server says the rest once it is back.
fn unsaid(id: &str) -> Opened {
    Opened {
        id: id.to_owned(),
        created: String::new(),
        name: id.to_owned(),
        named: true,
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
    /// The server did not take the copy's file, and said why in this sentence, which is the person's to
    /// read. The work stays in the copy here.
    Unsent(String),
    /// The server did not take the picture of this name, so the others will not see it.
    Unkept(String),
    /// What was written here fits no frame, so the server takes the copy's file whole. Save the copy, and
    /// call [`Line::saved`] once its file holds all of it.
    Whole,
    /// [`Line::link`] changed.
    Link,
    /// [`Line::progress`] changed.
    Progress,
    /// [`Line::peers`] changed.
    Peers,
}

/// What a line's window hands back to [`Line::heard`].
#[derive(Clone, Debug, PartialEq)]
pub enum Said {
    /// The socket of this number had something to say.
    Socket(u64, Heard),
    /// The pictures the server was owed were handed over, or could not be for now.
    Handed,
    /// The copy's file was sent to the server whole, or why it was not.
    Sent(Option<Refused>),
    /// The document's file was brought from the server whole: its history and its notes. `None` when it
    /// could not be had.
    Brought(Option<(Vec<u8>, Notes)>),
}

/// A document's file on its way whole, when what one side has for the other fits no frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Whole {
    No,
    /// The server is to be sent the copy's file, once it holds what was written here.
    Wanted,
    Sending,
    /// The server's file is on its way here.
    Bringing,
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

    /// Sends the server the copy's file whole. This waits on the network.
    fn send(&self) -> Result<(), Refused> {
        let file = fs::read(&self.copy).map_err(|error| Refused::Unreachable(error.to_string()))?;
        self.session.keep_file(&self.id, &file)
    }

    /// The history and the notes of the document as the server's file holds it, to merge here. This
    /// waits on the network.
    fn bring(&self) -> Option<(Vec<u8>, Notes)> {
        mergeable(self.session.file(&self.id, self.session.limits().ok()?.document).ok()?)
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

/// The history and the notes of a `.let` file that came from a server, for the window that has the
/// document open to merge. `None` if there is nothing to merge without harm.
///
/// Others wrote the file, so its history is tried in a process of its own, where it can hurt nothing,
/// before any document takes it in. A file with no history, or with none that stands, is not merged: a history
/// started here for it would bring every block a second time.
pub fn mergeable(file: Vec<u8>) -> Option<(Vec<u8>, Notes)> {
    let mut file = Archive::open(io::Cursor::new(file)).ok()?;
    crate::strangers::check(&file.read(OPLOG).ok()??).ok()?;
    // Nothing is written with it, so whose copy it is does not matter.
    let copy = WorkingCopy::open(&mut file, ReplicaId::new(&crate::platform::block_id())).ok()?;
    Some((copy.model().save(), copy.notes))
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
    whole: Whole,
    /// When to try again to send the copy's file, after the server could not be reached.
    send_at: Option<u64>,
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
            whole: Whole::No,
            send_at: None,
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
            Said::Sent(None) => {
                // The server has it all now, which a new handshake tells the client.
                self.whole = Whole::No;
                self.sync.stop();
                self.sync.start(now);
                return self.drain();
            }
            Said::Sent(Some(refused)) if passes(&refused) => {
                (self.whole, self.send_at) = (Whole::Wanted, Some(now + HAND_AGAIN));
                return Vec::new();
            }
            Said::Sent(Some(refused)) => {
                self.whole = Whole::No;
                // Too large is the line's own word. Whatever else the server found, it says for itself.
                return vec![match refused.code() {
                    Some(Code::TooLarge) | None => Told::Refused(Refusal::TooLarge),
                    Some(_) => Told::Unsent(sentence(&refused)),
                }];
            }
            Said::Brought(brought) => {
                self.whole = Whole::No;
                let Some((edits, notes)) = brought else { return vec![Told::Link] };
                // With the file merged, a handshake has little left to bring.
                self.sync.start(now);
                return [vec![Told::Merge(edits), Told::Notes(notes), Told::Link], self.drain()].concat();
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

    /// The copy's file holds all that was written here, as after a save. Whatever waited for that goes.
    pub fn saved(&mut self) {
        if self.whole == Whole::Wanted {
            self.send();
        }
    }

    /// Sends the server the copy's file whole, on a thread of its own.
    fn send(&mut self) {
        (self.whole, self.send_at) = (Whole::Sending, None);
        let (handing, tell) = (self.handing.clone(), self.tell.clone());
        thread::spawn(move || tell(Said::Sent(handing.send().err())));
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
        // The file held the work when the server could not be reached, and holds it still.
        if self.whole == Whole::Wanted && self.send_at.is_some_and(|at| at <= now) {
            self.send();
        }
        self.drain()
    }

    /// When to call [`Line::tick`] next. `None` when nothing is due.
    pub fn wake_at(&self) -> Option<u64> {
        [self.sync.wake_at(), self.hand_at, self.send_at].into_iter().flatten().min()
    }

    /// How the document stands to its server.
    pub fn link(&self) -> Link {
        match self.ended {
            Some(refusal) => Link::Refused(refusal),
            // The file is on its way, and the line starts again when it is here.
            None if self.whole == Whole::Bringing => Link::Connecting,
            None => self.sync.link(),
        }
    }

    /// Where the work written here is. Edits that wait for their pictures are on their way, and so is
    /// work that travels as the file.
    pub fn progress(&self) -> Progress {
        match self.sync.progress() {
            _ if matches!(self.whole, Whole::Wanted | Whole::Sending) => Progress::Saving,
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
                // No frame carries what was written here, so the file goes whole.
                Event::Refused(Refusal::TooLarge) => {
                    if self.whole != Whole::Sending {
                        self.whole = Whole::Wanted;
                        told.push(Told::Whole);
                    }
                }
                Event::Refused(refusal) => told.push(Told::Refused(refusal)),
                Event::Link => {
                    // No frame carries what the server has for this device, so its file comes whole.
                    if self.sync.link() == Link::Refused(Refusal::TooLarge) && self.whole == Whole::No {
                        self.whole = Whole::Bringing;
                        let (handing, tell) = (self.handing.clone(), self.tell.clone());
                        thread::spawn(move || tell(Said::Brought(handing.bring())));
                    }
                    told.push(Told::Link);
                }
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
