//! What the forms of the pages do. Each comes from a browser with a session, from one of the server's own
//! pages, and is answered with the page to show next: the one it came from, saying what was refused when
//! something was.

use axum::Router;
use axum::extract::{Form, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::post;
use letters_api::{Move, NewDocument, NewFolder, paths};
use serde::Deserialize;

use super::{ACT_MOVE, ACT_NEW_DOCUMENT, ACT_NEW_FOLDER, ACT_PUT_BACK, ACT_RENAME, ACT_TRASH, TRASH_PAGE};
use super::{Visitor, refusing};
use crate::api::blocking;
use crate::auth::local;
use crate::error::{Answer, Refusal};
use crate::{App, Shared};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route(ACT_NEW_DOCUMENT, post(new_document))
        .route(ACT_NEW_FOLDER, post(new_folder))
        .route(ACT_RENAME, post(rename))
        .route(ACT_MOVE, post(relocate))
        .route(ACT_TRASH, post(trash))
        .route(ACT_PUT_BACK, post(put_back))
}

/// Does `work`, which says the page to show next, and shows `back` with the refusal when it is refused.
async fn done(app: Shared, back: String, work: impl FnOnce(&App) -> Answer<String> + Send + 'static) -> Response {
    match blocking(move || work(&app)).await {
        Ok(next) => Redirect::to(&next).into_response(),
        Err(refusal) => Redirect::to(&refusing(&back, &refusal)).into_response(),
    }
}

/// A folder, as a form names it.
#[derive(Deserialize)]
struct Within {
    #[serde(default)]
    space: String,
    #[serde(default)]
    folder: String,
    /// The name of what to make there.
    #[serde(default)]
    name: String,
}

impl Within {
    fn page(&self) -> String {
        paths::folder_page(&self.space, &self.folder)
    }
}

/// Starts a document and opens it.
async fn new_document(State(app): State<Shared>, Visitor(who): Visitor, Form(within): Form<Within>) -> Response {
    done(app, within.page(), move |app| {
        let new = NewDocument { folder: within.folder, name: None };
        let entry = app.new_document(&who, &within.space, &new)?;
        Ok(paths::editor_page(&entry.id.unwrap_or_default()))
    })
    .await
}

async fn new_folder(State(app): State<Shared>, Visitor(who): Visitor, Form(within): Form<Within>) -> Response {
    done(app, within.page(), move |app| {
        let back = within.page();
        app.new_folder(&who, &within.space, &NewFolder { parent: within.folder, name: within.name })?;
        Ok(back)
    })
    .await
}

/// A document, as the form of a question about it names it.
#[derive(Deserialize)]
struct About {
    #[serde(default)]
    id: String,
    /// The page the question was asked on.
    #[serde(default)]
    back: String,
    #[serde(default)]
    name: String,
    /// Where to move it: a space's id, a `/`, and the folder's place in the space.
    #[serde(default)]
    to: String,
}

impl About {
    /// The page to show next, which only ever is one of this server's.
    fn back(&self) -> String {
        local(Some(&self.back))
    }
}

async fn rename(State(app): State<Shared>, Visitor(who): Visitor, Form(about): Form<About>) -> Response {
    done(app, about.back(), move |app| app.rename(&who, &about.id, &about.name).map(|_| about.back())).await
}

async fn relocate(State(app): State<Shared>, Visitor(who): Visitor, Form(about): Form<About>) -> Response {
    done(app, about.back(), move |app| {
        let (space, folder) = about.to.split_once('/').unwrap_or((&about.to, ""));
        if space.is_empty() {
            return Err(Refusal::invalid("Choose the folder to move it to."));
        }
        app.relocate(&who, &about.id, &Move { space: space.to_owned(), folder: folder.to_owned() })?;
        Ok(about.back())
    })
    .await
}

async fn trash(State(app): State<Shared>, Visitor(who): Visitor, Form(about): Form<About>) -> Response {
    done(app, about.back(), move |app| app.trash(&who, &about.id).map(|()| about.back())).await
}

/// A file in a space's trash.
#[derive(Deserialize)]
struct Thrown {
    #[serde(default)]
    space: String,
    #[serde(default)]
    file: String,
}

async fn put_back(State(app): State<Shared>, Visitor(who): Visitor, Form(thrown): Form<Thrown>) -> Response {
    let work = move |app: &App| app.put_back(&who, &thrown.space, &thrown.file).map(|_| TRASH_PAGE.to_owned());
    done(app, TRASH_PAGE.to_owned(), work).await
}
