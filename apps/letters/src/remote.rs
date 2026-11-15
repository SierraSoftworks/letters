//! Fetching a picture that is still a link, or the page a link preview is made from.
//!
//! `letters_trust` decides what may be fetched, and its [`Fetch`] is asked before every step. This
//! owns the socket: it connects only to addresses the guard passed, follows redirects by hand, sends no
//! cookies and nothing that tells one person from another, and gives up after [`LIMIT`].

use std::fmt;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use letters_convert::{PageSummary, page_summary};
use letters_trust::{Fetch, FetchError, HttpsUrl, ImageFetch, ImageType, Kind, PageFetch};
use ureq::config::{AutoHeaderValue, Config};
use ureq::http::{HeaderMap, header};
use ureq::tls::{Certificate, RootCerts, TlsConfig};
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use ureq::{Agent, Error};

/// How long a fetch may take from its first step to its last byte, redirects included.
pub const LIMIT: Duration = Duration::from_secs(10);
/// All a server is told about who asks.
const CLIENT: &str = "Letters";
/// The kinds of picture a document shows.
const PICTURES: &str = "image/png, image/jpeg, image/gif, image/webp";
/// What a page is asked for as.
const PAGES: &str = "text/html, application/xhtml+xml";

/// Whose word is taken that a server is the one it says it is.
#[derive(Clone, Debug, Default)]
pub enum Roots {
    /// The certificate authorities this system trusts.
    #[default]
    System,
    /// These certificates alone, each in DER.
    These(Vec<Vec<u8>>),
}

/// A picture as its server sent it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    pub kind: ImageType,
    pub bytes: Vec<u8>,
}

/// A page as its server sent it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedPage {
    /// Where the page was found once its redirects were followed. An address written on the page is read
    /// against this one.
    pub url: HttpsUrl,
    pub bytes: Vec<u8>,
}

/// What a page says of itself, for a link preview to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub title: Option<String>,
    pub description: Option<String>,
    /// The file of the picture the page shows itself by, as its server sent it.
    pub picture: Option<Vec<u8>>,
}

/// Why a fetch brought nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failed {
    /// The guard refused a step.
    Refused(FetchError),
    /// The server answered with this status and nothing to read.
    Status(u16),
    /// It took longer than [`LIMIT`].
    Slow,
    /// The server could not be reached or did not speak as expected.
    Unreachable(String),
    /// The page came, and has no title, description or picture for a preview to show.
    Nothing,
}

impl fmt::Display for Failed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(error) => error.fmt(f),
            Self::Status(status) => write!(f, "the server answered {status}"),
            Self::Slow => write!(f, "the server took longer than {} seconds", LIMIT.as_secs()),
            // The reason is the network's own wording, which is kept for whoever debugs and not shown.
            Self::Unreachable(_) => write!(f, "the server could not be reached"),
            Self::Nothing => write!(f, "the page says nothing about itself"),
        }
    }
}

impl std::error::Error for Failed {}

impl From<FetchError> for Failed {
    fn from(error: FetchError) -> Self {
        Self::Refused(error)
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

/// Fetches the picture `guard` was made for. This waits on the network, so it is not for the UI thread.
pub fn fetch(guard: ImageFetch, roots: &Roots) -> Result<Fetched, Failed> {
    let (_, kind, bytes) = get(guard, roots, PICTURES)?;
    Ok(Fetched { kind, bytes })
}

/// Fetches the page `guard` was made for, as [`fetch`] does a picture.
pub fn fetch_page(guard: PageFetch, roots: &Roots) -> Result<FetchedPage, Failed> {
    let (url, (), bytes) = get(guard, roots, PAGES)?;
    Ok(FetchedPage { url, bytes })
}

/// Fetches the page `guard` was made for and reads what it says of itself, for a link preview.
///
/// The page names its picture by an address, which is read against the page's own and fetched only if
/// `picture` gives a guard for it. A picture that is not to be had leaves the preview without one. Each
/// of the two fetches has its own [`LIMIT`].
pub fn preview(
    guard: PageFetch,
    picture: &dyn Fn(&str) -> Result<ImageFetch, FetchError>,
    roots: &Roots,
) -> Result<Preview, Failed> {
    let page = fetch_page(guard, roots)?;
    let PageSummary { title, description, image } = page_summary(&page.bytes);
    let address = image.and_then(|written| page.url.join(&written).ok());
    let guard = address.and_then(|address| picture(&address.to_string()).ok());
    let picture = guard.and_then(|guard| fetch(guard, roots).ok()).map(|fetched| fetched.bytes);
    if title.is_none() && description.is_none() && picture.is_none() {
        return Err(Failed::Nothing);
    }
    Ok(Preview { title, description, picture })
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
        other => Failed::Unreachable(other.to_string()),
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
