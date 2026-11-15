//! Every address a server answers at, from its root. The ones with a part to fill in are functions, which
//! write the part so that nothing in it can end its place in the address.
//!
//! Addresses under [`API`] answer JSON and take a session as a cookie or a bearer token. The rest are pages
//! for a browser.

use std::fmt::Write as _;

/// Where the interface lives. The number is [`PROTOCOL`](crate::PROTOCOL).
pub const API: &str = "/api/v1";

/// `GET`: a [`Server`](crate::Server). Needs no session.
pub const SERVER: &str = "/api/v1/server";

/// `GET`: the [`Person`](crate::Person) signed in.
pub const SESSION: &str = "/api/v1/session";
/// `POST` an [`Exchange`](crate::Exchange): a [`Token`](crate::Token). Needs no session.
pub const EXCHANGE: &str = "/api/v1/session/exchange";
/// `POST` a [`Renewal`](crate::Renewal): the next [`Token`](crate::Token). Needs no session.
pub const RENEW: &str = "/api/v1/session/renew";
/// `POST`: ends the session, so that its token and its renewal stop working.
pub const SIGN_OUT: &str = "/api/v1/session/sign-out";
/// `GET` and `PUT`: the person's [`Settings`](crate::Settings).
pub const SETTINGS: &str = "/api/v1/settings";

/// `GET`: every [`Space`](crate::Space) the session reaches.
pub const SPACES: &str = "/api/v1/spaces";
/// `GET`: the documents the person opened lately, as [`Entry`](crate::Entry)s, newest first.
pub const RECENT: &str = "/api/v1/recent";
/// `GET`: the documents shared with the person from beyond their spaces, as [`Entry`](crate::Entry)s.
pub const SHARED: &str = "/api/v1/shared";
/// `GET`: who a document can be shared with, as [`Grantee`](crate::Grantee)s: everyone else who has signed
/// in to the server, and then the groups it has heard of.
pub const PEOPLE: &str = "/api/v1/people";
/// `GET`: the sources the server itself fetches pictures and previews from, as a list of addresses.
pub const SOURCES: &str = "/api/v1/trust/sources";

/// A page: where a browser starts signing in. `?then=` names the page to come back to.
pub const SIGN_IN_PAGE: &str = "/auth/sign-in";
/// A page: `POST` from one of the server's own pages ends the browser's session.
pub const SIGN_OUT_PAGE: &str = "/auth/sign-out";
/// A page: where the desktop app sends the system's browser to sign in. See [`desktop_sign_in`].
pub const DESKTOP_PAGE: &str = "/auth/desktop";
/// A page: where the identity provider sends the browser back. The one redirect address to register.
pub const CALLBACK_PAGE: &str = "/auth/callback";
/// A page: the documents the person reaches.
pub const HOME_PAGE: &str = "/";

/// `GET`: the [`Listing`](crate::Listing) of the folder `?path=` names, or of the space itself.
pub fn entries(space: &str, folder: &str) -> String {
    let mut out = format!("{SPACES}/{}/entries", segment(space));
    if !folder.is_empty() {
        let _ = write!(out, "?path={}", segment(folder));
    }
    out
}

/// `POST` a [`NewDocument`](crate::NewDocument): its [`Entry`](crate::Entry).
pub fn documents(space: &str) -> String {
    format!("{SPACES}/{}/documents", segment(space))
}

/// `POST` a [`NewFolder`](crate::NewFolder): its [`Entry`](crate::Entry).
pub fn folders(space: &str) -> String {
    format!("{SPACES}/{}/folders", segment(space))
}

/// `GET`: the document as [`Opened`](crate::Opened).
pub fn document(id: &str) -> String {
    format!("{API}/documents/{}", segment(id))
}

/// `GET`: the whole `.let` file, pictures and history included, as the server would save it now. `PUT`: a
/// device's own copy of the document, for work that is too much for the socket. It is merged into the
/// document and replaces nothing, and may be as large as [`Limits::document`](crate::Limits::document).
pub fn file(id: &str) -> String {
    format!("{}/file", document(id))
}

/// `POST` a [`Rename`](crate::Rename): the document's [`Entry`](crate::Entry) under its new name.
pub fn rename(id: &str) -> String {
    format!("{}/rename", document(id))
}

/// `POST` a [`Move`](crate::Move): the document's [`Entry`](crate::Entry) where it now is.
pub fn relocate(id: &str) -> String {
    format!("{}/move", document(id))
}

/// `POST`: moves the document to its space's trash.
pub fn trash(id: &str) -> String {
    format!("{}/trash", document(id))
}

/// `POST`: merges into the document every conflict copy a sync tool left beside it, and moves each to
/// its space's trash: the names of the copies it took, as a list. No copy is merged unless this is asked.
pub fn merge_conflicts(id: &str) -> String {
    format!("{}/merge-conflicts", document(id))
}

/// `GET`: the names of the document's assets, as [`asset`] takes them. A device that wrote offline asks
/// this to learn which of its pictures the server lacks, and hands those over before its edits.
pub fn assets(id: &str) -> String {
    format!("{}/assets", document(id))
}

/// `GET`: one asset of the document, by the name its entry has without `assets/`, such as
/// `<sha256>.png`. `PUT`: adds it, before the edit that names it. The name must be the hash of the bytes.
pub fn asset(id: &str, name: &str) -> String {
    format!("{}/assets/{}", document(id), segment(name))
}

/// The document's socket, which speaks what `letters-sync` writes. A browser's cookie or the desktop
/// app's bearer header opens it.
pub fn socket(id: &str) -> String {
    format!("{}/sync", document(id))
}

/// `POST` a [`Fetch`](crate::Fetch): the [`Preview`](crate::Preview) of the page it names.
pub fn fetch_preview(id: &str) -> String {
    format!("{}/fetch/preview", document(id))
}

/// `POST` a [`Fetch`](crate::Fetch): the bytes of the picture it names, under their type.
pub fn fetch_picture(id: &str) -> String {
    format!("{}/fetch/picture", document(id))
}

/// `POST`: the person trusts the document from now on, which ends its safe mode for them.
pub fn trust(id: &str) -> String {
    format!("{}/trust", document(id))
}

/// `GET`: the document's [`Shares`](crate::Shares), for whoever says who it is shared with. Anyone else is
/// refused as [`Code::Forbidden`](crate::Code::Forbidden).
///
/// `PUT` a [`Shares`](crate::Shares): its people are all the document's people from here on, and its links
/// are not read. Answers the shares as they then are, people and links.
pub fn shares(id: &str) -> String {
    format!("{}/shares", document(id))
}

/// `POST` a [`NewLink`](crate::NewLink): a [`CreatedLink`](crate::CreatedLink), the one time the link's
/// address is told. A server makes links only when its [`Server::links`](crate::Server::links) says so.
pub fn links(id: &str) -> String {
    format!("{}/links", document(id))
}

/// `DELETE`: revokes the link with this [`Link::id`](crate::Link::id). Answers with no body.
pub fn link(id: &str, link: &str) -> String {
    format!("{}/links/{}", document(id), segment(link))
}

/// `GET`: the document's access log as [`Access`](crate::Access) rows, newest first.
pub fn access(id: &str) -> String {
    format!("{}/access", document(id))
}

/// A page: where the desktop app sends the system's browser to sign in, at [`DESKTOP_PAGE`].
///
/// `challenge` is the SHA-256 of a secret the app keeps, in base64 for addresses without padding. Once the
/// person has signed in and agreed, the browser is sent to `http://127.0.0.1:<port>/?code=…&state=…`,
/// where the app listens, and the app trades the code and its secret at [`EXCHANGE`].
pub fn desktop_sign_in(challenge: &str, port: u16, state: &str) -> String {
    format!("{DESKTOP_PAGE}?challenge={}&port={port}&state={}", segment(challenge), segment(state))
}

/// A page: [`SIGN_IN_PAGE`], coming back to the page `then` of this server once signed in.
pub fn sign_in_page(then: &str) -> String {
    format!("{SIGN_IN_PAGE}?then={}", segment(then))
}

/// A page: the document in the browser's editor.
pub fn editor_page(id: &str) -> String {
    format!("/d/{}", segment(id))
}

/// A page: the document as plain HTML, to read with no editor.
pub fn read_page(id: &str) -> String {
    format!("/d/{}/read", segment(id))
}

/// A page: the folder at `path` in a space.
pub fn folder_page(space: &str, path: &str) -> String {
    let mut out = format!("/s/{}", segment(space));
    for name in path.split('/').filter(|name| !name.is_empty()) {
        let _ = write!(out, "/{}", segment(name));
    }
    out
}

/// Writes `text` as one part of an address: every byte but letters, digits and `-._~` is escaped, so a
/// `/`, a `?` or a `#` in an id or a name stays in its part.
pub fn segment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(char::from(byte)),
            byte => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}
