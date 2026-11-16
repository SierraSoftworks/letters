//! The interface `letters_api` lays down, answered.
//!
//! Each address is a thin async wrapper: the work is in plain functions over [`App`], which read folders
//! and the store and so run where blocking is allowed.

use std::io::{Read, Seek};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, FailedToBufferBody};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, SET_COOKIE, X_CONTENT_TYPE_OPTIONS};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use letters_api::{
    Action, Allows, Code, Entry, EntryKind, Fetch, Listing, Move, NewDocument, NewFolder, Opened, PROTOCOL, Person,
    Preview, Rename, Renewal, Role, Server, Settings, Space, Tag, Token, Trashed, Trust, paths,
};
use letters_format::{ASSETS, Archive, MIMETYPE, NOTES, Reading, STYLE, file_name};
use letters_model::{Document, ReplicaId};
use serde::Deserialize;
use serde_json::Value;

use crate::config::Log;
use crate::error::{Answer, Refusal};
use crate::ids;
use crate::library::{Row, untitled};
use crate::session::{Carrier, Reader, Signed};
use crate::spaces::{EXTENSION, Place, Root, seconds, shown};
use crate::store::Table;
use crate::telemetry::Event;
use crate::{App, Shared};

/// A document's file, open to be read.
type File = letters_format::Archive<std::io::BufReader<std::fs::File>>;

/// By [`Person::id`]: the documents they opened, newest first.
const RECENT: Table<Vec<String>> = Table::new("recent");
/// How many documents a person's recents hold.
const RECENTS: usize = 30;

pub fn routes() -> Router<Shared> {
    let limits = letters_api::Limits::default();
    Router::new()
        .route(paths::SERVER, get(server))
        .route(paths::HEALTH, get(health))
        .route(paths::SESSION, get(session))
        .route(paths::RENEW, post(renew))
        .route(paths::SIGN_OUT, post(sign_out))
        .route(paths::SETTINGS, get(settings).put(set_settings))
        .route(paths::DICTIONARY, get(words))
        .route("/api/v1/settings/dictionary/{word}", put(add_word).delete(forget_word))
        .route(paths::OWN_SOURCES, get(own_sources))
        .route("/api/v1/settings/sources/{source}", put(add_source).delete(forget_source))
        .route(paths::SPACES, get(spaces))
        .route(paths::RECENT, get(recent))
        .route(paths::SOURCES, get(sources))
        .route("/api/v1/spaces/{space}/entries", get(entries))
        .route("/api/v1/spaces/{space}/documents", post(new_document))
        .route("/api/v1/spaces/{space}/import", post(import).layer(DefaultBodyLimit::max(limits.document as usize)))
        .route("/api/v1/spaces/{space}/folders", post(new_folder))
        .route("/api/v1/spaces/{space}/folders/rename", post(rename_folder))
        .route("/api/v1/spaces/{space}/folders/move", post(move_folder))
        .route("/api/v1/spaces/{space}/folders/trash", post(trash_folder))
        .route(paths::TRASH, get(trashed))
        .route("/api/v1/spaces/{space}/trash/put-back", post(put_back))
        .route("/api/v1/documents/{id}", get(document))
        .route(
            "/api/v1/documents/{id}/file",
            get(file).put(merge_file).layer(DefaultBodyLimit::max(limits.document as usize)),
        )
        .route("/api/v1/documents/{id}/rename", post(rename))
        .route("/api/v1/documents/{id}/move", post(relocate))
        .route("/api/v1/documents/{id}/trash", post(trash))
        .route("/api/v1/documents/{id}/merge-conflicts", post(merge_conflicts))
        .route("/api/v1/documents/{id}/assets", get(assets))
        .route(
            "/api/v1/documents/{id}/assets/{name}",
            get(asset).put(add_asset).layer(DefaultBodyLimit::max(limits.asset as usize)),
        )
        .route("/api/v1/documents/{id}/fetch/picture", post(fetch_picture))
        .route("/api/v1/documents/{id}/fetch/preview", post(fetch_preview))
        .route("/api/v1/documents/{id}/trust", post(trust))
}

/// Runs `work` where it may wait on a folder or the store.
pub async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Answer<T> + Send + 'static) -> Answer<T> {
    let failed = |_| Refusal::new(Code::Internal, "The server failed while answering.");
    tokio::task::spawn_blocking(work).await.map_err(failed)?
}

/// Whether the server runs, for a watcher: it does while its store answers.
async fn health(State(app): State<Shared>) -> (StatusCode, &'static str) {
    match tokio::task::spawn_blocking(move || app.store.answers()).await {
        Ok(true) => (StatusCode::OK, "ok\n"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "the state store does not answer\n"),
    }
}

async fn server(State(app): State<Shared>) -> Json<Server> {
    Json(Server {
        product: Server::PRODUCT.into(),
        name: app.config.server.name.clone(),
        version: env!("CARGO_PKG_VERSION").into(),
        protocols: vec![PROTOCOL],
        links: app.config.links.enabled,
        log_days: crate::access::KEPT_DAYS,
        limits: letters_api::Limits::default(),
    })
}

async fn session(who: Signed) -> Json<Person> {
    Json(who.person)
}

async fn renew(State(app): State<Shared>, Json(renewal): Json<Renewal>) -> Answer<Json<Token>> {
    blocking(move || app.renew(&renewal.renew).map(Json)).await
}

async fn sign_out(State(app): State<Shared>, who: Signed) -> Answer<Response> {
    let ended = app.clone();
    blocking(move || ended.sign_out(&who.token)).await?;
    // A browser is told to forget what no longer signs it in.
    Ok(match who.carrier {
        Carrier::Cookie => [(SET_COOKIE, crate::auth::no_session_cookie(&app))].into_response(),
        Carrier::Bearer => ().into_response(),
    })
}

async fn settings(State(app): State<Shared>, who: Signed) -> Answer<Json<Settings>> {
    blocking(move || app.settings(&who).map(Json)).await
}

async fn set_settings(
    State(app): State<Shared>,
    who: Signed,
    Json(settings): Json<Settings>,
) -> Answer<Json<Settings>> {
    blocking(move || app.set_settings(&who, settings).map(Json)).await
}

async fn words(State(app): State<Shared>, who: Signed) -> Answer<Json<Vec<String>>> {
    blocking(move || Ok(Json(app.settings(&who)?.dictionary))).await
}

async fn add_word(State(app): State<Shared>, who: Signed, Path(word): Path<String>) -> Answer<Json<Vec<String>>> {
    blocking(move || app.word(&who, &word, true).map(Json)).await
}

async fn forget_word(State(app): State<Shared>, who: Signed, Path(word): Path<String>) -> Answer<Json<Vec<String>>> {
    blocking(move || app.word(&who, &word, false).map(Json)).await
}

async fn own_sources(State(app): State<Shared>, who: Signed) -> Answer<Json<Vec<String>>> {
    blocking(move || app.own_sources(&who).map(Json)).await
}

async fn add_source(State(app): State<Shared>, who: Signed, Path(source): Path<String>) -> Answer<Json<Vec<String>>> {
    blocking(move || app.own_source(&who, &source, true).map(Json)).await
}

async fn forget_source(
    State(app): State<Shared>,
    who: Signed,
    Path(source): Path<String>,
) -> Answer<Json<Vec<String>>> {
    blocking(move || app.own_source(&who, &source, false).map(Json)).await
}

async fn spaces(State(app): State<Shared>, who: Signed) -> Json<Vec<Space>> {
    Json(app.spaces(&who).into_iter().map(|(space, role)| space.told(role)).collect())
}

async fn sources(State(app): State<Shared>, _: Signed) -> Json<Vec<String>> {
    Json(app.config.trust.sources.clone())
}

#[derive(Deserialize)]
struct InFolder {
    #[serde(default)]
    path: String,
}

async fn entries(
    State(app): State<Shared>,
    who: Signed,
    Path(space): Path<String>,
    Query(at): Query<InFolder>,
) -> Answer<Json<Listing>> {
    blocking(move || app.listing(&who, &space, &at.path).map(Json)).await
}

async fn new_document(
    State(app): State<Shared>,
    who: Signed,
    Path(space): Path<String>,
    Json(new): Json<NewDocument>,
) -> Answer<Json<Entry>> {
    app.telemetry.record(Event::DocumentMade);
    blocking(move || app.new_document(&who, &space, &new).map(Json)).await
}

/// A file to make a document of: the folder to make it in, and what the file was called.
#[derive(Deserialize)]
struct Brought {
    #[serde(default)]
    path: String,
    #[serde(default)]
    name: String,
}

/// `POST`: a Markdown or Word file, to make a document of.
async fn import(
    State(app): State<Shared>,
    who: Signed,
    Path(space): Path<String>,
    Query(brought): Query<Brought>,
    body: Result<Bytes, BytesRejection>,
) -> Answer<Json<Entry>> {
    let file = body.map_err(|rejection| unread(rejection, "The file is larger than the server takes."))?;
    blocking(move || app.import(&who, &space, &brought.path, &brought.name, &file).map(Json)).await
}

async fn new_folder(
    State(app): State<Shared>,
    who: Signed,
    Path(space): Path<String>,
    Json(new): Json<NewFolder>,
) -> Answer<Json<Entry>> {
    blocking(move || app.new_folder(&who, &space, &new).map(Json)).await
}

async fn rename_folder(
    State(app): State<Shared>,
    who: Signed,
    Path(space): Path<String>,
    Query(at): Query<InFolder>,
    Json(to): Json<Rename>,
) -> Answer<Json<Entry>> {
    blocking(move || app.rename_folder(&who, &space, &at.path, &to.name).map(Json)).await
}

async fn move_folder(
    State(app): State<Shared>,
    who: Signed,
    Path(space): Path<String>,
    Query(at): Query<InFolder>,
    Json(to): Json<Move>,
) -> Answer<Json<Entry>> {
    blocking(move || app.move_folder(&who, &space, &at.path, &to).map(Json)).await
}

async fn trash_folder(
    State(app): State<Shared>,
    who: Signed,
    Path(space): Path<String>,
    Query(at): Query<InFolder>,
) -> Answer<()> {
    blocking(move || app.trash_folder(&who, &space, &at.path)).await
}

async fn document(State(app): State<Shared>, Reader(who): Reader, Path(id): Path<String>) -> Answer<Json<Opened>> {
    blocking(move || app.opened(&who, &id, false).map(Json)).await
}

async fn file(State(app): State<Shared>, Reader(who): Reader, Path(id): Path<String>) -> Answer<Response> {
    let (name, bytes) = blocking(move || {
        let found = app.locate(&who, &id)?;
        // The app keeps a copy of what it has open and asks for the file to keep it whole: that is part of
        // its opening the document. A browser that asks takes the file away.
        if who.carrier == Carrier::Cookie {
            app.note(&who, &found, &id, Action::Download, !found.allows.download);
        }
        if !found.allows.download {
            return Err(Refusal::forbidden("This link does not let its reader take the file away."));
        }
        // What the document's room holds goes to the file first, so that the file sent is whole.
        app.rooms.save(&found.root, &found.place);
        // The history can show text that was since taken out. Whoever has no more than a link gets the
        // document as it stands.
        let bytes = match found.by_link() {
            true => app.library.clean(&found.root, &found.place, app.now())?,
            false => app.library.bytes(&found.root, &found.place)?,
        };
        Ok((found.place.name().to_owned(), bytes))
    })
    .await?;
    // The name is the file's own, less what a header cannot carry.
    let plain: String = name.chars().filter(|c| c.is_ascii() && !c.is_control() && !matches!(c, '"' | '\\')).collect();
    let disposition = format!("attachment; filename=\"{plain}\"; filename*=UTF-8''{}", paths::segment(&name));
    Ok(([(CONTENT_TYPE, MIMETYPE.to_owned()), (CONTENT_DISPOSITION, disposition)], bytes).into_response())
}

async fn assets(State(app): State<Shared>, Reader(who): Reader, Path(id): Path<String>) -> Answer<Json<Vec<String>>> {
    blocking(move || {
        let found = app.locate(&who, &id)?;
        let archive = app.library.open(&found.root, &found.place)?;
        // The file's, and the ones its room holds that the file is yet to get.
        let mut names: Vec<String> = archive.names().map(str::to_owned).collect();
        names.extend(app.rooms.assets(&found.root, &found.place));
        names.sort();
        names.dedup();
        Ok(Json(names.iter().filter_map(|name| name.strip_prefix(ASSETS)).map(str::to_owned).collect()))
    })
    .await
}

/// The entry an asset called `name` is in. An asset is named by the hash of its bytes, with an ending:
/// nothing else is one.
fn asset_entry(name: &str) -> Answer<String> {
    let plain = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.');
    plain.then(|| format!("{ASSETS}{name}")).ok_or_else(Refusal::not_found)
}

async fn asset(
    State(app): State<Shared>,
    Reader(who): Reader,
    Path((id, name)): Path<(String, String)>,
) -> Answer<Response> {
    let bytes = blocking(move || {
        let found = app.locate(&who, &id)?;
        let entry = asset_entry(&name)?;
        // A picture that was only just handed over is in the document's room before it is in the file.
        if let Some(bytes) = app.rooms.asset(&found.root, &found.place, &entry) {
            return Ok(bytes);
        }
        app.library.open(&found.root, &found.place)?.read(&entry)?.ok_or_else(Refusal::not_found)
    })
    .await?;
    // The bytes say what they are. A picture is shown, and nothing else is ever run or rendered.
    let kind = letters_trust::ImageType::sniff(&bytes).map_or("application/octet-stream", |kind| kind.media_type());
    let headers = [
        (CONTENT_TYPE, kind),
        (axum::http::header::CACHE_CONTROL, "private, max-age=31536000, immutable"),
        (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    ];
    Ok((headers, bytes).into_response())
}

/// `POST`: a picture the document links to, fetched here so that its site learns nothing of the reader.
async fn fetch_picture(
    State(app): State<Shared>,
    Reader(who): Reader,
    Path(id): Path<String>,
    Json(fetch): Json<Fetch>,
) -> Answer<Response> {
    app.fetching(&who)?;
    let turn = app.fetches.turn().await?;
    let fetched = blocking(move || {
        let _turn = turn;
        app.fetch_picture(&who, &id, &fetch)
    })
    .await?;
    let headers = [
        (CONTENT_TYPE, fetched.kind.media_type()),
        // The site may send another picture tomorrow, and what one person asked for is not another's.
        (CACHE_CONTROL, "no-store"),
        (X_CONTENT_TYPE_OPTIONS, "nosniff"),
    ];
    Ok((headers, fetched.bytes).into_response())
}

/// `POST`: what the page behind a link says of itself.
async fn fetch_preview(
    State(app): State<Shared>,
    Reader(who): Reader,
    Path(id): Path<String>,
    Json(fetch): Json<Fetch>,
) -> Answer<Json<Preview>> {
    app.fetching(&who)?;
    let turn = app.fetches.turn().await?;
    blocking(move || {
        let _turn = turn;
        app.fetch_preview(&who, &id, &fetch).map(Json)
    })
    .await
}

async fn trust(State(app): State<Shared>, Reader(who): Reader, Path(id): Path<String>) -> Answer<StatusCode> {
    blocking(move || app.trust(&who, &id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Why a body was not read, as a refusal. `large` is what is said of one over its limit.
fn unread(rejection: BytesRejection, large: &str) -> Refusal {
    match rejection {
        BytesRejection::FailedToBufferBody(FailedToBufferBody::LengthLimitError(_)) => {
            Refusal::new(Code::TooLarge, large)
        }
        _ => Refusal::invalid("The request did not arrive whole."),
    }
}

/// `PUT`: a device's own copy of the document, whole, for work that is too much for the socket. It is
/// merged into the document and replaces nothing.
async fn merge_file(
    State(app): State<Shared>,
    Reader(who): Reader,
    Path(id): Path<String>,
    body: Result<Bytes, BytesRejection>,
) -> Answer<StatusCode> {
    let file = body.map_err(|rejection| unread(rejection, "The file is larger than the server takes."))?;
    blocking(move || app.merge(&who, &id, &file)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT`: a picture for the document, ahead of the edit that names it.
async fn add_asset(
    State(app): State<Shared>,
    Reader(who): Reader,
    Path((id, name)): Path<(String, String)>,
    body: Result<Bytes, BytesRejection>,
) -> Answer<StatusCode> {
    let bytes = body.map_err(|rejection| unread(rejection, "The picture is larger than the server takes."))?;
    blocking(move || {
        let found = app.locate(&who, &id)?;
        found.must(Role::Editor)?;
        let entry = asset_entry(&name)?;
        // The bytes say what they are, whatever the request calls them.
        if letters_trust::ImageType::sniff(&bytes).is_none() {
            return Err(Refusal::invalid("That is not a picture Letters can show."));
        }
        // The same bytes go by the same name, so one the file holds is not taken twice.
        if app.library.open(&found.root, &found.place)?.names().any(|held| held == entry) {
            return Ok(());
        }
        app.rooms.add_asset(&app, &found.root, &found.place, &entry, bytes.into())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn rename(
    State(app): State<Shared>,
    Reader(who): Reader,
    Path(id): Path<String>,
    Json(to): Json<Rename>,
) -> Answer<Json<Entry>> {
    blocking(move || app.rename(&who, &id, &to.name).map(Json)).await
}

async fn relocate(
    State(app): State<Shared>,
    who: Signed,
    Path(id): Path<String>,
    Json(to): Json<Move>,
) -> Answer<Json<Entry>> {
    blocking(move || app.relocate(&who, &id, &to).map(Json)).await
}

async fn trashed(State(app): State<Shared>, who: Signed) -> Answer<Json<Vec<Trashed>>> {
    blocking(move || Ok(Json(app.trashed(&who)))).await
}

/// What is asked back out of a trash: its name there.
#[derive(Deserialize)]
struct Back {
    #[serde(default)]
    file: String,
}

async fn put_back(
    State(app): State<Shared>,
    who: Signed,
    Path(space): Path<String>,
    Query(back): Query<Back>,
) -> Answer<Json<Entry>> {
    blocking(move || app.put_back(&who, &space, &back.file).map(Json)).await
}

async fn trash(State(app): State<Shared>, who: Signed, Path(id): Path<String>) -> Answer<()> {
    blocking(move || app.trash(&who, &id)).await
}

async fn merge_conflicts(State(app): State<Shared>, who: Signed, Path(id): Path<String>) -> Answer<Json<Vec<String>>> {
    app.telemetry.record(Event::ConflictsMerged);
    blocking(move || app.merge_conflicts(&who, &id).map(Json)).await
}

async fn recent(State(app): State<Shared>, who: Signed) -> Answer<Json<Vec<Entry>>> {
    blocking(move || app.recent(&who).map(Json)).await
}

/// Reads a copy that is to be merged into a document, as a stranger's: its history is checked before it
/// is loaded. The replica only reads, so any id will do for it.
fn read<R: Read + Seek>(theirs: &mut Archive<R>) -> Answer<Reading<'_, R>> {
    Reading::of(theirs, ReplicaId::new(&ids::block_id())).map_err(crate::library::unmerged)
}

/// Everything there is to take away of a document.
const ALL: Allows = Allows { download: true, copy: true, print: true };

/// A document as a session reaches it.
pub struct Found {
    /// The space it is in, for a session that reaches the space. `None` when only a share leads to it.
    pub space: Option<Space>,
    /// What the session may do with it: what its space lets it, or a share when that is more.
    pub role: Role,
    /// What the session may take away of it: everything, but for a link's reader what the link allows.
    pub allows: Allows,
    /// When the session's reach of it ends by itself, as the visit of a link does.
    pub until: Option<u64>,
    pub root: Root,
    pub place: Place,
}

impl Found {
    /// Refuses unless the session may do what `role` may.
    pub fn must(&self, role: Role) -> Answer<()> {
        match self.role >= role {
            true => Ok(()),
            false => Err(Refusal::forbidden("You can read this document, but not change it.")),
        }
    }

    /// Whether a link is all that leads the session to the document: its space does not, and no share
    /// with them does.
    pub fn by_link(&self) -> bool {
        self.until.is_some()
    }

    /// Whether the document is only shown to read: a link for a reader is all that leads the session to
    /// it. Its history would go down an editor's socket, where a visitor with tools reads what was taken
    /// out, so there is no editor and no socket for it: the page of its text, as it stands.
    pub fn reading(&self) -> bool {
        self.by_link() && self.role == Role::Reader
    }

    /// The space the session reaches the document in, for what is done to its place and not to it. A
    /// share is of the document alone.
    pub fn placed(&self) -> Answer<&Space> {
        let lent = || Refusal::forbidden("This document is shared with you: where it is kept is its owner's to say.");
        self.space.as_ref().ok_or_else(lent)
    }
}

impl App {
    /// Where the document `id` is, for `who`: in a space they reach, or wherever a share leads them, or
    /// a link their browser was let in by. One that none of these leads to is not there for them.
    pub fn locate(&self, who: &Signed, id: &str) -> Answer<Found> {
        let reach = self.reach(who);
        let roots: Vec<&Root> = reach.iter().map(|(_, root)| root).collect();
        if let Some((at, place)) = self.library.find(&self.store, &roots, id, self.now())? {
            let (space, root) = reach.into_iter().nth(at).expect("the library answers with a root it was given");
            // A share may let someone do more with one document than its space lets them do with all.
            let role = self.in_space(who, id, &root, space.role).role;
            return Ok(Found { space: Some(space), role, allows: ALL, until: None, root, place });
        }
        if let Some((role, root, place)) = self.lent(who, id)? {
            return Ok(Found { space: None, role, allows: ALL, until: None, root, place });
        }
        let linked = who.link.as_ref().filter(|linked| linked.document == id).ok_or_else(Refusal::not_found)?;
        let (root, place) = self.led(linked)?.ok_or_else(Refusal::not_found)?;
        let (role, allows) = (linked.link.role, linked.link.allows);
        Ok(Found { space: None, role, allows, until: Some(linked.ends), root, place })
    }

    pub fn listing(&self, who: &Signed, space: &str, folder: &str) -> Answer<Listing> {
        let (space, role) = self.space(who, space)?;
        let (root, place) = (self.root(who, space)?, Place::parse(folder)?);
        let told = space.told(role);
        let rows = self.library.list(&self.store, &root, &place)?;
        let entries = rows.iter().map(|row| self.told(who, placed(&told, row), &root, row)).collect();
        Ok(Listing { space: told, path: place.as_str().to_owned(), entries })
    }

    /// The row of the file at `place` in a folder.
    fn row(&self, root: &Root, place: &Place) -> Answer<Row> {
        let (folder, name) = place.split();
        let rows = self.library.list(&self.store, root, &folder)?;
        rows.into_iter().find(|row| row.listed.name == name).ok_or_else(Refusal::not_found)
    }

    /// The row of the file at `place`, as a file table of its space shows it.
    pub(crate) fn entry(&self, who: &Signed, space: &Space, root: &Root, place: &Place) -> Answer<Entry> {
        let row = self.row(root, place)?;
        Ok(self.told(who, placed(space, &row), root, &row))
    }

    /// The row of a document for whoever a share leads to it: where it is kept is not told.
    pub(crate) fn lent_entry(&self, who: &Signed, role: Role, root: &Root, place: &Place) -> Answer<Entry> {
        let row = self.row(root, place)?;
        Ok(self.told(who, entry(&row, role), root, &row))
    }

    /// The row of the document that was found, when its file is at `place`.
    fn found_entry(&self, who: &Signed, found: &Found, place: &Place) -> Answer<Entry> {
        match &found.space {
            Some(space) => self.entry(who, space, &found.root, place),
            None => self.lent_entry(who, found.role, &found.root, place),
        }
    }

    /// A row as a file table shows it to `who`, with who has it open now. In its space, a share of it may
    /// let them do more than the space does, and whoever shares it is told with how many.
    fn told(&self, who: &Signed, mut entry: Entry, root: &Root, row: &Row) -> Entry {
        let people = if row.listed.folder { Vec::new() } else { self.rooms.people(root, &row.place) };
        if !people.is_empty() {
            entry.tags.push(Tag::Editing { people });
        }
        // A conflict copy has its original's id, and is shared with nobody.
        if let (Some(id), Some(_), None) = (&entry.id, &entry.space, &row.copy_of) {
            let shared = self.in_space(who, id, root, entry.role);
            entry.role = shared.role;
            if shared.with > 0 {
                entry.tags.push(Tag::Shared { with: shared.with });
            }
            if shared.links > 0 {
                entry.tags.push(Tag::Links { count: shared.links });
            }
        }
        entry
    }

    pub fn new_document(&self, who: &Signed, space: &str, new: &NewDocument) -> Answer<Entry> {
        let (space, role) = self.space(who, space)?;
        if !role.edits() {
            return Err(Refusal::forbidden("You can read this space, but not add to it."));
        }
        let (root, folder) = (self.root(who, space)?, Place::parse(&new.folder)?);
        let (style, author) = (self.new_style(who)?, self.signs_as(who)?);
        let place = self.library.create(&root, &folder, new.name.as_deref(), style, &author, self.now())?;
        self.entry(who, &space.told(role), &root, &place)
    }

    pub fn new_folder(&self, who: &Signed, space: &str, new: &NewFolder) -> Answer<Entry> {
        let (space, role) = self.space(who, space)?;
        if !role.edits() {
            return Err(Refusal::forbidden("You can read this space, but not add to it."));
        }
        let (root, parent) = (self.root(who, space)?, Place::parse(&new.parent)?);
        let name = file_name(&new.name).ok_or_else(|| Refusal::invalid("That name cannot be used for a folder."))?;
        let place = parent.child(&name)?;
        root.folder(&parent)?.make_folder(&name)?;
        self.entry(who, &space.told(role), &root, &place)
    }

    pub fn rename(&self, who: &Signed, id: &str, name: &str) -> Answer<Entry> {
        let found = self.locate(who, id)?;
        found.must(Role::Editor)?;
        let name = file_name(name).ok_or_else(|| Refusal::invalid("That name cannot be used for a document."))?;
        let to = found.place.split().0.child(&format!("{name}{EXTENSION}"))?;
        if to != found.place {
            let (from, at) = ((&found.root, &found.place), (&found.root, &to));
            self.rooms.moving(from, Some(at), || self.library.relocate(&self.store, from, at))?;
        }
        self.found_entry(who, &found, &to)
    }

    pub fn relocate(&self, who: &Signed, id: &str, to: &Move) -> Answer<Entry> {
        let found = self.locate(who, id)?;
        found.must(Role::Editor)?;
        found.placed()?;
        let (space, role) = self.space(who, &to.space)?;
        if !role.edits() {
            return Err(Refusal::forbidden("You can read that space, but not add to it."));
        }
        let root = self.root(who, space)?;
        let place = Place::parse(&to.folder)?.child(found.place.name())?;
        if (root.key.as_str(), &place) != (found.root.key.as_str(), &found.place) {
            let (from, at) = ((&found.root, &found.place), (&root, &place));
            self.rooms.moving(from, Some(at), || self.library.relocate(&self.store, from, at))?;
        }
        self.entry(who, &space.told(role), &root, &place)
    }

    /// The folder at `path` in `space`, for `who` to rename, move or throw away: its space as they are
    /// told it, the space's root, and the folder's place.
    fn folder_in(&self, who: &Signed, space: &str, path: &str) -> Answer<(Space, Root, Place)> {
        let (space, role) = self.space(who, space)?;
        if !role.edits() {
            return Err(Refusal::forbidden("You can read this space, but not change it."));
        }
        let (root, place) = (self.root(who, space)?, Place::parse(path)?);
        if place.is_root() {
            return Err(Refusal::invalid("A space itself cannot be renamed, moved or thrown away."));
        }
        // A document is asked for by its id, which follows it wherever it goes.
        let (parent, name) = place.split();
        let listed = root.folder(&parent)?.list()?;
        if !listed.iter().any(|listed| listed.folder && listed.name == name) {
            return Err(Refusal::not_found());
        }
        Ok((space.told(role), root, place))
    }

    /// Renames the folder at `path` in `space`. Whoever has a document in it open stays in it.
    pub fn rename_folder(&self, who: &Signed, space: &str, path: &str, name: &str) -> Answer<Entry> {
        let (space, root, place) = self.folder_in(who, space, path)?;
        let name = file_name(name).ok_or_else(|| Refusal::invalid("That name cannot be used for a folder."))?;
        let to = place.split().0.child(&name)?;
        if to != place {
            let (from, at) = ((&root, &place), (&root, &to));
            self.rooms.moving(from, Some(at), || self.library.relocate(&self.store, from, at))?;
        }
        self.entry(who, &space, &root, &to)
    }

    /// Moves the folder at `path` in `space` into another folder, with everything in it.
    pub fn move_folder(&self, who: &Signed, space: &str, path: &str, to: &Move) -> Answer<Entry> {
        let (_, root, place) = self.folder_in(who, space, path)?;
        let (space, role) = self.space(who, &to.space)?;
        if !role.edits() {
            return Err(Refusal::forbidden("You can read that space, but not add to it."));
        }
        let (there, into) = (self.root(who, space)?, Place::parse(&to.folder)?);
        let at = into.child(place.name())?;
        let own = there.key == root.key;
        if own && (into == place || into.as_str().starts_with(&format!("{}/", place.as_str()))) {
            return Err(Refusal::invalid("A folder cannot be moved into itself."));
        }
        if !own || at != place {
            let (from, at) = ((&root, &place), (&there, &at));
            self.rooms.moving(from, Some(at), || self.library.relocate(&self.store, from, at))?;
        }
        self.entry(who, &space.told(role), &there, &at)
    }

    /// Moves the folder at `path` in `space` to the space's trash, with everything in it. Whoever has a
    /// document in it open is told that it is gone. A folder holds the work of everyone in its space, so
    /// only an owner of the space throws one away: whoever edits there renames and moves it.
    pub fn trash_folder(&self, who: &Signed, space: &str, path: &str) -> Answer<()> {
        let (space, root, place) = self.folder_in(who, space, path)?;
        if space.role != Role::Owner {
            return Err(Refusal::forbidden("Only an owner of this space can move a folder to the trash."));
        }
        let thrown = || self.library.trash(&self.store, &root, &place, self.now());
        self.rooms.moving((&root, &place), None, thrown)
    }

    /// Moves the document `id` to the trash of its space.
    pub fn trash(&self, who: &Signed, id: &str) -> Answer<()> {
        let found = self.locate(who, id)?;
        found.must(Role::Editor)?;
        found.placed()?;
        let thrown = || self.library.trash(&self.store, &found.root, &found.place, self.now());
        self.rooms.moving((&found.root, &found.place), None, thrown)
    }

    /// Merges a device's own copy of the document `id` into it. Whoever has the document open is told
    /// what the copy brought.
    pub fn merge(&self, who: &Signed, id: &str, file: &[u8]) -> Answer<()> {
        let found = self.locate(who, id)?;
        found.must(Role::Editor)?;
        let mut theirs = Archive::open(std::io::Cursor::new(file))?;
        // A copy made by hand that became a document of its own is not this one any more.
        if theirs.manifest().id != id {
            return Err(Refusal::invalid("That file is of another document."));
        }
        self.merge_read(&found, id, read(&mut theirs)?)
    }

    /// Merges a copy that was read into the document `id`, which was found at `found` before the read:
    /// through its room when it is open, and into its file otherwise. A long history takes a while to
    /// read, so the copy is read with nothing held, and the document may have been moved or thrown away
    /// by now: the copy is then refused, and merged nowhere.
    pub fn merge_read<R: Read + Seek>(&self, found: &Found, id: &str, theirs: Reading<'_, R>) -> Answer<()> {
        let beside = |theirs| self.library.merge(&self.store, &found.root, &found.place, id, theirs, self.now());
        self.rooms.merge(&found.root, &found.place, id, theirs, beside)
    }

    /// Merges into the document `id` every conflict copy beside it, through its room when it is open, and
    /// moves each copy to the trash once the document holds what the copy held. Says the copies' names.
    pub fn merge_conflicts(&self, who: &Signed, id: &str) -> Answer<Vec<String>> {
        let found = self.locate(who, id)?;
        found.must(Role::Editor)?;
        let rows = self.library.list(&self.store, &found.root, &found.place.split().0)?;
        let mut merged = Vec::new();
        for copy in rows.iter().filter(|row| row.copy_of.as_ref() == Some(&found.place)) {
            let mut theirs = self.library.open(&found.root, &copy.place)?;
            self.merge_read(&found, id, read(&mut theirs)?)?;
            // Closed before it is moved, which a folder on Windows asks for.
            drop(theirs);
            self.library.trash(&self.store, &found.root, &copy.place, self.now())?;
            merged.push(shown(&copy.listed.name).to_owned());
        }
        Ok(merged)
    }

    /// What is in the trash of every space `who` reaches, the last thrown away first. A space whose trash
    /// cannot be read now tells none of it, and the others still do.
    pub fn trashed(&self, who: &Signed) -> Vec<Trashed> {
        let mut trashed = Vec::new();
        for (space, root) in self.reach(who) {
            for (listed, was) in self.library.thrown(&self.store, &root).unwrap_or_default() {
                // A file some other program put in the trash is known by its name there alone.
                let path = was.as_ref().map_or(listed.name.as_str(), |was| was.path.as_str());
                let (folder, name) = path.rsplit_once('/').unwrap_or(("", path));
                trashed.push(Trashed {
                    kind: if listed.folder { EntryKind::Folder } else { EntryKind::Document },
                    name: shown(name).to_owned(),
                    space: space.id.clone(),
                    folder: folder.to_owned(),
                    file: listed.name.clone(),
                    thrown: was.as_ref().map_or(seconds(listed.stamp), |was| was.at),
                    role: space.role,
                });
            }
        }
        trashed.sort_by_key(|trashed| std::cmp::Reverse(trashed.thrown));
        trashed
    }

    /// Puts `file` back from the trash of `space`, and says where it is now.
    pub fn put_back(&self, who: &Signed, space: &str, file: &str) -> Answer<Entry> {
        let (space, role) = self.space(who, space)?;
        if !role.edits() {
            return Err(Refusal::forbidden("You can read this space, but not add to it."));
        }
        let root = self.root(who, space)?;
        let place = self.library.put_back(&self.store, &root, file)?;
        self.entry(who, &space.told(role), &root, &place)
    }

    /// The document `id`, ready to edit. A `page` carries its style and its notes along.
    pub fn opened(&self, who: &Signed, id: &str, page: bool) -> Answer<Opened> {
        Ok(self.opening(who, &self.locate(who, id)?, page)?.0)
    }

    /// The document `id` as a page for a browser's editor is made of it: what it holds now, and how it
    /// is opened. `None` for whoever it is only [shown to read](Found::reading), who is noted nowhere.
    pub fn page(&self, who: &Signed, id: &str) -> Answer<Option<(Document, Opened)>> {
        let found = self.locate(who, id)?;
        if found.reading() {
            return Ok(None);
        }
        let (opened, mut file) = self.opening(who, &found, true)?;
        Ok(Some((file.document()?, opened)))
    }

    /// How the document that was found is opened, and its file as that was read from.
    fn opening(&self, who: &Signed, found: &Found, page: bool) -> Answer<(Opened, File)> {
        if page {
            // The style and the notes are read from the file, which its room may be ahead of.
            self.rooms.save(&found.root, &found.place);
        }
        let mut archive = self.library.open(&found.root, &found.place)?;
        let manifest = archive.manifest().clone();
        let json = |archive: &mut letters_format::Archive<_>, name: &str| -> Answer<Option<Value>> {
            let bytes = if page { archive.read(name)? } else { None };
            Ok(bytes.and_then(|bytes| serde_json::from_slice(&bytes).ok()))
        };
        let (style, notes) = (json(&mut archive, STYLE)?, json(&mut archive, NOTES)?);
        self.note_opened(who, &manifest.id)?;
        self.note(who, found, &manifest.id, Action::Open, false);
        let opened = Opened {
            id: manifest.id.clone(),
            created: manifest.created,
            name: shown(found.place.name()).to_owned(),
            named: !untitled(shown(found.place.name())),
            space: found.space.as_ref().map(|space| space.id.clone()),
            // Where a shared document is kept is its owner's business.
            path: found.space.as_ref().map(|_| found.place.as_str().to_owned()).unwrap_or_default(),
            role: found.role,
            author: self.signs_as(who)?,
            socket: paths::socket(&manifest.id),
            trust: if self.trusts(who, found, &manifest.id)? { Trust::Trusted } else { Trust::Safe },
            allows: found.allows,
            by_link: found.by_link(),
            last_seen: self.last_seen(who, &manifest.id)?,
            style,
            notes,
        };
        Ok((opened, archive))
    }

    /// Puts the document first among the person's recents. Whoever only a link let in has none.
    pub fn note_opened(&self, who: &Signed, id: &str) -> Answer<()> {
        if who.alone() {
            return Ok(());
        }
        Ok(self.store.write(|batch| {
            let mut recent = batch.get(&RECENT, &who.person.id)?.unwrap_or_default();
            recent.retain(|other| other != id);
            recent.insert(0, id.to_owned());
            recent.truncate(RECENTS);
            batch.put(&RECENT, &who.person.id, &recent)
        })?)
    }

    /// The documents the person opened lately that they still reach, newest first.
    pub fn recent(&self, who: &Signed) -> Answer<Vec<Entry>> {
        let ids = self.store.get(&RECENT, &who.person.id)?.unwrap_or_default();
        let mut entries = Vec::new();
        for id in ids {
            let found = match self.locate(who, &id) {
                Ok(found) => found,
                // One that is gone, or out of reach now, is left out and not an error.
                Err(refusal) if refusal.code() == Code::NotFound => continue,
                Err(refusal) => return Err(refusal),
            };
            if let Ok(entry) = self.found_entry(who, &found, &found.place) {
                entries.push(entry);
            }
        }
        Ok(entries)
    }
}

/// A row as a file table of its space shows it.
fn placed(space: &Space, row: &Row) -> Entry {
    Entry { space: Some(space.id.clone()), path: row.place.as_str().to_owned(), ..entry(row, space.role) }
}

/// A row as a file table shows it to whoever may do what `role` may. Where it is, is not said.
fn entry(row: &Row, role: Role) -> Entry {
    let known = row.known.as_ref();
    let mut tags = Vec::new();
    if let Some(original) = &row.copy_of {
        tags.push(Tag::ConflictCopy { of: original.as_str().to_owned() });
    }
    match known {
        Some(known) if known.id.is_none() => tags.push(Tag::Damaged),
        Some(known) if known.import_notes > 0 => tags.push(Tag::ImportNotes { count: known.import_notes }),
        _ => {}
    }
    Entry {
        kind: if row.listed.folder { EntryKind::Folder } else { EntryKind::Document },
        name: shown(&row.listed.name).to_owned(),
        space: None,
        path: String::new(),
        id: known.and_then(|known| known.id.clone()),
        created: known.and_then(|known| known.created.clone()),
        modified: seconds(row.listed.stamp),
        role,
        contributors: known.map(|known| known.contributors.clone()).unwrap_or_default(),
        tags,
    }
}

/// The routes with their state, for the server and for tests.
pub fn router(app: Arc<App>) -> Router {
    let routes = routes().merge(crate::shares::routes()).merge(crate::links::routes());
    let routes = routes.merge(crate::access::routes()).merge(crate::socket::routes()).merge(crate::seen::routes());
    let routes = routes.merge(crate::dictionaries::routes());
    let routes = routes.merge(crate::auth::routes()).merge(crate::pages::routes());
    // Around every route, and around the answer where there is none.
    let routes = match app.config.server.log {
        Log::Debug => routes.layer(axum::middleware::from_fn(crate::logs::request)),
        Log::Info => routes,
    };
    routes.with_state(app)
}
