//! The desktop's side of connected mode against the server itself: `letters-server`, running on this
//! machine over scratch folders, behind a certificate nobody but a test trusts.

mod common;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::thread;
use std::time::Duration;

use common::Scratch;
use letters::connected;
use letters::files;
use letters::keychain::Keychain;
use letters::remote::Roots;
use letters::server::Client;
use letters::session::Session;
use letters::signin::SignIn;
use letters_api::{Entry, EntryKind, NewDocument, Person, Role, SpaceKind, Trust, paths};
use letters_model::Style;
use letters_server::config::Config;
use letters_server::session::{COOKIE_NAME, Carrier};
use letters_server::store::Store;
use letters_server::{App, api};

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
    _scratch: Scratch,
    _runtime: tokio::runtime::Runtime,
}

impl Server {
    fn start(name: &str) -> Self {
        let scratch = Scratch::new(name);
        let (tls, certificate) = common::tls();
        let front = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = front.local_addr().unwrap().port();
        let at = scratch.0.display();
        let settings = format!(
            r#"
[server]
name = "Home"
url = "https://localhost:{port}"
state = "{at}/state"

[auth.oidc]
issuer = "https://id.example.test"
client_id = "letters"

[[space]]
name = "Home"
path = "{at}/home/{{username}}"
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
        // The server speaks plain HTTP and expects TLS in front of it, as it has wherever it runs.
        thread::spawn(move || {
            for socket in front.incoming().flatten() {
                let Ok(connection) = rustls::ServerConnection::new(tls.clone()) else { continue };
                let mut stream = rustls::StreamOwned::new(connection, socket);
                thread::spawn(move || {
                    let _ = pass(&mut stream, behind);
                    stream.conn.send_close_notify();
                    let _ = stream.flush();
                });
            }
        });
        Self { app, port, certificate, clock, _scratch: scratch, _runtime: runtime }
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

/// Passes one request on to the server behind, and its answer back.
fn pass(stream: &mut (impl Read + Write), behind: SocketAddr) -> std::io::Result<()> {
    let (mut head, mut byte) = (Vec::new(), [0]);
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte)?;
        head.push(byte[0]);
    }
    let said = String::from_utf8_lossy(&head).to_ascii_lowercase();
    let length = said.lines().find_map(|line| line.strip_prefix("content-length:")?.trim().parse().ok());
    let mut body = vec![0; length.unwrap_or(0)];
    stream.read_exact(&mut body)?;
    // One request to a connection, so that the answer ends where the server hangs up.
    let line = head.iter().position(|byte| *byte == b'\n').unwrap_or(0) + 1;
    let mut server = TcpStream::connect(behind)?;
    server.write_all(&[&head[..line], b"connection: close\r\n", &head[line..], &body].concat())?;
    let mut answer = Vec::new();
    server.read_to_end(&mut answer)?;
    stream.write_all(&answer)
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
    let (kept, again) =
        connected::enter(&client, &id, None, Some(*KEYCHAIN), &home.0, &no_browser, &AtomicBool::new(false)).unwrap();
    assert_eq!((kept.person(), again.path), (ada(), reached.path));

    // Signed out, nothing more is had with what was kept.
    assert_eq!(kept.sign_out(), Ok(()));
    assert!(Session::kept(client.clone(), *KEYCHAIN).unwrap().is_none());
    assert!(session.spaces().is_err(), "the server ended the session for every holder");
}
