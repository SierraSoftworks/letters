//! Milestone 3's exit test: two desktops write one document through the server, over real sockets and on
//! the server's own clock, while a third writer saves the same file on the share, and nothing any of them
//! wrote is lost. Then the failures the design lists, each brought about as far as a test can.
//!
//! The server is `letters_server::serve` in a process of its own: this test binary run again, as
//! [`the_server_as_a_program`]. So a test stops a server as a service manager does, kills it as a power
//! cut does, and starts another over the same folders. People are signed in without an identity provider,
//! which `signin.rs` covers.

mod common;

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Cursor, ErrorKind, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::Scratch;
use letters_api::{Entry, Listing, NewDocument, Person, Tag, paths};
use letters_format::{Archive, Files, Journal, Save, Saved, Stamp, Unsaved, WorkingCopy, is_journal};
use letters_model::{Block, BlockKind, Document, Model, NoteKind, Notes, Op, Pos, ReplicaId};
use letters_server::config::Config;
use letters_server::session::Carrier;
use letters_server::store::Store;
use letters_server::{App, ids};
use letters_sync::{Client as Line, ClientMessage, Event, Hello, Link, Local, PROTOCOL, Role, ServerMessage};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::header::AUTHORIZATION;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Error as SocketError, Message, WebSocket};

/// Set for the process that is the server: the folders it serves, and the name of its state folder.
const SERVED: &str = "LETTERS_EXIT_SERVED";
const STATE: &str = "LETTERS_EXIT_STATE";
/// What the server's process starts a line with that is meant for the test.
const SAYS: &str = "the server says: ";
/// Who is signed in on every server, each in the family's group.
const CAST: [&str; 2] = ["ada", "ben"];
/// The document every test opens, and its file in the family's folder.
const FILE: &str = "Plan.let";
/// The longest a test waits for anything. The server's own timers are 2 s to a save, 10 s to a look at
/// the file and 30 s between a device's tries, so nothing that works takes this long.
const PATIENCE: Duration = Duration::from_secs(90);

/// Not a test: the server as a program. It says where it listens and what its people sign in with, serves
/// until its input says a line or closes, and stops as `letters-server` does on a signal.
#[test]
fn the_server_as_a_program() {
    let Ok(scratch) = std::env::var(SERVED) else { return };
    let scratch = PathBuf::from(scratch);
    let settings = common::settings(&scratch, "https://id.example.test", "");
    let mut config = Config::parse(&settings).expect("the settings are sound");
    config.server.state = scratch.join(std::env::var(STATE).expect("a state folder is named"));
    let store = Store::open(&config.server.state).expect("the state folder opens");
    let app = Arc::new(App::new(config, store));
    let say = |line: String| {
        println!("{SAYS}{line}");
        io::stdout().flush().expect("the test listens");
    };
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a runtime starts");
    let served = runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        say(format!("at {}", listener.local_addr()?));
        for name in CAST {
            let person = Person { id: format!("sub-{name}"), username: name.into(), name: name.into() };
            let token = app.sign_in(person, vec!["family".into()], Carrier::Bearer).expect("a session starts");
            say(format!("token {name} {}", token.token));
        }
        say("ready".into());
        let asked = tokio::task::spawn_blocking(|| io::stdin().read_line(&mut String::new()));
        letters_server::serve(app, listener, async move { drop(asked.await) }).await
    });
    // At once: nothing waits for the thread that reads the input.
    std::process::exit(if served.is_ok() { 0 } else { 1 });
}

/// A server that runs as a program of its own.
struct Served {
    child: Child,
    input: Option<ChildStdin>,
    at: SocketAddr,
    tokens: BTreeMap<String, String>,
    /// The lines of its log so far.
    log: Arc<Mutex<Vec<String>>>,
}

impl Served {
    /// Starts a server over the folders in `scratch` that keeps its state in the folder `state` there.
    /// It has taken its journals to their files before this returns.
    fn start(scratch: &Path, state: &str) -> Self {
        let mut program = Command::new(std::env::current_exe().expect("the test binary has a path"));
        program.args(["--exact", "the_server_as_a_program", "--nocapture", "--test-threads=1"]);
        program.env(SERVED, scratch).env(STATE, state);
        program.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = program.spawn().expect("the test binary runs again");
        let (said, hears) = mpsc::channel();
        let out = BufReader::new(child.stdout.take().expect("its output is piped"));
        thread::spawn(move || {
            let lines = out.lines().map_while(Result::ok);
            lines.filter_map(|line| Some(line.split_once(SAYS)?.1.to_owned())).for_each(|line| drop(said.send(line)));
        });
        // The server's log is shown with a test that fails, and kept for one that reads it.
        let (log, lines) = (Arc::new(Mutex::new(Vec::new())), BufReader::new(child.stderr.take().expect("its log")));
        let kept = log.clone();
        thread::spawn(move || {
            for line in lines.lines().map_while(Result::ok) {
                eprintln!("{line}");
                kept.lock().expect("the log is kept").push(line);
            }
        });

        let (input, at) = (child.stdin.take(), ([0, 0, 0, 0], 0).into());
        let mut served = Self { input, child, at, tokens: BTreeMap::new(), log };
        loop {
            let line: String = hears.recv_timeout(PATIENCE).expect("the server says when it is ready");
            let words: Vec<&str> = line.split(' ').collect();
            match words[..] {
                ["at", at] => served.at = at.parse().expect("an address"),
                ["token", who, token] => drop(served.tokens.insert(who.into(), token.into())),
                ["ready"] => return served,
                _ => panic!("the server said {line:?}"),
            }
        }
    }

    /// Asks the server to stop, as a service manager does, and waits until it has.
    fn stop(mut self) {
        drop(self.input.take());
        let ended = self.ended(PATIENCE).expect("a server that is asked to stop does");
        assert!(ended.success(), "the server stopped with {ended}");
    }

    /// Ends the server where it stands, as a power cut does.
    fn kill(mut self) {
        self.child.kill().expect("the server can be killed");
        self.ended(PATIENCE).expect("a server that was killed ends");
    }

    /// How the server's process ended, once it has. `None` while it still runs after `within`.
    fn ended(&mut self, within: Duration) -> Option<ExitStatus> {
        let until = Instant::now() + within;
        loop {
            match self.child.try_wait().expect("the server's process can be asked") {
                None if Instant::now() < until => thread::sleep(Duration::from_millis(10)),
                ended => return ended,
            }
        }
    }

    /// Asks the server's interface as the person `token` signs in: the status, and the answer's text.
    fn ask(&self, token: &str, path: &str, body: Option<String>) -> (u16, String) {
        let agent: ureq::Agent =
            ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(PATIENCE)).build().into();
        let (url, bearer) = (format!("http://{}{path}", self.at), format!("Bearer {token}"));
        let answer = match body {
            Some(body) => {
                agent.post(&url).header("Authorization", &bearer).header("Content-Type", "application/json").send(body)
            }
            None => agent.get(&url).header("Authorization", &bearer).call(),
        };
        let mut answer = answer.expect("the server answers");
        (answer.status().as_u16(), answer.body_mut().read_to_string().expect("the answer is text"))
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("the clock is set").as_millis() as u64
}

/// Asks `at` for the socket of the document `id`. `Err` with the status of a refusal, or 0 when nobody
/// answered.
fn dial(at: SocketAddr, id: &str, token: &str) -> Result<Socket, u16> {
    let mut request = format!("ws://{at}{}", paths::socket(id)).into_client_request().expect("an address");
    request.headers_mut().insert(AUTHORIZATION, format!("Bearer {token}").parse().expect("a header"));
    match tungstenite::connect(request) {
        Ok((socket, _)) => {
            if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
                // A read that finds nothing comes back, so that one thread keeps several desktops going.
                stream.set_read_timeout(Some(Duration::from_millis(5))).expect("a socket takes a timeout");
                stream.set_nodelay(true).expect("a socket sends at once");
            }
            Ok(socket)
        }
        Err(SocketError::Http(refused)) => Err(refused.status().as_u16()),
        Err(_) => Err(0),
    }
}

/// The desktop app with a server's document open: the line `letters-sync` gives a shell, a real socket,
/// and the copy the device holds.
struct Desktop {
    line: Line,
    model: Option<Model>,
    notes: Notes,
    socket: Option<Socket>,
    /// Where its server is, the document it has open and what it signs in with.
    at: SocketAddr,
    id: String,
    token: String,
    /// Whether its network reaches the server.
    plugged: bool,
    /// The highest number the server said it holds what was written up to.
    held: u64,
    /// The status its last try for a socket was refused with.
    turned_away: Option<u16>,
}

impl Desktop {
    /// The person `who` opens the document `id` on a device that holds no copy of it.
    fn open(served: &Served, who: &str, id: &str) -> Self {
        let mut line = Line::new("");
        line.start(now());
        Self {
            line,
            model: None,
            notes: Notes::default(),
            socket: None,
            at: served.at,
            id: id.into(),
            token: served.tokens[who].clone(),
            plugged: true,
            held: 0,
            turned_away: None,
        }
    }

    fn model(&self) -> &Model {
        self.model.as_ref().expect("the desktop holds a copy")
    }

    fn live(&self) -> bool {
        self.line.link() == Link::Live(Role::Editor)
    }

    fn texts(&self) -> Vec<String> {
        texts(self.model().doc())
    }

    fn said(&self) -> String {
        said(self.model().doc(), &self.notes)
    }

    /// The person edits. Returns the number the line gave for what the desktop holds now.
    fn edit(&mut self, ops: impl IntoIterator<Item = Op>) -> u64 {
        let model = self.model.as_mut().expect("the desktop holds a copy");
        model.apply(ops).expect("the edit fits the document");
        self.notes.follow(model);
        self.line.changed(Local { model: self.model.as_ref(), notes: &self.notes }, now())
    }

    /// The person types `text` at the start of the block `block`.
    fn write(&mut self, block: &str, text: &str) -> u64 {
        self.edit([Op::InsertText { at: Pos::new(block, 0), text: text.into() }])
    }

    /// The person types `text` at the start of the document.
    fn begin(&mut self, text: &str) -> u64 {
        let first = self.model().doc().blocks[0].id.clone();
        self.edit([Op::InsertText { at: Pos::new(first, 0), text: text.into() }])
    }

    /// The network goes: the socket dies without a word, and no other is to be had.
    fn unplug(&mut self) {
        self.plugged = false;
        if let Some(socket) = self.socket.take()
            && let MaybeTlsStream::Plain(stream) = socket.get_ref()
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
        self.line.closed(now());
    }

    /// The person signs in again, and the line starts over with what they were given.
    fn sign_in(&mut self, served: &Served, who: &str) {
        (self.at, self.token, self.turned_away) = (served.at, served.tokens[who].clone(), None);
        self.line.stop();
        self.line.start(now());
    }

    /// Does what the line asks, hears what the socket holds and keeps the line's time. Says whether there
    /// was anything.
    fn pump(&mut self) -> bool {
        let mut busy = false;
        if self.line.wake_at().is_some_and(|at| at <= now()) {
            self.line.tick(now());
        }
        loop {
            // The line is done with before the socket is read again: it may have closed it.
            while let Some(event) = self.line.poll() {
                busy = true;
                self.handle(event);
            }
            let Some(socket) = &mut self.socket else { return busy };
            match socket.read() {
                Ok(Message::Binary(frame)) => {
                    self.line.received(&frame, Local { model: self.model.as_ref(), notes: &self.notes }, now());
                }
                Ok(Message::Close(_)) => self.lost(),
                Ok(_) => {}
                Err(SocketError::Io(error)) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    return busy;
                }
                // The server went without a word.
                Err(_) => self.lost(),
            }
            busy = true;
        }
    }

    /// The socket ended from the other side.
    fn lost(&mut self) {
        self.socket = None;
        self.line.closed(now());
    }

    fn handle(&mut self, event: Event) {
        match event {
            // An unplugged device reaches nobody.
            Event::Open => match if self.plugged { dial(self.at, &self.id, &self.token) } else { Err(0) } {
                Ok(socket) => {
                    (self.socket, self.turned_away) = (Some(socket), None);
                    self.line.opened(Local { model: self.model.as_ref(), notes: &self.notes }, now());
                }
                Err(status) => {
                    self.turned_away = (status > 0).then_some(status);
                    self.line.closed(now());
                }
            },
            Event::Send(frame) => {
                let sent = self.socket.as_mut().map(|socket| socket.send(Message::Binary(frame.into())));
                if matches!(sent, Some(Err(_))) {
                    self.lost();
                }
            }
            // Nothing more is told of a socket the line closed.
            Event::Close => {
                if let Some(mut socket) = self.socket.take() {
                    let _ = socket.close(None);
                    let _ = socket.flush();
                }
            }
            Event::Merge(updates) => {
                match &mut self.model {
                    Some(model) => drop(model.merge(&updates).expect("what the server sends merges")),
                    None => {
                        let replica = ReplicaId::new(&ids::block_id());
                        self.model = Some(Model::load(&updates, replica).expect("what the server sends loads"));
                    }
                }
                if let Some(model) = &self.model {
                    self.notes.follow(model);
                }
                self.line.changed(Local { model: self.model.as_ref(), notes: &self.notes }, now());
            }
            Event::Notes(notes) => {
                self.notes.merge(&notes);
                if let Some(model) = &self.model {
                    self.notes.follow(model);
                }
                self.line.changed(Local { model: self.model.as_ref(), notes: &self.notes }, now());
            }
            Event::Held(number) => self.held = self.held.max(number),
            Event::Refused(_) | Event::Link | Event::Progress | Event::Peers => {}
        }
    }
}

/// A folder on the share, as a program that has it mounted reaches it.
struct Share(PathBuf);

impl Files for Share {
    fn stamp(&mut self, name: &str) -> io::Result<Option<Stamp>> {
        let found = match std::fs::metadata(self.0.join(name)) {
            Ok(found) => found,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let modified = found.modified()?.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos() as u64);
        Ok(Some(Stamp { len: found.len(), modified }))
    }

    fn read(&mut self, name: &str) -> io::Result<Option<Vec<u8>>> {
        match std::fs::read(self.0.join(name)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let mut file = std::fs::File::create(self.0.join(name))?;
        file.write_all(bytes)?;
        file.sync_all()
    }

    fn rename(&mut self, from: &str, to: &str) -> io::Result<()> {
        std::fs::rename(self.0.join(from), self.0.join(to))
    }

    fn remove(&mut self, name: &str) -> io::Result<()> {
        std::fs::remove_file(self.0.join(name))
    }

    fn wait(&mut self, retry: u32) {
        thread::sleep(Duration::from_millis(50) * retry);
    }
}

/// The third writer: the document's file open from the share, with no server in between.
struct Outsider {
    share: Share,
    copy: WorkingCopy,
    saves: u32,
}

impl Outsider {
    fn open(folder: &Path) -> Self {
        let bytes = std::fs::read(folder.join(FILE)).expect("the file is on the share");
        let mut file = Archive::open(Cursor::new(bytes)).expect("the file is a document");
        let copy = WorkingCopy::open(&mut file, ReplicaId::new(&ids::block_id())).expect("the file opens");
        Self { share: Share(folder.into()), copy, saves: 0 }
    }

    fn write(&mut self, block: &str, text: &str) {
        self.copy.apply([Op::InsertText { at: Pos::new(block, 0), text: text.into() }]).expect("the edit fits");
    }

    /// Saves as every writer of a shared folder does: what the file holds is merged first.
    fn save(&mut self) -> Saved {
        self.saves += 1;
        let token = format!("outside{}", self.saves);
        let save = Save { name: FILE, token: &token, label: "the share", attempts: 3 };
        letters_format::save(&mut self.share, &mut self.copy, &save).expect("the share takes a file")
    }
}

fn texts(document: &Document) -> Vec<String> {
    document.walk().map(Block::plain_text).collect()
}

/// Everything a copy of the document says: its text, and what its conflict notes keep of a passage that
/// two wrote apart.
fn said(document: &Document, notes: &Notes) -> String {
    let versions = notes.notes.iter().filter_map(|note| match &note.kind {
        NoteKind::Conflict { versions } => Some(versions.iter()),
        _ => None,
    });
    let mut kept: Vec<String> =
        versions.flatten().filter_map(|version| version.block.as_ref()).map(Block::plain_text).collect();
    // Two copies hold the same notes in whatever order they came.
    kept.sort();
    kept.dedup();
    texts(document).into_iter().chain(kept).collect::<Vec<String>>().join("\n")
}

/// The document a file on the share holds, and its notes. `None` for a file that does not read.
fn on_file(file: &Path) -> Option<(Document, Notes)> {
    let mut file = Archive::open(Cursor::new(std::fs::read(file).ok()?)).ok()?;
    Some((file.document().ok()?, file.notes().ok()?))
}

/// The text of the file on the share, or nothing while it does not read.
fn file_says(file: &Path) -> String {
    on_file(file).map(|(document, notes)| said(&document, &notes)).unwrap_or_default()
}

/// Keeps the desktops going until `done` says so. Fails with `what` when that does not come about.
fn until(what: &str, desks: &mut [&mut Desktop], mut done: impl FnMut(&[&mut Desktop]) -> bool) {
    let (ends, mut looked) = (Instant::now() + PATIENCE, Instant::now());
    loop {
        let busy = desks.iter_mut().fold(false, |busy, desk| desk.pump() | busy);
        if !busy && looked <= Instant::now() {
            if done(desks) {
                return;
            }
            looked = Instant::now() + Duration::from_millis(50);
        }
        if Instant::now() > ends {
            let links: Vec<String> = desks.iter().map(|desk| format!("{:?}", desk.line.link())).collect();
            panic!("{what}: that did not come about within {PATIENCE:?}, with the lines {links:?}");
        }
        if !busy {
            thread::sleep(Duration::from_millis(5));
        }
    }
}

/// Keeps the desktops going for a moment, as when nothing in particular is waited for.
fn a_moment(desks: &mut [&mut Desktop]) {
    let ends = Instant::now() + Duration::from_millis(300);
    until("a moment passes", desks, |_| Instant::now() > ends);
}

/// Fresh folders, a server over them, and the id of the document "Plan" in the family's space.
fn planned(name: &str) -> (Scratch, Served, String) {
    let scratch = Scratch::new(&format!("exit-{name}"));
    std::fs::create_dir_all(scratch.0.join("family")).expect("the family's folder is made");
    let served = Served::start(&scratch.0, "state");
    let new = NewDocument { folder: String::new(), name: Some("Plan".into()) };
    let body = serde_json::to_string(&new).expect("a request is written");
    let (status, made) = served.ask(&served.tokens["ada"], &paths::documents("family"), Some(body));
    assert_eq!(status, 200, "the document is made: {made}");
    let made: Entry = serde_json::from_str(&made).expect("the server answers with the document's row");
    (scratch, served, made.id.expect("a document has an id"))
}

/// Ada and Ben with the document open on a desktop each, both in session.
fn desktops(served: &Served, id: &str) -> (Desktop, Desktop) {
    let (mut ada, mut ben) = (Desktop::open(served, "ada", id), Desktop::open(served, "ben", id));
    until("both are in session", &mut [&mut ada, &mut ben], |desks| desks.iter().all(|desk| desk.live()));
    (ada, ben)
}

/// Gives the document a paragraph for each writer and one they all share, each named for who writes it.
fn lay_out(desk: &mut Desktop) {
    let blocks = desk.model().doc().blocks.len();
    let paragraph = |(nth, id): (usize, &str)| Op::InsertBlock {
        parent: None,
        index: blocks + nth,
        block: Block::text(id, BlockKind::Paragraph, ""),
    };
    desk.edit(["ada", "ben", "share", "all"].into_iter().enumerate().map(paragraph));
}

fn agree(desks: &[&mut Desktop]) -> bool {
    desks.iter().all(|desk| desk.live() && desk.model.is_some() && desk.said() == desks[0].said())
}

#[test]
fn two_desktops_and_a_writer_on_the_share_lose_nothing() {
    let (scratch, served, id) = planned("together");
    let (family, file) = (scratch.0.join("family"), scratch.0.join("family").join(FILE));
    let (mut ada, mut ben) = desktops(&served, &id);
    lay_out(&mut ada);
    // The file has the paragraphs once the edits pause, and the third writer opens it from the share.
    until("the file holds the paragraphs", &mut [&mut ada, &mut ben], |_| {
        on_file(&file).is_some_and(|(document, _)| document.block(&"all".into()).is_some())
    });
    let mut cy = Outsider::open(&family);

    let (mut written, mut last) = (Vec::new(), (0, 0));
    for round in 1..=3 {
        let word = |who: &str| format!("{who}{round}");
        // Each in a paragraph of their own, and all three in one, at the same place.
        ada.write("ada", &format!("{} ", word("ada")));
        ben.write("ben", &format!("{} ", word("ben")));
        last =
            (ada.write("all", &format!("{} ", word("with-ada"))), ben.write("all", &format!("{} ", word("with-ben"))));
        a_moment(&mut [&mut ada, &mut ben]);
        cy.write("share", &format!("{} ", word("cy")));
        cy.write("all", &format!("{} ", word("with-cy")));
        assert!(matches!(cy.save(), Saved::Written), "round {round}");
        written.extend(["ada", "ben", "cy", "with-ada", "with-ben", "with-cy"].map(&word));
        // The server takes the file in at its next save, and the desktops hear of it at once.
        until("the desktops have what was written on the share", &mut [&mut ada, &mut ben], |desks| {
            desks.iter().all(|desk| desk.said().contains(&word("cy")) && desk.said().contains(&word("with-cy")))
        });
    }

    // Once all is quiet the two desktops, the file and the third writer hold one document.
    until("everyone holds the same document", &mut [&mut ada, &mut ben], |desks| {
        agree(desks) && matches!(cy.save(), Saved::Unchanged) && file_says(&file) == desks[0].said()
    });
    assert_eq!(texts(cy.copy.model().doc()), ada.texts());
    assert_eq!(ada.texts(), ben.texts());
    for word in &written {
        assert!(ada.said().contains(word.as_str()), "{word} is lost: {}", ada.said());
    }
    assert!(ada.held >= last.0 && ben.held >= last.1, "the server said it holds all that was written");
    assert_eq!(scratch.names("family"), [FILE], "nothing was set aside or copied");
}

/// The design's row "A sync tool creates a conflict copy": it is known by its name and the document it
/// holds, merged when an editor asks and not before, and then waits in the trash.
#[test]
fn a_sync_tools_conflict_copy_is_merged_when_an_editor_asks() {
    let (scratch, served, id) = planned("conflict-copy");
    let (family, file) = (scratch.0.join("family"), scratch.0.join("family").join(FILE));
    let (mut ada, mut ben) = desktops(&served, &id);
    ada.begin("First. ");
    until("the file holds what both machines start from", &mut [&mut ada, &mut ben], |_| {
        file_says(&file).contains("First.")
    });

    // A sync tool carries the folder to another machine. Someone writes there with no network while Ben
    // writes here, so the tool keeps both files, the other machine's under a name of its own.
    let away = scratch.0.join("away");
    std::fs::create_dir_all(&away).unwrap();
    std::fs::copy(&file, away.join(FILE)).unwrap();
    let mut cy = Outsider::open(&away);
    let first = cy.copy.model().doc().blocks[0].id.clone();
    cy.write(first.as_str(), "Written away. ");
    assert!(matches!(cy.save(), Saved::Written));
    ben.begin("Written here. ");
    let copy = "Plan.sync-conflict-20261115-101500-ABCDEFG.let";
    std::fs::rename(away.join(FILE), family.join(copy)).unwrap();

    // The copy is shown as one. A save goes by, and nothing of it is merged.
    let (status, listed) = served.ask(&served.tokens["ada"], &paths::entries("family", ""), None);
    assert_eq!(status, 200, "{listed}");
    let listed: Listing = serde_json::from_str(&listed).expect("a listing");
    let copies = |entry: &&Entry| entry.tags.iter().any(|tag| matches!(tag, Tag::ConflictCopy { .. }));
    let shown: Vec<&str> = listed.entries.iter().filter(copies).map(|entry| entry.name.as_str()).collect();
    assert!(matches!(shown[..], [name] if name != "Plan"), "one copy is marked, and not the document: {shown:?}");
    ada.begin("And more. ");
    until("the file holds what was written since", &mut [&mut ada, &mut ben], |_| {
        file_says(&file).contains("And more.") && file_says(&file).contains("Written here.")
    });
    assert!(!file_says(&file).contains("Written away.") && !ada.said().contains("Written away."));

    // An editor asks: everyone with the document open has both, and so has the file.
    let merge = || {
        let (status, took) = served.ask(&served.tokens["ada"], &paths::merge_conflicts(&id), Some("null".into()));
        assert_eq!(status, 200, "{took}");
        serde_json::from_str::<Vec<String>>(&took).expect("the names of the copies it took")
    };
    assert_eq!(merge().len(), 1);
    until("everyone holds what was written on both machines", &mut [&mut ada, &mut ben], |desks| {
        agree(desks) && desks[0].said().contains("Written away.") && file_says(&file) == desks[0].said()
    });
    for word in ["First.", "Written away.", "Written here.", "And more."] {
        assert!(ben.said().contains(word), "{word} is lost: {}", ben.said());
    }
    assert_eq!(scratch.names("family"), [".letters-trash", FILE], "the copy is out of the folder");
    assert_eq!(scratch.names("family/.letters-trash"), [copy], "and can be put back");
    assert!(merge().is_empty(), "with no copy left there is nothing to merge");
}

/// The design's row "Crash or power loss mid-save": the journal replays what was not saved.
#[test]
fn a_server_that_is_killed_brings_what_it_held_to_the_file_when_it_is_back() {
    let (scratch, served, id) = planned("killed");
    let file = scratch.0.join("family").join(FILE);
    let (mut ada, mut ben) = desktops(&served, &id);
    let number = ada.begin("Kept. ");
    until("the server says it holds the edit", &mut [&mut ada, &mut ben], |desks| desks[0].held >= number);
    // The power goes before the edits have paused for long enough to be saved, and a save is caught
    // half done: its temporary file lies beside the document.
    served.kill();
    std::fs::write(scratch.0.join("family").join(".Plan.let.cut.tmp"), b"PK\x03\x04 half a file").unwrap();

    // Nobody opens the document. The server that starts next takes its journal to the file by itself.
    let served = Served::start(&scratch.0, "state");
    until("the file holds what the server had said it holds", &mut [], |_| file_says(&file).contains("Kept."));

    // The devices find the server again, with the sign-in they had.
    (ada.at, ben.at) = (served.at, served.at);
    until("both are back and agree", &mut [&mut ada, &mut ben], agree);
    assert!(ben.said().contains("Kept."));
}

/// A journal with an edit that does not stand must not keep its document shut until someone removes the
/// journal by hand: what stands goes to the file, and the journal is kept where nothing reads it.
#[test]
fn a_journal_with_an_edit_that_does_not_stand_is_set_aside_and_its_document_opens() {
    let (scratch, served, id) = planned("refused");
    let file = scratch.0.join("family").join(FILE);
    let (mut ada, mut ben) = desktops(&served, &id);
    let number = ada.begin("Kept. ");
    until("the server says it holds the edit", &mut [&mut ada, &mut ben], |desks| desks[0].held >= number);
    served.kill();

    // The run's journal gains an edit the history library panics on as it lands, one of fuzz issue 4.
    let rooms = scratch.0.join("state").join("rooms");
    let [room] = &scratch.names("state/rooms")[..] else { panic!("one room's folder holds the journal") };
    let mut state = Share(rooms.join(room));
    let name = scratch.names(&format!("state/rooms/{room}")).into_iter().find(|name| is_journal(name));
    let name = name.expect("the run left a journal");
    let left = Unsaved::read(&mut state, &name).unwrap().expect("the journal holds the edit");
    let (mut journal, _) = Journal::open(&mut state, &left.id, &left.created, left.home.as_deref()).unwrap();
    let refused = include_bytes!("../../../fuzz/corpus/history/a-first-edit-that-builds-on-one-before-it.bin");
    journal.edits(&mut state, refused).unwrap();
    let found = std::fs::read(rooms.join(room).join(&name)).unwrap();
    // A run that ended between a room's last save and the end of its folder left one with nothing in it.
    std::fs::create_dir(rooms.join("emptied")).unwrap();

    // Nobody opens the document. The server that starts next takes what stands to the file by itself.
    let served = Served::start(&scratch.0, "state");
    until("the file holds what stands", &mut [], |_| file_says(&file).contains("Kept."));
    let aside: Vec<String> =
        scratch.names("state/rooms").into_iter().filter(|name| name.ends_with(".journal")).collect();
    let [aside] = &aside[..] else { panic!("the journal lies in rooms/ as one file: {aside:?}") };
    assert!(!is_journal(aside), "under a name that no journal is read by");
    assert_eq!(std::fs::read(rooms.join(aside)).unwrap(), found, "as it was found");
    let log = served.log.lock().unwrap().join("\n");
    let told = log.lines().find(|line| line.contains(&id) && line.contains("warn"));
    let told = told.unwrap_or_else(|| panic!("the log names the document:\n{log}"));
    assert!(told.contains("1 of its 2 edits") && told.contains("1 left out") && told.contains(aside), "{told}");

    // The document opens for whoever had it open, with the sign-in they had.
    (ada.at, ben.at) = (served.at, served.at);
    until("both are back and agree", &mut [&mut ada, &mut ben], agree);
    assert!(ben.said().contains("Kept."));
    // Once its file holds everything a room's folder goes, and what was set aside stays.
    drop((ada, ben));
    served.stop();
    assert_eq!(scratch.names("state/rooms"), std::slice::from_ref(aside));
}

/// The design's row "Truncated or corrupt file": the damaged file is kept aside, and the writer's replica
/// restores the document.
#[test]
fn a_damaged_file_is_set_aside_and_written_again_from_what_the_server_holds() {
    let (scratch, served, id) = planned("damaged");
    let file = scratch.0.join("family").join(FILE);
    let (mut ada, mut ben) = desktops(&served, &id);
    ada.begin("Before. ");
    until("the file holds the first edit", &mut [&mut ada, &mut ben], |_| file_says(&file).contains("Before."));

    // Half a file, as a sync tool leaves that was cut off.
    let whole = std::fs::read(&file).unwrap();
    let half = &whole[..whole.len() / 2];
    std::fs::write(&file, half).unwrap();
    ada.begin("After. ");
    until("the file is whole again", &mut [&mut ada, &mut ben], |_| {
        let says = file_says(&file);
        says.contains("Before.") && says.contains("After.")
    });

    let names = scratch.names("family");
    let aside = names.iter().find(|name| name.contains("damaged")).unwrap_or_else(|| panic!("{names:?}"));
    assert_eq!(std::fs::read(scratch.0.join("family").join(aside)).unwrap(), half, "the damaged file is kept");
    assert!(agree(&[&mut ada, &mut ben]), "nobody was put out");
}

/// The design's row "Network drop or server offline": editing goes on, and merges on reconnect. The
/// server is stopped as a service manager stops it, and writes its files before it goes.
#[test]
fn what_is_written_while_the_network_or_the_server_is_away_meets_when_it_is_back() {
    let (scratch, served, id) = planned("away");
    let file = scratch.0.join("family").join(FILE);
    let (mut ada, mut ben) = desktops(&served, &id);
    lay_out(&mut ada);
    until("both have the paragraphs", &mut [&mut ada, &mut ben], agree);

    // Ada's network drops, and both write on.
    ada.unplug();
    ada.write("ada", "Ada offline. ");
    let number = ben.write("ben", "Ben meanwhile. ");
    until("the server holds what Ben wrote", &mut [&mut ada, &mut ben], |desks| desks[1].held >= number);
    assert!(!ada.said().contains("Ben meanwhile."), "nothing reaches a device with no network");
    ada.plugged = true;
    until("the two meet", &mut [&mut ada, &mut ben], |desks| {
        agree(desks) && ["Ada offline.", "Ben meanwhile."].iter().all(|words| desks[1].said().contains(words))
    });

    // The server is asked to stop a moment after an edit, long before the edits have paused for a save.
    let number = ben.write("ben", "Just before. ");
    until("the server holds the edit", &mut [&mut ada, &mut ben], |desks| desks[1].held >= number);
    served.stop();
    assert!(file_says(&file).contains("Just before."), "a server that stops writes its files first");

    ada.write("ada", "Ada alone. ");
    ben.write("ben", "Ben alone. ");
    a_moment(&mut [&mut ada, &mut ben]);
    assert!(!ada.live() && !ben.live());
    let served = Served::start(&scratch.0, "state");
    (ada.at, ben.at) = (served.at, served.at);
    until("the two meet again, and the file has it all", &mut [&mut ada, &mut ben], |desks| {
        let all = ["Ada offline.", "Ben meanwhile.", "Just before.", "Ada alone.", "Ben alone."];
        agree(desks) && all.iter().all(|words| desks[0].said().contains(words)) && file_says(&file) == desks[0].said()
    });
}

/// The design's row "Two servers on one mount by mistake": each is just another writer.
#[test]
fn two_servers_on_one_mount_are_two_writers_of_its_files() {
    let (scratch, one, id) = planned("two-servers");
    let file = scratch.0.join("family").join(FILE);
    let mut ada = Desktop::open(&one, "ada", &id);
    until("Ada is in session", &mut [&mut ada], |desks| desks[0].live());
    lay_out(&mut ada);
    until("the file holds the paragraphs", &mut [&mut ada], |_| {
        on_file(&file).is_some_and(|(document, _)| document.block(&"all".into()).is_some())
    });

    // A second server is set up over the same folders, with a state folder of its own, and Ben's
    // desktop is pointed at that one.
    let two = Served::start(&scratch.0, "state-two");
    let mut ben = Desktop::open(&two, "ben", &id);
    until("Ben is in session on the other server", &mut [&mut ada, &mut ben], |desks| desks[1].live());

    for round in 1..=2 {
        let words = [format!("one{round}"), format!("two{round}")];
        ada.write("ada", &format!("{} ", words[0]));
        ben.write("ben", &format!("{} ", words[1]));
        // Each server learns of the other's save from the file alone.
        until("each desktop has what the other wrote", &mut [&mut ada, &mut ben], |desks| {
            desks.iter().all(|desk| words.iter().all(|word| desk.said().contains(word.as_str())))
        });
    }
    until("both servers and the file hold one document", &mut [&mut ada, &mut ben], |desks| {
        agree(desks) && file_says(&file) == desks[0].said()
    });
    assert_eq!(scratch.names("family"), [FILE], "nothing was set aside or copied");
}

/// The design's row "Server state volume lost": every document intact, and access fails closed. Share
/// links and the access log come with a later milestone.
#[test]
fn a_lost_state_volume_costs_no_document_and_lets_nobody_in_unasked() {
    let (scratch, served, id) = planned("state-lost");
    let file = scratch.0.join("family").join(FILE);
    let mut ada = Desktop::open(&served, "ada", &id);
    until("Ada is in session", &mut [&mut ada], |desks| desks[0].live());
    ada.begin("On file. ");
    until("the file holds the first edit", &mut [&mut ada], |_| file_says(&file).contains("On file."));
    let number = ada.begin("Only held. ");
    until("the server says it holds the second", &mut [&mut ada], |desks| desks[0].held >= number);
    let before = served.tokens["ada"].clone();
    // The server dies with the second edit in its journal alone, and the journal goes with the volume.
    served.kill();
    std::fs::remove_dir_all(scratch.0.join("state")).unwrap();
    let served = Served::start(&scratch.0, "state");

    // Whoever was signed in is nobody now.
    ada.at = served.at;
    until("Ada's desktop is turned away", &mut [&mut ada], |desks| desks[0].turned_away == Some(401));
    assert_eq!(served.ask(&before, &paths::entries("family", ""), None).0, 401);

    // Every document is where it was, under the id it had, for whoever signs in.
    let (status, listed) = served.ask(&served.tokens["ben"], &paths::entries("family", ""), None);
    assert_eq!(status, 200, "{listed}");
    let listed: Listing = serde_json::from_str(&listed).expect("a listing");
    assert_eq!(listed.entries.iter().filter_map(|entry| entry.id.as_deref()).collect::<Vec<&str>>(), [id.as_str()]);
    let mut ben = Desktop::open(&served, "ben", &id);
    until("Ben has the document", &mut [&mut ben], |desks| desks[0].live() && desks[0].said().contains("On file."));

    // What the lost journal alone held comes back with the device that wrote it.
    ada.sign_in(&served, "ada");
    until("the edit is back, and on file", &mut [&mut ada, &mut ben], |desks| {
        agree(desks) && desks[1].said().contains("Only held.") && file_says(&file).contains("Only held.")
    });
}

/// An update that passes its checksum and trips a debug assertion of the history library, as the `Edits`
/// frame numbered 2.
const UNREADABLE: &str = "020200000000000000620000006c6f726f0000000000000000000000001078c2f700044b0001fb0201011c\
    020800000000000000b5d4f4e5281c21950101010101010201000000050100000100060104010200000504626f6479000f01040201\
    00030186010201050201010002010a";

#[test]
fn a_frame_the_history_library_trips_on_does_not_stop_its_document() {
    let (_scratch, mut served, id) = planned("unreadable");
    let mut ada = Desktop::open(&served, "ada", &id);
    until("Ada is in session", &mut [&mut ada], |desks| desks[0].live());

    // Ben's device says the frame, then an edit of its own, then a ping: the answer to the ping comes
    // after the server has dealt with both.
    let mut socket = dial(served.at, &id, &served.tokens["ben"]).expect("an editor is let in");
    let byte = |at: usize| u8::from_str_radix(&UNREADABLE[2 * at..2 * at + 2], 16).expect("two hex digits");
    let frame: Vec<u8> = (0..UNREADABLE.len() / 2).map(byte).collect();
    assert!(matches!(ClientMessage::decode(&frame), Ok(Some(ClientMessage::Edits { .. }))), "the frame itself reads");
    let mut ben = ada.model().fork(ReplicaId::from(7));
    let (before, first) = (ben.extent(), ben.doc().blocks[0].id.clone());
    ben.apply([Op::InsertText { at: Pos::new(first, 0), text: "After the frame. ".into() }]).expect("the edit fits");
    let hello = Hello { protocol: PROTOCOL, document: String::new(), extent: Default::default(), revision: None };
    let edit = ClientMessage::Edits { seq: 3, updates: ben.updates_for(&before) };
    for said in [ClientMessage::Hello(hello).encode(), frame, edit.encode(), ClientMessage::Ping.encode()] {
        socket.send(Message::Binary(said.into())).unwrap();
    }
    let (mut heard, patience) = (Vec::new(), Instant::now() + PATIENCE);
    while !heard.contains(&ServerMessage::Pong) {
        assert!(Instant::now() < patience, "the server answers a ping, after {heard:?}");
        match socket.read() {
            Ok(Message::Binary(frame)) => heard.push(ServerMessage::decode(&frame).unwrap().expect("a known message")),
            Ok(_) => {}
            Err(SocketError::Io(error)) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => panic!("the server ended the session: {error}"),
        }
    }
    assert!(heard.contains(&ServerMessage::Held { seq: 3 }), "what is written after the frame is held");

    // The document goes on for everyone: the others have the edit, and a device can still open it.
    let mut cy = Desktop::open(&served, "ben", &id);
    until("the document goes on after the frame", &mut [&mut ada, &mut cy], |desks| {
        agree(desks) && desks[1].said().contains("After the frame.")
    });
    assert!(served.ended(Duration::ZERO).is_none(), "the server runs on");
}
