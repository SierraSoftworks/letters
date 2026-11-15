//! Fetching a picture that is still a link, from a server on this machine.

mod common;

use std::io::{Read, Write};
use std::net::{TcpListener, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::thread;

use common::Scratch;
use letters::remote::{self, Failed, Roots};
use letters::trust::{self, Trusted};
use letters_trust::{FetchError, ImageFetch, ImageType, Limits, Origin, Source};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// The largest picture the fetches here take, in bytes.
const MOST: u64 = 2048;

/// A page with no picture in it.
const PAGE: &[u8] = b"<p>No picture</p>";

/// A page that says what it is and names its picture by `address`.
fn article(address: &str) -> Vec<u8> {
    let head =
        r#"<meta property="og:title" content="Roof repairs"><meta property="og:description" content="A quote.">"#;
    format!(r#"<html><head>{head}<meta property="og:image" content="{address}"></head></html>"#).into_bytes()
}

/// A small picture's file.
fn png() -> Vec<u8> {
    letters::pictures::from_rgba(4, 2, &[0x80; 32]).unwrap().bytes
}

/// What the server answers when it is asked for `path`.
fn answer(path: &str) -> Vec<u8> {
    let with = |status: &str, headers: &str, body: &[u8]| {
        let head = format!("HTTP/1.1 {status}\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n", body.len());
        [head.as_bytes(), body].concat()
    };
    match path {
        "/sky.png" => with("200 OK", "content-type: image/png\r\n", &png()),
        "/moved" => with("302 Found", "location: /sky.png\r\n", b""),
        "/round" => with("302 Found", "location: /round\r\n", b""),
        "/away" => with("302 Found", "location: https://elsewhere.example/sky.png\r\n", b""),
        "/page" => with("200 OK", "content-type: text/html; charset=utf-8\r\n", PAGE),
        "/moved/page" => with("302 Found", "location: /page\r\n", b""),
        "/article" => with("200 OK", "content-type: text/html\r\n", &article("pictures/sky.png")),
        "/pictures/sky.png" => with("200 OK", "content-type: image/png\r\n", &png()),
        "/article/lost" => with("200 OK", "content-type: text/html\r\n", &article("/nothing")),
        "/article/plain" => with("200 OK", "content-type: text/html\r\n", &article("http://localhost/sky.png")),
        "/long" => with("200 OK", "content-type: text/html\r\n", &[PAGE, &vec![b' '; MOST as usize]].concat()),
        "/unlabelled" => with("200 OK", "", b"words, with no word on what they are"),
        "/huge" => with("200 OK", "content-type: image/png\r\n", &[png(), vec![0; MOST as usize]].concat()),
        _ => with("404 Not Found", "", b""),
    }
}

/// A server on this machine that answers over HTTPS as `localhost`.
struct Server {
    port: u16,
    /// The certificate it shows, which nobody but a test trusts.
    certificate: Vec<u8>,
    /// What it was asked, each request in lower case.
    asked: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn start() -> Self {
        let made = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
        let certificate = made.cert.der().to_vec();
        let key = PrivateKeyDer::try_from(made.signing_key.serialize_der()).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![CertificateDer::from(certificate.clone())], key)
            .unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let (config, heard) = (Arc::new(config), asked.clone());
        thread::spawn(move || {
            for socket in listener.incoming().flatten() {
                let Ok(connection) = rustls::ServerConnection::new(config.clone()) else { continue };
                let mut stream = rustls::StreamOwned::new(connection, socket);
                let (mut request, mut byte) = (Vec::new(), [0]);
                while !request.ends_with(b"\r\n\r\n") && matches!(stream.read(&mut byte), Ok(1)) {
                    request.push(byte[0]);
                }
                // A client that does not believe the certificate asks nothing.
                if !request.ends_with(b"\r\n\r\n") {
                    continue;
                }
                let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
                let path = request.split(' ').nth(1).unwrap_or_default().to_owned();
                heard.lock().unwrap().push(request);
                let _ = stream.write_all(&answer(&path));
                stream.conn.send_close_notify();
                let _ = stream.flush();
            }
        });
        Self { port, certificate, asked }
    }

    fn address(&self, path: &str) -> String {
        format!("https://localhost:{}{path}", self.port)
    }

    fn roots(&self) -> Roots {
        Roots::These(vec![self.certificate.clone()])
    }
}

/// What a person trusts who trusts pictures from `server`, which is on this machine.
fn trusting(home: &Scratch, server: &Server) -> Trusted {
    let mut store = trust::store(&home.0);
    store.load().unwrap();
    store.trust_source(&Source::parse(&server.address("/")).unwrap()).unwrap();
    store
}

/// The guard on a fetch of `address` as a document made here opens.
fn on_open(store: &Trusted, address: &str) -> Result<ImageFetch, FetchError> {
    let limits = Limits { max_bytes: MOST, ..Limits::default() };
    store.trust().gate("document", Origin::Created).fetch_on_open(address, limits)
}

#[test]
fn a_linked_picture_is_fetched_only_as_its_guard_allows() {
    let (home, server) = (Scratch::new("remote-fetch"), Server::start());
    let store = trusting(&home, &server);
    let refused = |error| Err(Failed::Refused(error));
    let cases = [
        ("/sky.png", Ok(ImageType::Png)),
        ("/moved", Ok(ImageType::Png)),
        ("/round", refused(FetchError::TooManyRedirects)),
        ("/away", refused(FetchError::UntrustedSource)),
        ("/page", refused(FetchError::NotAnImage)),
        ("/unlabelled", refused(FetchError::NotAnImage)),
        ("/huge", refused(FetchError::TooLarge(MOST))),
        ("/nothing", Err(Failed::Status(404))),
    ];
    for (path, expected) in cases {
        let guard = on_open(&store, &server.address(path)).expect("a trusted source may be asked");
        let fetched = remote::fetch(guard, &server.roots());

        assert_eq!(fetched.as_ref().map(|fetched| fetched.kind).map_err(Clone::clone), expected, "{path}");
        if let Ok(fetched) = fetched {
            assert_eq!(fetched.bytes, png(), "{path}");
        }
    }
}

#[test]
fn a_page_is_fetched_as_a_picture_is_and_says_where_it_was_found() {
    let (home, server) = (Scratch::new("remote-page"), Server::start());
    let store = trusting(&home, &server);
    let refused = |error| Err(Failed::Refused(error));
    let cases = [
        ("/page", Ok("/page")),
        ("/moved/page", Ok("/page")),
        ("/sky.png", refused(FetchError::NotAPage)),
        ("/unlabelled", refused(FetchError::NotAPage)),
        ("/long", refused(FetchError::TooLarge(MOST))),
        ("/nothing", Err(Failed::Status(404))),
    ];
    for (path, expected) in cases {
        let limits = Limits { max_bytes: MOST, ..Limits::page() };
        let guard = store.trust().gate("document", Origin::Created).fetch_page(&server.address(path), limits);
        let fetched = remote::fetch_page(guard.expect("a page may be asked for"), &server.roots());

        let found = fetched.as_ref().map(|page| page.url.to_string()).map_err(Clone::clone);
        assert_eq!(found, expected.map(|path| server.address(path)), "{path}");
        if let Ok(page) = fetched {
            assert_eq!(page.bytes, PAGE, "{path}");
        }
    }
    let asked = server.asked.lock().unwrap();
    assert!(asked.iter().all(|request| request.contains("accept: text/html")), "a page is asked for as one");
}

#[test]
fn a_preview_is_what_a_page_says_of_itself_with_its_picture_if_that_may_be_had() {
    let (home, server) = (Scratch::new("remote-preview"), Server::start());
    let store = trusting(&home, &server);
    let gate = store.trust().gate("document", Origin::Created);
    let allowed = |address: &str| gate.fetch_on_request(address, Limits { max_bytes: MOST, ..Limits::default() });
    let held = |_: &str| Err(FetchError::Held);
    let preview = |path: &str, picture: &dyn Fn(&str) -> Result<ImageFetch, FetchError>| {
        let guard = gate.fetch_page(&server.address(path), Limits { max_bytes: MOST, ..Limits::page() });
        remote::preview(guard.expect("a page may be asked for"), picture, &server.roots())
    };
    // (the page, whether its picture may be fetched, the picture the preview has)
    let cases: [(&str, bool, Option<Vec<u8>>); 4] = [
        ("/article", true, Some(png())),
        ("/article", false, None),
        ("/article/lost", true, None),
        ("/article/plain", true, None),
    ];
    for (path, may, picture) in cases {
        let found =
            preview(path, if may { &allowed } else { &held }).unwrap_or_else(|failed| panic!("{path}: {failed}"));

        assert_eq!(found.title.as_deref(), Some("Roof repairs"), "{path}");
        assert_eq!(found.description.as_deref(), Some("A quote."), "{path}");
        assert_eq!(found.picture, picture, "{path}, picture allowed: {may}");
    }
    assert_eq!(preview("/page", &allowed), Err(Failed::Nothing), "a page with nothing to show is no preview");
    assert_eq!(preview("/nothing", &allowed), Err(Failed::Status(404)));
}

#[test]
fn a_server_is_told_nothing_about_who_asks() {
    let (home, server) = (Scratch::new("remote-quiet"), Server::start());
    let store = trusting(&home, &server);
    remote::fetch(on_open(&store, &server.address("/moved")).unwrap(), &server.roots()).expect("the picture comes");

    let asked = server.asked.lock().unwrap();
    assert_eq!(asked.len(), 2, "the redirect is followed by hand, as a request of its own");
    for request in asked.iter() {
        assert!(request.contains("user-agent: letters\r\n"), "{request}");
        for telling in ["cookie", "referer", "authorization", "from:", "accept-language"] {
            assert!(!request.contains(telling), "{telling} in {request}");
        }
    }
}

#[test]
fn a_server_nobody_vouches_for_is_not_believed() {
    let (home, server) = (Scratch::new("remote-roots"), Server::start());
    let store = trusting(&home, &server);
    let stranger = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap().cert.der().to_vec();
    for (case, roots) in [("the system's roots", Roots::System), ("another certificate", Roots::These(vec![stranger]))]
    {
        let fetched = remote::fetch(on_open(&store, &server.address("/sky.png")).unwrap(), &roots);

        assert!(matches!(fetched, Err(Failed::Unreachable(_))), "{case}: {fetched:?}");
    }
    assert!(server.asked.lock().unwrap().is_empty(), "no request is sent to a server that is not believed");
}

#[test]
fn a_name_that_leads_to_this_machine_is_not_connected_to() {
    let (home, server) = (Scratch::new("remote-private"), Server::start());
    let mut store = trust::store(&home.0);
    store.load().unwrap();
    // This machine's own name reads as any name on the internet does, and leads back here.
    let name = std::fs::read_to_string("/etc/hostname").unwrap_or_default().trim().to_owned();
    let here =
        (name.as_str(), server.port).to_socket_addrs().map(|found| found.collect::<Vec<_>>()).unwrap_or_default();
    if name.is_empty() || here.is_empty() || !here.iter().all(|address| address.ip().is_loopback()) {
        eprintln!("this machine's name does not lead back to it, so there is nothing to refuse");
        return;
    }
    let address = format!("https://{name}:{}/sky.png", server.port);
    let guard = store.trust().gate("document", Origin::Created).fetch_on_request(&address, Limits::default());
    let fetched = remote::fetch(guard.expect("the name itself says nothing"), &server.roots());

    assert!(matches!(fetched, Err(Failed::Refused(FetchError::PrivateAddress(_)))), "{fetched:?}");
    assert!(server.asked.lock().unwrap().is_empty());
}
