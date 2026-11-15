//! Asking a Letters server on this machine: finding it, signing in to it, and holding the session.

mod common;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::Duration;

use common::Scratch;
use letters::connected::{self, Reached, Shared, Sharing, Unreached};
use letters::keychain::Keychain;
use letters::remote::Roots;
use letters::server::{Client, Refused};
use letters::session::Session;
use letters::signin::{SignIn, Unsigned};
use letters_api::{
    Access, Action, Allows, Code, CreatedLink, Exchange, Grant, Grantee, Link, NewLink, Opened, Person, Renewal, Role,
    Shares, Space, SpaceKind, Token, Trust, Who, paths,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Sign-ins are kept in memory here, never in the keychain of whoever runs the tests. Each test asks a
/// server of its own, so none sees what another kept.
static KEYCHAIN: LazyLock<Keychain> = LazyLock::new(|| Keychain::in_memory().unwrap());

/// A time no test lives to see, in seconds since 1970.
const LATER: u64 = u64::MAX / 2;

/// A document's file, as far as a server that reads nothing of it cares.
const FILE: &[u8] = b"a whole document";

/// One request, as the server heard it.
#[derive(Clone, Debug, Default)]
struct Asked {
    /// Such as `GET /api/v1/server`.
    line: String,
    /// The headers, each name in lower case.
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl Asked {
    fn header(&self, name: &str) -> &str {
        self.headers.get(name).map_or("", String::as_str)
    }
}

/// What the server says back: a status line's end, headers and a body.
struct Answer(&'static str, &'static str, Vec<u8>);

fn json(body: &impl Serialize) -> Answer {
    Answer("200 OK", "content-type: application/json\r\n", serde_json::to_vec(body).unwrap())
}

fn said(code: Code, message: &str) -> Answer {
    let status = match code {
        Code::Invalid => "400 Bad Request",
        Code::Forbidden => "403 Forbidden",
        _ => "401 Unauthorized",
    };
    let body = serde_json::to_vec(&letters_api::Error::new(code, message)).unwrap();
    Answer(status, "content-type: application/json\r\n", body)
}

fn ada() -> Person {
    Person { id: "p1".into(), username: "ada".into(), name: "Ada Lovelace".into() }
}

/// The `n`th session the server hands out. The first has run out by the time it is used.
fn token(n: u32) -> Token {
    Token {
        token: format!("token-{n}"),
        expires: if n == 0 { 0 } else { LATER },
        renew: format!("renew-{n}"),
        ends: LATER,
        person: ada(),
    }
}

fn spaces() -> Vec<Space> {
    vec![Space { id: "ada".into(), name: "Ada".into(), kind: SpaceKind::Personal, role: Role::Owner }]
}

/// A small picture's file.
fn png() -> Vec<u8> {
    letters::pictures::from_rgba(4, 2, &[0x80; 32]).unwrap().bytes
}

/// The name the server holds that picture under: the hash of its bytes.
fn held() -> String {
    let hash: String = Sha256::digest(png()).iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{hash}.png")
}

/// What the server says of its one document.
fn opened() -> Opened {
    Opened {
        id: "d1".into(),
        created: "2026-11-15T04:00:00Z".into(),
        name: "Roof repairs".into(),
        named: true,
        space: Some("ada".into()),
        path: "House".into(),
        role: Role::Editor,
        author: "Ada Lovelace".into(),
        socket: paths::socket("d1"),
        trust: Trust::Trusted,
        allows: Allows::default(),
        style: None,
        notes: None,
    }
}

/// What the Letters server of these tests knows.
#[derive(Default)]
struct Letters {
    /// How many sessions it handed out since the first. Only the latest works.
    issued: u32,
    /// The challenge each code was made for, as its sign-in page noted when the browser was there.
    codes: HashMap<String, String>,
    /// Takes the next token shown for run out, whatever its time says.
    strict: bool,
    signed_out: bool,
    /// The person may no longer read the document.
    barred: bool,
    /// Who the document is shared with.
    shares: Shares,
}

impl Letters {
    fn answer(&mut self, asked: &Asked) -> Answer {
        let next = |letters: &mut Self| {
            letters.issued += 1;
            json(&token(letters.issued))
        };
        match asked.line.as_str() {
            "GET /api/v1/server" => json(&serde_json::json!({
                "product": "letters-server", "name": "Home", "version": "0.1.0", "protocols": [1],
            })),
            "POST /api/v1/session/exchange" => {
                let Exchange { code, verifier } = serde_json::from_slice(&asked.body).unwrap();
                match self.codes.remove(&code) {
                    Some(challenge) if challenge == base64(&Sha256::digest(verifier)) => next(self),
                    _ => said(Code::Invalid, "That code is not one of this server's."),
                }
            }
            "POST /api/v1/session/renew" => {
                let Renewal { renew } = serde_json::from_slice(&asked.body).unwrap();
                if self.signed_out || renew != format!("renew-{}", self.issued) {
                    return said(Code::SignedOut, "Sign in again.");
                }
                next(self)
            }
            _ if self.signed_out || asked.header("authorization") != format!("Bearer token-{}", self.issued) => {
                said(Code::SignedOut, "Sign in again.")
            }
            _ if std::mem::take(&mut self.strict) => said(Code::Expired, "The token has run out."),
            "POST /api/v1/session/sign-out" => {
                self.signed_out = true;
                Answer("204 No Content", "", Vec::new())
            }
            "GET /api/v1/spaces" => json(&spaces()),
            "GET /api/v1/documents/d1" if self.barred => said(Code::Forbidden, "That is not yours to read."),
            "GET /api/v1/documents/d1" => json(&opened()),
            "GET /api/v1/documents/d1/file" => Answer("200 OK", "content-type: application/zip\r\n", FILE.to_vec()),
            "PUT /api/v1/documents/d1/assets/sky.png" => Answer("204 No Content", "", Vec::new()),
            // A picture is asked for by the hash of its bytes, and one name gets another's bytes.
            line if line.strip_prefix("GET /api/v1/documents/d1/assets/").is_some_and(|name| name == held()) => {
                Answer("200 OK", "content-type: image/png\r\n", png())
            }
            "GET /api/v1/documents/d1/assets/0000.png" => Answer("200 OK", "content-type: image/png\r\n", png()),
            "GET /api/v1/documents/d1/shares" => json(&self.shares),
            "PUT /api/v1/documents/d1/shares" => {
                let Shares { people, .. } = serde_json::from_slice(&asked.body).unwrap();
                self.shares.people = people;
                json(&self.shares)
            }
            "POST /api/v1/documents/d1/links" => {
                let NewLink { label, role, expires, allows, .. } = serde_json::from_slice(&asked.body).unwrap();
                let id = format!("link-{}", self.shares.links.len() + 1);
                let link = Link { id, label, role, expires, passphrase: false, allows, created: 1, revoked: None };
                self.shares.links.push(link.clone());
                json(&CreatedLink { url: format!("https://localhost/l/secret-of-{}", link.id), link })
            }
            line if line.starts_with("DELETE /api/v1/documents/d1/links/") => {
                let id = line.rsplit('/').next().unwrap_or_default();
                match self.shares.links.iter_mut().find(|link| link.id == id) {
                    Some(link) => link.revoked = Some(2),
                    None => return said(Code::Forbidden, "That link is not this document's."),
                }
                Answer("204 No Content", "", Vec::new())
            }
            "GET /api/v1/documents/d1/access" => json(&[Access {
                time: 3,
                who: Who::Link { id: "link-1".into(), label: "Accountant".into() },
                action: Action::Open,
                role: Role::Reader,
                refused: false,
                address: "192.0.2.7".into(),
                browser: "Firefox".into(),
            }]),
            "GET /api/v1/forbidden" => said(Code::Forbidden, "That is not yours to read."),
            "GET /api/v1/elsewhere" => Answer("302 Found", "location: https://elsewhere.example/\r\n", Vec::new()),
            "GET /api/v1/page" => {
                Answer("200 OK", "content-type: text/html\r\n", b"<p>Sign in to the Wi-Fi</p>".to_vec())
            }
            "GET /api/v1/broken" => {
                Answer("502 Bad Gateway", "content-type: text/html\r\n", b"<p>Bad gateway</p>".to_vec())
            }
            _ => Answer("404 Not Found", "", Vec::new()),
        }
    }
}

/// A Letters server on this machine, which answers over HTTPS as `localhost`.
struct Server {
    port: u16,
    /// The certificate it shows, which nobody but a test trusts.
    certificate: Vec<u8>,
    letters: Arc<Mutex<Letters>>,
    asked: Arc<Mutex<Vec<Asked>>>,
}

impl Server {
    fn start() -> Self {
        let (config, certificate) = common::tls();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (letters, asked) = (Arc::new(Mutex::new(Letters::default())), Arc::new(Mutex::new(Vec::new())));
        let (knows, heard) = (letters.clone(), asked.clone());
        thread::spawn(move || {
            for socket in listener.incoming().flatten() {
                let Ok(connection) = rustls::ServerConnection::new(config.clone()) else { continue };
                let mut stream = rustls::StreamOwned::new(connection, socket);
                // A client that does not believe the certificate asks nothing.
                let Some(request) = hear(&mut stream) else { continue };
                let Answer(status, headers, body) = knows.lock().unwrap().answer(&request);
                heard.lock().unwrap().push(request);
                let head = format!(
                    "HTTP/1.1 {status}\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(&[head.as_bytes(), &body].concat());
                stream.conn.send_close_notify();
                let _ = stream.flush();
            }
        });
        Self { port, certificate, letters, asked }
    }

    fn client(&self) -> Client {
        Client::new(&format!("localhost:{}", self.port), &Roots::These(vec![self.certificate.clone()])).unwrap()
    }

    /// A session that begins with the first token, which has run out.
    fn session(&self) -> Session {
        Session::begin(self.client(), Some(*KEYCHAIN), token(0))
    }

    /// What was asked since the last look, as each request's line and the token it showed.
    fn asked(&self) -> Vec<(String, String)> {
        let asked = std::mem::take(&mut *self.asked.lock().unwrap());
        asked.iter().map(|asked| (asked.line.clone(), asked.header("authorization").to_owned())).collect()
    }
}

/// Reads one request: its line, its headers and as much body as it says it has.
fn hear(stream: &mut impl Read) -> Option<Asked> {
    let (mut head, mut byte) = (Vec::new(), [0]);
    while !head.ends_with(b"\r\n\r\n") {
        if !matches!(stream.read(&mut byte), Ok(1)) {
            return None;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).ok()?;
    let mut lines = head.lines();
    let line = lines.next()?.rsplit_once(' ')?.0.to_owned();
    let headers: HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let mut body = vec![0; headers.get("content-length").and_then(|length| length.parse().ok()).unwrap_or(0)];
    stream.read_exact(&mut body).ok()?;
    Some(Asked { line, headers, body })
}

/// `bytes` in base64 for addresses, without padding.
fn base64(bytes: &[u8]) -> String {
    let letters: Vec<char> = ('A'..='Z').chain('a'..='z').chain('0'..='9').chain(['-', '_']).collect();
    let bits: String = bytes.iter().map(|byte| format!("{byte:08b}")).collect();
    let sixes = bits.as_bytes().chunks(6).map(|six| format!("{:0<6}", std::str::from_utf8(six).unwrap()));
    sixes.map(|six| letters[usize::from_str_radix(&six, 2).unwrap()]).collect()
}

/// Asks `GET target` of what listens on this machine at `port`, as a browser sent there does, and gives
/// the answer.
fn browse(port: u16, target: &str) -> String {
    let mut browser = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(browser, "GET {target} HTTP/1.1\r\nhost: 127.0.0.1:{port}\r\naccept: text/html\r\n\r\n").unwrap();
    let mut answer = String::new();
    browser.read_to_string(&mut answer).unwrap();
    answer
}

/// The value of `name` in the query of an address the app made.
fn query(address: &str, name: &str) -> String {
    let (_, query) = address.split_once('?').unwrap();
    let mut parts = query.split('&').filter_map(|part| part.split_once('='));
    parts.find(|(key, _)| *key == name).unwrap().1.to_owned()
}

#[test]
fn a_server_is_named_by_an_https_address_only() {
    let cases = [
        ("letters.example", Some("https://letters.example")),
        ("  https://letters.example:8443/  ", Some("https://letters.example:8443")),
        ("letters.example/some/page", Some("https://letters.example")),
        ("http://letters.example", None),
        ("wss://letters.example", None),
        ("https://ada:secret@letters.example", None),
        ("", None),
    ];
    for (typed, expected) in cases {
        let client = Client::new(typed, &Roots::System);

        assert_eq!(client.as_ref().map(Client::origin).ok(), expected, "{typed:?}");
        if expected.is_none() {
            assert_eq!(client.unwrap_err(), Refused::Address, "{typed:?}");
        }
    }
    let client = Client::new("letters.example:8443", &Roots::System).unwrap();
    assert_eq!(client.socket("/api/v1/documents/d1/sync"), "wss://letters.example:8443/api/v1/documents/d1/sync");
}

#[test]
fn a_letters_server_is_told_from_whatever_else_answers() {
    let server = Server::start();

    let found = server.client().discover().expect("a Letters server answers there");
    assert!(found.speaks());
    assert_eq!(found.name, "Home");
    // Nobody has signed in yet, so nothing of a sign-in is shown.
    let asked = server.asked.lock().unwrap().clone();
    assert_eq!(asked[0].header("authorization"), "");
    assert_eq!(asked[0].header("user-agent"), "Letters");

    // A certificate nobody vouches for is no server to sign in to.
    let stranger = Client::new(&format!("localhost:{}", server.port), &Roots::These(Vec::new())).unwrap();
    assert!(matches!(stranger.discover(), Err(Refused::Unreachable(_))));
}

#[test]
fn a_refusal_says_what_the_server_said() {
    let server = Server::start();
    server.letters.lock().unwrap().issued = 1;
    let client = server.client();
    let cases = [
        ("/api/v1/forbidden", Refused::Said(letters_api::Error::new(Code::Forbidden, "That is not yours to read."))),
        // What stands in front of a server answers in its own words.
        ("/api/v1/broken", Refused::Status(502)),
        ("/api/v1/page", Refused::NotLetters),
        // An answer that points elsewhere is not followed, and the token goes nowhere else.
        ("/api/v1/elsewhere", Refused::Status(302)),
        ("/api/v1/nothing", Refused::Status(404)),
    ];
    let asks = cases.len();
    for (path, expected) in cases {
        let answer = client.get::<Vec<Space>>("token-1", path);

        assert_eq!(answer, Err(expected), "{path}");
    }
    assert_eq!(server.asked().len(), asks, "each is asked once, and nothing after it");

    let unknown = client.get::<Vec<Space>>("token-0", paths::SPACES).unwrap_err();
    assert_eq!(unknown.code(), Some(Code::SignedOut));
}

#[test]
fn a_document_and_its_pictures_travel_whole() {
    let server = Server::start();
    server.letters.lock().unwrap().issued = 1;
    let client = server.client();
    let picture = png();

    assert_eq!(client.file("token-1", "d1", 1024).as_deref(), Ok(FILE));
    assert_eq!(client.file("token-1", "d1", 4), Err(Refused::TooLarge));
    assert_eq!(client.keep_asset("token-1", "d1", "sky.png", &picture), Ok(()));

    let asked = server.asked.lock().unwrap().clone();
    let put = asked.last().unwrap();
    assert_eq!(put.body, picture);
    assert_eq!(put.header("content-type"), "image/png");
}

#[test]
fn signing_in_trades_what_the_browser_brings_for_a_session() {
    let server = Server::start();
    let client = server.client();
    let sign_in = SignIn::begin().unwrap();
    let page = sign_in.page(&client).unwrap();
    assert!(page.starts_with(&format!("{}/auth/desktop?", client.origin())), "{page}");
    let (port, state) = (query(&page, "port").parse::<u16>().unwrap(), query(&page, "state"));
    // The person signs in at the server's page, which notes the challenge and makes up a code.
    server.letters.lock().unwrap().codes.insert("c0de/1".to_owned(), query(&page, "challenge"));

    let browser = thread::spawn(move || {
        let stray = browse(port, "/?code=mine&state=guessed");
        let favicon = browse(port, "/favicon.ico");
        let back = browse(port, &format!("/?code=c0de%2F1&state={state}"));
        (stray, favicon, back)
    });
    let token = sign_in.finish(&client, Duration::from_secs(30), &AtomicBool::new(false));
    let (stray, favicon, back) = browser.join().unwrap();

    assert_eq!(token.map(|token| token.person), Ok(ada()));
    // Only the browser that was sent ends the wait. Anything else on this machine is turned away.
    assert!(stray.starts_with("HTTP/1.1 400") && favicon.starts_with("HTTP/1.1 400"), "{stray}{favicon}");
    assert!(back.starts_with("HTTP/1.1 200") && back.contains("Letters"), "{back}");
    let asked = server.asked();
    assert_eq!(asked, [("POST /api/v1/session/exchange".to_owned(), String::new())]);
}

#[test]
fn a_sign_in_nobody_finishes_ends() {
    let server = Server::start();
    let client = server.client();

    let waited = SignIn::begin().unwrap().finish(&client, Duration::ZERO, &AtomicBool::new(false));
    let given_up = SignIn::begin().unwrap().finish(&client, Duration::from_secs(30), &AtomicBool::new(true));

    assert_eq!(waited, Err(Unsigned::Waited));
    assert_eq!(given_up, Err(Unsigned::GivenUp));
    assert_eq!(server.asked(), [], "the server is asked nothing");
}

#[test]
fn a_code_from_another_sign_in_buys_nothing() {
    let server = Server::start();
    let client = server.client();
    let (sign_in, other) = (SignIn::begin().unwrap(), SignIn::begin().unwrap());
    let page = sign_in.page(&client).unwrap();
    let (port, state) = (query(&page, "port").parse::<u16>().unwrap(), query(&page, "state"));
    // The code was made for the other sign-in's secret.
    let challenge = query(&other.page(&client).unwrap(), "challenge");
    server.letters.lock().unwrap().codes.insert("stolen".to_owned(), challenge);

    let browser = thread::spawn(move || browse(port, &format!("/?code=stolen&state={state}")));
    let token = sign_in.finish(&client, Duration::from_secs(30), &AtomicBool::new(false));
    browser.join().unwrap();

    assert!(matches!(token, Err(Unsigned::Refused(refused)) if refused.code() == Some(Code::Invalid)));
}

#[test]
fn a_session_renews_a_token_that_ran_out_and_keeps_the_next() {
    let server = Server::start();
    let session = server.session();

    assert_eq!(session.spaces(), Ok(spaces()));
    assert_eq!(session.unkept(), None);
    let renewed = [
        ("POST /api/v1/session/renew".to_owned(), String::new()),
        ("GET /api/v1/spaces".to_owned(), "Bearer token-1".to_owned()),
    ];
    assert_eq!(server.asked(), renewed);

    // The server's clock decides when a token has run out.
    server.letters.lock().unwrap().strict = true;
    assert_eq!(session.file("d1", 1024).as_deref(), Ok(FILE));
    let again = server.asked();
    assert_eq!(again.len(), 3, "{again:?}");
    assert_eq!(again[2].1, "Bearer token-2");

    // The app is opened again: it is signed in, with the session that was renewed last.
    let later = Session::kept(server.client(), *KEYCHAIN).unwrap().expect("the sign-in was kept");
    assert_eq!(later.person(), ada());
    assert_eq!(later.token().as_deref(), Ok("token-2"));
    assert_eq!(server.asked(), [], "a token with time left is not renewed");
}

#[test]
fn two_holders_of_one_sign_in_do_not_end_it_for_each_other() {
    let server = Server::start();
    let (one, other) = (server.session(), server.session());

    assert_eq!(one.spaces(), Ok(spaces()));
    // The other's renewal is spent. What the first kept is the session for both now.
    assert_eq!(other.spaces(), Ok(spaces()));

    assert_eq!(server.asked().last().unwrap().1, "Bearer token-1");
}

#[test]
fn a_session_the_server_ended_asks_for_a_new_sign_in() {
    let server = Server::start();
    let session = server.session();
    server.letters.lock().unwrap().signed_out = true;

    let refused = session.spaces().unwrap_err();
    assert_eq!(refused.code(), Some(Code::SignedOut));
    assert_eq!(server.asked().len(), 1, "only the renewal was tried");

    // Nothing more is asked with what is left, and nothing of it is kept.
    assert_eq!(session.token().unwrap_err().code(), Some(Code::SignedOut));
    assert_eq!(server.asked(), []);
    assert!(Session::kept(server.client(), *KEYCHAIN).unwrap().is_none());
}

#[test]
fn a_server_out_of_reach_ends_nothing() {
    let server = Server::start();
    let session = server.session();
    // The same sign-in, asked of a server whose certificate is not believed just now.
    let nowhere = Client::new(server.client().origin(), &Roots::These(Vec::new())).unwrap();
    let unreachable = Session::begin(nowhere, Some(*KEYCHAIN), token(0));

    assert!(matches!(unreachable.spaces(), Err(Refused::Unreachable(_))));

    assert_eq!(session.spaces(), Ok(spaces()), "the sign-in still works once the server is back");
}

#[test]
fn signing_out_ends_the_session_here_and_there() {
    let server = Server::start();
    let session = server.session();
    assert_eq!(session.spaces(), Ok(spaces()));
    server.asked();

    assert_eq!(session.sign_out(), Ok(()));

    assert_eq!(server.asked(), [("POST /api/v1/session/sign-out".to_owned(), "Bearer token-1".to_owned())]);
    assert!(server.letters.lock().unwrap().signed_out);
    assert_eq!(session.spaces().unwrap_err().code(), Some(Code::SignedOut));
    assert!(Session::kept(server.client(), *KEYCHAIN).unwrap().is_none());
}

#[test]
fn the_address_of_a_documents_page_names_its_server_and_the_document() {
    let at = |server: &str, id: &str| Some((server.to_owned(), id.to_owned()));
    let cases = [
        ("https://letters.example/d/abc", at("https://letters.example", "abc")),
        ("  https://letters.example:8443/d/a%2Fb/read ", at("https://letters.example:8443", "a/b")),
        ("https://letters.example/d/abc/", at("https://letters.example", "abc")),
        ("https://letters.example/d/", None),
        ("https://letters.example/d/abc/shares", None),
        ("https://letters.example/s/ada", None),
        ("http://letters.example/d/abc", None),
        ("/home/ada/d/abc", None),
        ("Roof repairs.let", None),
    ];
    for (typed, expected) in cases {
        assert_eq!(connected::address(typed), expected, "{typed:?}");
    }
}

#[test]
fn no_id_leads_a_copy_out_of_its_servers_folder() {
    let home = Scratch::new("connected-cache");
    let client = Client::new("letters.example:8443", &Roots::System).unwrap();
    let ids = ["abc", "ABC", "../../etc/passwd", "..", ".", ".hidden", "a/b", "a\\b", "a%2Fb", ""];

    let copies: Vec<_> = ids.iter().map(|id| connected::cached(&home.0, &client, id)).collect();

    let folder = copies[0].parent().unwrap();
    assert!(folder.starts_with(home.0.join(".letters").join("servers")), "{folder:?}");
    for (id, copy) in ids.iter().zip(&copies) {
        assert_eq!(copy.parent(), Some(folder), "{id:?}");
        assert_eq!(copy.extension().and_then(|ending| ending.to_str()), Some("let"), "{id:?}");
        assert_eq!(copies.iter().filter(|other| *other == copy).count(), 1, "{id:?} has a copy of its own");
    }
    let elsewhere = Client::new("letters.example", &Roots::System).unwrap();
    assert_ne!(connected::cached(&home.0, &elsewhere, "abc").parent(), Some(folder));
}

#[test]
fn a_servers_document_is_brought_here_once_and_opens_with_no_server_in_reach() {
    let (home, server) = (Scratch::new("connected-reach"), Server::start());
    let session = server.session();
    let file = ("GET /api/v1/documents/d1/file".to_owned(), "Bearer token-1".to_owned());
    // The same sign-in, asked of a server whose certificate is not believed just now.
    let nowhere = Client::new(server.client().origin(), &Roots::These(Vec::new())).unwrap();
    let offline = Session::begin(nowhere, Some(*KEYCHAIN), token(1));

    // With no copy here, a server out of reach has nothing to open.
    assert!(matches!(connected::reach(&offline, "d1", &home.0), Err(Unreached::Refused(Refused::Unreachable(_)))));

    let first = connected::reach(&session, "d1", &home.0).unwrap();
    assert_eq!(
        first,
        Reached { opened: opened(), path: connected::cached(&home.0, session.server(), "d1"), online: true }
    );
    assert_eq!(std::fs::read(&first.path).unwrap(), FILE);
    assert!(server.asked().contains(&file));

    // The copy is the document from here on: the line brings what changed, not the file again.
    std::fs::write(&first.path, b"written in since").unwrap();
    assert_eq!(connected::reach(&session, "d1", &home.0).unwrap(), first);
    assert!(!server.asked().contains(&file));

    let later = connected::reach(&offline, "d1", &home.0).unwrap();
    assert_eq!(later, Reached { online: false, ..first.clone() });
    assert_eq!(std::fs::read(&first.path).unwrap(), b"written in since");
}

#[test]
fn a_document_the_person_may_no_longer_read_is_not_kept_for_them() {
    let (home, server) = (Scratch::new("connected-barred"), Server::start());
    let session = server.session();
    let copy = connected::reach(&session, "d1", &home.0).unwrap().path;
    server.letters.lock().unwrap().barred = true;

    let refused = connected::reach(&session, "d1", &home.0);

    assert!(matches!(refused, Err(Unreached::Refused(refused)) if refused.code() == Some(Code::Forbidden)));
    assert!(!copy.exists());
}

/// What the system's browser does with the page it is sent to, when the person signs in there: the
/// server notes the challenge and sends the browser back to the app with a code.
fn sign_in_at(server: &Server) -> impl Fn(&str) -> std::io::Result<()> + '_ {
    |page| {
        let (port, state) = (query(page, "port").parse::<u16>().unwrap(), query(page, "state"));
        server.letters.lock().unwrap().codes.insert("c0de".to_owned(), query(page, "challenge"));
        thread::spawn(move || browse(port, &format!("/?code=c0de&state={state}")));
        Ok(())
    }
}

#[test]
fn a_document_is_entered_with_the_session_at_hand_or_a_new_sign_in() {
    let (home, server) = (Scratch::new("connected-enter"), Server::start());
    let client = server.client();
    let (waiting, exchange) = (AtomicBool::new(false), "POST /api/v1/session/exchange".to_owned());
    let exchanges = |server: &Server| server.asked().iter().filter(|(line, _)| *line == exchange).count();
    let no_browser = |_: &str| -> std::io::Result<()> { panic!("nobody is sent to sign in") };

    // Nobody is signed in: the browser is, and the session is kept.
    let (first, reached) =
        connected::enter(&client, "d1", None, Some(*KEYCHAIN), &home.0, &sign_in_at(&server), &waiting).unwrap();
    assert_eq!((first.person(), reached.opened, reached.online), (ada(), opened(), true));
    assert_eq!(exchanges(&server), 1);

    // The session the app holds opens the next document, and the one that was kept opens it after a restart.
    for held in [Some(first.clone()), None] {
        let (session, _) =
            connected::enter(&client, "d1", held, Some(*KEYCHAIN), &home.0, &no_browser, &waiting).unwrap();
        assert_eq!(session.person(), ada());
    }
    assert_eq!(exchanges(&server), 0);

    // A session the server ended is followed by a new sign-in.
    server.letters.lock().unwrap().signed_out = true;
    let again = |page: &str| {
        server.letters.lock().unwrap().signed_out = false;
        sign_in_at(&server)(page)
    };
    let (session, reached) =
        connected::enter(&client, "d1", Some(first), Some(*KEYCHAIN), &home.0, &again, &waiting).unwrap();
    assert!(reached.online && !session.ended());
    assert_eq!(exchanges(&server), 1);
}

#[test]
fn nobody_is_sent_to_sign_in_at_what_is_no_letters_server() {
    let (home, server) = (Scratch::new("connected-stranger"), Server::start());
    // What answers there is believed by nobody.
    let stranger = Client::new(server.client().origin(), &Roots::These(Vec::new())).unwrap();
    let no_browser = |_: &str| -> std::io::Result<()> { panic!("nobody is sent to sign in") };

    let entered = connected::enter(&stranger, "d1", None, None, &home.0, &no_browser, &AtomicBool::new(false));

    assert!(matches!(entered, Err(Unreached::Refused(Refused::Unreachable(_)))));
}

#[test]
fn a_picture_the_copy_lacks_is_had_from_the_server_only_as_what_its_name_says() {
    let server = Server::start();
    let session = server.session();
    let cases = [
        (format!("assets/{}", held()), Some(png())),
        // Bytes that are not what the name says are no picture of the document.
        ("assets/0000.png".to_owned(), None),
        ("assets/gone.png".to_owned(), None),
        (held(), None),
    ];
    for (name, expected) in cases {
        assert_eq!(connected::picture(&session, "d1", &name), expected, "{name}");
    }
}

#[test]
fn the_share_dialog_is_answered_by_the_documents_server() {
    let server = Server::start();
    let session = server.session();
    let share = |ask: Sharing| connected::share(&session, "d1", &ask);
    let family = Grant { to: Grantee::Group { name: "family".into() }, role: Role::Commenter };
    let new = NewLink {
        label: "Accountant".into(),
        role: Role::Reader,
        expires: None,
        passphrase: None,
        allows: Allows::default(),
    };

    assert_eq!(share(Sharing::Look), Shared::Shares(Shares::default(), false));
    // Its people are replaced whole, and the answer says who they are now.
    let Shared::Shares(now, _) = share(Sharing::People(vec![family.clone()])) else { panic!("the people are told") };
    assert_eq!(now.people, [family]);
    // A link's address is told once, when it is made. Afterwards it is listed, and can be revoked.
    let Shared::Made(made) = share(Sharing::Link(new)) else { panic!("the link is made") };
    assert!(made.url.contains(&made.link.id) && made.link.label == "Accountant", "{made:?}");
    assert_eq!(share(Sharing::Revoke(made.link.id.clone())), Shared::Revoked);
    let Shared::Shares(now, _) = share(Sharing::Look) else { panic!("the shares are told") };
    assert!(now.links.iter().any(|link| link.id == made.link.id && link.revoked.is_some()), "{now:?}");

    // Who came through a share is listed as the server logged it.
    let Shared::Log(log) = share(Sharing::Log) else { panic!("the log is told") };
    assert!(matches!(&log[..], [Access { who: Who::Link { label, .. }, .. }] if label == "Accountant"), "{log:?}");

    // A refusal is the server's own sentence, and a server that shares nothing yet is named as one.
    assert_eq!(
        share(Sharing::Revoke("no-such-link".into())),
        Shared::Refused("That link is not this document's.".into())
    );
    let elsewhere = connected::share(&session, "d2", &Sharing::Look);
    assert_eq!(elsewhere, Shared::Refused("This server does not share documents yet.".into()));
}
