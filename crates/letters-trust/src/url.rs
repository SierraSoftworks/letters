//! HTTPS addresses, read strictly.
//!
//! A trust decision is only worth what the fetcher does with the same text. Two URL parsers can disagree
//! about which host an odd address names, and that gap is how a guard is slipped. So this reader accepts
//! only addresses that every parser reads the same way, and writes them back in one plain form for the
//! fetcher to use.

use std::fmt::{self, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::is_public;

/// An HTTPS address on the public internet, in its plain form. `Display` writes the text to fetch.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HttpsUrl {
    host: Host,
    port: u16,
    path: String,
    query: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Host {
    /// A DNS name, in lower case and without a trailing dot.
    Name(String),
    Address(IpAddr),
}

/// Why an address is not loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UrlError {
    /// Anything but `https://`: plain HTTP would show the request to everyone on the way.
    NotHttps,
    /// A user name or password in the address, which also disguises the real host.
    Credentials,
    /// The host is this machine or a private network.
    Private,
    /// Not a form that every program reads the same way.
    Malformed,
}

impl HttpsUrl {
    pub fn parse(text: &str) -> Result<Self, UrlError> {
        let text = text.trim();
        let Some(rest) = strip_scheme(text) else {
            return Err(UrlError::NotHttps);
        };
        // Browsers read `\` as `/` and drop tabs and line breaks. Others do not.
        if text.contains('\\') || text.chars().any(char::is_control) {
            return Err(UrlError::Malformed);
        }
        let rest = rest.strip_prefix("//").ok_or(UrlError::Malformed)?;
        let (authority, tail) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
        if authority.contains('@') {
            return Err(UrlError::Credentials);
        }
        let (host, port) = split_authority(authority)?;
        if !host.is_public() {
            return Err(UrlError::Private);
        }
        // The fragment never reaches the server.
        let tail = tail.split_once('#').map_or(tail, |(tail, _)| tail);
        let (path, query) = match tail.split_once('?') {
            Some((path, query)) => (path, Some(encode(query)?)),
            None => (tail, None),
        };
        Ok(Self { host, port, path: resolve(path)?, query })
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Starts with `/`, with `.` and `..` already resolved.
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    pub(crate) fn same_origin(&self, other: &Self) -> bool {
        self.host == other.host && self.port == other.port
    }

    /// The address at this host's root.
    pub(crate) fn root(&self) -> Self {
        Self { host: self.host.clone(), port: self.port, path: "/".into(), query: None }
    }

    /// Reads the `Location` of a redirect, which may be relative to this address.
    pub(crate) fn join(&self, location: &str) -> Result<Self, UrlError> {
        let location = location.trim();
        let origin = self.root().to_string();
        let origin = origin.trim_end_matches('/');
        let folder = &self.path[..=self.path.rfind('/').unwrap_or(0)];
        let absolute = if scheme(location).is_some() {
            location.to_string()
        } else if location.starts_with("//") {
            format!("https:{location}")
        } else if location.starts_with('/') {
            format!("{origin}{location}")
        } else if location.starts_with(['?', '#']) || location.is_empty() {
            format!("{origin}{}{location}", self.path)
        } else {
            format!("{origin}{folder}{location}")
        };
        Self::parse(&absolute)
    }
}

impl Host {
    fn is_public(&self) -> bool {
        match self {
            Self::Name(name) => name != "localhost" && !name.ends_with(".localhost"),
            Self::Address(address) => is_public(*address),
        }
    }
}

impl fmt::Display for HttpsUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "https://{}", self.host)?;
        if self.port != 443 {
            write!(f, ":{}", self.port)?;
        }
        f.write_str(&self.path)?;
        match &self.query {
            Some(query) => write!(f, "?{query}"),
            None => Ok(()),
        }
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(name) => f.write_str(name),
            Self::Address(IpAddr::V4(address)) => write!(f, "{address}"),
            Self::Address(IpAddr::V6(address)) => write!(f, "[{address}]"),
        }
    }
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotHttps => "the address is not HTTPS",
            Self::Credentials => "the address carries a user name or password",
            Self::Private => "the address is on this device or a private network",
            Self::Malformed => "the address is not written in a standard form",
        })
    }
}

impl std::error::Error for UrlError {}

/// The scheme of an address, in lower case: the letters before its first `:`.
pub(crate) fn scheme(text: &str) -> Option<String> {
    let (scheme, _) = text.split_once(':')?;
    let sound = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    sound.then(|| scheme.to_ascii_lowercase())
}

fn strip_scheme(text: &str) -> Option<&str> {
    (scheme(text)? == "https").then(|| &text["https:".len()..])
}

fn split_authority(text: &str) -> Result<(Host, u16), UrlError> {
    let (host, port) = match text.strip_prefix('[') {
        Some(rest) => {
            let (address, after) = rest.split_once(']').ok_or(UrlError::Malformed)?;
            let address: Ipv6Addr = address.parse().map_err(|_| UrlError::Malformed)?;
            let port = match after {
                "" => None,
                _ => Some(after.strip_prefix(':').ok_or(UrlError::Malformed)?),
            };
            (Host::Address(address.into()), port)
        }
        None => match text.split_once(':') {
            Some((name, port)) => (name_host(name)?, Some(port)),
            None => (name_host(text)?, None),
        },
    };
    let port = match port {
        None => 443,
        // `parse` alone would take a sign.
        Some(port) if port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit()) => {
            port.parse().ok().filter(|&port| port != 0).ok_or(UrlError::Malformed)?
        }
        Some(_) => return Err(UrlError::Malformed),
    };
    Ok((host, port))
}

fn name_host(text: &str) -> Result<Host, UrlError> {
    let name = text.strip_suffix('.').unwrap_or(text).to_ascii_lowercase();
    let label = |label: &str| {
        (1..=63).contains(&label.len()) && label.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    };
    if name.len() > 253 || !name.split('.').all(label) {
        return Err(UrlError::Malformed);
    }
    // A name that ends in a number is an IPv4 address. Browsers also take `127.1`, `0x7f.0.0.1` and
    // `2130706433`, and other programs take those for names, so only the plain dotted form passes.
    let last = name.rsplit('.').next().unwrap_or_default();
    let hex = last.strip_prefix("0x").is_some_and(|digits| digits.bytes().all(|b| b.is_ascii_hexdigit()));
    if hex || last.bytes().all(|b| b.is_ascii_digit()) {
        let address: Ipv4Addr = name.parse().map_err(|_| UrlError::Malformed)?;
        return Ok(Host::Address(address.into()));
    }
    Ok(Host::Name(name))
}

/// Resolves `.` and `..`, however they are spelled, so a path cannot climb out of a trusted prefix.
fn resolve(path: &str) -> Result<String, UrlError> {
    let mut segments: Vec<String> = Vec::new();
    let mut parts = path.strip_prefix('/').unwrap_or(path).split('/').peekable();
    while let Some(part) = parts.next() {
        let dots = part.to_ascii_lowercase().replace("%2e", ".");
        match dots.as_str() {
            "." => {}
            ".." => drop(segments.pop()),
            _ => {
                segments.push(encode(part)?);
                continue;
            }
        }
        // A path that ends in dots names a folder.
        if parts.peek().is_none() {
            segments.push(String::new());
        }
    }
    Ok(format!("/{}", segments.join("/")))
}

/// Escapes what a path or query may not hold as written. An existing escape must be whole.
fn encode(text: &str) -> Result<String, UrlError> {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    for (at, &byte) in bytes.iter().enumerate() {
        match byte {
            b'%' => {
                let whole = bytes.get(at + 1..at + 3).is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit));
                if !whole {
                    return Err(UrlError::Malformed);
                }
                out.push('%');
            }
            b' ' | b'"' | b'<' | b'>' | b'^' | b'`' | b'{' | b'|' | b'}' | 0x80.. => {
                write!(out, "%{byte:02X}").expect("writing to a string");
            }
            _ => out.push(char::from(byte)),
        }
    }
    Ok(out)
}
