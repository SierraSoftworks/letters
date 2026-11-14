//! Trusted image sources.

use std::fmt;

use crate::{HttpsUrl, UrlError};

/// A place images may load from: an HTTPS host, or the addresses under one of its paths.
///
/// A host is exactly that host and port. Its subdomains are other sources.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Source(HttpsUrl);

impl Source {
    /// Reads `https://img.example.org/charts/`, or a bare `img.example.org`.
    pub fn parse(text: &str) -> Result<Self, UrlError> {
        let text = text.trim();
        let url = match text.contains("://") {
            true => HttpsUrl::parse(text)?,
            false => HttpsUrl::parse(&format!("https://{text}"))?,
        };
        // A source is a prefix, and a prefix cannot end in a query.
        if url.query().is_some() || text.contains('#') {
            return Err(UrlError::Malformed);
        }
        Ok(Self(url))
    }

    /// The whole host an address is on: what "Always load from this host" adds.
    pub fn host_of(url: &HttpsUrl) -> Self {
        Self(url.root())
    }

    pub fn allows(&self, url: &HttpsUrl) -> bool {
        self.0.same_origin(url) && under(self.0.path(), url.path())
    }
}

/// Writes the source as the trust file stores it.
impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

fn under(prefix: &str, path: &str) -> bool {
    if prefix == "/" {
        return true;
    }
    // A server may read an escaped separator as a real one, and so step out of the prefix.
    let lower = path.to_ascii_lowercase();
    if lower.contains("%2f") || lower.contains("%5c") {
        return false;
    }
    // `/charts` covers `/charts/q3.png` and never `/charts-private/`.
    path.strip_prefix(prefix).is_some_and(|rest| prefix.ends_with('/') || rest.is_empty() || rest.starts_with('/'))
}
