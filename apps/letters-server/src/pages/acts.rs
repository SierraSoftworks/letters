//! What the forms of the pages do. Each comes from a browser with a session, from one of the server's own
//! pages, and is answered with the page to show next: the one it came from, saying what was refused when
//! something was.

use axum::Router;
use axum::extract::{DefaultBodyLimit, Form, Multipart, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::post;
use letters_api::{Code, Grant, Grantee, Move, NewDocument, NewFolder, Role, paths};
use serde::Deserialize;

use super::{ACT_IMPORT, ACT_IMPORT_FILE, ACT_REVOKE, ACT_SHARE, ACT_UNSHARE, Visitor, asking, refusing, unmerged};
use super::{ACT_MERGE, ACT_MOVE, ACT_NEW_DOCUMENT, ACT_NEW_FOLDER, ACT_PUT_BACK, ACT_RENAME, ACT_TRASH, TRASH_PAGE};
use crate::api::blocking;
use crate::auth::local;
use crate::error::{Answer, Refusal};
use crate::session::Signed;
use crate::shares::same;
use crate::{App, Shared};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route(ACT_NEW_DOCUMENT, post(new_document))
        .route(ACT_NEW_FOLDER, post(new_folder))
        .route(
            ACT_IMPORT,
            post(import).layer(DefaultBodyLimit::max(FORM + letters_api::Limits::default().document as usize)),
        )
        .route(ACT_RENAME, post(rename))
        .route(ACT_MOVE, post(relocate))
        .route(ACT_TRASH, post(trash))
        .route(ACT_IMPORT_FILE, post(import_file))
        .route(ACT_MERGE, post(merge_conflicts))
        .route(ACT_PUT_BACK, post(put_back))
        .route(ACT_SHARE, post(share))
        .route(ACT_UNSHARE, post(unshare))
        .route(ACT_REVOKE, post(revoke))
}

/// What a form that carries a file may weigh beyond the file: its other fields and what parts them.
const FORM: usize = 64 << 10;

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

/// Makes a document of the Markdown or Word file a form carries, and opens it.
async fn import(State(app): State<Shared>, Visitor(who): Visitor, mut form: Multipart) -> Response {
    let (mut space, mut folder, mut file) = (String::new(), String::new(), None);
    // A form that stops short was cut off at what a form may weigh.
    let mut whole = true;
    loop {
        let field = match form.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(_) => {
                whole = false;
                break;
            }
        };
        let named = field.name().unwrap_or_default().to_owned();
        match named.as_str() {
            "space" => space = field.text().await.unwrap_or_default(),
            "folder" => folder = field.text().await.unwrap_or_default(),
            "file" => {
                let name = field.file_name().unwrap_or_default().to_owned();
                match field.bytes().await {
                    Ok(bytes) => file = Some((name, bytes)).filter(|(name, _)| !name.is_empty()),
                    Err(_) => whole = false,
                }
            }
            _ => {}
        }
    }
    let back = paths::folder_page(&space, &folder);
    done(app, back, move |app| {
        if !whole {
            return Err(Refusal::new(Code::TooLarge, "The file is larger than the server takes."));
        }
        let (name, file) = file.ok_or_else(|| Refusal::invalid("Choose a Markdown or Word file to import."))?;
        let entry = app.import(&who, &space, &folder, &name, &file)?;
        Ok(paths::editor_page(&entry.id.unwrap_or_default()))
    })
    .await
}

/// A Markdown or Word file a space keeps, and the page it is listed on.
#[derive(Deserialize)]
struct Kept {
    #[serde(default)]
    space: String,
    #[serde(default)]
    path: String,
}

/// Makes a document of a file the space keeps, beside it, and opens it.
async fn import_file(State(app): State<Shared>, Visitor(who): Visitor, Form(kept): Form<Kept>) -> Response {
    let folder = kept.path.rsplit_once('/').map_or("", |(folder, _)| folder);
    done(app, paths::folder_page(&kept.space, folder), move |app| {
        let entry = app.import_file(&who, &kept.space, &kept.path)?;
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

/// A document, as the form of a question about it names it, or a folder when the form names no document.
#[derive(Deserialize)]
struct About {
    #[serde(default)]
    id: String,
    /// The folder the question is about, and the space it is in.
    #[serde(default)]
    space: String,
    #[serde(default)]
    folder: String,
    /// The page the question was asked on.
    #[serde(default)]
    back: String,
    #[serde(default)]
    name: String,
    /// Where to move it: a space's id, a `/`, and the folder's place in the space. Who to share it with or
    /// to take out of its shares: `p:` and a person's id, or `g:` and a group's name.
    #[serde(default)]
    to: String,
    /// What to share it to do: a role, as it is on the wire.
    #[serde(default)]
    role: String,
    /// The link of it to revoke, by its id.
    #[serde(default)]
    link: String,
}

impl About {
    /// The page to show next, which only ever is one of this server's.
    fn back(&self) -> String {
        local(Some(&self.back))
    }
}

async fn rename(State(app): State<Shared>, Visitor(who): Visitor, Form(about): Form<About>) -> Response {
    done(app, about.back(), move |app| {
        match about.id.is_empty() {
            true => app.rename_folder(&who, &about.space, &about.folder, &about.name)?,
            false => app.rename(&who, &about.id, &about.name)?,
        };
        Ok(about.back())
    })
    .await
}

async fn relocate(State(app): State<Shared>, Visitor(who): Visitor, Form(about): Form<About>) -> Response {
    done(app, about.back(), move |app| {
        let (space, folder) = about.to.split_once('/').unwrap_or((&about.to, ""));
        if space.is_empty() {
            return Err(Refusal::invalid("Choose the folder to move it to."));
        }
        let to = Move { space: space.to_owned(), folder: folder.to_owned() };
        match about.id.is_empty() {
            true => app.move_folder(&who, &about.space, &about.folder, &to)?,
            false => app.relocate(&who, &about.id, &to)?,
        };
        Ok(about.back())
    })
    .await
}

async fn trash(State(app): State<Shared>, Visitor(who): Visitor, Form(about): Form<About>) -> Response {
    done(app, about.back(), move |app| {
        match about.id.is_empty() {
            true => app.trash_folder(&who, &about.space, &about.folder)?,
            false => app.trash(&who, &about.id)?,
        };
        Ok(about.back())
    })
    .await
}

impl About {
    /// Who the form names.
    fn grantee(&self) -> Answer<Grantee> {
        match self.to.split_once(':') {
            Some(("p", id)) => Ok(Grantee::Person { id: id.to_owned(), name: String::new() }),
            Some(("g", name)) => Ok(Grantee::Group { name: name.to_owned() }),
            _ => Err(Refusal::invalid("Choose who to share it with.")),
        }
    }

    /// Changes who the document is shared with besides the one the form names: `then` says what of them.
    /// The Share dialog is shown again, with the shares as they then stand.
    fn sharing(self, who: &Signed, app: &App, then: impl FnOnce(Grantee) -> Answer<Option<Grant>>) -> Answer<String> {
        let to = self.grantee()?;
        let mut people = app.shares(who, &self.id)?.people;
        people.retain(|grant| !same(&grant.to, &to));
        people.extend(then(to)?);
        app.share(who, &self.id, &people)?;
        Ok(asking(&self.back(), "share", &self.id))
    }
}

/// Shares a document with one more, or gives someone it is shared with another role.
async fn share(State(app): State<Shared>, Visitor(who): Visitor, Form(about): Form<About>) -> Response {
    done(app, about.back(), move |app| {
        let role = serde_json::from_value::<Role>(about.role.clone().into());
        let role = role.map_err(|_| Refusal::invalid("A document is shared to read, to comment on or to edit."))?;
        about.sharing(&who, app, |to| Ok(Some(Grant { to, role })))
    })
    .await
}

async fn unshare(State(app): State<Shared>, Visitor(who): Visitor, Form(about): Form<About>) -> Response {
    done(app, about.back(), move |app| about.sharing(&who, app, |_| Ok(None))).await
}

/// Revokes a link of a document. The Share dialog is shown again, without it.
async fn revoke(State(app): State<Shared>, Visitor(who): Visitor, Form(about): Form<About>) -> Response {
    done(app, about.back(), move |app| {
        app.revoke_link(&who, &about.id, &about.link)?;
        Ok(asking(&about.back(), "share", &about.id))
    })
    .await
}

/// Merges the conflict copies beside a document into it. Nothing else a form asks is refused for what a
/// file holds, so this refusal has words of its own.
async fn merge_conflicts(State(app): State<Shared>, Visitor(who): Visitor, Form(about): Form<About>) -> Response {
    let back = about.back();
    let next = match blocking(move || app.merge_conflicts(&who, &about.id)).await {
        Ok(_) => back,
        Err(refusal) if refusal.code() == Code::Invalid => unmerged(&back),
        Err(refusal) => refusing(&back, &refusal),
    };
    Redirect::to(&next).into_response()
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
