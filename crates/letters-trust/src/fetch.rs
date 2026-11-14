//! The guard on one remote image fetch.
//!
//! The shell owns the socket and asks before each step:
//!
//! 1. Resolve the host of [`ImageFetch::url`], pass every address to [`ImageFetch::connect`], and connect
//!    to an address that passed. A private address passes only for a trusted source.
//! 2. On a redirect call [`ImageFetch::redirect`] and start again from step 1. Never let the HTTP client
//!    follow redirects by itself.
//! 3. Pass the headers of a successful response to [`ImageFetch::response`], and the size of each piece of
//!    the body to [`ImageFetch::receive`] as it arrives. Count the body as it will be kept: a compressed
//!    response is counted after it is unpacked.
//! 4. Pass the whole body to [`ImageFetch::finish`], which names the image's type.
//!
//! Any error ends the fetch. The shell sets its own time limit, since the core has no clock, and sends no
//! cookies or other credentials.

use std::fmt;
use std::net::IpAddr;

use crate::{HttpsUrl, Source, UrlError, is_public};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The largest body, in bytes.
    pub max_bytes: u64,
    pub max_redirects: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_bytes: 16 * 1024 * 1024, max_redirects: 3 }
    }
}

/// The image types a fetch accepts: the ones Letters decodes. SVG is left out, since it can carry scripts
/// and load further addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageType {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl ImageType {
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }

    /// Names the type by how the bytes begin. What the server calls them is not evidence.
    pub fn sniff(body: &[u8]) -> Option<Self> {
        match body {
            [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', ..] => Some(Self::Png),
            [0xff, 0xd8, 0xff, ..] => Some(Self::Jpeg),
            [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => Some(Self::Gif),
            [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(Self::Webp),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchError {
    Url(UrlError),
    /// The document is in safe mode.
    Held,
    /// The address, or one it redirected to, is not a trusted source and the user did not ask for it.
    UntrustedSource,
    /// The host's name led to this machine or a private network, and no trusted source vouches for it.
    PrivateAddress(IpAddr),
    TooManyRedirects,
    /// The body is, or says it will be, larger than the limit in bytes.
    TooLarge(u64),
    /// The response is not a PNG, JPEG, GIF or WebP image.
    NotAnImage,
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Url(error) => error.fmt(f),
            Self::Held => write!(f, "the document is in safe mode"),
            Self::UntrustedSource => write!(f, "the address is not a trusted source"),
            Self::PrivateAddress(address) => write!(f, "the host is at {address}, on this device or a private network"),
            Self::TooManyRedirects => write!(f, "the address redirected too many times"),
            Self::TooLarge(limit) => write!(f, "the image is larger than {limit} bytes"),
            Self::NotAnImage => write!(f, "the address does not hold a PNG, JPEG, GIF or WebP image"),
        }
    }
}

impl std::error::Error for FetchError {}

/// One fetch in progress. Made by [`Gate::fetch_on_open`](crate::Gate::fetch_on_open) or
/// [`Gate::fetch_on_request`](crate::Gate::fetch_on_request).
#[derive(Clone, Debug)]
pub struct ImageFetch {
    url: HttpsUrl,
    limits: Limits,
    /// The sources a redirect must stay inside. `None` when the user asked for the image.
    within: Option<Vec<Source>>,
    /// The sources that may be on a private address.
    vouchers: Vec<Source>,
    redirects: u32,
    received: u64,
}

impl ImageFetch {
    pub(crate) fn new(url: HttpsUrl, limits: Limits, within: Option<Vec<Source>>, vouchers: Vec<Source>) -> Self {
        Self { url, limits, within, vouchers, redirects: 0, received: 0 }
    }

    /// The address to request now. Request this text and not the document's own spelling of it.
    pub fn url(&self) -> &HttpsUrl {
        &self.url
    }

    /// Checks an address the host's name resolved to, before connecting to it. Only a trusted source may
    /// be on a private address.
    pub fn connect(&self, address: IpAddr) -> Result<(), FetchError> {
        if is_public(address) || self.vouched(&self.url) { Ok(()) } else { Err(FetchError::PrivateAddress(address)) }
    }

    /// Follows a redirect to the response's `Location`, which is held to every rule the first address was.
    pub fn redirect(&mut self, location: &str) -> Result<(), FetchError> {
        if self.redirects >= self.limits.max_redirects {
            return Err(FetchError::TooManyRedirects);
        }
        let next = self.url.join(location).map_err(FetchError::Url)?;
        if !next.is_public() && !self.vouched(&next) {
            return Err(FetchError::Url(UrlError::Private));
        }
        if self.within.as_ref().is_some_and(|sources| !sources.iter().any(|source| source.allows(&next))) {
            return Err(FetchError::UntrustedSource);
        }
        self.redirects += 1;
        self.url = next;
        Ok(())
    }

    /// Checks the headers of the final response, before any of the body is read.
    pub fn response(&self, content_type: Option<&str>, content_length: Option<u64>) -> Result<(), FetchError> {
        if content_length.is_some_and(|length| length > self.limits.max_bytes) {
            return Err(FetchError::TooLarge(self.limits.max_bytes));
        }
        let essence = content_type.and_then(|value| value.split(';').next()).unwrap_or_default().trim();
        match essence.to_ascii_lowercase().as_str() {
            // File stores often send images unlabelled. The bytes decide in `finish`.
            "" | "application/octet-stream" | "binary/octet-stream" => Ok(()),
            "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp" => Ok(()),
            _ => Err(FetchError::NotAnImage),
        }
    }

    /// Counts a piece of the body as it arrives, so an endless one is cut off.
    pub fn receive(&mut self, bytes: usize) -> Result<(), FetchError> {
        self.received = self.received.saturating_add(bytes as u64);
        self.check_size(self.received)
    }

    /// Checks the whole body and names its type.
    pub fn finish(self, body: &[u8]) -> Result<ImageType, FetchError> {
        self.check_size(body.len() as u64)?;
        ImageType::sniff(body).ok_or(FetchError::NotAnImage)
    }

    fn vouched(&self, url: &HttpsUrl) -> bool {
        self.vouchers.iter().any(|source| source.allows(url))
    }

    fn check_size(&self, bytes: u64) -> Result<(), FetchError> {
        if bytes > self.limits.max_bytes { Err(FetchError::TooLarge(self.limits.max_bytes)) } else { Ok(()) }
    }
}
