//! A server over scratch folders, and people to ask it things.
#![allow(dead_code)]

pub mod desk;
pub mod idp;

use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::body::Body;
use axum::http::header::{ACCEPT_ENCODING, AUTHORIZATION, CONTENT_TYPE, COOKIE, LOCATION, ORIGIN, SET_COOKIE};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use http_body_util::BodyExt;
use letters_api::{Entry, Error, NewDocument, Person, paths};
use letters_server::config::Config;
use letters_server::fetch::Roots;
use letters_server::pages::editor::Editor;
use letters_server::session::{COOKIE_NAME, Carrier};
use letters_server::store::Store;
use letters_server::{App, Shared, api};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tower::ServiceExt;

/// The address the scratch server is told it is reached at.
pub const URL: &str = "https://letters.example.test";
/// When the scratch server's clock starts.
pub const START: u64 = 1_800_000_000;

/// A scratch folder that is removed when the test ends.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("letters-server-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// The names of what a folder in it holds, hidden ones too.
    pub fn names(&self, folder: &str) -> Vec<String> {
        let entries = std::fs::read_dir(self.0.join(folder)).into_iter().flatten();
        let mut names: Vec<String> = entries.map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The settings every test starts from: each person's own space, and one the `family` group edits, the
/// `friends` group reads and the `reviewers` group comments in. `oidc` is what else to say of the provider.
pub fn settings(scratch: &Path, issuer: &str, oidc: &str) -> String {
    // In literal strings below: a path on Windows has backslashes, which a basic string reads as escapes.
    let at = scratch.display();
    format!(
        r#"
[server]
name = "Test"
url = "{URL}"
state = '{at}/state'

[auth.oidc]
issuer = "{issuer}"
client_id = "letters"
{oidc}

[[space]]
name = "Home"
path = '{at}/home/{{username}}'

[[space]]
name = "Family"
path = '{at}/family'
editors = ["family"]
commenters = ["reviewers"]
readers = ["friends"]
"#
    )
}

/// The settings of a server whose provider is not there to be asked.
fn sound(scratch: &Path) -> String {
    settings(scratch, "https://id.example.test", "")
}

pub struct Server {
    pub scratch: Scratch,
    pub app: Shared,
    pub router: Router,
    clock: Arc<AtomicU64>,
}

impl Server {
    /// A server over scratch folders named for the test. Its provider is not there to be asked.
    pub fn new(name: &str) -> Self {
        Self::with_provider(name, "https://id.example.test", "")
    }

    /// A server that signs people in at `issuer`, with `oidc` as its other settings for the provider.
    pub fn with_provider(name: &str, issuer: &str, oidc: &str) -> Self {
        Self::over(Scratch::new(name), &|at| settings(at, issuer, oidc), |app| app)
    }

    /// A server whose browsers are sent `editor`.
    pub fn editing(name: &str, editor: Editor) -> Self {
        Self::over(Scratch::new(name), &sound, |app| app.with_editor(Some(editor)))
    }

    /// A server whose owner trusts `sources`, and which believes a site that shows one of `roots`.
    pub fn fetching(name: &str, sources: &[String], roots: Roots) -> Self {
        let trusting = |at: &Path| format!("{}\n[trust]\nsources = {sources:?}\n", sound(at));
        Self::over(Scratch::new(name), &trusting, |app| app.with_roots(roots))
    }

    /// A server whose owner lets links be made.
    pub fn linking(name: &str) -> Self {
        let linking = |at: &Path| format!("{}\n[links]\nenabled = true\n", sound(at));
        Self::over(Scratch::new(name), &linking, |app| app)
    }

    /// The server after it stopped and started again: the same folders, and nothing that was in memory.
    pub fn restarted(self) -> Self {
        Self::over(self.scratch, &sound, |app| app)
    }

    /// A server over the folders of `scratch`, with the settings `written` for them. It has no editor
    /// unless `more` gives it one, whatever it was built with.
    fn over(scratch: Scratch, written: &dyn Fn(&Path) -> String, more: impl FnOnce(App) -> App) -> Self {
        std::fs::create_dir_all(scratch.0.join("family")).unwrap();
        let config = Config::parse(&written(&scratch.0)).expect("the settings are sound");
        let clock = Arc::new(AtomicU64::new(START));
        let told = clock.clone();
        let app = App::new(config, Store::in_memory()).with_clock(Box::new(move || told.load(Ordering::Relaxed)));
        let app = Arc::new(more(app.with_editor(None)));
        Self { scratch, router: api::router(app.clone()), app, clock }
    }

    /// Serves on a port of this machine, for what a router alone cannot be asked: a socket.
    pub async fn listen(&self) -> SocketAddr {
        let (address, stop, _) = self.serving().await;
        // Never told to stop.
        std::mem::forget(stop);
        address
    }

    /// Serves on a port of this machine until the sender is used: the address, the way to stop the
    /// server, and how its serving ends.
    pub async fn serving(&self) -> (SocketAddr, oneshot::Sender<()>, JoinHandle<std::io::Result<()>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a port is free");
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel();
        let served = letters_server::serve(self.app.clone(), listener, async move { drop(stopped.await) });
        (address, stop, tokio::spawn(served))
    }

    /// Lets `seconds` pass.
    pub fn wait(&self, seconds: u64) {
        self.clock.fetch_add(seconds, Ordering::Relaxed);
    }

    /// Someone signed in on the desktop app, in these groups.
    pub fn person(&self, username: &str, groups: &[&str]) -> Client {
        self.signed(username, groups, Carrier::Bearer)
    }

    /// Someone signed in with a browser.
    pub fn browser(&self, username: &str, groups: &[&str]) -> Client {
        self.signed(username, groups, Carrier::Cookie)
    }

    fn signed(&self, username: &str, groups: &[&str], carrier: Carrier) -> Client {
        // "ben" is Ben. A name that is no name, as some tests sign in with, is left as it is.
        let mut name = username.to_owned();
        if let Some(first) = name.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        let person = Person { id: format!("sub-{username}"), username: username.into(), name };
        let groups = groups.iter().map(|group| group.to_string()).collect();
        let token = self.app.sign_in(person, groups, carrier).expect("a session starts");
        Client {
            router: self.router.clone(),
            token: Some(token.token),
            renew: token.renew,
            carrier,
            origin: Some(URL.into()),
        }
    }

    /// A browser that has not been here before.
    pub fn visitor(&self) -> Browser {
        Browser { router: self.router.clone(), cookies: BTreeMap::new() }
    }

    /// A browser that someone in these groups is signed in with, at the server's pages.
    pub fn reader(&self, username: &str, groups: &[&str]) -> Browser {
        let mut browser = self.visitor();
        let token = self.browser(username, groups).token.expect("a session has a token");
        browser.cookies.insert(COOKIE_NAME.into(), token);
        browser
    }

    /// Someone with no session.
    pub fn stranger(&self) -> Client {
        Client {
            router: self.router.clone(),
            token: None,
            renew: String::new(),
            carrier: Carrier::Bearer,
            origin: None,
        }
    }
}

/// One person's requests.
#[derive(Clone)]
pub struct Client {
    pub router: Router,
    pub token: Option<String>,
    pub renew: String,
    pub carrier: Carrier,
    /// What a browser says the page that asks came from.
    pub origin: Option<String>,
}

/// An answer that was not read as anything yet.
pub struct Raw {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Raw {
    pub fn header(&self, name: &str) -> &str {
        self.headers.get(name).and_then(|value| value.to_str().ok()).unwrap_or_default()
    }

    /// Where the answer sends a browser on to. Empty when it does not.
    pub fn location(&self) -> &str {
        if self.status.is_redirection() { self.header(LOCATION.as_str()) } else { "" }
    }

    /// The cookies the answer sets, each as it was written.
    pub fn cookies(&self) -> Vec<&str> {
        self.headers.get_all(SET_COOKIE).into_iter().filter_map(|value| value.to_str().ok()).collect()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

impl Client {
    pub async fn send(&self, method: Method, path: &str, kind: &str, body: Vec<u8>) -> Raw {
        let mut request = Request::builder().method(method).uri(path);
        match (&self.token, self.carrier) {
            (Some(token), Carrier::Bearer) => request = request.header(AUTHORIZATION, format!("Bearer {token}")),
            (Some(token), Carrier::Cookie) => {
                request = request.header(COOKIE, format!("theme=dark; {COOKIE_NAME}={token}"));
                if let Some(origin) = &self.origin {
                    request = request.header(ORIGIN, origin);
                }
            }
            (None, _) => {}
        }
        if !kind.is_empty() {
            request = request.header(CONTENT_TYPE, kind);
        }
        let response = self.router.clone().oneshot(request.body(Body::from(body)).unwrap()).await.unwrap();
        let (parts, body) = response.into_parts();
        let body = body.collect().await.unwrap().to_bytes().to_vec();
        Raw { status: parts.status, headers: parts.headers, body }
    }

    async fn json<T: DeserializeOwned>(&self, method: Method, path: &str, body: Option<Vec<u8>>) -> Result<T, Error> {
        let kind = if body.is_some() { letters_api::JSON } else { "" };
        let raw = self.send(method, path, kind, body.unwrap_or_default()).await;
        if raw.status.is_success() {
            // An answer with nothing in it reads as `()`.
            let body = if raw.body.is_empty() { b"null".as_slice() } else { &raw.body };
            return Ok(
                serde_json::from_slice(body).unwrap_or_else(|error| panic!("{path} answered {}: {error}", raw.text()))
            );
        }
        let error: Error = serde_json::from_slice(&raw.body)
            .unwrap_or_else(|_| panic!("{path} refused with {}: {}", raw.status, raw.text()));
        assert_eq!(error.code.status(), raw.status.as_u16(), "the status and the code of a refusal agree");
        Err(error)
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        self.json(Method::GET, path, None).await
    }

    pub async fn post<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T, Error> {
        self.json(Method::POST, path, Some(serde_json::to_vec(body).unwrap())).await
    }

    pub async fn put<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T, Error> {
        self.json(Method::PUT, path, Some(serde_json::to_vec(body).unwrap())).await
    }

    /// Starts a document called `name` in a folder of a space, and hands back its row.
    pub async fn start(&self, space: &str, folder: &str, name: &str) -> Entry {
        let new = NewDocument { folder: folder.into(), name: Some(name.into()) };
        self.post(&paths::documents(space), &new).await.expect("the document is made")
    }
}

/// A browser: it keeps the cookies it is handed, and follows nothing by itself.
pub struct Browser {
    router: Router,
    pub cookies: BTreeMap<String, String>,
}

impl Browser {
    pub async fn get(&mut self, path: &str) -> Raw {
        self.send(Method::GET, path, "", Vec::new(), &[]).await
    }

    /// Fetches a file as a browser that reads the encodings `accepts` names.
    pub async fn fetch(&mut self, path: &str, accepts: &str) -> Raw {
        self.send(Method::GET, path, "", Vec::new(), &[(ACCEPT_ENCODING.as_str(), accepts)]).await
    }

    /// Sends a form from one of the server's own pages.
    pub async fn fill(&mut self, path: &str, fields: &[(&str, &str)]) -> Raw {
        self.submit(path, fields, Some(URL)).await
    }

    /// Sends a form, as a page that came from `origin` would.
    pub async fn submit(&mut self, path: &str, fields: &[(&str, &str)], origin: Option<&str>) -> Raw {
        let body = letters_server::oidc::form(fields).into_bytes();
        let origin: Vec<(&str, &str)> = origin.map(|origin| (ORIGIN.as_str(), origin)).into_iter().collect();
        self.send(Method::POST, path, "application/x-www-form-urlencoded", body, &origin).await
    }

    async fn send(&mut self, method: Method, path: &str, kind: &str, body: Vec<u8>, said: &[(&str, &str)]) -> Raw {
        let mut request = Request::builder().method(method).uri(path);
        if !self.cookies.is_empty() {
            let cookies: Vec<String> = self.cookies.iter().map(|(name, value)| format!("{name}={value}")).collect();
            request = request.header(COOKIE, cookies.join("; "));
        }
        for (name, value) in said {
            request = request.header(*name, *value);
        }
        if !kind.is_empty() {
            request = request.header(CONTENT_TYPE, kind);
        }
        let response = self.router.clone().oneshot(request.body(Body::from(body)).unwrap()).await.unwrap();
        let (parts, body) = response.into_parts();
        let raw =
            Raw { status: parts.status, headers: parts.headers, body: body.collect().await.unwrap().to_bytes().into() };
        for cookie in raw.cookies() {
            let (pair, rules) = cookie.split_once(';').unwrap_or((cookie, ""));
            let (name, value) = pair.split_once('=').expect("a cookie has a name");
            match rules.contains("Max-Age=0") {
                true => self.cookies.remove(name),
                false => self.cookies.insert(name.to_owned(), value.to_owned()),
            };
        }
        raw
    }

    /// The browser's pages asking the interface, with the session the browser holds.
    pub fn pages(&self) -> Client {
        Client {
            router: self.router.clone(),
            token: self.cookies.get(COOKIE_NAME).cloned(),
            renew: String::new(),
            carrier: Carrier::Cookie,
            origin: Some(URL.into()),
        }
    }
}

/// The fields of a query or a form, as they were before they were written into it.
pub fn query(text: &str) -> HashMap<String, String> {
    let unescaped = |text: &str| {
        let (mut out, mut bytes) = (Vec::new(), text.bytes());
        while let Some(byte) = bytes.next() {
            match byte {
                b'%' => {
                    let hex: Vec<u8> = bytes.by_ref().take(2).collect();
                    out.push(u8::from_str_radix(std::str::from_utf8(&hex).unwrap(), 16).unwrap());
                }
                b'+' => out.push(b' '),
                byte => out.push(byte),
            }
        }
        String::from_utf8(out).unwrap()
    };
    let pairs = text.split('&').filter_map(|pair| pair.split_once('='));
    pairs.map(|(name, value)| (unescaped(name), unescaped(value))).collect()
}

/// The text of a `.let` file, block by block.
pub fn texts(file: Vec<u8>) -> Vec<String> {
    let document = letters_format::Archive::open(Cursor::new(file)).expect("a document").document().expect("it reads");
    document.walk().map(letters_model::Block::plain_text).collect()
}

/// The names of some rows, in their order.
pub fn names(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.name.as_str()).collect()
}
