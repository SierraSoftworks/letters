//! How one open document is treated.

use crate::fetch::{Fetch, FetchError, ImageFetch, Kind, Limits, PageFetch};
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
    /// Never fetched, unless the user adds a trusted source that covers it.
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
    /// The administrator's sources, when the decisions are made on a server.
    server: Option<&'a [Source]>,
    mode: Mode,
}

impl Trust {
    /// A document is trusted when the user made it, trusted it by id, or keeps it in a trusted folder.
    /// One that arrives from outside is vouched for by its id alone, wherever it is saved, and so is one
    /// the user imported, whenever it is opened again.
    pub fn mode(&self, id: &str, origin: Origin<'_>) -> Mode {
        let trusted = match origin {
            Origin::Created => true,
            Origin::File(path) => self.trusts_document(id) || (self.trusts_folder(path) && !self.imported(id)),
            Origin::Outside => self.trusts_document(id),
        };
        if trusted { Mode::Trusted } else { Mode::Safe }
    }

    pub fn gate(&self, id: &str, origin: Origin<'_>) -> Gate<'_> {
        Gate { trust: self, server: None, mode: self.mode(id, origin) }
    }
}

impl<'a> Gate<'a> {
    /// On a server: adds the sources its administrator trusts for everyone.
    ///
    /// The server fetches inside the administrator's network, so there only these sources may be on a
    /// private address. A user's own source could otherwise point the server at its neighbours.
    pub fn on_server(self, sources: &'a [Source]) -> Self {
        Self { server: Some(sources), ..self }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether an image that is still a remote link may load.
    ///
    /// An address on this device or a private network is refused unless a trusted source covers it.
    pub fn image(&self, url: &str) -> ImageDecision {
        let url = match HttpsUrl::parse(url) {
            Ok(url) => url,
            Err(error) => return ImageDecision::Refused(error),
        };
        if !url.is_public() && !self.vouchers().iter().any(|source| source.allows(&url)) {
            return ImageDecision::Refused(UrlError::Private);
        }
        match self.mode {
            Mode::Safe => ImageDecision::Held,
            Mode::Trusted if self.sources().any(|source| source.allows(&url)) => ImageDecision::Load,
            Mode::Trusted => ImageDecision::OnRequest,
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
        self.fetch(url, limits, true)
    }

    /// Starts a fetch the user asked for: "Load once", "Always load from this host" or "Embed". Safe mode
    /// still refuses it.
    pub fn fetch_on_request(&self, url: &str, limits: Limits) -> Result<ImageFetch, FetchError> {
        self.fetch(url, limits, false)
    }

    /// Starts the fetch of the page a link preview is made from, when the writer asks for the preview:
    /// never on open, and never in safe mode. The address is held to the rules a picture's is.
    ///
    /// The page's own picture is then fetched with [`fetch_on_request`](Self::fetch_on_request), from the
    /// address [`HttpsUrl::join`] reads against the page's.
    pub fn fetch_page(&self, url: &str, limits: Limits) -> Result<PageFetch, FetchError> {
        self.fetch(url, limits, false)
    }

    fn fetch<K: Kind>(&self, url: &str, limits: Limits, on_open: bool) -> Result<Fetch<K>, FetchError> {
        match self.image(url) {
            ImageDecision::Refused(error) => Err(FetchError::Url(error)),
            ImageDecision::Held => Err(FetchError::Held),
            ImageDecision::OnRequest if on_open => Err(FetchError::UntrustedSource),
            ImageDecision::Load | ImageDecision::OnRequest => {
                let url = HttpsUrl::parse(url).map_err(FetchError::Url)?;
                let within = on_open.then(|| self.sources().cloned().collect());
                Ok(Fetch::new(url, limits, within, self.vouchers().to_vec()))
            }
        }
    }

    /// Every source images load from on open.
    fn sources(&self) -> impl Iterator<Item = &'a Source> {
        self.trust.sources().iter().chain(self.server.unwrap_or_default())
    }

    /// The sources that may be on a private address: those of whoever owns the network the fetch runs in.
    fn vouchers(&self) -> &'a [Source] {
        self.server.unwrap_or(self.trust.sources())
    }
}

/// The part of an address that names its host, with any user name still on it.
fn authority(href: &str) -> &str {
    let rest = href.split_once(':').map_or(href, |(_, rest)| rest).trim_start_matches(['/', '\\']);
    &rest[..rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len())]
}
