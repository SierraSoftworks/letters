//! Where a person read a document up to, kept by their account so that it follows them across devices.
//!
//! A shell says it as it closes a document and as it saves, and is told it when the document next opens,
//! to show what others wrote since. It is the model's revision as text, which the server keeps and does
//! not read. A link's own visitor has no account to keep it by. It is lost with the state folder, and
//! then a document opens as on a first visit.
//!
//! Nothing drops a row yet: the server deletes no document for good, and the trash keeps what it holds.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::put;
use axum::{Json, Router};
use letters_api::Seen;

use crate::api::blocking;
use crate::error::{Answer, Refusal};
use crate::session::{Reader, Signed};
use crate::shares::BETWEEN;
use crate::store::Table;
use crate::{App, Shared};

/// By the person's id and the document's.
const SEEN: Table<String> = Table::new("seen");
/// The most a revision may weigh. A model's is a few hundred bytes.
const MOST: usize = 4096;

fn key(who: &Signed, id: &str) -> String {
    format!("{}{BETWEEN}{id}", who.person.id)
}

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/v1/documents/{id}/seen", put(set))
}

async fn set(
    State(app): State<Shared>,
    Reader(who): Reader,
    Path(id): Path<String>,
    Json(seen): Json<Seen>,
) -> Answer<StatusCode> {
    blocking(move || app.set_seen(&who, &id, &seen.revision)).await?;
    Ok(StatusCode::NO_CONTENT)
}

impl App {
    /// Where `who` last read the document `id` up to, if they ever said.
    pub fn last_seen(&self, who: &Signed, id: &str) -> Answer<Option<String>> {
        match who.alone() {
            true => Ok(None),
            false => Ok(self.store.get(&SEEN, &key(who, id))?),
        }
    }

    /// Keeps that `who` read the document `id` up to `revision`. They must reach the document. What a
    /// link's own visitor says is taken and not kept, so that a shell need not tell the two apart.
    pub fn set_seen(&self, who: &Signed, id: &str, revision: &str) -> Answer<()> {
        self.locate(who, id)?;
        if revision.is_empty() || revision.len() > MOST {
            return Err(Refusal::invalid("That is not a revision of a document."));
        }
        if !who.alone() {
            self.store.put(&SEEN, &key(who, id), &revision.to_owned())?;
        }
        Ok(())
    }
}
