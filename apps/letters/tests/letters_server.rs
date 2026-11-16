//! The desktop's side of connected mode against the server itself: `letters-server`, running on this
//! machine over scratch folders, behind a certificate nobody but a test trusts.

mod common;

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use common::{Scratch, Writer};
use letters::connected::{self, Line, Placing, Said, Told};
use letters::files::{self, Home};
use letters::import::Plan;
use letters::keychain::Keychain;
use letters::pictures;
use letters::remote::Roots;
use letters::server::Client;
use letters::session::Session;
use letters::signin::SignIn;
use letters_api::{Entry, EntryKind, NewDocument, Opened, Person, Role, SpaceKind, Trust, paths};
use letters_model::{Block, BlockKind, Notes, Style};
use letters_server::config::Config;
use letters_server::session::{COOKIE_NAME, Carrier};
use letters_server::store::Store;
use letters_server::{App, api};
use letters_sync::{ClientMessage, Link, Local, MAX_FRAME, Progress, Refusal};
use sha2::{Digest, Sha256};

/// Sign-ins are kept in memory here, never in the keychain of whoever runs the tests.
static KEYCHAIN: LazyLock<Keychain> = LazyLock::new(|| Keychain::in_memory().unwrap());

/// When the server's clock starts: later than any clock this test runs by, so that the server alone
/// decides when a token has run out.
const START: u64 = 4_000_000_000;

/// A Letters server on this machine, reached over HTTPS as `localhost`.
struct Server {
    app: Arc<App>,
    port: u16,
    /// The certificate it shows.
    certificate: Vec<u8>,
    clock: Arc<AtomicU64>,
    front: Arc<Front>,
    _scratch: Scratch,
    _runtime: tokio::runtime::Runtime,
}

impl Server {
    fn start(name: &str) -> Self {
        let scratch = Scratch::new(name);
        let (tls, certificate) = common::tls();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        // In literal strings below: a path on Windows has backslashes, which a basic string reads as escapes.
        let at = scratch.0.display();
        let settings = format!(
            r#"
[server]
name = "Home"
url = "https://localhost:{port}"
state = '{at}/state'

[auth.oidc]
issuer = "https://id.example.test"
client_id = "letters"

[[space]]
name = "Home"
path = '{at}/home/{{username}}'
"#
        );
        let clock = Arc::new(AtomicU64::new(START));
        let told = clock.clone();
        let app = App::new(Config::parse(&settings).expect("the settings are sound"), Store::in_memory())
            .with_clock(Box::new(move || told.load(Ordering::Relaxed)));
        let app = Arc::new(app);
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let router = api::router(app.clone());
        let behind = runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let behind = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, router).await });
            behind
        });
        let front = Arc::new(Front {
            behind,
            down: AtomicBool::new(false),
            sockets: AtomicUsize::new(0),
            log: Mutex::default(),
        });
        let serving = front.clone();
        // The server speaks plain HTTP and expects TLS in front of it, as it has wherever it runs.
        thread::spawn(move || {
            for socket in listener.incoming().flatten() {
                if serving.down.load(Ordering::Relaxed) {
                    continue;
                }
                let Ok(connection) = rustls::ServerConnection::new(tls.clone()) else { continue };
                let mut stream = rustls::StreamOwned::new(connection, socket);
                let serving = serving.clone();
                thread::spawn(move || {
                    let _ = serving.serve(&mut stream);
                    stream.conn.send_close_notify();
                    let _ = stream.flush();
                });
            }
        });
        Self { app, port, certificate, clock, front, _scratch: scratch, _runtime: runtime }
    }

    fn origin(&self) -> String {
        format!("https://localhost:{}", self.port)
    }

    fn roots(&self) -> Roots {
        Roots::These(vec![self.certificate.clone()])
    }

    fn client(&self) -> Client {
        Client::new(&self.origin(), &self.roots()).unwrap()
    }

    /// A session of `person`, as a sign-in on one more device ends in. Nothing of it is kept.
    fn session(&self, person: Person) -> Arc<Session> {
        let token = self.app.sign_in(person, Vec::new(), Carrier::Bearer).expect("a session starts");
        Arc::new(Session::begin(self.client(), None, token))
    }

    /// A new document in the person's own space, by its id.
    fn document(&self, session: &Session, name: &str) -> String {
        let spaces = session.spaces().expect("the spaces are listed");
        let own = spaces.iter().find(|space| space.kind == SpaceKind::Personal).expect("a space of their own");
        let new = NewDocument { name: Some(name.into()), ..NewDocument::default() };
        let made: Entry = session.ask(|server, token| server.post(token, &paths::documents(&own.id), &new)).unwrap();
        made.id.expect("a document has an id")
    }

    /// The network to the server goes: every socket ends without a word, and nothing is answered.
    fn cut(&self) {
        self.front.down.store(true, Ordering::Relaxed);
        let ended = Instant::now() + Duration::from_secs(10);
        while self.front.sockets.load(Ordering::Relaxed) > 0 {
            assert!(Instant::now() < ended, "the sockets end");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// The network is back.
    fn mend(&self) {
        self.front.down.store(false, Ordering::Relaxed);
    }

    /// What came in since this was last asked: `picture NAME` and `edits`, in their order.
    fn came(&self) -> Vec<String> {
        std::mem::take(&mut self.front.log.lock().unwrap())
    }

    /// The system's browser, signed in to the server as `person`: sent to the page the app made, it
    /// agrees, and goes where the server sends it, which is back to the app.
    fn browser(&self, person: Person) -> impl Fn(&str) -> std::io::Result<()> + '_ {
        let cookie = self.app.sign_in(person, Vec::new(), Carrier::Cookie).expect("a session starts").token;
        move |page| {
            let agent = ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .max_redirects(0)
                    .max_redirects_will_error(false)
                    .http_status_as_error(false)
                    .tls_config(
                        ureq::tls::TlsConfig::builder()
                            .root_certs(ureq::tls::RootCerts::new_with_certs(&[ureq::tls::Certificate::from_der(
                                &self.certificate,
                            )
                            .to_owned()]))
                            .build(),
                    )
                    .build(),
            );
            let cookie = format!("{COOKIE_NAME}={cookie}");
            let asks = agent.get(page).header("cookie", &cookie).call().expect("the page answers");
            assert_eq!(asks.status(), 200, "the page asks the person who is signed in");
            let query = page.split_once('?').unwrap().1.to_owned();
            let agreed = agent
                .post(format!("{}{}", self.origin(), paths::DESKTOP_PAGE))
                .header("cookie", &cookie)
                .header("origin", self.origin())
                .content_type("application/x-www-form-urlencoded")
                .send(query.as_bytes())
                .expect("the server answers");
            let back = agreed.headers().get("location").expect("the browser is sent on").to_str().unwrap().to_owned();
            let (app, target) = back.strip_prefix("http://").unwrap().split_once('/').unwrap();
            let (app, target) = (app.to_owned(), format!("/{target}"));
            thread::spawn(move || {
                let mut browser = TcpStream::connect(&app).unwrap();
                write!(browser, "GET {target} HTTP/1.1\r\nhost: {app}\r\n\r\n").unwrap();
                let _ = browser.read_to_string(&mut String::new());
            });
            Ok(())
        }
    }
}

type Tls = rustls::StreamOwned<rustls::ServerConnection, TcpStream>;

/// What stands in front of the server, as TLS does wherever it runs. It passes everything on, and notes
/// what a test asks the order of.
struct Front {
    behind: SocketAddr,
    /// Whether the network to the server is down, and how many sockets are open through here.
    down: AtomicBool,
    sockets: AtomicUsize,
    /// What came in for the server: `picture NAME` and `edits`.
    log: Mutex<Vec<String>>,
}

/// Whether a read only ran out of time.
fn waits(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
}

impl Front {
    /// Answers one connection.
    fn serve(&self, stream: &mut Tls) -> io::Result<()> {
        let (mut head, mut byte) = (Vec::new(), [0]);
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte)?;
            head.push(byte[0]);
        }
        let said = String::from_utf8_lossy(&head).into_owned();
        let mut line = said.split(' ');
        let (verb, path) = (line.next().unwrap_or_default(), line.next().unwrap_or_default());
        let of_document = path.strip_prefix("/api/v1/documents/").and_then(|rest| rest.split_once('/'));
        if let Some((_, "sync")) = of_document {
            self.sockets.fetch_add(1, Ordering::Relaxed);
            let carried = self.carry(stream, &head);
            self.sockets.fetch_sub(1, Ordering::Relaxed);
            return carried;
        }
        if let ("PUT", Some((_, rest))) = (verb, of_document)
            && let Some(name) = rest.strip_prefix("assets/")
        {
            self.log.lock().unwrap().push(format!("picture {name}"));
        }
        let length = said
            .to_ascii_lowercase()
            .lines()
            .find_map(|line| line.strip_prefix("content-length:")?.trim().parse().ok());
        let mut body = vec![0; length.unwrap_or(0)];
        stream.read_exact(&mut body)?;
        // One request to a connection, so that the answer ends where the server hangs up.
        let line = head.iter().position(|byte| *byte == b'\n').unwrap_or(0) + 1;
        let mut server = TcpStream::connect(self.behind)?;
        server.write_all(&[&head[..line], b"connection: close\r\n", &head[line..], &body].concat())?;
        let mut answer = Vec::new();
        server.read_to_end(&mut answer)?;
        stream.write_all(&answer)
    }

    /// Carries a document's socket both ways until either side ends it, or the network goes.
    fn carry(&self, stream: &mut Tls, head: &[u8]) -> io::Result<()> {
        let mut server = TcpStream::connect(self.behind)?;
        server.write_all(head)?;
        // Each side is looked at often, so that neither is kept waiting for the other.
        stream.sock.set_read_timeout(Some(Duration::from_millis(2)))?;
        server.set_read_timeout(Some(Duration::from_millis(2)))?;
        let (mut some, mut sent) = ([0; 64 * 1024], Vec::new());
        while !self.down.load(Ordering::Relaxed) {
            match stream.read(&mut some) {
                Ok(0) => break,
                Ok(read) => {
                    server.write_all(&some[..read])?;
                    sent.extend_from_slice(&some[..read]);
                    self.note(&mut sent);
                }
                Err(error) if waits(&error) => {}
                Err(error) => return Err(error),
            }
            match server.read(&mut some) {
                Ok(0) => break,
                Ok(read) => stream.write_all(&some[..read])?,
                Err(error) if waits(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Notes the edits among the whole frames a device has sent so far, and takes those frames off.
    fn note(&self, sent: &mut Vec<u8>) {
        loop {
            let Some(&[_, second]) = sent.get(..2) else { return };
            // A device masks what it sends, and says how long it is in one of three ways.
            let (at, length) = match second & 0x7f {
                126 if sent.len() >= 4 => (4, u16::from_be_bytes([sent[2], sent[3]]) as usize),
                127 if sent.len() >= 10 => (10, u64::from_be_bytes(sent[2..10].try_into().unwrap()) as usize),
                126 | 127 => return,
                short => (2, short as usize),
            };
            if sent.len() < at + 4 + length {
                return;
            }
            let frame: Vec<u8> = sent.drain(..at + 4 + length).collect();
            let mask = &frame[at..at + 4];
            let said: Vec<u8> =
                frame[at + 4..].iter().zip(mask.iter().cycle()).map(|(byte, mask)| byte ^ mask).collect();
            if matches!(ClientMessage::decode(&said), Ok(Some(ClientMessage::Edits { .. }))) {
                self.log.lock().unwrap().push("edits".to_owned());
            }
        }
    }
}

/// A document of the server's open on one device, with what a window holds beside its line.
struct Desk {
    id: String,
    writer: Writer,
    /// Where the copy on this device is saved.
    copy: Home,
    notes: Notes,
    /// Pictures no save has put in the copy's file yet.
    pictures: Vec<(String, Vec<u8>)>,
    line: Line,
    heard: mpsc::Receiver<Said>,
    born: Instant,
    /// How far this device's clock was put on, so that no test waits for a line to try again.
    ahead: u64,
    /// What the server would not take: work, and pictures by their names.
    refused: Vec<Refusal>,
    unkept: Vec<String>,
    home: Scratch,
}

impl Desk {
    /// Brings the document here and opens it, as a window does. The line is not started.
    fn open(server: &Server, session: Arc<Session>, id: &str, home: &str) -> Self {
        Self::open_in(server, session, id, Scratch::new(home))
    }

    fn open_in(server: &Server, session: Arc<Session>, id: &str, home: Scratch) -> Self {
        let reached = connected::reach(&session, id, &home.0).expect("the document is reached");
        let (writer, copy) = Writer::of(files::open(&reached.path, Style::default).expect("the copy is a document"));
        let (tell, heard) = mpsc::channel();
        let tell: connected::Tell = Arc::new(move |said| drop(tell.send(said)));
        let line = Line::new(session, &reached.opened, &reached.path, server.roots(), tell);
        let (notes, born) = (writer.copy.notes.clone(), Instant::now());
        let (id, pictures, refused, unkept) = (id.to_owned(), Vec::new(), Vec::new(), Vec::new());
        Self { id, writer, copy, notes, pictures, line, heard, born, ahead: 0, refused, unkept, home }
    }

    /// The app is closed and opened again: the document is opened from its copy, on a new line.
    fn again(self, server: &Server) -> Self {
        Self::open_in(server, self.line.session().clone(), &self.id, self.home)
    }

    /// Saves the copy on this device, as an autosave does.
    fn save(&mut self) {
        self.writer.copy.entries.extend(self.pictures.drain(..));
        let document = self.writer.model.doc().clone();
        self.copy = self.writer.save(&self.copy, &document).0;
    }

    fn now(&self) -> u64 {
        self.born.elapsed().as_millis() as u64 + self.ahead
    }

    fn start(&mut self) {
        let told = self.line.start(false, self.now());
        self.does(told);
    }

    /// Does what the line left to do, as a window does.
    fn does(&mut self, told: Vec<Told>) {
        let mut told = VecDeque::from(told);
        while let Some(next) = told.pop_front() {
            match next {
                Told::Merge(edits) => {
                    self.writer.model.merge(&edits).expect("what the server sent is merged");
                    self.notes.follow(&self.writer.model);
                    told.extend(self.tell());
                }
                Told::Notes(notes) => {
                    if self.notes.merge(&notes) {
                        self.notes.follow(&self.writer.model);
                    }
                    told.extend(self.tell());
                }
                Told::Refused(refusal) => self.refused.push(refusal),
                Told::Unsent(sentence) => panic!("the server did not take the file: {sentence}"),
                Told::Unkept(name) => self.unkept.push(name),
                Told::Whole => {
                    self.save();
                    self.line.saved();
                }
                Told::Link | Told::Progress | Told::Peers => {}
            }
        }
    }

    fn tell(&mut self) -> Vec<Told> {
        let now = self.now();
        self.line.changed(Local { model: Some(&self.writer.model), notes: &self.notes }, now)
    }

    /// The person writes one more paragraph.
    fn write(&mut self, block: &str, text: &str) {
        let mut document = self.writer.model.doc().clone();
        document.blocks.push(Block::text(block, BlockKind::Paragraph, text));
        self.writer.write(&document);
        let told = self.tell();
        self.does(told);
    }

    /// The person puts a picture in the document. Its name there.
    fn picture(&mut self, block: &str, bytes: Vec<u8>) -> String {
        let hash: String = Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
        let name = format!("assets/{hash}.png");
        self.line.keep(&name, bytes.clone());
        self.pictures.push((name.clone(), bytes));
        let (asset, alt) = (Some(name.clone()), String::new());
        let figure = BlockKind::Figure { asset, src: None, alt, width: 40.0, height: 20.0, numbered: false };
        let mut document = self.writer.model.doc().clone();
        document.blocks.push(Block::text(block, figure, ""));
        self.writer.write(&document);
        let told = self.tell();
        self.does(told);
        name
    }

    /// Whether the document here has a figure of the picture of this name.
    fn shows(&self, name: &str) -> bool {
        let shown =
            |block: &Block| matches!(&block.kind, BlockKind::Figure { asset, .. } if asset.as_deref() == Some(name));
        self.writer.model.doc().walk().any(shown)
    }

    /// The person comments on a paragraph.
    fn comment(&mut self, id: &str, block: &str, words: &str) {
        self.notes.notes.push(common::comment(id, block, words));
        self.notes.pin(&self.writer.model);
        let told = self.tell();
        self.does(told);
    }

    /// What the document reads here, a paragraph to a line.
    fn reads(&self) -> Vec<String> {
        self.writer.model.doc().walk().map(Block::plain_text).filter(|text| !text.is_empty()).collect()
    }

    /// Hears what its sockets said since, and does what is due.
    fn turn(&mut self) {
        while let Ok(said) = self.heard.try_recv() {
            let now = self.now();
            let told = self.line.heard(said, Local { model: Some(&self.writer.model), notes: &self.notes }, now);
            self.does(told);
        }
        // A line that waits to try again does not have to.
        if self.line.link() == Link::Offline
            && let Some(at) = self.line.wake_at()
        {
            self.ahead += at.saturating_sub(self.now());
        }
        if self.line.wake_at().is_some_and(|at| at <= self.now()) {
            let told = self.line.tick(self.now());
            self.does(told);
        }
    }
}

/// Lets the devices and the server carry on until `until` holds. Whether it came to hold in good time.
fn settle(desks: &mut [&mut Desk], until: impl Fn(&[&mut Desk]) -> bool) -> bool {
    settle_within(Duration::from_secs(20), desks, until)
}

/// As [`settle`], for what takes a machine long to work out.
fn settle_within(patience: Duration, desks: &mut [&mut Desk], until: impl Fn(&[&mut Desk]) -> bool) -> bool {
    let ended = Instant::now() + patience;
    while Instant::now() < ended {
        desks.iter_mut().for_each(|desk| desk.turn());
        if until(desks) {
            return true;
        }
        thread::sleep(Duration::from_millis(2));
    }
    false
}

/// Whether the device is in session as someone who edits.
fn live(desk: &Desk) -> bool {
    desk.line.link() == Link::Live(letters_sync::Role::Editor)
}

fn ada() -> Person {
    Person { id: "sub-ada".into(), username: "ada".into(), name: "Ada Lovelace".into() }
}

#[test]
fn the_app_signs_in_through_the_browser_and_opens_a_document_of_the_server() {
    let (home, server) = (Scratch::new("real-home"), Server::start("real-server"));
    let client = server.client();

    assert!(client.discover().expect("a Letters server answers").speaks());

    // Signing in: the app's secret and the server's code meet, and the session is the person's.
    let sign_in = SignIn::begin().unwrap();
    server.browser(ada())(&sign_in.page(&client).unwrap()).unwrap();
    let token = sign_in.finish(&client, Duration::from_secs(30), &AtomicBool::new(false)).expect("the code is good");
    assert_eq!(token.person, ada());
    let session = Arc::new(Session::begin(client.clone(), Some(*KEYCHAIN), token));

    // The person's own space is there, and takes a new document.
    let spaces = session.spaces().expect("the spaces are listed");
    let own = spaces.iter().find(|space| space.kind == SpaceKind::Personal).expect("a space of their own");
    assert_eq!(own.role, Role::Owner);
    let new = NewDocument { name: Some("Roof repairs".into()), ..NewDocument::default() };
    let made: Entry = session.ask(|server, token| server.post(token, &paths::documents(&own.id), &new)).unwrap();
    let id = made.id.clone().expect("a document has an id");
    let listed = session.entries(&own.id, "").unwrap().entries;
    assert!(listed.iter().any(|entry| entry.kind == EntryKind::Document && entry.id == made.id), "{listed:?}");
    assert!(session.recent().is_ok() && session.shared().is_ok());

    // It is brought here whole, and is a document this app opens.
    let reached = connected::reach(&session, &id, &home.0).expect("the document is reached");
    assert!(reached.online);
    assert_eq!((reached.opened.name.as_str(), reached.opened.trust), ("Roof repairs", Trust::Trusted));
    assert_eq!(reached.opened.socket, paths::socket(&id));
    let opened = files::open(&reached.path, Style::default).expect("the copy is a document");
    assert!(opened.home.is_some());

    // The server's clock runs a token out: the session renews it and asks again, unnoticed.
    server.clock.fetch_add(24 * 60 * 60, Ordering::Relaxed);
    assert_eq!(session.open(&id).map(|opened| opened.id), Ok(id.clone()));

    // The sign-in was kept, so the app opens signed in, and the next document needs no browser.
    let no_browser = |_: &str| -> std::io::Result<()> { panic!("nobody is sent to sign in") };
    let (kept, again) = connected::enter(
        &client,
        &id,
        None,
        Some(*KEYCHAIN),
        &home.0,
        &no_browser,
        &|_, _| (),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!((kept.person(), again.path), (ada(), reached.path));

    // Signed out, nothing more is had with what was kept.
    assert_eq!(kept.sign_out(), Ok(()));
    assert!(Session::kept(client.clone(), *KEYCHAIN).unwrap().is_none());
    assert!(session.spaces().is_err(), "the server ended the session for every holder");
}

#[test]
fn two_devices_write_one_document_together() {
    let server = Server::start("line-together");
    let id = server.document(&server.session(ada()), "Roof repairs");
    let mut here = Desk::open(&server, server.session(ada()), &id, "line-together-here");
    let mut there = Desk::open(&server, server.session(ada()), &id, "line-together-there");
    here.start();
    there.start();
    let both = |desks: &[&mut Desk]| desks.iter().all(|desk| live(desk) && desk.line.peers().len() == 1);
    assert!(settle(&mut [&mut here, &mut there], both), "{:?} {:?}", here.line.link(), there.line.link());
    assert_eq!(here.line.peers()[0].name, ada().name);

    // What one writes, the other reads, and the server says it keeps it.
    here.write("slates", "Slates first.");
    assert_eq!(here.line.progress(), Progress::Saving);
    let read =
        |desks: &[&mut Desk]| desks[1].reads() == ["Slates first."] && desks[0].line.progress() == Progress::Saved;
    assert!(settle(&mut [&mut here, &mut there], read), "{:?} {:?}", there.reads(), here.line.progress());

    // A comment comes the other way with its text.
    there.write("flashing", "Then the flashing.");
    there.comment("which", "slates", "Which slates?");
    let read = |desks: &[&mut Desk]| desks[0].reads().len() == 2 && common::comments(&desks[0].notes) == ["which"];
    assert!(settle(&mut [&mut here, &mut there], read), "{:?} {:?}", here.reads(), here.notes);
    assert_eq!(here.reads(), there.reads());
    assert!(here.refused.is_empty() && there.refused.is_empty(), "{:?} {:?}", here.refused, there.refused);

    // One leaves, and the other is told.
    there.line.stop();
    assert!(settle(&mut [&mut here], |desks| desks[0].line.peers().is_empty()));
}

#[test]
fn work_written_while_the_server_is_away_goes_up_when_it_is_back() {
    let server = Server::start("line-away");
    let id = server.document(&server.session(ada()), "Roof repairs");
    let mut here = Desk::open(&server, server.session(ada()), &id, "line-away-here");
    let mut there = Desk::open(&server, server.session(ada()), &id, "line-away-there");
    here.start();
    there.start();
    assert!(settle(&mut [&mut here, &mut there], |desks| desks.iter().all(|desk| live(desk))));

    server.cut();
    let neither = |desks: &[&mut Desk]| desks.iter().all(|desk| !live(desk));
    assert!(settle(&mut [&mut here, &mut there], neither), "{:?} {:?}", here.line.link(), there.line.link());
    here.write("slates", "Slates first.");
    there.write("flashing", "Then the flashing.");
    // Nothing says the server has what it cannot have.
    assert_ne!(here.line.progress(), Progress::Saved);
    assert_eq!(there.reads(), ["Then the flashing."]);

    server.mend();
    let met = |desks: &[&mut Desk]| {
        desks.iter().all(|desk| live(desk) && desk.reads().len() == 2 && desk.line.progress() == Progress::Saved)
    };
    assert!(settle(&mut [&mut here, &mut there], met), "{:?} {:?}", here.reads(), there.reads());
    assert_eq!(here.reads(), there.reads());
}

#[test]
fn a_line_outlives_its_token_and_ends_with_the_sign_in() {
    let server = Server::start("line-token");
    let session = server.session(ada());
    let id = server.document(&session, "Roof repairs");
    let mut desk = Desk::open(&server, session.clone(), &id, "line-token-desk");
    desk.start();
    assert!(settle(&mut [&mut desk], |desks| live(desks[0])), "{:?}", desk.line.link());

    // The server's clock runs the token out: the next socket is refused, and the one after has a new one.
    let first = session.token().unwrap();
    server.clock.fetch_add(24 * 60 * 60, Ordering::Relaxed);
    server.cut();
    server.mend();
    assert!(settle(&mut [&mut desk], |desks| !live(desks[0])));
    assert!(settle(&mut [&mut desk], |desks| live(desks[0])), "{:?}", desk.line.link());
    assert_ne!(session.token().unwrap(), first);

    // Signed out, the server ends the socket, and the line asks for no other.
    assert_eq!(session.sign_out(), Ok(()));
    let over = |desks: &[&mut Desk]| desks[0].line.link() == Link::Refused(Refusal::SignedOut);
    assert!(settle(&mut [&mut desk], over), "{:?}", desk.line.link());
    assert_eq!(desk.line.wake_at(), None);
}

#[test]
fn a_document_the_server_will_not_open_has_no_line() {
    let server = Server::start("line-refused");
    let session = server.session(ada());
    let opened = session.open(&server.document(&session, "Roof repairs")).unwrap();
    let nowhere = Opened { socket: paths::socket("no-such-document"), ..opened.clone() };
    let bob = Person { id: "sub-bob".into(), username: "bob".into(), name: "Bob".into() };
    let cases = [("someone else's", server.session(bob), opened), ("one that is gone", session, nowhere)];

    for (name, session, opened) in cases {
        let (tell, heard) = mpsc::channel();
        let tell: connected::Tell = Arc::new(move |said| drop(tell.send(said)));
        let mut line = Line::new(session, &opened, std::path::Path::new("no-copy.let"), server.roots(), tell);
        let nothing = Notes::default();
        line.start(false, 0);
        while let Ok(said) = heard.recv_timeout(Duration::from_secs(10)) {
            line.heard(said, Local { model: None, notes: &nothing }, 0);
            if !matches!(line.link(), Link::Connecting) {
                break;
            }
        }
        // It is not tried again, however long the window stays open.
        assert!(matches!(line.link(), Link::Refused(Refusal::NoAccess | Refusal::Gone)), "{name}: {:?}", line.link());
        assert_eq!(line.wake_at(), None, "{name}");
    }
}

#[test]
fn a_document_is_named_renamed_and_moved_at_its_server() {
    let server = Server::start("placing");
    let session = server.session(ada());
    let spaces = session.spaces().unwrap();
    let space = spaces.iter().find(|space| space.kind == SpaceKind::Personal).unwrap().id.clone();
    server.document(&session, "Roof repairs");
    let id = session.new_document(&space, "", None).unwrap().id.unwrap();
    let home = Scratch::new("placing-home");
    let copy = connected::reach(&session, &id, &home.0).expect("the document is brought here").path;
    let place = |asks: &[Placing]| connected::place(&session, &id, &copy, asks);
    let called = |placed: &connected::Placed| placed.opened.as_ref().map(|opened| opened.name.clone());

    // Its first heading names it once, with a number where another document has the name.
    for heading in ["Roof repairs", "Something else"] {
        let named = place(&[Placing::Name(heading.into())]);
        assert_eq!((called(&named).as_deref(), named.refused), (Some("Roof repairs 2"), None), "{heading}");
        assert!(named.opened.is_some_and(|opened| opened.named), "{heading}");
    }

    // What the server refuses is said in a sentence, and what was asked after it is left alone.
    let nowhere = Placing::Move { space: space.clone(), folder: "Nowhere".into() };
    let refusals = [
        ("a name that is taken", vec![Placing::Rename("Roof repairs".into())]),
        ("a folder that is not there", vec![nowhere, Placing::Rename("Never".into())]),
    ];
    for (case, asks) in refusals {
        let placed = place(&asks);
        assert!(placed.refused.as_ref().is_some_and(|refused| refused.ends_with('.')), "{case}");
        assert_eq!(called(&placed).as_deref(), Some("Roof repairs 2"), "{case}");
    }

    // Folders are made for it, and it moves among those of its space.
    let folder = |parent: &str, name: &str| Placing::NewFolder {
        space: space.clone(),
        parent: parent.into(),
        name: name.into(),
    };
    let quotes = Placing::Move { space: space.clone(), folder: "House/Quotes".into() };
    let placed = place(&[folder("", "House"), folder("House", "Quotes"), quotes, Placing::Rename("Quote".into())]);
    assert_eq!(placed.refused, None);
    let opened = placed.opened.expect("the server says where it is");
    assert_eq!(
        (opened.name.as_str(), opened.path.rsplit_once('/').map(|(folder, _)| folder)),
        ("Quote", Some("House/Quotes"))
    );
    let shelves = placed.shelves.expect("its space has folders");
    let tree: Vec<_> = shelves.iter().map(|shelf| (shelf.path.as_str(), shelf.depth)).collect();
    assert_eq!(tree, [("", 0), ("House", 1), ("House/Quotes", 2)]);

    // With the server away nothing is known anew, and the document opens under the name it last had.
    server.cut();
    assert_eq!(place(&[Placing::Look]), connected::Placed { opened: None, shelves: None, refused: None });
    assert!(place(&[Placing::Rename("Later".into())]).refused.is_some());
    let offline = connected::reach(&session, &id, &home.0).expect("the copy opens");
    assert_eq!((offline.online, offline.opened.name.as_str()), (false, "Quote"));
}

#[test]
fn a_picture_is_with_the_server_before_the_edit_that_names_it() {
    let server = Server::start("line-pictures");
    let (ours, theirs) = (server.session(ada()), server.session(ada()));
    let id = server.document(&ours, "Roof repairs");
    let mut here = Desk::open(&server, ours, &id, "line-pictures-here");
    let mut there = Desk::open(&server, theirs.clone(), &id, "line-pictures-there");
    here.start();
    there.start();
    assert!(settle(&mut [&mut here, &mut there], |desks| desks.iter().all(|desk| live(desk))));

    // A picture comes into the document while the line is live, while the server is away, and in a run of
    // the app that ends before the server is back.
    for (case, shade) in ["live", "away", "closed"].into_iter().zip(1u8..) {
        if case != "live" {
            server.cut();
            assert!(settle(&mut [&mut here, &mut there], |desks| desks.iter().all(|desk| !live(desk))), "{case}");
        }
        server.came();
        let picture = pictures::from_rgba(4, 2, &[shade; 32]).unwrap().bytes;
        let name = here.picture(case, picture.clone());
        if case == "closed" {
            here.save();
            here = here.again(&server);
            here.start();
        }
        server.mend();

        let shown = |desks: &[&mut Desk]| desks[1].shows(&name) && desks[0].line.progress() == Progress::Saved;
        assert!(settle(&mut [&mut here, &mut there], shown), "{case}: {:?}", here.line.link());
        let came = server.came();
        assert_eq!(came.first(), Some(&format!("picture {}", &name["assets/".len()..])), "{case}: {came:?}");
        assert!(came.contains(&"edits".to_owned()), "{case}: {came:?}");
        // Whoever reads the edit has the picture for the asking.
        assert_eq!(connected::picture(&theirs, &id, &name), Some(picture), "{case}");
    }
    assert_eq!(here.unkept, [""; 0]);

    // A picture the server will not take is told, and the writing goes on.
    here.line.keep("assets/not-its-hash.png", vec![1, 2, 3]);
    here.write("gutters", "Then the gutters.");
    let read = |desks: &[&mut Desk]| desks[1].reads().contains(&"Then the gutters.".to_owned());
    assert!(settle(&mut [&mut here, &mut there], read), "{:?}", there.reads());
    assert_eq!(here.unkept, ["assets/not-its-hash.png"]);
}

#[test]
fn a_file_brought_in_on_this_device_is_made_a_document_at_the_server_with_its_pictures() {
    let server = Server::start("bring");
    let (ours, theirs) = (server.session(ada()), server.session(ada()));
    let spaces = ours.spaces().expect("the spaces are listed");
    let space = spaces.iter().find(|space| space.kind == SpaceKind::Personal).expect("a space of their own");
    // Markdown that shows a picture beside it and one that is gone, which the import leaves a note on.
    let (files, home) = (Scratch::new("bring-files"), Scratch::new("bring-home"));
    let picture = pictures::from_rgba(4, 2, &[7; 32]).unwrap().bytes;
    std::fs::write(files.0.join("roof.png"), &picture).unwrap();
    let markdown = "# Rates\n\nRates change in January.\n\n![The roof](roof.png)\n\n![Gone](gone.png)\n";
    std::fs::write(files.0.join("Rates.md"), markdown).unwrap();
    let bring = |space: &str| {
        let plan = Plan::of(&files.0.join("Rates.md"), &Style::default()).expect("Markdown can be brought in");
        let imported = plan.clone().import(&plan.choices());
        connected::bring(&ours, space, "", &plan.name, &imported.package, &home.0)
    };

    let brought = bring(&space.id).expect("the server takes it");
    assert_eq!(brought.opened.name, "Rates", "named after the file");
    // Whoever opens it on another device reads all of it, with nothing more said by this one.
    let there = Desk::open(&server, theirs.clone(), &brought.opened.id, "bring-there");
    assert!(there.reads().contains(&"Rates change in January.".to_owned()), "{:?}", there.reads());
    let shown = there.writer.model.doc().walk().find_map(|block| match &block.kind {
        BlockKind::Figure { asset, .. } => asset.clone(),
        _ => None,
    });
    let shown = shown.expect("the picture beside the file came along");
    assert_eq!(connected::picture(&theirs, &brought.opened.id, &shown), Some(picture));
    assert_eq!(there.notes.notes.len(), 1, "the import's note on the picture that stayed a link");

    // The copy on this device opens as the same document, and owes the server nothing.
    let mut here = Desk::open_in(&server, ours.clone(), &brought.opened.id, home);
    assert_eq!(here.reads(), there.reads());
    here.start();
    assert!(settle(&mut [&mut here], |desks| live(desks[0]) && desks[0].line.progress() == Progress::Saved));
    assert!(server.came().iter().all(|came| came != "edits"), "nothing was left to send");

    // A second file of that name replaces nothing, and a space that is not theirs gets no document.
    let home = Scratch::new("bring-home-again");
    let bring = |space: &str| {
        let plan = Plan::of(&files.0.join("Rates.md"), &Style::default()).expect("Markdown can be brought in");
        connected::bring(&ours, space, "", &plan.name, &plan.clone().import(&plan.choices()).package, &home.0)
    };
    let second = bring(&space.id).expect("the next name is free");
    assert_eq!(second.opened.name, "Rates 2");
    assert_ne!(second.opened.id, brought.opened.id);
    assert!(matches!(bring("nobodys"), Err(connected::Unreached::Refused(_))));
    let listed = ours.entries(&space.id, "").expect("the space is listed").entries;
    assert_eq!(listed.iter().filter(|entry| entry.name.starts_with("Rates")).count(), 2);
}

// A debug build takes two minutes to merge this much, and three sides here do.
#[test]
#[ignore = "takes minutes in a debug build: cargo test --release -p letters --test letters_server -- --ignored"]
fn work_that_fits_no_frame_travels_as_the_file() {
    let server = Server::start("line-whole");
    let id = server.document(&server.session(ada()), "Roof repairs");
    let mut here = Desk::open(&server, server.session(ada()), &id, "line-whole-here");
    let mut there = Desk::open(&server, server.session(ada()), &id, "line-whole-there");
    here.start();
    there.start();
    assert!(settle(&mut [&mut here, &mut there], |desks| desks.iter().all(|desk| live(desk))));

    // More than a frame holds, in letters that do not pack: one device sends its file whole, and the
    // other is brought the server's.
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut letters = |count: usize| -> String {
        let letter = |_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            char::from(b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_"[(seed >> 58) as usize])
        };
        (0..count).map(letter).collect()
    };
    let paragraphs = MAX_FRAME / (64 << 10) + 32;
    let mut document = here.writer.model.doc().clone();
    document.blocks.extend(
        (0..paragraphs).map(|n| Block::text(format!("p{n}").as_str(), BlockKind::Paragraph, letters(64 << 10))),
    );
    here.writer.write(&document);
    let told = here.tell();
    here.does(told);
    assert_ne!(here.line.progress(), Progress::Saved);

    let met = |desks: &[&mut Desk]| {
        desks
            .iter()
            .all(|desk| live(desk) && desk.reads().len() == paragraphs && desk.line.progress() == Progress::Saved)
    };
    // Nothing here is waited out: the server reads and merges the file for seconds of one core, and so
    // does the other device. A slow machine is given the time.
    let all = settle_within(Duration::from_secs(120), &mut [&mut here, &mut there], met);
    assert!(all, "{:?} {:?} {} {}", here.line.link(), there.line.link(), here.reads().len(), there.reads().len());
    assert!(here.refused.is_empty() && there.refused.is_empty(), "{:?} {:?}", here.refused, there.refused);

    // And the line carries what is written after as it did before.
    there.write("after", "Then the gutters.");
    assert!(settle(&mut [&mut here, &mut there], |desks| desks[0].reads().len() == paragraphs + 1));
}
