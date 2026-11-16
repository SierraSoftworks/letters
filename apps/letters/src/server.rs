//! What the desktop app asks of a Letters server over HTTP, in the words of `letters-api`.
//!
//! Every call waits on the network, so none is for the UI thread. A server is spoken to over TLS only,
//! since a sign-in goes with every request, and is followed nowhere: an answer that points elsewhere is
//! a failure. It may be on this network or on this machine, as a person's own server is, so no address is
//! refused for where it leads, unlike the addresses a document names.

use std::fmt;
use std::time::Duration;

use letters_api::{Exchange, Renewal, Seen, Token, paths};
use serde::Serialize;
use serde::de::DeserializeOwned;
use ureq::config::{AutoHeaderValue, Config};
use ureq::http::Uri;
use ureq::tls::{Certificate, RootCerts, TlsConfig};
use ureq::{Agent, Error};

use crate::remote::{LIMIT, Roots};

/// All a server is told about what asks, beside who signed in.
const CLIENT: &str = "Letters";
/// How long a call may take that carries a document's file or a picture.
const CARRYING: Duration = Duration::from_secs(300);
/// The most an answer in JSON may hold, in bytes.
const ANSWER: u64 = 4 << 20;

/// Why a server did not do what it was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The server said no, with a code to act on and a sentence to show.
    Said(letters_api::Error),
    /// The server answered with this status and no word of why, as something in front of it may.
    Status(u16),
    /// What answers at the address is no Letters server, or one this build cannot talk to.
    NotLetters,
    /// The address is none a server is asked at: only `https` is spoken.
    Address,
    /// The answer is larger than was to be taken.
    TooLarge,
    /// It took longer than was allowed.
    Slow,
    /// The server could not be reached, or did not speak as expected. The reason is the network's own
    /// wording, which is kept for whoever debugs and not shown.
    Unreachable(String),
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Said(said) => said.fmt(f),
            Self::Status(status) => write!(f, "the server answered {status}"),
            Self::NotLetters => write!(f, "there is no Letters server this app can talk to at that address"),
            Self::Address => write!(f, "the address is not one of a Letters server"),
            Self::TooLarge => write!(f, "the server sent more than this app takes"),
            Self::Slow => write!(f, "the server took too long"),
            Self::Unreachable(_) => write!(f, "the server could not be reached"),
        }
    }
}

impl std::error::Error for Refused {}

impl Refused {
    /// The code to act on, if the server gave one.
    pub fn code(&self) -> Option<letters_api::Code> {
        match self {
            Self::Said(said) => Some(said.code),
            _ => None,
        }
    }
}

/// What a request asks to be done with what is at its address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verb {
    Get,
    Post,
    Put,
    Delete,
}

/// A request's body, and what kind of thing it is.
type Body<'a> = Option<(&'a str, &'a [u8])>;

/// One Letters server, by its address.
#[derive(Clone, Debug)]
pub struct Client {
    /// `https://host`, with a port if the server is not at the usual one.
    origin: String,
    roots: Roots,
}

impl Client {
    /// The server at `address`, as a person types it: `letters.example`, or the whole of
    /// `https://letters.example:8443/`. Nothing is asked of it yet.
    pub fn new(address: &str, roots: &Roots) -> Result<Self, Refused> {
        let typed = address.trim();
        let whole = if typed.contains("://") { typed.to_owned() } else { format!("https://{typed}") };
        let uri: Uri = whole.parse().map_err(|_| Refused::Address)?;
        let authority = uri.authority().filter(|_| uri.scheme_str() == Some("https")).ok_or(Refused::Address)?;
        // A name and a password in the address would go to the server with every request.
        if authority.as_str().contains('@') || authority.host().is_empty() {
            return Err(Refused::Address);
        }
        Ok(Self { origin: format!("https://{authority}"), roots: roots.clone() })
    }

    /// The server's address, which also names it wherever its sign-in and its documents are kept.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// The whole address of a document's socket, from the path [`letters_api::Opened::socket`] gives.
    pub fn socket(&self, path: &str) -> String {
        format!("wss{}{path}", self.origin.trim_start_matches("https"))
    }

    /// Asks what is at the address. Needs no sign-in, and is the first thing asked of a server.
    pub fn discover(&self) -> Result<letters_api::Server, Refused> {
        let answer = self.call(Verb::Get, paths::SERVER, None, None, LIMIT, ANSWER)?;
        let server: letters_api::Server = serde_json::from_slice(&answer).map_err(|_| Refused::NotLetters)?;
        if server.speaks() { Ok(server) } else { Err(Refused::NotLetters) }
    }

    /// Trades what the browser came back with for a session, to end a sign-in.
    pub fn exchange(&self, code: &str, verifier: &str) -> Result<Token, Refused> {
        let exchange = Exchange { code: code.to_owned(), verifier: verifier.to_owned() };
        self.send(Verb::Post, paths::EXCHANGE, None, &exchange)
    }

    /// Trades a session's renewal for the next session. A renewal works once, so the answer is kept
    /// before it is used.
    pub fn renew(&self, renew: &str) -> Result<Token, Refused> {
        self.send(Verb::Post, paths::RENEW, None, &Renewal { renew: renew.to_owned() })
    }

    /// Ends a session, so that its token and its renewal stop working.
    pub fn sign_out(&self, token: &str) -> Result<(), Refused> {
        self.act(token, paths::SIGN_OUT)
    }

    /// Reads what the server has at `path`, one of [`letters_api::paths`], as the session of `token`.
    pub fn get<T: DeserializeOwned>(&self, token: &str, path: &str) -> Result<T, Refused> {
        read(&self.call(Verb::Get, path, Some(token), None, LIMIT, ANSWER)?)
    }

    /// Sends `body` to `path` as the session of `token`, and reads the answer.
    pub fn post<T: DeserializeOwned>(&self, token: &str, path: &str, body: &impl Serialize) -> Result<T, Refused> {
        self.send(Verb::Post, path, Some(token), body)
    }

    /// Puts `body` in place of what the server has at `path`, and reads the answer.
    pub fn put<T: DeserializeOwned>(&self, token: &str, path: &str, body: &impl Serialize) -> Result<T, Refused> {
        self.send(Verb::Put, path, Some(token), body)
    }

    /// Asks for something to be done that needs no more said than its `path`, as moving to the trash does.
    pub fn act(&self, token: &str, path: &str) -> Result<(), Refused> {
        self.call(Verb::Post, path, Some(token), None, LIMIT, ANSWER).map(drop)
    }

    /// Asks for something to be done that needs no more said than its `path`, and gives what the server
    /// answers of it, as putting a document back gives where it now is.
    pub fn done<T: DeserializeOwned>(&self, token: &str, path: &str) -> Result<T, Refused> {
        read(&self.call(Verb::Post, path, Some(token), None, LIMIT, ANSWER)?)
    }

    /// Hands the server a file as it is, to make something of, and gives what it made: a Markdown or Word
    /// file becomes a document.
    pub fn hand<T: DeserializeOwned>(&self, token: &str, path: &str, file: &[u8]) -> Result<T, Refused> {
        let body = Some(("application/octet-stream", file));
        read(&self.call(Verb::Post, path, Some(token), body, CARRYING, ANSWER)?)
    }

    /// Takes away what the server has at `path`, as revoking a link does.
    pub fn delete(&self, token: &str, path: &str) -> Result<(), Refused> {
        self.call(Verb::Delete, path, Some(token), None, LIMIT, ANSWER).map(drop)
    }

    /// The whole `.let` file of a document, of `most` bytes at most.
    pub fn file(&self, token: &str, id: &str, most: u64) -> Result<Vec<u8>, Refused> {
        self.call(Verb::Get, &paths::file(id), Some(token), None, CARRYING, most)
    }

    /// One picture of a document, by its name without `assets/`.
    pub fn asset(&self, token: &str, id: &str, name: &str, most: u64) -> Result<Vec<u8>, Refused> {
        self.call(Verb::Get, &paths::asset(id, name), Some(token), None, CARRYING, most)
    }

    /// Hands the server a picture of a document, before the edit that names it is sent.
    pub fn keep_asset(&self, token: &str, id: &str, name: &str, bytes: &[u8]) -> Result<(), Refused> {
        let kind = crate::pictures::mime(bytes).unwrap_or("application/octet-stream");
        self.call(Verb::Put, &paths::asset(id, name), Some(token), Some((kind, bytes)), CARRYING, ANSWER).map(drop)
    }

    /// Says that the person read a document up to `revision`, which their account keeps for the next
    /// device they open it on. The server answers nothing.
    pub fn saw(&self, token: &str, id: &str, revision: &str) -> Result<(), Refused> {
        let seen = serde_json::to_vec(&Seen { revision: revision.to_owned() });
        let seen = seen.map_err(|error| Refused::Unreachable(error.to_string()))?;
        self.call(Verb::Put, &paths::seen(id), Some(token), Some((letters_api::JSON, &seen)), LIMIT, ANSWER).map(drop)
    }

    /// Hands the server a document's whole `.let` file, which it merges into the document it holds.
    pub fn keep_file(&self, token: &str, id: &str, bytes: &[u8]) -> Result<(), Refused> {
        let body = Some((letters_format::MIMETYPE, bytes));
        self.call(Verb::Put, &paths::file(id), Some(token), body, CARRYING, ANSWER).map(drop)
    }

    fn send<T: DeserializeOwned>(
        &self,
        verb: Verb,
        path: &str,
        token: Option<&str>,
        body: &impl Serialize,
    ) -> Result<T, Refused> {
        let body = serde_json::to_vec(body).map_err(|error| Refused::Unreachable(error.to_string()))?;
        read(&self.call(verb, path, token, Some((letters_api::JSON, &body)), LIMIT, ANSWER)?)
    }

    /// Asks the server for `path` and gives the answer's body, of `most` bytes at most, if the server
    /// said yes within `within`.
    fn call(
        &self,
        verb: Verb,
        path: &str,
        token: Option<&str>,
        body: Body<'_>,
        within: Duration,
        most: u64,
    ) -> Result<Vec<u8>, Refused> {
        // An agent for each call: it keeps no connection and remembers nothing between them.
        let agent = Agent::new_with_config(config(&self.roots, within));
        let url = format!("{}{path}", self.origin);
        let bearer = token.map(|token| format!("Bearer {token}"));
        let answered = match verb {
            Verb::Get | Verb::Delete => {
                let request = if verb == Verb::Get { agent.get(&url) } else { agent.delete(&url) };
                bearer.iter().fold(request, |request, bearer| request.header("authorization", bearer)).call()
            }
            Verb::Post | Verb::Put => {
                let request = if verb == Verb::Post { agent.post(&url) } else { agent.put(&url) };
                let request = bearer.iter().fold(request, |request, bearer| request.header("authorization", bearer));
                match body {
                    Some((kind, bytes)) => request.content_type(kind).send(bytes),
                    None => request.send_empty(),
                }
            }
        };
        let mut response = answered.map_err(failed)?;
        let status = response.status();
        // A refusal is a sentence or two, whatever the answer to the request might have held.
        let most = if status.is_success() { most } else { ANSWER };
        let bytes = response.body_mut().with_config().limit(most).read_to_vec().map_err(failed)?;
        if status.is_success() {
            return Ok(bytes);
        }
        Err(match serde_json::from_slice::<letters_api::Error>(&bytes) {
            Ok(said) => Refused::Said(said),
            Err(_) => Refused::Status(status.as_u16()),
        })
    }
}

/// Reads an answer the server gave in JSON. One that cannot be read is not from a server this build knows.
fn read<T: DeserializeOwned>(answer: &[u8]) -> Result<T, Refused> {
    serde_json::from_slice(answer).map_err(|_| Refused::NotLetters)
}

fn failed(error: Error) -> Refused {
    match error {
        Error::Timeout(_) => Refused::Slow,
        Error::Io(error) if error.kind() == std::io::ErrorKind::TimedOut => Refused::Slow,
        Error::BodyExceedsLimit(_) => Refused::TooLarge,
        other => Refused::Unreachable(other.to_string()),
    }
}

fn config(roots: &Roots, within: Duration) -> Config {
    let roots = match roots {
        Roots::System => RootCerts::PlatformVerifier,
        Roots::These(certificates) => {
            let certificates: Vec<_> = certificates.iter().map(|der| Certificate::from_der(der).to_owned()).collect();
            RootCerts::new_with_certs(&certificates)
        }
    };
    Agent::config_builder()
        .https_only(true)
        // The sign-in goes to the server it is for and to no other, whatever an answer points to.
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .user_agent(CLIENT)
        .accept_encoding(AutoHeaderValue::None)
        .max_idle_connections(0)
        .timeout_global(Some(within))
        .tls_config(TlsConfig::builder().root_certs(roots).build())
        .build()
}
