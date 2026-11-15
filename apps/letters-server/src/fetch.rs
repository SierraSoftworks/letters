//! What the server fetches for a document: a picture that is still a link, or the page a link preview is
//! made from. A page in a browser may read neither from another site, and fetched here the other site
//! learns nothing of who reads the document.
//!
//! `letters_trust` decides what may be fetched, and its guard is asked before every step. This owns the
//! connection: it connects only to addresses the guard passed, follows redirects by hand, sends no cookie
//! and nothing of the session, and gives up after [`LIMIT`]. The server sits inside its owner's network,
//! so only the sources its owner listed may be on a private address.

use std::fmt;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use letters_api::{Code, Preview};
use letters_convert::{PageSummary, page_summary};
use letters_trust::{Fetch, FetchError, Gate, HttpsUrl, ImageType, Kind, Limits, Origin, Trust};
use serde::{Deserialize, Serialize};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use ureq::config::{AutoHeaderValue, Config};
use ureq::http::{HeaderMap, header};
use ureq::tls::{Certificate, RootCerts, TlsConfig};
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use ureq::{Agent, Error};

use crate::App;
use crate::api::Found;
use crate::error::{Answer, Refusal};
use crate::session::Signed;
use crate::store::Table;

/// How long a fetch may take from its first step to its last byte, redirects included.
pub const LIMIT: Duration = Duration::from_secs(10);
/// How many fetches run at once, for everyone together. The rest wait their turn.
pub const AT_ONCE: usize = 16;
/// All a site is told about who asks.
const CLIENT: &str = "Letters";
/// The kinds of picture a document shows.
const PICTURES: &str = "image/png, image/jpeg, image/gif, image/webp";
/// What a page is asked for as.
const PAGES: &str = "text/html, application/xhtml+xml";

/// Whose word is taken that a site is the one it says it is.
#[derive(Clone, Debug, Default)]
pub enum Roots {
    /// The certificate authorities this system trusts.
    #[default]
    System,
    /// These certificates alone, each in DER.
    These(Vec<Vec<u8>>),
}

/// What the server's fetches share: whose word on a certificate they take, and their turns.
pub struct Fetches {
    roots: Roots,
    turns: Arc<Semaphore>,
}

impl Fetches {
    pub fn new(roots: Roots) -> Self {
        Self { roots, turns: Arc::new(Semaphore::new(AT_ONCE)) }
    }

    /// Waits for a turn to fetch, as long as a fetch itself may take. The turn is to be kept until the
    /// fetch has ended, whether or not whoever asked still waits for it.
    pub async fn turn(&self) -> Answer<OwnedSemaphorePermit> {
        let busy = || Refusal::new(Code::Slow, "The server is fetching all it can just now. Try again in a moment.");
        let turn = tokio::time::timeout(LIMIT, self.turns.clone().acquire_owned()).await;
        turn.map_err(|_| busy())?.map_err(|_| busy())
    }
}

/// A picture as its site sent it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    pub kind: ImageType,
    pub bytes: Vec<u8>,
}

/// Why a fetch brought nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failed {
    /// The guard refused a step.
    Refused(FetchError),
    /// The site answered with this status and nothing to read.
    Status(u16),
    /// It took longer than [`LIMIT`].
    Slow,
    /// The site could not be reached or did not speak as expected. The reason is the network's own
    /// wording, which is kept for the log and not told.
    Unreachable(String),
}

impl fmt::Display for Failed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Where a name leads inside the owner's network is not for whoever asks to learn.
            Self::Refused(FetchError::PrivateAddress(_)) => write!(f, "the host is on a private network"),
            Self::Refused(error) => error.fmt(f),
            Self::Status(status) => write!(f, "the site answered {status}"),
            Self::Slow => write!(f, "the site took longer than {} seconds", LIMIT.as_secs()),
            Self::Unreachable(_) => write!(f, "the site could not be reached"),
        }
    }
}

impl From<FetchError> for Failed {
    fn from(error: FetchError) -> Self {
        Self::Refused(error)
    }
}

impl From<Failed> for Refusal {
    fn from(failed: Failed) -> Self {
        use FetchError::{Held, PrivateAddress, UntrustedSource, Url};
        match &failed {
            // The rules refuse it: asking again changes nothing.
            Failed::Refused(Url(_) | Held | UntrustedSource | PrivateAddress(_)) => {
                Self::new(Code::Untrusted, format!("This address is not fetched: {failed}."))
            }
            _ => Self::new(Code::Upstream, format!("Nothing was fetched: {failed}.")),
        }
    }
}

/// What a person trusts beyond what the server's owner does. By [`Person::id`](letters_api::Person).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Trusted {
    /// The documents they took out of safe mode, by id.
    #[serde(default)]
    documents: Vec<String>,
}

const TRUSTED: Table<Trusted> = Table::new("trusted");

impl App {
    /// The person trusts the document `id` from now on.
    pub fn trust(&self, who: &Signed, id: &str) -> Answer<()> {
        self.locate(who, id)?;
        Ok(self.store.write(|batch| {
            let mut trusted = batch.get(&TRUSTED, &who.person.id)?.unwrap_or_default();
            if !trusted.documents.iter().any(|document| document == id) {
                trusted.documents.push(id.to_owned());
            }
            batch.put(&TRUSTED, &who.person.id, &trusted)
        })?)
    }

    /// Whether `who` trusts the document `id`, which they were found to reach. One in a space of theirs is
    /// vouched for by whoever let them into the space. One that only a share leads to is in safe mode
    /// until they say they trust it.
    pub fn trusts(&self, who: &Signed, found: &Found, id: &str) -> Answer<bool> {
        if found.space.is_some() {
            return Ok(true);
        }
        let trusted = self.store.get(&TRUSTED, &who.person.id)?.unwrap_or_default();
        Ok(trusted.documents.iter().any(|document| document == id))
    }

    /// Starts a fetch for the document `id` as `who` may: `start` asks the document's gate for it.
    fn guarded<T>(&self, who: &Signed, id: &str, start: impl FnOnce(&Gate) -> Result<T, FetchError>) -> Answer<T> {
        let found = self.locate(who, id)?;
        let sources = self.config.sources().unwrap_or_default();
        // What a person trusts by themselves adds no source: only the owner's sources reach into the
        // owner's network.
        let origin = if self.trusts(who, &found, id)? { Origin::Created } else { Origin::Outside };
        let trust = Trust::default();
        let gate = trust.gate(id, origin).on_server(&sources);
        Ok(start(&gate).map_err(Failed::Refused)?)
    }

    /// Fetches the picture a document links to. One nobody asked for is fetched from a trusted source
    /// alone, as when a document opens.
    pub fn fetch_picture(&self, who: &Signed, id: &str, fetch: &letters_api::Fetch) -> Answer<Fetched> {
        let guard = self.guarded(who, id, |gate| match fetch.asked {
            true => gate.fetch_on_request(&fetch.url, Limits::default()),
            false => gate.fetch_on_open(&fetch.url, Limits::default()),
        })?;
        let (_, kind, bytes) = get(guard, &self.fetches.roots, PICTURES)?;
        Ok(Fetched { kind, bytes })
    }

    /// Reads what the page behind a link says of itself. The page's picture is named by its whole
    /// address, to be fetched in turn.
    pub fn fetch_preview(&self, who: &Signed, id: &str, fetch: &letters_api::Fetch) -> Answer<Preview> {
        if !fetch.asked {
            return Err(Refusal::new(Code::Untrusted, "A preview is fetched only when someone asks for it."));
        }
        let guard = self.guarded(who, id, |gate| gate.fetch_page(&fetch.url, Limits::page()))?;
        let (url, (), bytes) = get(guard, &self.fetches.roots, PAGES)?;
        let PageSummary { title, description, image } = page_summary(&bytes);
        let picture = image.and_then(|written| url.join(&written).ok()).map(|address| address.to_string());
        Ok(Preview { title, description, picture })
    }
}

/// Looks a host up and keeps the addresses the guard lets a fetch connect to. The connection is made to
/// one of them and the name is not looked up again, so it cannot lead elsewhere a moment later.
#[derive(Debug)]
struct Guarded<K> {
    guard: Fetch<K>,
    inner: DefaultResolver,
    /// Why no address was kept, when none was.
    refused: Arc<Mutex<Option<FetchError>>>,
}

impl<K: Kind + fmt::Debug + Send + Sync + 'static> Resolver for Guarded<K> {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, Error> {
        let found = self.inner.resolve(uri, config, timeout)?;
        let mut kept = self.empty();
        for address in found.iter() {
            match self.guard.connect(address.ip()) {
                Ok(()) => kept.push(*address),
                Err(error) => *self.refused.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error),
            }
        }
        if kept.is_empty() { Err(Error::HostNotFound) } else { Ok(kept) }
    }
}

/// Fetches what `guard` was made for, asked for as `accept`, and says where it was found at last.
fn get<K>(mut guard: Fetch<K>, roots: &Roots, accept: &str) -> Result<(HttpsUrl, K::Found, Vec<u8>), Failed>
where
    K: Kind + Clone + fmt::Debug + Send + Sync + 'static,
{
    let started = Instant::now();
    loop {
        let left = LIMIT.checked_sub(started.elapsed()).filter(|left| !left.is_zero()).ok_or(Failed::Slow)?;
        let refused = Arc::new(Mutex::new(None));
        let resolver = Guarded { guard: guard.clone(), inner: DefaultResolver::default(), refused: refused.clone() };
        // An agent for each address: it keeps no connection and remembers nothing between them.
        let agent = Agent::with_parts(config(roots, left, accept), DefaultConnector::default(), resolver);
        let mut response = agent.get(guard.url().to_string()).call().map_err(|error| {
            let refused = refused.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
            refused.map_or_else(|| failed(error), Failed::Refused)
        })?;

        let (status, headers) = (response.status(), response.headers());
        if status.is_redirection() {
            let to = text(headers, header::LOCATION).ok_or(Failed::Status(status.as_u16()))?;
            guard.redirect(to)?;
            continue;
        }
        if !status.is_success() {
            return Err(Failed::Status(status.as_u16()));
        }
        let length = text(headers, header::CONTENT_LENGTH).and_then(|length| length.trim().parse().ok());
        guard.response(text(headers, header::CONTENT_TYPE), length)?;

        let (mut bytes, mut piece) = (Vec::new(), vec![0; 64 * 1024]);
        let mut body = response.body_mut().as_reader();
        loop {
            match body.read(&mut piece).map_err(|error| failed(Error::Io(error)))? {
                0 => break,
                read => {
                    guard.receive(read)?;
                    bytes.extend_from_slice(&piece[..read]);
                }
            }
        }
        let url = guard.url().clone();
        return Ok((url, guard.finish(&bytes)?, bytes));
    }
}

fn text(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name)?.to_str().ok()
}

fn failed(error: Error) -> Failed {
    match error {
        Error::Timeout(_) => Failed::Slow,
        Error::Io(error) if error.kind() == std::io::ErrorKind::TimedOut => Failed::Slow,
        other => {
            crate::log(&format!("a fetch failed: {other}"));
            Failed::Unreachable(other.to_string())
        }
    }
}

fn config(roots: &Roots, left: Duration, accept: &str) -> Config {
    let roots = match roots {
        Roots::System => RootCerts::PlatformVerifier,
        Roots::These(certificates) => {
            let certificates: Vec<_> = certificates.iter().map(|der| Certificate::from_der(der).to_owned()).collect();
            RootCerts::new_with_certs(&certificates)
        }
    };
    Agent::config_builder()
        .https_only(true)
        // A proxy would look the name up itself, where the guard cannot see the address.
        .proxy(None)
        // Redirects are the guard's to allow, one at a time.
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .user_agent(CLIENT)
        .accept(accept)
        // The body is counted as it arrives, so it is asked for as it is.
        .accept_encoding(AutoHeaderValue::None)
        .max_idle_connections(0)
        .timeout_global(Some(left))
        .tls_config(TlsConfig::builder().root_certs(roots).build())
        .build()
}
