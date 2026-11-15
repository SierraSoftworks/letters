//! Fetching a picture that is still a link.
//!
//! `letters_trust` decides what may be fetched, and its [`ImageFetch`] is asked before every step. This
//! owns the socket: it connects only to addresses the guard passed, follows redirects by hand, sends no
//! cookies and nothing that tells one person from another, and gives up after [`LIMIT`].

use std::fmt;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use letters_trust::{FetchError, ImageFetch, ImageType};
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
const ACCEPT: &str = "image/png, image/jpeg, image/gif, image/webp";

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

/// Why a fetch brought no picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failed {
    /// The guard refused a step.
    Refused(FetchError),
    /// The server answered with this status and no picture.
    Status(u16),
    /// It took longer than [`LIMIT`].
    Slow,
    /// The server could not be reached or did not speak as expected.
    Unreachable(String),
}

impl fmt::Display for Failed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(error) => error.fmt(f),
            Self::Status(status) => write!(f, "the server answered {status}"),
            Self::Slow => write!(f, "the server took longer than {} seconds", LIMIT.as_secs()),
            Self::Unreachable(reason) => write!(f, "the server could not be reached: {reason}"),
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
struct Guarded {
    guard: ImageFetch,
    inner: DefaultResolver,
    /// Why no address was kept, when none was.
    refused: Arc<Mutex<Option<FetchError>>>,
}

impl Resolver for Guarded {
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
pub fn fetch(mut guard: ImageFetch, roots: &Roots) -> Result<Fetched, Failed> {
    let started = Instant::now();
    loop {
        let left = LIMIT.checked_sub(started.elapsed()).filter(|left| !left.is_zero()).ok_or(Failed::Slow)?;
        let refused = Arc::new(Mutex::new(None));
        let resolver = Guarded { guard: guard.clone(), inner: DefaultResolver::default(), refused: refused.clone() };
        // An agent for each address: it keeps no connection and remembers nothing between them.
        let agent = Agent::with_parts(config(roots, left), DefaultConnector::default(), resolver);
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
        let kind = guard.finish(&bytes)?;
        return Ok(Fetched { kind, bytes });
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

fn config(roots: &Roots, left: Duration) -> Config {
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
        .accept(ACCEPT)
        // The body is counted as it arrives, so it is asked for as it is.
        .accept_encoding(AutoHeaderValue::None)
        .max_idle_connections(0)
        .timeout_global(Some(left))
        .tls_config(TlsConfig::builder().root_certs(roots).build())
        .build()
}
