//! The guard on one remote fetch: a picture, or the page a link preview is made from.
//!
//! The shell owns the socket and asks before each step:
//!
//! 1. Resolve the host of [`Fetch::url`], pass every address to [`Fetch::connect`], and connect to an
//!    address that passed. A private address passes only for a trusted source.
//! 2. On a redirect call [`Fetch::redirect`] and start again from step 1. Never let the HTTP client follow
//!    redirects by itself.
//! 3. Pass the headers of a successful response to [`Fetch::response`], and the size of each piece of the
//!    body to [`Fetch::receive`] as it arrives. Count the body as it will be kept: a compressed response is
//!    counted after it is unpacked.
//! 4. Pass the whole body to [`Fetch::finish`], which for a picture names its type.
//!
//! Any error ends the fetch, and says why in words the shell can show. The shell sets its own time limit,
//! since the core has no clock, and sends no cookies or other credentials.

use std::fmt;
use std::marker::PhantomData;
use std::net::IpAddr;

use crate::{HttpsUrl, Source, UrlError, is_public};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The largest body, in bytes.
    pub max_bytes: u64,
    pub max_redirects: u32,
}

/// The limits on a picture.
impl Default for Limits {
    fn default() -> Self {
        Self { max_bytes: 16 * 1024 * 1024, max_redirects: 3 }
    }
}

impl Limits {
    /// The limits on a page read for a link preview. What a preview shows sits at the top of the page.
    pub fn page() -> Self {
        Self { max_bytes: 1024 * 1024, max_redirects: 3 }
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
    /// The response does not say it is an HTML page.
    NotAPage,
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Url(error) => error.fmt(f),
            Self::Held => write!(f, "the document is in safe mode"),
            Self::UntrustedSource => write!(f, "the address is not a trusted source"),
            Self::PrivateAddress(address) => write!(f, "the host is at {address}, on this device or a private network"),
            Self::TooManyRedirects => write!(f, "the address redirected too many times"),
            Self::TooLarge(limit) => write!(f, "the address holds more than {limit} bytes"),
            Self::NotAnImage => write!(f, "the address does not hold a PNG, JPEG, GIF or WebP image"),
            Self::NotAPage => write!(f, "the address does not hold a web page"),
        }
    }
}

impl std::error::Error for FetchError {}

/// What a fetch is for, which decides the responses it accepts: [`Image`] or [`Page`].
pub trait Kind: sealed::Sealed {
    /// What [`Fetch::finish`] learns from the body.
    type Found;
    #[doc(hidden)]
    const REFUSAL: FetchError;
    /// Whether a response with this media type, in lower case and empty when it has none, is read at all.
    #[doc(hidden)]
    fn labelled(media_type: &str) -> bool;
    #[doc(hidden)]
    fn find(body: &[u8]) -> Option<Self::Found>;
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Image {}
    impl Sealed for super::Page {}
}

/// A picture. Its bytes say what it is, and a response that claims to be something else is not read.
#[derive(Clone, Copy, Debug)]
pub struct Image;

/// An HTML page, read for its title, description and picture. The response must say it is one.
#[derive(Clone, Copy, Debug)]
pub struct Page;

impl Kind for Image {
    type Found = ImageType;
    const REFUSAL: FetchError = FetchError::NotAnImage;

    fn labelled(media_type: &str) -> bool {
        // File stores often send images unlabelled. The bytes decide in `finish`.
        let unlabelled = matches!(media_type, "" | "application/octet-stream" | "binary/octet-stream");
        unlabelled || matches!(media_type, "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp")
    }

    fn find(body: &[u8]) -> Option<ImageType> {
        ImageType::sniff(body)
    }
}

impl Kind for Page {
    type Found = ();
    const REFUSAL: FetchError = FetchError::NotAPage;

    fn labelled(media_type: &str) -> bool {
        matches!(media_type, "text/html" | "application/xhtml+xml")
    }

    fn find(_: &[u8]) -> Option<()> {
        Some(())
    }
}

/// The fetch of a picture. Made by [`Gate::fetch_on_open`](crate::Gate::fetch_on_open) or
/// [`Gate::fetch_on_request`](crate::Gate::fetch_on_request).
pub type ImageFetch = Fetch<Image>;

/// The fetch of a page for a link preview. Made by [`Gate::fetch_page`](crate::Gate::fetch_page).
pub type PageFetch = Fetch<Page>;

/// One fetch in progress.
#[derive(Clone, Debug)]
pub struct Fetch<K> {
    url: HttpsUrl,
    limits: Limits,
    /// The sources a redirect must stay inside. `None` when the user asked for the fetch.
    within: Option<Vec<Source>>,
    /// The sources that may be on a private address.
    vouchers: Vec<Source>,
    redirects: u32,
    received: u64,
    kind: PhantomData<K>,
}

impl<K: Kind> Fetch<K> {
    pub(crate) fn new(url: HttpsUrl, limits: Limits, within: Option<Vec<Source>>, vouchers: Vec<Source>) -> Self {
        Self { url, limits, within, vouchers, redirects: 0, received: 0, kind: PhantomData }
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
        let media_type = content_type.and_then(|value| value.split(';').next()).unwrap_or_default().trim();
        if K::labelled(&media_type.to_ascii_lowercase()) { Ok(()) } else { Err(K::REFUSAL) }
    }

    /// Counts a piece of the body as it arrives, so an endless one is cut off.
    pub fn receive(&mut self, bytes: usize) -> Result<(), FetchError> {
        self.received = self.received.saturating_add(bytes as u64);
        self.check_size(self.received)
    }

    /// Checks the whole body. A picture's bytes name its type.
    pub fn finish(self, body: &[u8]) -> Result<K::Found, FetchError> {
        self.check_size(body.len() as u64)?;
        K::find(body).ok_or(K::REFUSAL)
    }

    fn vouched(&self, url: &HttpsUrl) -> bool {
        self.vouchers.iter().any(|source| source.allows(url))
    }

    fn check_size(&self, bytes: u64) -> Result<(), FetchError> {
        if bytes > self.limits.max_bytes { Err(FetchError::TooLarge(self.limits.max_bytes)) } else { Ok(()) }
    }
}
