//! How one open document is treated.

use crate::fetch::{FetchError, ImageFetch, Limits};
use crate::url::scheme;
use crate::{HttpsUrl, Source, Trust, UrlError};

/// Where an opened document came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin<'a> {
    /// The user made it here. The shell also records its id with `TrustStore::trust_document`, so that it
    /// stays trusted wherever it is saved.
    Created,
    /// Read from a folder on this device. The path is absolute, with links resolved by the shell.
    File(&'a str),
    /// Imported, downloaded or opened from a share link. No folder vouches for it.
    Outside,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Trusted,
    /// The document reads and edits as normal. Remote images, links, linking maths and merges are held back.
    Safe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageDecision {
    /// A trusted source in a trusted document: fetch on open.
    Load,
    /// Keep the frame and its cloud badge, and fetch when the user asks: "Load once", "Always load from
    /// this host" or "Embed".
    OnRequest,
    /// Safe mode: keep the frame, and fetch nothing until the document is trusted.
    Held,
    /// Never fetched.
    Refused(UrlError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkDecision {
    Open,
    /// Show the whole address and open it only on a yes.
    Ask,
    /// Never opened: it would run something on this device instead of visiting a page.
    Refuse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeDecision {
    /// The user's click on "Merge conflicts" is enough.
    Merge,
    /// Name the copy and where it lies, and merge only on a yes.
    Ask,
}

/// The decisions for one open document.
#[derive(Clone, Copy, Debug)]
pub struct Gate<'a> {
    trust: &'a Trust,
    server: &'a [Source],
    mode: Mode,
}

impl Trust {
    /// A document is trusted when the user made it, trusted it by id, or keeps it in a trusted folder.
    /// One that arrives from outside is vouched for by its id alone, wherever it is saved.
    pub fn mode(&self, id: &str, origin: Origin<'_>) -> Mode {
        let trusted = match origin {
            Origin::Created => true,
            Origin::File(path) => self.trusts_document(id) || self.trusts_folder(path),
            Origin::Outside => self.trusts_document(id),
        };
        if trusted { Mode::Trusted } else { Mode::Safe }
    }

    pub fn gate(&self, id: &str, origin: Origin<'_>) -> Gate<'_> {
        Gate { trust: self, server: &[], mode: self.mode(id, origin) }
    }
}

impl<'a> Gate<'a> {
    /// Adds the sources the server's administrator trusts for everyone.
    pub fn with_sources(self, server: &'a [Source]) -> Self {
        Self { server, ..self }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether an image that is still a remote link may load.
    pub fn image(&self, url: &str) -> ImageDecision {
        match HttpsUrl::parse(url) {
            Err(error) => ImageDecision::Refused(error),
            Ok(_) if self.mode == Mode::Safe => ImageDecision::Held,
            Ok(url) if self.sources().any(|source| source.allows(&url)) => ImageDecision::Load,
            Ok(_) => ImageDecision::OnRequest,
        }
    }

    /// What a click on a link does.
    pub fn link(&self, href: &str) -> LinkDecision {
        let href = href.trim();
        if href.is_empty() || href.chars().any(char::is_control) {
            return LinkDecision::Refuse;
        }
        // A place in this document.
        if href.starts_with('#') {
            return LinkDecision::Open;
        }
        let by_mode = match self.mode {
            Mode::Trusted => LinkDecision::Open,
            Mode::Safe => LinkDecision::Ask,
        };
        match scheme(href).as_deref() {
            // `https://bank.example@evil.example/` goes to the host after the `@`.
            Some("http" | "https") if authority(href).contains('@') => LinkDecision::Ask,
            Some("http" | "https" | "mailto") => by_mode,
            Some("javascript" | "vbscript" | "data" | "blob" | "file") => LinkDecision::Refuse,
            // Another app's scheme, or a relative address: the user sees it first, trusted or not.
            _ => LinkDecision::Ask,
        }
    }

    /// Whether maths may use the commands that link or load, such as `\href` and `\includegraphics`.
    pub fn maths_links(&self) -> bool {
        self.mode == Mode::Trusted
    }

    /// Whether a copy that carries this document's id may be merged into it on one click. `copy` is the
    /// copy's path.
    ///
    /// The id cannot vouch for the copy, since anyone who has seen the document can forge it. Only a
    /// trusted folder can.
    pub fn merge(&self, copy: &str) -> MergeDecision {
        match self.mode == Mode::Trusted && self.trust.trusts_folder(copy) {
            true => MergeDecision::Merge,
            false => MergeDecision::Ask,
        }
    }

    /// Starts the fetch of an image that loads on open. It is refused unless [`image`](Self::image) says
    /// `Load`, and a redirect may not leave the trusted sources.
    pub fn fetch_on_open(&self, url: &str, limits: Limits) -> Result<ImageFetch, FetchError> {
        match self.image(url) {
            ImageDecision::OnRequest => Err(FetchError::UntrustedSource),
            _ => self.fetch(url, limits, Some(self.sources().cloned().collect())),
        }
    }

    /// Starts a fetch the user asked for: "Load once", "Always load from this host" or "Embed". Safe mode
    /// still refuses it.
    pub fn fetch_on_request(&self, url: &str, limits: Limits) -> Result<ImageFetch, FetchError> {
        self.fetch(url, limits, None)
    }

    fn fetch(&self, url: &str, limits: Limits, within: Option<Vec<Source>>) -> Result<ImageFetch, FetchError> {
        let url = HttpsUrl::parse(url).map_err(FetchError::Url)?;
        match self.mode {
            Mode::Trusted => Ok(ImageFetch::new(url, limits, within)),
            Mode::Safe => Err(FetchError::Held),
        }
    }

    fn sources(&self) -> impl Iterator<Item = &'a Source> {
        self.trust.sources().iter().chain(self.server)
    }
}

/// The part of an address that names its host, with any user name still on it.
fn authority(href: &str) -> &str {
    let rest = href.split_once(':').map_or(href, |(_, rest)| rest).trim_start_matches(['/', '\\']);
    &rest[..rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len())]
}
