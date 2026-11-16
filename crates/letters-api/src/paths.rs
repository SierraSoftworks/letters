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
/// `GET`: 200 while the server can answer and 503 while its state cannot be read, for whatever watches
/// that it runs. Needs no session, and tells nothing of the server.
pub const HEALTH: &str = "/health";

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
/// `GET`: the words the person taught the spell checker, as a list of strings. They change a word at a
/// time, at [`word`].
pub const DICTIONARY: &str = "/api/v1/settings/dictionary";

/// `GET`: the languages the server has a spelling dictionary for, as a list of tags such as `de-DE`.
/// English is not among them: the editor has it built in. Needs no session.
pub const DICTIONARIES: &str = "/api/v1/dictionaries";

/// `GET`: the affix file of the Hunspell dictionary that best serves the language `tag`, as UTF-8 text.
/// With [`dictionary_dic`] it is all a spell checker needs. The dictionary is the one for the tag's region,
/// else for its language alone, else any of that language, so `de-CH` may be answered with German as
/// Germany writes it. A language the server has none for is not found. Needs no session. A browser keeps
/// the file and is told when it has not changed.
pub fn dictionary_aff(tag: &str) -> String {
    format!("{DICTIONARIES}/{}/aff", segment(tag))
}

/// `GET`: the word list of the dictionary [`dictionary_aff`] answers for `tag`, as UTF-8 text.
pub fn dictionary_dic(tag: &str) -> String {
    format!("{DICTIONARIES}/{}/dic", segment(tag))
}

/// `GET`: every [`Space`](crate::Space) the session reaches.
pub const SPACES: &str = "/api/v1/spaces";
/// `GET`: the documents the person opened lately, as [`Entry`](crate::Entry)s, newest first.
pub const RECENT: &str = "/api/v1/recent";
/// `GET`: what is in the trash of every space the session reaches, as [`Trashed`](crate::Trashed)s, the last
/// thrown away first.
pub const TRASH: &str = "/api/v1/trash";
/// `GET`: the documents shared with the person from beyond their spaces, as [`Entry`](crate::Entry)s.
pub const SHARED: &str = "/api/v1/shared";
/// `GET`: who a document can be shared with, as [`Grantee`](crate::Grantee)s: everyone else who has signed
/// in to the server, and then the groups it has heard of.
pub const PEOPLE: &str = "/api/v1/people";
/// `GET`: the sources the server itself fetches pictures and previews from, as a list of addresses.
pub const SOURCES: &str = "/api/v1/trust/sources";
/// `GET`: the sources the person always loads pictures from, as a list of addresses: what "Always load
/// from this host" adds, on whatever device. The server fetches a picture under one of them without being
/// asked ([`Fetch::asked`](crate::Fetch::asked)), as it does under its own. They change one at a time, at
/// [`own_source`].
pub const OWN_SOURCES: &str = "/api/v1/settings/sources";

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

/// `PUT` adds the word to the person's own and `DELETE` takes it out. Each answers the words as they are
/// then, and minds neither a word already there nor one already gone. Two devices that change words at
/// once both have their way, which two `PUT`s of a whole list would not give them.
pub fn word(word: &str) -> String {
    format!("{DICTIONARY}/{}", segment(word))
}

/// `PUT` adds the source to the person's own and `DELETE` takes it out: a host such as `img.example.org`,
/// or an `https` address that the pictures are under. Each answers the sources as they are then, written as
/// whole addresses, and minds neither one already there nor one already gone.
pub fn own_source(source: &str) -> String {
    format!("{OWN_SOURCES}/{}", segment(source))
}

/// `GET`: the [`Listing`](crate::Listing) of the folder `?path=` names, or of the space itself.
pub fn entries(space: &str, folder: &str) -> String {
    let mut out = format!("{SPACES}/{}/entries", segment(space));
    if !folder.is_empty() {
        let _ = write!(out, "?path={}", segment(folder));
    }
    out
}

/// `GET`: a [`Listing`](crate::Listing) of everything in the space, in whatever folder of it: what the
/// space itself holds first, then what its folders hold, and so on down. It is what a shell searches a
/// space in, with a [`Query`](crate::Query).
pub fn everything(space: &str) -> String {
    format!("{SPACES}/{}/entries?deep=true", segment(space))
}

/// `POST` a [`NewDocument`](crate::NewDocument): its [`Entry`](crate::Entry).
pub fn documents(space: &str) -> String {
    format!("{SPACES}/{}/documents", segment(space))
}

/// `POST` a [`NewFolder`](crate::NewFolder): its [`Entry`](crate::Entry).
pub fn folders(space: &str) -> String {
    format!("{SPACES}/{}/folders", segment(space))
}

/// `POST` a [`Rename`](crate::Rename): the folder at `path` under its new name, as an
/// [`Entry`](crate::Entry). Whoever has a document in it open stays in it.
pub fn rename_folder(space: &str, path: &str) -> String {
    folder_act(space, path, "rename")
}

/// `POST` a [`Move`](crate::Move): the folder at `path` in another folder, with everything in it, as an
/// [`Entry`](crate::Entry). A folder is not moved into itself, and nothing where it goes is replaced.
pub fn move_folder(space: &str, path: &str) -> String {
    folder_act(space, path, "move")
}

/// `POST`: moves the folder at `path` to its space's trash, with everything in it.
pub fn trash_folder(space: &str, path: &str) -> String {
    folder_act(space, path, "trash")
}

fn folder_act(space: &str, path: &str, act: &str) -> String {
    format!("{SPACES}/{}/folders/{act}?path={}", segment(space), segment(path))
}

/// `POST`: a Markdown or Word file, as it is, to make a document of in the folder at `folder`. `name` is
/// the file's own, which names the document without its ending, or by the next name after it that is free:
/// nothing is replaced. Which of the two the file is, is told from what it holds. It may be as large as
/// [`Limits::document`](crate::Limits::document). Answers the document's [`Entry`](crate::Entry), with a
/// tag for what the import simplified. A file that is neither is [`Code::Invalid`](crate::Code::Invalid).
pub fn import(space: &str, folder: &str, name: &str) -> String {
    format!("{SPACES}/{}/import?path={}&name={}", segment(space), segment(folder), segment(name))
}

/// `POST`: makes a document of the Markdown or Word file at `path` in the space, a row tagged
/// [`Tag::Unimported`](crate::Tag::Unimported), in the folder the file is in. The file is left as it is, and
/// a Markdown file's pictures are read from its folder and the folders in it. Answers as [`import`] does.
pub fn import_file(space: &str, path: &str) -> String {
    format!("{SPACES}/{}/files/import?path={}", segment(space), segment(path))
}

/// `GET`: the document as [`Opened`](crate::Opened).
pub fn document(id: &str) -> String {
    format!("{API}/documents/{}", segment(id))
}

/// `GET`: the whole `.let` file, pictures and history included, as the server would save it now. Whoever
/// has no more than a link gets a document of its own in its place, with an id of its own and without the
/// history or the notes, since a history can show what was since taken out. `PUT`: a
/// device's own copy of the document, for work that is too much for the socket. It is merged into the
/// document and replaces nothing, and may be as large as [`Limits::document`](crate::Limits::document).
/// A copy is read before it is merged, and [`Code::Stale`](crate::Code::Stale) says the document was
/// moved or thrown away meanwhile: asked again, the merge finds the document where it is now.
pub fn file(id: &str) -> String {
    format!("{}/file", document(id))
}

/// `GET`: the document in another format, written by the server as the document stands now, for a
/// browser to keep under the document's name.
///
/// - Markdown is a zip when the document holds pictures: the text, and the pictures in the folder it names
///   them in.
/// - A picture that is still a link is fetched for a PDF where the reader is shown it unasked. Every other
///   format writes the link.
/// - Whoever has no more than a link gets no comments and no names of who wrote the document, as with
///   [`file()`], and is refused as [`Code::Forbidden`](crate::Code::Forbidden) where the link allows no
///   download.
pub fn export(id: &str, to: crate::Export) -> String {
    format!("{}/export/{}", document(id), to.name())
}

/// `POST` a [`Rename`](crate::Rename): the document's [`Entry`](crate::Entry) under its new name.
pub fn rename(id: &str) -> String {
    format!("{}/rename", document(id))
}

/// `POST` a [`Move`](crate::Move): the document's [`Entry`](crate::Entry) where it now is.
pub fn relocate(id: &str) -> String {
    format!("{}/move", document(id))
}

/// `POST`: takes what a [`Trashed`](crate::Trashed) names by its `file` out of the trash of `space` and puts
/// it back where it was, or at the top of the space where its folder is gone: its [`Entry`](crate::Entry).
/// Under its own name, or the first numbered one that is free there.
pub fn put_back(space: &str, file: &str) -> String {
    format!("{SPACES}/{}/trash/put-back?file={}", segment(space), segment(file))
}

/// `POST`: moves the document to its space's trash.
pub fn trash(id: &str) -> String {
    format!("{}/trash", document(id))
}

/// `POST`: merges into the document every conflict copy a sync tool left beside it, and moves each to
/// its space's trash: the names of the copies it took, as a list. No copy is merged unless this is asked.
/// Refused as a `PUT` of [`file`] is when the document goes elsewhere meanwhile.
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

/// `PUT` a [`Seen`](crate::Seen): the person read the document up to there. Their account keeps it for
/// [`Opened::last_seen`](crate::Opened::last_seen), on whatever device they open the document next. A
/// shell says it as it closes the document and as it saves. Answers nothing.
pub fn seen(id: &str) -> String {
    format!("{}/seen", document(id))
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

/// `GET`: the document's access log as [`Access`](crate::Access) rows, newest first, for whoever may say
/// who the document is shared with. `?limit=<n>` asks for so many rows: 200 unless asked, and never more
/// than 1000. `?before=<time>` asks for the rows from before a row's [`Access::time`], which is how the
/// next page is asked for: a page runs over its limit to end between two seconds, so none is missed. How
/// long a row is kept is [`Server::log_days`](crate::Server::log_days).
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

/// A page: where a share link leads. `token` is the link's secret, which only its address holds.
///
/// Following it, after the passphrase when the link asks one, lets the browser in to the link's document
/// and sends it on to [`editor_page`]. From then on that browser is answered at what is asked of the
/// document itself ([`document`], [`file()`], [`assets`], [`socket`], [`rename`] and the fetches) as the
/// link says: with its role and its [`Allows`](crate::Allows), in no space, and by the link's label for a
/// name. With nobody signed in, everything else of the server refuses it as signed out.
pub fn link_page(token: &str) -> String {
    format!("/l/{}", segment(token))
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
