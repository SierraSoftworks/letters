//! A server over scratch folders, and people to ask it things.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, ORIGIN};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use http_body_util::BodyExt;
use letters_api::{Entry, Error, NewDocument, Person, paths};
use letters_server::config::Config;
use letters_server::session::{COOKIE_NAME, Carrier};
use letters_server::store::Store;
use letters_server::{App, Shared, api};
use serde::Serialize;
use serde::de::DeserializeOwned;
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
/// `friends` group reads and the `reviewers` group comments in.
pub fn settings(scratch: &Path) -> String {
    let at = scratch.display();
    format!(
        r#"
[server]
name = "Test"
url = "{URL}"
state = "{at}/state"

[auth.oidc]
issuer = "https://id.example.test"
client_id = "letters"

[[space]]
name = "Home"
path = "{at}/home/{{username}}"

[[space]]
name = "Family"
path = "{at}/family"
editors = ["family"]
commenters = ["reviewers"]
readers = ["friends"]
"#
    )
}

pub struct Server {
    pub scratch: Scratch,
    pub app: Shared,
    pub router: Router,
    clock: Arc<AtomicU64>,
}

impl Server {
    /// A server over scratch folders named for the test.
    pub fn new(name: &str) -> Self {
        let scratch = Scratch::new(name);
        std::fs::create_dir_all(scratch.0.join("family")).unwrap();
        let config = Config::parse(&settings(&scratch.0)).expect("the settings are sound");
        let clock = Arc::new(AtomicU64::new(START));
        let told = clock.clone();
        let app = App::new(config, Store::in_memory()).with_clock(Box::new(move || told.load(Ordering::Relaxed)));
        let app = Arc::new(app);
        Self { scratch, router: api::router(app.clone()), app, clock }
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

/// The names of some rows, in their order.
pub fn names(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.name.as_str()).collect()
}
