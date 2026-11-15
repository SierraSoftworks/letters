//! The interface `letters_api` lays down, answered.
//!
//! Each address is a thin async wrapper: the work is in plain functions over [`App`], which read folders
//! and the store and so run where blocking is allowed.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE, SET_COOKIE};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use letters_api::{
    Code, Entry, EntryKind, Listing, Move, NewDocument, NewFolder, Opened, PROTOCOL, Person, Rename, Renewal, Role,
    Server, Settings, Space, Tag, Token, Trust, paths,
};
use letters_format::{ASSETS, MIMETYPE, NOTES, STYLE, file_name};
use letters_model::Style;
use serde::Deserialize;
use serde_json::Value;

use crate::error::{Answer, Refusal};
use crate::library::Row;
use crate::session::{Carrier, Signed};
use crate::spaces::{EXTENSION, Place, Root, seconds, shown};
use crate::store::Table;
use crate::{App, Shared};

/// By [`Person::id`].
const SETTINGS: Table<Settings> = Table::new("settings");
/// By [`Person::id`]: the documents they opened, newest first.
const RECENT: Table<Vec<String>> = Table::new("recent");
/// How many documents a person's recents hold.
const RECENTS: usize = 30;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route(paths::SERVER, get(server))
        .route(paths::SESSION, get(session))
        .route(paths::RENEW, post(renew))
        .route(paths::SIGN_OUT, post(sign_out))
        .route(paths::SETTINGS, get(settings).put(set_settings))
        .route(paths::SPACES, get(spaces))
        .route(paths::RECENT, get(recent))
        .route(paths::SHARED, get(shared))
        .route(paths::SOURCES, get(sources))
        .route("/api/v1/spaces/{space}/entries", get(entries))
        .route("/api/v1/spaces/{space}/documents", post(new_document))
        .route("/api/v1/spaces/{space}/folders", post(new_folder))
        .route("/api/v1/documents/{id}", get(document))
        .route("/api/v1/documents/{id}/file", get(file))
        .route("/api/v1/documents/{id}/rename", post(rename))
        .route("/api/v1/documents/{id}/move", post(relocate))
        .route("/api/v1/documents/{id}/trash", post(trash))
        .route("/api/v1/documents/{id}/assets", get(assets))
        .route("/api/v1/documents/{id}/assets/{name}", get(asset))
}

/// Runs `work` where it may wait on a folder or the store.
pub async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Answer<T> + Send + 'static) -> Answer<T> {
    let failed = |_| Refusal::new(Code::Internal, "The server failed while answering.");
    tokio::task::spawn_blocking(work).await.map_err(failed)?
}

async fn server(State(app): State<Shared>) -> Json<Server> {
    Json(Server {
        product: Server::PRODUCT.into(),
        name: app.config.server.name.clone(),
        version: env!("CARGO_PKG_VERSION").into(),
        protocols: vec![PROTOCOL],
        links: app.config.links.enabled,
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
    blocking(move || Ok(Json(app.store.get(&SETTINGS, &who.person.id)?.unwrap_or_default()))).await
}

async fn set_settings(
    State(app): State<Shared>,
    who: Signed,
    Json(settings): Json<Settings>,
) -> Answer<Json<Settings>> {
    if let Some(style) = &settings.default_style
        && let Err(error) = Style::from_json(&style.to_string())
    {
        return Err(Refusal::invalid(format!("The default style cannot be read: {error}.")));
    }
    blocking(move || {
        app.store.put(&SETTINGS, &who.person.id, &settings)?;
        Ok(Json(settings))
    })
    .await
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
    blocking(move || app.new_document(&who, &space, &new).map(Json)).await
}

async fn new_folder(
    State(app): State<Shared>,
    who: Signed,
    Path(space): Path<String>,
    Json(new): Json<NewFolder>,
) -> Answer<Json<Entry>> {
    blocking(move || app.new_folder(&who, &space, &new).map(Json)).await
}

async fn document(State(app): State<Shared>, who: Signed, Path(id): Path<String>) -> Answer<Json<Opened>> {
    blocking(move || app.opened(&who, &id, false).map(Json)).await
}

async fn file(State(app): State<Shared>, who: Signed, Path(id): Path<String>) -> Answer<Response> {
    let (name, bytes) = blocking(move || {
        let found = app.locate(&who, &id)?;
        Ok((found.place.name().to_owned(), app.library.bytes(&found.root, &found.place)?))
    })
    .await?;
    // The name is the file's own, less what a header cannot carry.
    let plain: String = name.chars().filter(|c| c.is_ascii() && !c.is_control() && !matches!(c, '"' | '\\')).collect();
    let disposition = format!("attachment; filename=\"{plain}\"; filename*=UTF-8''{}", paths::segment(&name));
    Ok(([(CONTENT_TYPE, MIMETYPE.to_owned()), (CONTENT_DISPOSITION, disposition)], bytes).into_response())
}

async fn assets(State(app): State<Shared>, who: Signed, Path(id): Path<String>) -> Answer<Json<Vec<String>>> {
    blocking(move || {
        let found = app.locate(&who, &id)?;
        let archive = app.library.open(&found.root, &found.place)?;
        Ok(Json(archive.names().filter_map(|name| name.strip_prefix(ASSETS)).map(str::to_owned).collect()))
    })
    .await
}

async fn asset(State(app): State<Shared>, who: Signed, Path((id, name)): Path<(String, String)>) -> Answer<Response> {
    let bytes = blocking(move || {
        let found = app.locate(&who, &id)?;
        // An asset is named by the hash of its bytes, with an ending: nothing else is one.
        let plain = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.');
        let entry = plain.then(|| format!("{ASSETS}{name}")).ok_or_else(Refusal::not_found)?;
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

async fn rename(
    State(app): State<Shared>,
    who: Signed,
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

async fn trash(State(app): State<Shared>, who: Signed, Path(id): Path<String>) -> Answer<()> {
    blocking(move || app.trash(&who, &id)).await
}

async fn recent(State(app): State<Shared>, who: Signed) -> Answer<Json<Vec<Entry>>> {
    blocking(move || app.recent(&who).map(Json)).await
}

/// Nothing is shared from beyond a person's spaces until documents can be shared with people.
async fn shared(_: Signed) -> Json<Vec<Entry>> {
    Json(Vec::new())
}

/// A document as a session reaches it.
pub struct Found {
    pub space: Space,
    pub root: Root,
    pub place: Place,
}

impl Found {
    /// Refuses unless the session may do what `role` may.
    pub fn must(&self, role: Role) -> Answer<()> {
        match self.space.role >= role {
            true => Ok(()),
            false => Err(Refusal::forbidden("You can read this document, but not change it.")),
        }
    }
}

impl App {
    /// Where the document `id` is, for `who`. One in no space they reach is not there for them.
    pub fn locate(&self, who: &Signed, id: &str) -> Answer<Found> {
        let reach = self.reach(who);
        let roots: Vec<&Root> = reach.iter().map(|(_, root)| root).collect();
        let (at, place) = self.library.find(&self.store, &roots, id, self.now())?.ok_or_else(Refusal::not_found)?;
        let (space, root) = reach.into_iter().nth(at).expect("the library answers with one of the roots it was given");
        Ok(Found { space, root, place })
    }

    pub fn listing(&self, who: &Signed, space: &str, folder: &str) -> Answer<Listing> {
        let (space, role) = self.space(who, space)?;
        let (root, place) = (self.root(who, space)?, Place::parse(folder)?);
        let told = space.told(role);
        let rows = self.library.list(&self.store, &root, &place)?;
        let entries = rows.iter().map(|row| entry(&told, row)).collect();
        Ok(Listing { space: told, path: place.as_str().to_owned(), entries })
    }

    /// The row of the file at `place`, as a file table shows it.
    fn entry(&self, space: &Space, root: &Root, place: &Place) -> Answer<Entry> {
        let (folder, name) = place.split();
        let rows = self.library.list(&self.store, root, &folder)?;
        rows.iter().find(|row| row.listed.name == name).map(|row| entry(space, row)).ok_or_else(Refusal::not_found)
    }

    pub fn new_document(&self, who: &Signed, space: &str, new: &NewDocument) -> Answer<Entry> {
        let (space, role) = self.space(who, space)?;
        if !role.edits() {
            return Err(Refusal::forbidden("You can read this space, but not add to it."));
        }
        let (root, folder) = (self.root(who, space)?, Place::parse(&new.folder)?);
        let settings = self.store.get(&SETTINGS, &who.person.id)?.unwrap_or_default();
        let style = settings.default_style.and_then(|style| Style::from_json(&style.to_string()).ok());
        let author = settings.name.as_deref().unwrap_or(&who.person.name);
        let name = new.name.as_deref();
        let place = self.library.create(&root, &folder, name, style.unwrap_or_default(), author, self.now())?;
        self.entry(&space.told(role), &root, &place)
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
        self.entry(&space.told(role), &root, &place)
    }

    pub fn rename(&self, who: &Signed, id: &str, name: &str) -> Answer<Entry> {
        let found = self.locate(who, id)?;
        found.must(Role::Editor)?;
        let name = file_name(name).ok_or_else(|| Refusal::invalid("That name cannot be used for a document."))?;
        let to = found.place.split().0.child(&format!("{name}{EXTENSION}"))?;
        if to != found.place {
            self.library.relocate(&self.store, (&found.root, &found.place), (&found.root, &to))?;
        }
        self.entry(&found.space, &found.root, &to)
    }

    pub fn relocate(&self, who: &Signed, id: &str, to: &Move) -> Answer<Entry> {
        let found = self.locate(who, id)?;
        found.must(Role::Editor)?;
        let (space, role) = self.space(who, &to.space)?;
        if !role.edits() {
            return Err(Refusal::forbidden("You can read that space, but not add to it."));
        }
        let root = self.root(who, space)?;
        let place = Place::parse(&to.folder)?.child(found.place.name())?;
        if (root.key.as_str(), &place) != (found.root.key.as_str(), &found.place) {
            self.library.relocate(&self.store, (&found.root, &found.place), (&root, &place))?;
        }
        self.entry(&space.told(role), &root, &place)
    }

    /// Moves the document `id` to the trash of its space.
    pub fn trash(&self, who: &Signed, id: &str) -> Answer<()> {
        let found = self.locate(who, id)?;
        found.must(Role::Editor)?;
        self.library.trash(&self.store, &found.root, &found.place, self.now())
    }

    /// Puts `file` back from the trash of `space`, and says where it is now.
    pub fn put_back(&self, who: &Signed, space: &str, file: &str) -> Answer<Entry> {
        let (space, role) = self.space(who, space)?;
        if !role.edits() {
            return Err(Refusal::forbidden("You can read this space, but not add to it."));
        }
        let root = self.root(who, space)?;
        let place = self.library.put_back(&self.store, &root, file)?;
        self.entry(&space.told(role), &root, &place)
    }

    /// The document `id`, ready to edit. A `page` carries its style and its notes along.
    pub fn opened(&self, who: &Signed, id: &str, page: bool) -> Answer<Opened> {
        let found = self.locate(who, id)?;
        let mut archive = self.library.open(&found.root, &found.place)?;
        let manifest = archive.manifest().clone();
        let json = |archive: &mut letters_format::Archive<_>, name: &str| -> Answer<Option<Value>> {
            let bytes = if page { archive.read(name)? } else { None };
            Ok(bytes.and_then(|bytes| serde_json::from_slice(&bytes).ok()))
        };
        let (style, notes) = (json(&mut archive, STYLE)?, json(&mut archive, NOTES)?);
        let settings = self.store.get(&SETTINGS, &who.person.id)?.unwrap_or_default();
        self.note_opened(who, &manifest.id)?;
        Ok(Opened {
            id: manifest.id.clone(),
            created: manifest.created,
            name: shown(found.place.name()).to_owned(),
            space: Some(found.space.id.clone()),
            path: found.place.as_str().to_owned(),
            role: found.space.role,
            author: settings.name.unwrap_or_else(|| who.person.name.clone()),
            socket: paths::socket(&manifest.id),
            // A document in a space the person reaches is vouched for by whoever let them in.
            trust: Trust::Trusted,
            allows: letters_api::Allows { download: true, copy: true, print: true },
            style,
            notes,
        })
    }

    /// Puts the document first among the person's recents.
    pub fn note_opened(&self, who: &Signed, id: &str) -> Answer<()> {
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
        let reach = self.reach(who);
        let roots: Vec<&Root> = reach.iter().map(|(_, root)| root).collect();
        let mut entries = Vec::new();
        for id in ids {
            // One that is gone, or out of reach now, is left out and not an error.
            let Some((at, place)) = self.library.find(&self.store, &roots, &id, self.now())? else { continue };
            if let Ok(entry) = self.entry(&reach[at].0, &reach[at].1, &place) {
                entries.push(entry);
            }
        }
        Ok(entries)
    }
}

/// A row of a folder as a file table shows it.
fn entry(space: &Space, row: &Row) -> Entry {
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
        space: Some(space.id.clone()),
        path: row.place.as_str().to_owned(),
        id: known.and_then(|known| known.id.clone()),
        created: known.and_then(|known| known.created.clone()),
        modified: seconds(row.listed.stamp),
        role: space.role,
        contributors: known.map(|known| known.contributors.clone()).unwrap_or_default(),
        tags,
    }
}

/// The routes with their state, for the server and for tests.
pub fn router(app: Arc<App>) -> Router {
    routes().merge(crate::auth::routes()).merge(crate::pages::routes()).with_state(app)
}
