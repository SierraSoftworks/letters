//! The access log of a document: who reached it through a share or a link, what they did, and from where.
//!
//! Only what a share or a link let someone do is noted. Whoever a space lets in is there by the say of
//! whoever runs the server, and is not watched. A row is kept for a year. The same thing done again within
//! five minutes, by the same reader from the same address, is one row: nobody fills the log by asking over
//! and over, and an edit session that goes on is one row that says how long it has lasted.
//!
//! Like a share, the log is of the file where it is: it is kept under the document's id and the folder its
//! space is in, and whoever may share the document reads it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use letters_api::{Access, Action, Link, Role, Via, Who};
use serde::Deserialize;

use crate::api::{Found, blocking};
use crate::error::Answer;
use crate::links::Caller;
use crate::session::{Linked, Signed};
use crate::shares::BETWEEN;
use crate::store::Table;
use crate::{App, Shared};

/// By the document's id, the key of the root its file is in, and the time with a few random digits after
/// it, so that a document's rows are read in the order they were written.
const ACCESS: Table<Access> = Table::new("access");

/// How many days a row is kept.
pub const KEPT_DAYS: u32 = 365;
const KEPT: u64 = KEPT_DAYS as u64 * 24 * 3600;
/// How long the same thing by the same reader from the same address counts as one.
const AGAIN: u64 = 5 * 60;
/// How often an edit session that goes on says anew how long it has lasted.
const LASTING: u64 = 60;
/// How often rows that were kept long enough are cleared away.
const SWEEP: u64 = 24 * 3600;
/// How many things noted of late are remembered, to tell one done again.
const COUNTED: usize = 10_000;
/// How many rows are sent when no number is asked for, and the most that ever are.
pub(crate) const LISTED: usize = 200;
const MOST: usize = 1000;

fn under(id: &str, root: &str) -> String {
    format!("{id}{BETWEEN}{root}{BETWEEN}")
}

/// What was noted of late, and when the old rows were last cleared.
#[derive(Default)]
pub struct Log {
    /// By document, reader, action and address: when it was last noted, and the row that says so with the
    /// time that row has.
    noted: Mutex<HashMap<String, (u64, String, u64)>>,
    /// How many rows this run has written, which orders the rows of one second.
    written: AtomicU64,
    swept: AtomicU64,
}

/// What the log says of whoever a share or a link led to a document: all of a row but what they did.
#[derive(Clone, Debug)]
pub(crate) struct Reach {
    document: String,
    /// The key of the root the document's file is in.
    root: String,
    who: Who,
    /// The link someone signed in came by.
    link: Option<Via>,
    role: Role,
    from: Caller,
}

impl Reach {
    /// Whoever tries the link `link` to a document, before it has let them in.
    pub(crate) fn of_link(document: &str, root: &str, link: &Link, from: &Caller) -> Self {
        let who = Who::Link { id: link.id.clone(), label: link.label.clone() };
        let (document, root) = (document.to_owned(), root.to_owned());
        Self { document, root, who, link: None, role: link.role, from: from.clone() }
    }
}

/// What a request for the log may say of the rows it wants.
#[derive(Deserialize)]
struct Paged {
    /// Only rows from before this time.
    before: Option<u64>,
    limit: Option<usize>,
}

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/v1/documents/{id}/access", get(list))
}

async fn list(
    State(app): State<Shared>,
    who: Signed,
    Path(id): Path<String>,
    Query(paged): Query<Paged>,
) -> Answer<Json<Vec<Access>>> {
    blocking(move || app.access(&who, &id, paged.before, paged.limit).map(Json)).await
}

impl App {
    /// How the log knows `who` at the document `id`, when it is a share or a link that lets them do what
    /// `found` says. `None` for whoever the document's space lets do as much.
    pub(crate) fn reached(&self, who: &Signed, found: &Found, id: &str) -> Option<Reach> {
        if found.space.as_ref().is_some_and(|space| space.role >= found.role) {
            return None;
        }
        // A link names whoever it alone let in. Someone signed in is known by name, and when it was a link
        // that led them here the row names the link as well: the log is there to show who used which.
        let came = who.link.as_ref().filter(|_| found.until.is_some());
        let via = |linked: &Linked| Via { id: linked.link.id.clone(), label: linked.link.label.clone() };
        let (named, link) = match came {
            Some(linked) if linked.alone => {
                (Who::Link { id: linked.link.id.clone(), label: linked.link.label.clone() }, None)
            }
            came => (Who::Person { id: who.person.id.clone(), name: who.person.name.clone() }, came.map(via)),
        };
        let (document, root) = (id.to_owned(), found.root.key.clone());
        Some(Reach { document, root, who: named, link, role: found.role, from: who.from.clone() })
    }

    /// Notes that `who` did `action` with the document `id`, or was refused it, if a share or a link is
    /// what let them reach it.
    pub(crate) fn note(&self, who: &Signed, found: &Found, id: &str, action: Action, refused: bool) {
        if let Some(reach) = self.reached(who, found, id) {
            self.noted(&reach, action, refused);
        }
    }

    /// Notes what whoever `reach` describes did. A fault in keeping the log keeps nobody from their
    /// document: it is said in the server's own log.
    pub(crate) fn noted(&self, reach: &Reach, action: Action, refused: bool) {
        let now = self.now();
        let reader = match &reach.who {
            Who::Person { id, .. } | Who::Link { id, .. } => id.as_str(),
        };
        let mark = format!(
            "{}{BETWEEN}{reader}{BETWEEN}{action:?}{BETWEEN}{refused}{BETWEEN}{}",
            under(&reach.document, &reach.root),
            reach.from.address
        );
        let mut noted = self.log.noted.lock().unwrap_or_else(PoisonError::into_inner);
        if noted.len() >= COUNTED {
            noted.retain(|_, (last, _, _)| now < last.saturating_add(AGAIN));
        }
        let written = match noted.get_mut(&mark) {
            Some((last, key, began)) if now < last.saturating_add(AGAIN) => {
                // An edit session that goes on says how long it has lasted, a minute at a time.
                if action != Action::Edit || now < last.saturating_add(LASTING) {
                    return;
                }
                *last = now;
                let (key, lasted) = (key.clone(), now.saturating_sub(*began));
                self.store.write(|batch| match batch.get(&ACCESS, &key)? {
                    Some(row) => batch.put(&ACCESS, &key, &Access { lasted: Some(lasted), ..row }),
                    None => Ok(()),
                })
            }
            _ => {
                let nth = self.log.written.fetch_add(1, Ordering::Relaxed);
                let key = format!("{}{now:012}{nth:012}", under(&reach.document, &reach.root));
                let row = Access {
                    time: now,
                    who: reach.who.clone(),
                    link: reach.link.clone(),
                    action,
                    role: reach.role,
                    refused,
                    lasted: None,
                    address: reach.from.address.clone(),
                    browser: reach.from.browser.clone(),
                };
                let written = self.store.put(&ACCESS, &key, &row);
                noted.insert(mark, (now, key, now));
                written
            }
        };
        drop(noted);
        if let Err(error) = written {
            crate::logs::warn(&format!("the access log could not be written: {error}"));
        }
        self.sweep_log(now);
    }

    /// Clears away the rows that were kept long enough, once a day.
    fn sweep_log(&self, now: u64) {
        let last = self.log.swept.load(Ordering::Relaxed);
        let due = now >= last.saturating_add(SWEEP);
        if !due || self.log.swept.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_err() {
            return;
        }
        // What is left behind is cleared the next time.
        let _ = self.store.write(|batch| batch.prune(&ACCESS, "", |_, row| now < row.time.saturating_add(KEPT)));
    }

    /// The access log of the document `id`, newest first, for whoever may share it: the rows from before
    /// `before`, and `limit` of them, with what else the last one's second holds.
    pub fn access(&self, who: &Signed, id: &str, before: Option<u64>, limit: Option<usize>) -> Answer<Vec<Access>> {
        let found = self.sharing(who, id)?;
        let now = self.now();
        let rows = self.store.under(&ACCESS, &under(id, &found.root.key))?;
        let wanted =
            |row: &Access| now < row.time.saturating_add(KEPT) && before.is_none_or(|before| row.time < before);
        let most = limit.unwrap_or(LISTED).min(MOST);
        let mut page: Vec<Access> = Vec::new();
        for row in rows.into_iter().rev().map(|(_, row)| row).filter(wanted) {
            // A page ends between two seconds, and runs over for that: the next one is asked for by the time
            // of this one's last row, and must miss nothing.
            if page.len() >= most && page.last().is_none_or(|last| last.time != row.time) {
                break;
            }
            page.push(row);
        }
        Ok(page)
    }
}
