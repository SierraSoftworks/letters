//! The server's own pages, for a browser: the documents a person reaches, their folders and their trash.
//!
//! A page is drawn where it is served, from yewi's components, and loads a stylesheet and one small script
//! of yewi's: no wasm, so everything on it is a link or a form.
//!
//! - [`view`] is what a page shows, and [`acts`] what its forms do.
//! - [`statics`] is what a page loads.
//! - [`read`] is a document as plain HTML, to read with no editor.
//! - [`editor`] is a document in the browser's editor, and the files of that editor.
//! - [`door`] is where a share link leads.

mod activity;
mod acts;
mod door;
pub mod editor;
mod read;
mod statics;
pub mod view;

use std::rc::Rc;

use axum::Router;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use letters_api::{Code, Entry, EntryKind, Link, Person, Role, paths};
use serde::Deserialize;
use yew::ServerRenderer;

use crate::api::blocking;
use crate::auth::{Desktop, browser};
use crate::error::{Answer, Refusal};
use crate::links::Caller;
use crate::session::{Carrier, Reader, Signed, named_document};
use crate::shares::may_share;
use crate::spaces::Place;
use crate::{App, Shared};

use self::statics::{MANIFEST, TAB_ICON, TOUCH_ICON, statics};
use self::view::{Ask, At, Body, Card, Folder, Linked, Page, Root, RootProps, Row, Target, Trashed};

/// A page: the documents the person opened lately.
pub const RECENT_PAGE: &str = "/recent";
/// A page: the documents shared with the person from beyond their spaces.
pub const SHARED_PAGE: &str = "/shared";
/// A page: what was thrown away in the spaces the person reaches.
pub const TRASH_PAGE: &str = "/trash";

/// Where the forms of the pages go. Each answers with the page to show next.
pub const ACT_NEW_DOCUMENT: &str = "/act/new-document";
pub const ACT_NEW_FOLDER: &str = "/act/new-folder";
pub const ACT_IMPORT: &str = "/act/import";
pub const ACT_IMPORT_FILE: &str = "/act/import-file";
pub const ACT_RENAME: &str = "/act/rename";
pub const ACT_MOVE: &str = "/act/move";
pub const ACT_TRASH: &str = "/act/trash";
pub const ACT_MERGE: &str = "/act/merge-conflicts";
pub const ACT_PUT_BACK: &str = "/act/put-back";
pub const ACT_SHARE: &str = "/act/share";
pub const ACT_UNSHARE: &str = "/act/unshare";
pub const ACT_REVOKE: &str = "/act/revoke-link";

/// How many folders a document is offered to be moved to. A space with more is tidied in the editor's own
/// file dialogs or on the disk.
const MOST_TARGETS: usize = 500;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route(paths::HOME_PAGE, get(home))
        .route("/s/{space}", get(space))
        .route("/s/{space}/{*path}", get(folder))
        .route(RECENT_PAGE, get(recent))
        .route(SHARED_PAGE, get(shared))
        .route(TRASH_PAGE, get(trash))
        .route("/d/{id}", get(editor::page))
        .route("/d/{id}/read", get(read::page))
        .route("/l/{token}", get(door::follow).post(door::tried))
        .route("/static/{version}/{*file}", get(statics::file))
        .route("/assets/{version}/{*file}", get(editor::file))
        .merge(acts::routes())
}

/// Whoever a page is for. A browser with no session is sent to sign in, and comes back to the page it
/// asked for.
pub struct Visitor(pub Signed);

impl FromRequestParts<Shared> for Visitor {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, app: &Shared) -> Result<Self, Response> {
        match browser(app, &parts.method, &parts.headers) {
            Ok(who) => Ok(Self(who)),
            Err(refusal) => Err(turned_away(app, parts, &refusal).await),
        }
    }
}

/// What a browser that a page is not for is answered with.
async fn turned_away(app: &Shared, parts: &Parts, refusal: &Refusal) -> Response {
    // A form that some other site's page sent.
    if refusal.code() == Code::Forbidden {
        return refused(app, None, refusal).await;
    }
    // Only a page can be come back to: what a form asked for is asked again from its page.
    let asked = parts.uri.path_and_query().map_or(paths::HOME_PAGE, |asked| asked.as_str());
    let here = if parts.method == Method::GET { asked } else { paths::HOME_PAGE };
    Redirect::to(&paths::sign_in_page(here)).into_response()
}

/// Whoever a document's page is for: someone signed in with this browser, or someone a share link let in
/// to the document. A browser that is neither is sent to sign in, as for any page.
pub struct Opener(pub Signed);

impl FromRequestParts<Shared> for Opener {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, app: &Shared) -> Result<Self, Response> {
        let id = named_document(parts, app).await;
        match Reader::read(app, &parts.method, &parts.headers, id.as_deref()) {
            Ok(Reader(mut who)) if who.carrier == Carrier::Cookie => {
                who.from = Caller::of(parts, app);
                Ok(Self(who))
            }
            Ok(_) => Err(turned_away(app, parts, &Refusal::signed_out()).await),
            Err(refusal) => Err(turned_away(app, parts, &refusal).await),
        }
    }
}

/// Draws `page` and sends it under `title`. A page loads the server's own files and nothing else, and a
/// form on it goes to this server, or to `beyond` when the page names one.
async fn send(status: StatusCode, title: &str, page: Page, beyond: Option<&str>) -> Response {
    let (server, base) = (escape(&page.server), page.statics.clone());
    let drawn = ServerRenderer::<Root>::with_props(move || RootProps { page: Rc::new(page) });
    let body = drawn.hydratable(false).render().await;
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta name=\"color-scheme\" content=\"light dark\"><title>{} · {server}</title>\
         <link rel=\"stylesheet\" href=\"{base}/fonts/fonts.css\"><link rel=\"stylesheet\" href=\"{base}/yewi.css\">\
         <link rel=\"stylesheet\" href=\"{base}/letters.css\">\
         <link rel=\"icon\" href=\"{base}/{TAB_ICON}\" sizes=\"any\">\
         <link rel=\"apple-touch-icon\" href=\"{base}/{TOUCH_ICON}\">\
         <link rel=\"manifest\" href=\"{base}/{MANIFEST}\"></head><body>{body}</body></html>",
        escape(title)
    );
    let policy = format!(
        "default-src 'none'; style-src 'self' 'unsafe-inline'; script-src 'self'; font-src 'self'; \
         img-src 'self' data:; manifest-src 'self'; form-action 'self'{}{}; base-uri 'none'; frame-ancestors 'none'",
        if beyond.is_some() { " " } else { "" },
        beyond.unwrap_or_default()
    );
    (status, guarded(&policy), html).into_response()
}

/// The headers of every page: what it may load, and that it is for one person at one moment.
fn guarded(policy: &str) -> [(axum::http::HeaderName, HeaderValue); 5] {
    [
        (CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8")),
        (CACHE_CONTROL, HeaderValue::from_static("no-store")),
        (CONTENT_SECURITY_POLICY, HeaderValue::from_str(policy).expect("a policy is plain text")),
        (X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
        // The address of a sign-in holds what ends it, and a document's is nobody else's to see. The
        // server's own pages are told, since a browser that may name no referrer says `Origin: null` with a
        // form, and a form is only taken from these pages.
        (REFERRER_POLICY, HeaderValue::from_static("same-origin")),
    ]
}

/// Text as it stands in HTML, in an attribute as between tags.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A page for `who` with the frame around it, or one with no frame for nobody.
fn page(app: &App, who: Option<&Signed>, at: At, body: Body) -> Page {
    let spaces = who.map(|who| app.spaces(who).into_iter().map(|(space, role)| space.told(role)).collect());
    Page {
        server: app.config.server.name.clone(),
        statics: statics().base.clone(),
        who: who.map(|who| who.person.clone()),
        spaces: spaces.unwrap_or_default(),
        at,
        refused: None,
        body,
    }
}

/// A page with no frame: one thing said in the middle of it.
async fn card(app: &App, status: StatusCode, title: &str, card: Card, beyond: Option<&str>) -> Response {
    send(status, title, page(app, None, At::Nowhere, Body::Card(card)), beyond).await
}

fn status(refusal: &Refusal) -> StatusCode {
    StatusCode::from_u16(refusal.code().status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

/// Tells the person that signing in did not work, and offers the way to try again.
pub async fn failed(app: &App, refusal: &Refusal) -> Response {
    let title = "Signing in did not work";
    let again = Some((paths::SIGN_IN_PAGE.to_owned(), "Sign in again".to_owned()));
    let said = Card::Failed { title: title.into(), message: refusal.0.message.clone(), again };
    card(app, status(refusal), title, said, None).await
}

/// Tells the person what a page could not show, inside the frame when they are signed in. Whoever only a
/// link let in has no frame: nothing in it would be theirs.
pub async fn refused(app: &App, who: Option<&Signed>, refusal: &Refusal) -> Response {
    let linked = who.is_some_and(Signed::alone);
    let who = who.filter(|who| !who.alone());
    let title = match refusal.code() {
        Code::NotFound => "Nothing here",
        Code::Forbidden if linked => "Not with this link",
        Code::Forbidden => "Not yours to change",
        Code::Unavailable => "Out of reach just now",
        _ => "That did not work",
    };
    // Whoever only a link let in has no documents here to go back to.
    let again = (!linked).then(|| (paths::HOME_PAGE.to_owned(), "Back to your documents".to_owned()));
    let said = Card::Failed { title: title.into(), message: refusal.0.message.clone(), again };
    match who {
        Some(who) => send(status(refusal), title, page(app, Some(who), At::Nowhere, Body::Card(said)), None).await,
        None => card(app, status(refusal), title, said, None).await,
    }
}

/// Asks the person whether the Letters app that opened this page may be signed in as them. Agreeing sends
/// the browser to the app, which listens on this computer.
pub async fn hand_over(app: &App, person: &Person, asked: &Desktop) -> Response {
    let said = Card::HandOver {
        name: person.name.clone(),
        challenge: asked.challenge.clone(),
        port: asked.port.to_string(),
        state: asked.state.clone(),
    };
    card(app, StatusCode::OK, "Sign in to Letters", said, Some(&asked.listener())).await
}

/// The front page: the first of the person's spaces, or the way to sign in. Nobody is sent to the
/// provider from here unasked, so that signing out ends somewhere.
async fn home(State(app): State<Shared>, headers: HeaderMap) -> Response {
    let Ok(who) = browser(&app, &Method::GET, &headers) else {
        let said = Card::SignIn { then: paths::HOME_PAGE.into() };
        return card(&app, StatusCode::OK, "Sign in", said, None).await;
    };
    match app.spaces(&who).first() {
        Some((space, _)) => Redirect::to(&paths::folder_page(&space.id(), "")).into_response(),
        None => send(StatusCode::OK, "Documents", page(&app, Some(&who), At::Nowhere, Body::NoSpaces), None).await,
    }
}

/// What a folder's address asks for beside the folder.
#[derive(Default, Deserialize)]
struct Asked {
    /// The question to open the page with: `rename`, `move`, `trash`, `share` or `export`, about the
    /// document `id`, or one of the first three about the folder at `folder`.
    #[serde(default)]
    ask: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    folder: Option<String>,
    /// The code of what a form on the page was refused with.
    #[serde(default)]
    error: Option<String>,
    /// With `share`: the dialog opens on the access log's rows from before this time.
    #[serde(default)]
    before: Option<u64>,
}

/// The address that opens the folder at `here` with a question about the document `id`.
pub fn asking(here: &str, what: &str, id: &str) -> String {
    format!("{here}?ask={what}&id={}", paths::segment(id))
}

/// The address that opens the folder at `here` with a question about the folder at `path`, which is in
/// it.
pub fn asking_folder(here: &str, what: &str, path: &str) -> String {
    format!("{here}?ask={what}&folder={}", paths::segment(path))
}

/// What an address says in place of a code when a conflict copy could not be merged.
const UNMERGED: &str = "unmerged";

/// The address of `back` saying `word` of what was asked there.
fn saying(back: &str, word: &str) -> String {
    format!("{}?error={word}", back.split(['?', '#']).next().unwrap_or_default())
}

/// The address of `back` saying that what was asked there was refused.
pub fn refusing(back: &str, refusal: &Refusal) -> String {
    let code = serde_json::to_value(refusal.code()).ok();
    saying(back, code.as_ref().and_then(|code| code.as_str()).unwrap_or_default())
}

/// The address of `back` saying that a conflict copy could not be merged.
pub fn unmerged(back: &str) -> String {
    saying(back, UNMERGED)
}

/// What a page says of a refusal an address names. Only words of the server's own: whoever writes the
/// address chooses among them.
fn refusal(code: &str) -> String {
    if code == UNMERGED {
        return "That copy could not be merged, and is still where it was.".to_owned();
    }
    let code = serde_json::from_value(serde_json::Value::String(code.to_owned())).unwrap_or(Code::Unknown);
    let said = match code {
        Code::NameTaken => "A document or folder with that name already exists.",
        Code::Invalid => "That name cannot be used.",
        Code::Forbidden => "You can read this, but not change it.",
        Code::NotFound => "It is no longer there.",
        Code::Unavailable => "The folder behind this space cannot be used just now.",
        _ => "That did not work. Try again.",
    };
    said.to_owned()
}

async fn space(app: State<Shared>, who: Visitor, Path(space): Path<String>, asked: Query<Asked>) -> Response {
    folder(app, who, Path((space, String::new())), asked).await
}

async fn folder(
    State(app): State<Shared>,
    Visitor(who): Visitor,
    Path((space, path)): Path<(String, String)>,
    Query(asked): Query<Asked>,
) -> Response {
    let shown = blocking({
        let (app, who) = (app.clone(), who.clone());
        move || {
            let listing = app.listing(&who, &space, &path, false)?;
            let now = app.now();
            let (edits, owns) = (listing.space.role.edits(), listing.space.role == Role::Owner);
            let about = |id: &String| listing.entries.iter().find(|entry| entry.id.as_ref() == Some(id)).cloned();
            let within = |path: &String| {
                let folders = listing.entries.iter().filter(|entry| entry.kind == EntryKind::Folder);
                folders.clone().find(|entry| entry.path == *path).cloned()
            };
            let about = asked.id.as_ref().and_then(about).or_else(|| asked.folder.as_ref().and_then(within));
            let shares = may_share(listing.space.role);
            let ask = match (asked.ask.as_deref(), about) {
                // Whoever may not share is asked nothing, as with a document that is gone.
                (Some("share"), Some(entry)) => {
                    let shares = entry.id.as_ref().and_then(|id| app.shares(&who, id).ok());
                    shares.map(|shares| {
                        // A link that opens nothing any more is the access log's to remember.
                        let live = |link: &&Link| link.revoked.is_none() && link.expires.is_none_or(|ends| now < ends);
                        let linked = |link: &Link| Linked {
                            id: link.id.clone(),
                            label: link.label.clone(),
                            role: link.role,
                            passphrase: link.passphrase,
                            ends: link.expires.map(|ends| day(ends, now)),
                        };
                        let links = shares.links.iter().filter(live).map(linked).collect();
                        // Whoever may share reads the log. One that cannot be read now lists nothing.
                        let id = entry.id.as_deref().unwrap_or_default();
                        let rows = app.access(&who, id, asked.before, None).unwrap_or_default();
                        let here = paths::folder_page(&listing.space.id, &listing.path);
                        let before = |time: u64| format!("{}&before={time}", asking(&here, "share", id));
                        let log = activity::listed(&rows, now, asked.before.is_some(), before);
                        Ask::Share(entry, shares.people, app.directory(&who).unwrap_or_default(), links, log)
                    })
                }
                // Whoever reads a document takes it away in any format. One that cannot be read now is
                // asked nothing of.
                (Some("export"), Some(entry)) if entry.kind == EntryKind::Document => {
                    let offers = entry.id.as_ref().and_then(|id| {
                        let found = app.locate(&who, id).ok()?;
                        app.offers(&who, &found, id).ok()
                    });
                    offers.map(|offers| Ask::Export(entry, offers))
                }
                (Some("rename"), Some(entry)) if edits => Some(Ask::Rename(entry)),
                // A folder is thrown away by an owner of its space alone.
                (Some("trash"), Some(entry)) if edits && (entry.kind != EntryKind::Folder || owns) => {
                    Some(Ask::Trash(entry))
                }
                (Some("move"), Some(entry)) if edits => {
                    let mut targets = targets(&app, &who, (&listing.space.id, &listing.path));
                    if entry.kind == EntryKind::Folder {
                        // A folder goes into no folder of its own.
                        let (own, inside) =
                            (format!("{}/{}", listing.space.id, entry.path), |own: &str| format!("{own}/"));
                        targets.retain(|target| target.value != own && !target.value.starts_with(&inside(&own)));
                    }
                    Some(Ask::Move(entry, targets))
                }
                _ => None,
            };
            let at = At::Space(listing.space.id.clone());
            let title = listing.path.rsplit('/').next().filter(|name| !name.is_empty());
            let title = title.unwrap_or(&listing.space.name).to_owned();
            let rows = listing.entries.into_iter().map(|entry| row(entry, None, now)).collect();
            let body = Body::Folder(Box::new(Folder { space: listing.space, path: listing.path, rows, shares, ask }));
            let mut page = page(&app, Some(&who), at, body);
            page.refused = asked.error.as_deref().map(refusal);
            Ok((title, page))
        }
    });
    match shown.await {
        Ok((title, page)) => send(StatusCode::OK, &title, page, None).await,
        Err(refusal) => refused(&app, Some(&who), &refusal).await,
    }
}

/// A folder as a person reads where it is: its space, and the folders on the way down.
fn trail(space: &str, path: &str) -> String {
    let names = path.split('/').filter(|name| !name.is_empty());
    std::iter::once(space).chain(names).collect::<Vec<_>>().join(" / ")
}

/// Every folder the person may put a document in but the one at `here`, each space from its top down.
fn targets(app: &App, who: &Signed, here: (&str, &str)) -> Vec<Target> {
    let mut out = Vec::new();
    for (space, root) in app.reach(who).into_iter().filter(|(space, _)| space.role.edits()) {
        let mut folders = vec![Place::default()];
        while let Some(place) = folders.pop().filter(|_| out.len() < MOST_TARGETS) {
            if (space.id.as_str(), place.as_str()) != here {
                let value = format!("{}/{}", space.id, place.as_str());
                out.push(Target { value, label: trail(&space.name, place.as_str()) });
            }
            // A folder that cannot be read now holds none to offer.
            let Ok(listed) = root.folder(&place).and_then(|folder| folder.list()) else { continue };
            // The last is pushed first, so that they come off by name.
            let inside = listed.iter().rev().filter(|listed| listed.folder);
            folders.extend(inside.filter_map(|listed| place.child(&listed.name).ok()));
        }
    }
    out
}

async fn recent(State(app): State<Shared>, Visitor(who): Visitor, Query(asked): Query<Asked>) -> Response {
    let shown = blocking({
        let (app, who) = (app.clone(), who.clone());
        move || {
            let (entries, now) = (app.recent(&who)?, app.now());
            let spaces = app.spaces(&who);
            // What a share led to is in no space of theirs.
            let named = |entry: &Entry| {
                let space = spaces.iter().find(|(space, _)| Some(space.id()) == entry.space);
                Some(space.map_or_else(|| "Shared with me".to_owned(), |(space, _)| space.name.clone()))
            };
            let rows = entries.into_iter().map(|entry| row(entry.clone(), named(&entry), now)).collect();
            Ok(page(&app, Some(&who), At::Recent, Body::Recent(rows)))
        }
    });
    listed(&app, &who, "Recent", shown.await, asked).await
}

async fn shared(State(app): State<Shared>, Visitor(who): Visitor, Query(asked): Query<Asked>) -> Response {
    let shown = blocking({
        let (app, who) = (app.clone(), who.clone());
        move || {
            let (lent, now) = (app.shared(&who)?, app.now());
            let rows = lent.into_iter().map(|(entry, whose)| row(entry, Some(whose), now)).collect();
            Ok(page(&app, Some(&who), At::Shared, Body::Shared(rows)))
        }
    });
    listed(&app, &who, "Shared with me", shown.await, asked).await
}

async fn trash(State(app): State<Shared>, Visitor(who): Visitor, Query(asked): Query<Asked>) -> Response {
    let shown = blocking({
        let (app, who) = (app.clone(), who.clone());
        move || {
            let now = app.now();
            let spaces: Vec<_> = app.spaces(&who).into_iter().map(|(space, role)| space.told(role)).collect();
            let row = |thrown: letters_api::Trashed| {
                let space = spaces.iter().find(|space| space.id == thrown.space)?.clone();
                Some(Trashed {
                    folder: thrown.kind == EntryKind::Folder,
                    from: trail(&space.name, &thrown.folder),
                    name: thrown.name,
                    file: thrown.file,
                    thrown: when(thrown.thrown, now),
                    space,
                })
            };
            let trashed = app.trashed(&who).into_iter().filter_map(row).collect();
            Ok(page(&app, Some(&who), At::Trash, Body::Trash(trashed)))
        }
    });
    listed(&app, &who, "Trash", shown.await, asked).await
}

/// Sends a page that lists documents, with what a form on it was refused with.
async fn listed(app: &App, who: &Signed, title: &str, shown: Answer<Page>, asked: Asked) -> Response {
    match shown {
        Ok(mut page) => {
            page.refused = asked.error.as_deref().map(refusal);
            send(StatusCode::OK, title, page, None).await
        }
        Err(refusal) => refused(app, Some(who), &refusal).await,
    }
}

/// A row of a file table, with its times as the table says them.
fn row(entry: Entry, space: Option<String>, now: u64) -> Row {
    let made = entry.created.as_deref().and_then(|made| made.parse::<Timestamp>().ok());
    let created = made.map(|made| when(made.as_second().max(0) as u64, now)).unwrap_or_default();
    let modified = when(entry.modified, now);
    Row { entry, created, modified, space }
}

/// When something happened, as a file table says it: how long ago while that is short, and then the day,
/// in the server's time zone.
pub fn when(at: u64, now: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    let ago = now.saturating_sub(at);
    let count = |n: u64, one: &str| if n == 1 { format!("1 {one} ago") } else { format!("{n} {one}s ago") };
    if ago < MINUTE {
        return "Just now".into();
    } else if ago < HOUR {
        return count(ago / MINUTE, "minute");
    } else if ago < DAY {
        return count(ago / HOUR, "hour");
    } else if ago < 7 * DAY {
        return count(ago / DAY, "day");
    }
    day(at, now)
}

/// The day of `at` in the server's time zone, with its year when that is not this one.
pub fn day(at: u64, now: u64) -> String {
    let zone = TimeZone::system();
    let day = |seconds: u64| Timestamp::from_second(seconds as i64).ok().map(|time| time.to_zoned(zone.clone()));
    match (day(at), day(now)) {
        (Some(at), Some(now)) if at.year() == now.year() => at.strftime("%-d %b").to_string(),
        (Some(at), _) => at.strftime("%-d %b %Y").to_string(),
        (None, _) => String::new(),
    }
}
