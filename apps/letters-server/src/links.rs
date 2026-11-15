//! Links to a document, for whoever is handed one.
//!
//! A link is made by whoever says who the document is shared with, on a server whose owner lets links be
//! made. It is a secret in an address: 256 random bits, of which only a hash is kept, so that reading the
//! store opens nothing. What it lets its reader do, when it ends, the passphrase it asks and what its
//! reader may take away are fixed when it is made. A link is revoked, never changed, and a revoked link
//! stays listed so that the access log can still name it.
//!
//! Like a share, a link is of the file where it is: it is kept under the document's id and the folder its
//! space is in.

use argon2::Argon2;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, post};
use axum::{Json, Router};
use letters_api::{CreatedLink, Link, NewLink, Role, paths};
use serde::{Deserialize, Serialize};

use crate::api::blocking;
use crate::error::{Answer, Refusal};
use crate::ids::{self, hashed, secret};
use crate::session::Signed;
use crate::shares::BETWEEN;
use crate::spaces::Root;
use crate::store::Table;
use crate::{App, Shared};

/// What is kept of a link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Kept {
    #[serde(flatten)]
    pub link: Link,
    /// The hash of its secret, which is its row among the [`TOKENS`].
    token: String,
    /// What is kept of the passphrase it asks.
    #[serde(default)]
    asks: Option<String>,
}

/// Where a link's secret leads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Leads {
    document: String,
    /// The key of the root the document's file is in.
    root: String,
    link: String,
}

/// By the document's id, the key of the root its file is in, and the link's id, with [`BETWEEN`] between.
const LINKS: Table<Kept> = Table::new("links");
/// By the hash of the link's secret.
const TOKENS: Table<Leads> = Table::new("link_tokens");

/// How long a link's label and its passphrase may be.
const LONGEST: usize = 200;
/// How many links lead to one document at once. Each is someone to remember having let in.
const MOST: usize = 50;

fn under(id: &str, root: &str) -> String {
    format!("{id}{BETWEEN}{root}{BETWEEN}")
}

impl Kept {
    /// Whether the link opens the document at `now`.
    pub fn live(&self, now: u64) -> bool {
        self.link.revoked.is_none() && self.link.expires.is_none_or(|expires| now < expires)
    }
}

/// What is kept of a passphrase: its salt and what Argon2 makes of the two, so that whoever reads the
/// store has to guess each passphrase slowly and by itself.
fn kept_passphrase(passphrase: &str) -> String {
    let salt = ids::random::<16>();
    format!("{}${}", ids::hex(&salt), ids::hex(&stretched(passphrase, &salt)))
}

fn stretched(passphrase: &str, salt: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    // The sizes are fixed, so Argon2 has nothing to refuse.
    Argon2::default().hash_password_into(passphrase.as_bytes(), salt, &mut out).expect("a passphrase is hashed");
    out
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/v1/documents/{id}/links", post(make))
        .route("/api/v1/documents/{id}/links/{link}", delete(revoke))
}

async fn make(
    State(app): State<Shared>,
    who: Signed,
    Path(id): Path<String>,
    Json(new): Json<NewLink>,
) -> Answer<Json<CreatedLink>> {
    blocking(move || app.make_link(&who, &id, &new).map(Json)).await
}

async fn revoke(
    State(app): State<Shared>,
    who: Signed,
    Path((id, link)): Path<(String, String)>,
) -> Answer<StatusCode> {
    blocking(move || app.revoke_link(&who, &id, &link)).await?;
    Ok(StatusCode::NO_CONTENT)
}

impl App {
    /// The links of the file of the document `id` in `root`, in the order they were made.
    pub(crate) fn links_of(&self, id: &str, root: &Root) -> Answer<Vec<Kept>> {
        let mut links: Vec<Kept> =
            self.store.under(&LINKS, &under(id, &root.key))?.into_iter().map(|(_, kept)| kept).collect();
        links.sort_by(|one, other| (one.link.created, &one.link.id).cmp(&(other.link.created, &other.link.id)));
        Ok(links)
    }

    /// Makes a link to the document `id`. Its address is told here and never again.
    pub fn make_link(&self, who: &Signed, id: &str, new: &NewLink) -> Answer<CreatedLink> {
        let found = self.sharing(who, id)?;
        if !self.config.links.enabled {
            return Err(Refusal::forbidden("This server makes no links: whoever runs it has not turned them on."));
        }
        let (label, now) = (new.label.trim(), self.now());
        if label.is_empty() || label.len() > LONGEST {
            return Err(Refusal::invalid("A link has a label that says who it is for."));
        }
        if new.role == Role::Owner {
            return Err(Refusal::invalid("A link lets its reader read, comment or edit."));
        }
        if new.expires.is_some_and(|expires| expires <= now) {
            return Err(Refusal::invalid("A link ends at a time yet to come."));
        }
        if new.passphrase.as_ref().is_some_and(|passphrase| passphrase.is_empty() || passphrase.len() > LONGEST) {
            return Err(Refusal::invalid("A passphrase is some text, and not a long one."));
        }
        if self.links_of(id, &found.root)?.iter().filter(|kept| kept.live(now)).count() >= MOST {
            return Err(Refusal::invalid("Too many links lead to this document. Revoke one first."));
        }
        let token = secret();
        let link = Link {
            id: ids::hex(&ids::random::<16>()),
            label: label.to_owned(),
            role: new.role,
            expires: new.expires,
            passphrase: new.passphrase.is_some(),
            allows: new.allows,
            created: now,
            revoked: None,
        };
        let kept =
            Kept { link: link.clone(), token: hashed(&token), asks: new.passphrase.as_deref().map(kept_passphrase) };
        let leads = Leads { document: id.to_owned(), root: found.root.key.clone(), link: link.id.clone() };
        self.store.write(|batch| {
            batch.put(&LINKS, &format!("{}{}", under(id, &found.root.key), link.id), &kept)?;
            batch.put(&TOKENS, &kept.token, &leads)
        })?;
        let url = format!("{}{}", self.config.server.url.trim_end_matches('/'), paths::link_page(&token));
        Ok(CreatedLink { link, url })
    }

    /// Revokes a link of the document `id`: it opens nothing from now on, and whoever it led in is let go.
    pub fn revoke_link(&self, who: &Signed, id: &str, link: &str) -> Answer<()> {
        let found = self.sharing(who, id)?;
        let (key, now) = (format!("{}{link}", under(id, &found.root.key)), self.now());
        self.store
            .write(|batch| {
                let Some(mut kept) = batch.get(&LINKS, &key)? else { return Ok(None) };
                batch.remove(&TOKENS, &kept.token)?;
                // Revoked once: when it was is not said anew.
                kept.link.revoked.get_or_insert(now);
                batch.put(&LINKS, &key, &kept).map(Some)
            })?
            .ok_or_else(Refusal::not_found)?;
        self.rooms.reseat(&found.root, &found.place);
        Ok(())
    }
}
