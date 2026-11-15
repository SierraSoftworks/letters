//! What the server fetches for a document from elsewhere: a linked picture and the page behind a link,
//! from a site on this machine.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use axum::http::{Method, StatusCode};
use common::{Client, Server};
use letters_api::{Code, Error, Fetch, JSON, Preview, paths};
use letters_server::fetch::Roots;

/// A smallest PNG, by its first bytes.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n and the rest";

/// A page that says what it is, and names its picture beside itself.
const ARTICLE: &str = r#"<html><head><meta property="og:title" content="Roof repairs">
<meta property="og:description" content="A quote."><meta property="og:image" content="pictures/sky.png"></head></html>"#;

/// What the site answers when it is asked for `path`.
fn answer(path: &str) -> Vec<u8> {
    let with = |status: &str, headers: &str, body: &[u8]| {
        let head = format!("HTTP/1.1 {status}\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n", body.len());
        [head.as_bytes(), body].concat()
    };
    match path {
        "/sky.png" => with("200 OK", "content-type: image/png\r\n", PNG),
        "/moved" => with("302 Found", "location: /sky.png\r\n", b""),
        "/article" => with("200 OK", "content-type: text/html; charset=utf-8\r\n", ARTICLE.as_bytes()),
        _ => with("404 Not Found", "", b""),
    }
}

/// A site on this machine that answers over HTTPS as `localhost`.
struct Site {
    port: u16,
    /// The certificate it shows, which nobody but a test believes.
    certificate: Vec<u8>,
    /// What it was asked, each request in lower case.
    asked: Arc<Mutex<Vec<String>>>,
}

impl Site {
    fn start() -> Self {
        let made = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
        let certificate = made.cert.der().to_vec();
        let key = rustls::pki_types::PrivateKeyDer::try_from(made.signing_key.serialize_der()).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let shown = vec![rustls::pki_types::CertificateDer::from(certificate.clone())];
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(shown, key)
            .unwrap();
        let config = Arc::new(config);
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let heard = asked.clone();
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
}

/// A server whose owner trusts `site`, with a document in the space of the family.
async fn trusting(name: &str, site: &Site) -> (Server, Client, String) {
    let server = Server::fetching(name, &[site.address("/")], Roots::These(vec![site.certificate.clone()]));
    let ada = server.person("ada", &["family"]);
    let id = ada.start("family", "", "Plan").await.id.unwrap();
    (server, ada, id)
}

/// Asks the server to fetch `url` for the document `id`: the status, and the body or what was refused.
async fn fetched(who: &Client, path: &str, url: &str, asked: bool) -> Result<Vec<u8>, Code> {
    let fetch = serde_json::to_vec(&Fetch { url: url.into(), asked }).unwrap();
    let raw = who.send(Method::POST, path, JSON, fetch).await;
    match raw.status {
        StatusCode::OK => Ok(raw.body),
        _ => Err(serde_json::from_slice::<Error>(&raw.body).unwrap_or_else(|_| panic!("{url}: {}", raw.text())).code),
    }
}

/// Who asks, for what, whether they asked themselves, and what comes of it.
type Asked<'a> = (&'a Client, String, bool, Result<&'a [u8], Code>);

#[tokio::test(flavor = "multi_thread")]
async fn a_linked_picture_is_fetched_only_as_the_owners_rules_allow() {
    let site = Site::start();
    let (server, ada, id) = trusting("fetch-picture", &site).await;
    let reader = server.browser("guest", &["friends"]);
    let stranger = server.person("stranger", &[]);
    let numbered = format!("https://127.0.0.1:{}/sky.png", site.port);
    let cases: [Asked; 9] = [
        (&ada, site.address("/sky.png"), false, Ok(PNG)),
        (&reader, site.address("/sky.png"), true, Ok(PNG)),
        (&ada, site.address("/moved"), false, Ok(PNG)),
        (&ada, site.address("/article"), true, Err(Code::Upstream)),
        (&ada, site.address("/nothing"), true, Err(Code::Upstream)),
        // Nobody asked for it, and the owner does not trust where it is.
        (&ada, "https://pictures.example/sky.png".into(), false, Err(Code::Untrusted)),
        (&ada, "http://localhost/sky.png".into(), true, Err(Code::Untrusted)),
        // The same machine under a name the owner did not list: nothing inside the network is reached.
        (&ada, numbered, true, Err(Code::Untrusted)),
        (&stranger, site.address("/sky.png"), true, Err(Code::NotFound)),
    ];
    for (who, url, asked, expected) in cases {
        let got = fetched(who, &paths::fetch_picture(&id), &url, asked).await;

        assert_eq!(got.as_deref().map_err(|code| *code), expected, "{url}, asked: {asked}");
    }

    let asked = site.asked.lock().unwrap();
    assert!(!asked.is_empty(), "the site was asked");
    for request in asked.iter() {
        for telling in ["cookie", "referer", "authorization", "from:", "accept-language", "ada", "guest"] {
            assert!(!request.contains(telling), "the site is told nothing of who asks: {telling} in {request}");
        }
    }
}

/// A change to someone's own sources, and the sources as they are then or why it was refused.
type Step<'a> = (Method, &'a str, Result<&'a [&'a str], Code>);

/// Changes the sources of `who`'s own, and answers them as they are then.
async fn sources(who: &Client, method: Method, source: &str) -> Result<Vec<String>, Code> {
    let raw = who.send(method, &paths::own_source(source), "", Vec::new()).await;
    match raw.status {
        StatusCode::OK => Ok(raw.json()),
        _ => Err(raw.json::<Error>().code),
    }
}

#[tokio::test]
async fn a_persons_own_sources_change_one_at_a_time() {
    let server = Server::new("own-sources");
    let ada = server.person("ada", &["family"]);
    assert!(ada.get::<Vec<String>>(paths::OWN_SOURCES).await.unwrap().is_empty());

    // A host and its whole address are one source, and one already there or already gone is no trouble.
    let (host, charts) = ("https://img.example.org/", "https://img.example.org/charts/");
    let steps: [Step; 7] = [
        (Method::PUT, "img.example.org", Ok(&[host])),
        (Method::PUT, host, Ok(&[host])),
        (Method::PUT, charts, Ok(&[host, charts])),
        (Method::PUT, "http://img.example.org/", Err(Code::Invalid)),
        (Method::PUT, "not a source", Err(Code::Invalid)),
        (Method::DELETE, "img.example.org", Ok(&[charts])),
        (Method::DELETE, host, Ok(&[charts])),
    ];
    for (method, source, then) in steps {
        let told = sources(&ada, method.clone(), source).await;
        let then = then.map(|sources| sources.iter().map(|source| (*source).to_owned()).collect::<Vec<_>>());
        assert_eq!(told, then, "{method} {source}");
    }
    assert_eq!(ada.get::<Vec<String>>(paths::OWN_SOURCES).await.unwrap(), [charts]);
    assert!(server.person("guest", &["friends"]).get::<Vec<String>>(paths::OWN_SOURCES).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_picture_under_a_persons_own_source_is_fetched_unasked_but_never_from_inside() {
    let site = Site::start();
    let (server, ada, id) = trusting("fetch-own", &site).await;
    let reader = server.browser("guest", &["friends"]);
    let path = paths::fetch_picture(&id);
    // No such site exists, so a fetch that is let through comes back with nothing, and one that is not
    // let through never leaves.
    let elsewhere = "https://pictures.invalid/sky.png";
    let inside = format!("https://127.0.0.1:{}/sky.png", site.port);
    assert_eq!(fetched(&ada, &path, elsewhere, false).await, Err(Code::Untrusted), "nobody trusts it yet");

    sources(&ada, Method::PUT, "pictures.invalid").await.expect("a source of her own");
    sources(&ada, Method::PUT, &format!("127.0.0.1:{}", site.port)).await.expect("kept, though never reached");
    let cases = [
        (&ada, elsewhere, Code::Upstream, "her own source is tried without her asking"),
        (&reader, elsewhere, Code::Untrusted, "her sources are hers alone"),
        (&ada, inside.as_str(), Code::Untrusted, "a source of her own reaches nothing inside the network"),
    ];
    for (who, url, code, why) in cases {
        assert_eq!(fetched(who, &path, url, false).await, Err(code), "{why}");
    }
    assert!(site.asked.lock().unwrap().is_empty(), "the site inside the network was never asked");

    sources(&ada, Method::DELETE, "pictures.invalid").await.expect("taken out");
    assert_eq!(fetched(&ada, &path, elsewhere, false).await, Err(Code::Untrusted), "no longer hers");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_preview_is_what_the_page_behind_a_link_says_of_itself() {
    let site = Site::start();
    let (_server, ada, id) = trusting("fetch-preview", &site).await;
    let path = paths::fetch_preview(&id);

    let read = fetched(&ada, &path, &site.address("/article"), true).await.expect("the page is read");
    let preview: Preview = serde_json::from_slice(&read).expect("a preview");
    assert_eq!(preview.title.as_deref(), Some("Roof repairs"));
    assert_eq!(preview.description.as_deref(), Some("A quote."));
    let picture = preview.picture.expect("the page names its picture");
    assert_eq!(picture, site.address("/pictures/sky.png"), "by its whole address, to be fetched in turn");

    let cases = [
        (site.address("/article"), false, Code::Untrusted, "nobody asked for it"),
        (site.address("/sky.png"), true, Code::Upstream, "a picture is no page"),
        (site.address("/nothing"), true, Code::Upstream, "the site has no such page"),
    ];
    for (url, asked, code, why) in cases {
        assert_eq!(fetched(&ada, &path, &url, asked).await, Err(code), "{why}");
    }
}

#[tokio::test]
async fn a_person_trusts_a_document_they_reach() {
    let server = Server::new("trust");
    let ada = server.person("ada", &["family"]);
    let id = ada.start("family", "", "Plan").await.id.unwrap();
    let (reader, stranger) = (server.browser("guest", &["friends"]), server.person("stranger", &[]));
    let cases = [
        (&ada, StatusCode::NO_CONTENT, "its editor"),
        (&ada, StatusCode::NO_CONTENT, "its editor again"),
        (&reader, StatusCode::NO_CONTENT, "a reader"),
        (&stranger, StatusCode::NOT_FOUND, "someone who does not reach it"),
    ];
    for (who, expected, case) in cases {
        let raw = who.send(Method::POST, &paths::trust(&id), "", Vec::new()).await;

        assert_eq!(raw.status, expected, "{case}");
    }
}
