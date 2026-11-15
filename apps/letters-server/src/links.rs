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
//!
//! Whoever follows a link is let in for a while by a cookie of that link, after the passphrase when the
//! link asks one. From then on they ask about the document as a [`Reader`](crate::session::Reader), and
//! reach nothing else. Following links is counted by address, so that nobody guesses at them fast.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use argon2::Argon2;
use axum::extract::{ConnectInfo, FromRequestParts, Path, State};
use axum::http::header::USER_AGENT;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, post};
use axum::{Json, Router};
use letters_api::{Action, Code, CreatedLink, Link, NewLink, Role, paths};
use serde::{Deserialize, Serialize};

use crate::access::Reach;
use crate::api::blocking;
use crate::error::{Answer, Refusal};
use crate::ids::{self, hashed, secret};
use crate::session::{Linked, Signed, cookies};
use crate::shares::BETWEEN;
use crate::spaces::{Place, Root};
use crate::store::Table;
use crate::telemetry::Event;
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
    /// How many links the document had before it, which orders two that were made in the same second.
    #[serde(default)]
    nth: u32,
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

/// A browser a link let in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Visit {
    leads: Leads,
    /// When the link is to be followed again.
    ends: u64,
    /// Whether whoever sits at the browser took the document out of safe mode. A link is passed on, so
    /// one holder of it does not decide for the next.
    #[serde(default)]
    trusted: bool,
}

/// By the hash of the secret in the browser's cookie.
const VISITS: Table<Visit> = Table::new("link_visits");

/// How long a link's label and its passphrase may be.
const LONGEST: usize = 200;
/// How many links lead to one document at once. Each is someone to remember having let in.
const MOST: usize = 50;
/// What the cookie of a visit is called, before the link's id.
pub const COOKIE: &str = "letters_link_";
/// How long a browser stays let in before its link is followed again.
const STAYS: u64 = 12 * 60 * 60;
/// How many link cookies of one request are looked at.
const MOST_COOKIES: usize = 8;
/// How often the visits that are over are cleared away.
const SWEEP: u64 = 60 * 60;
/// What one address may spend on following links each minute: a look at a link costs 1 and a try at a
/// passphrase [`TRY`], which makes five tries a minute. A visit may ask the server for as many pictures
/// and previews a minute, so that one link does not take up the fetches everyone shares.
const PER_MINUTE: u32 = 30;
const TRY: u32 = 6;
/// How many addresses are counted before the ones of minutes past are forgotten.
const COUNTED: usize = 10_000;

fn under(id: &str, root: &str) -> String {
    format!("{id}{BETWEEN}{root}{BETWEEN}")
}

impl Kept {
    /// Whether the link opens the document at `now`.
    pub fn live(&self, now: u64) -> bool {
        self.link.revoked.is_none() && self.link.expires.is_none_or(|expires| now < expires)
    }

    /// Whether `tried` is the passphrase the link asks. One that asks none takes anything.
    fn admits(&self, tried: &str) -> bool {
        self.asks.as_deref().is_none_or(|kept| passphrase_is(kept, tried))
    }
}

/// What following links has cost each address this minute, and when the visits were last cleared.
#[derive(Default)]
pub struct Doors {
    /// By address: the minute that is counted, and what was spent in it.
    spent: Mutex<HashMap<String, (u64, u32)>>,
    swept: AtomicU64,
}

impl Doors {
    /// Spends `cost` of what `address` may in this minute. False once it has no more.
    fn spend(&self, address: &str, cost: u32, now: u64) -> bool {
        let mut spent = self.spent.lock().unwrap_or_else(PoisonError::into_inner);
        let minute = now / 60;
        if spent.len() >= COUNTED {
            spent.retain(|_, (counted, _)| *counted == minute);
        }
        let entry = spent.entry(address.to_owned()).or_insert((minute, 0));
        if entry.0 != minute {
            *entry = (minute, 0);
        }
        entry.1 = entry.1.saturating_add(cost);
        entry.1 <= PER_MINUTE
    }
}

/// Where a request came from, as far as the server can tell.
#[derive(Clone, Debug, Default)]
pub struct Caller {
    /// The address of the browser. Empty when the server was not told one.
    pub address: String,
    /// The browser or the app, as it names itself.
    pub browser: String,
}

impl Caller {
    /// Where the request in `parts` came from.
    pub fn of(parts: &Parts, app: &App) -> Self {
        let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|connected| connected.0.ip().to_canonical());
        // A proxy the settings name, as `tailscale serve` on this machine is unless they say otherwise,
        // says last who it asks for. Anything else that says so is not believed.
        let said = parts.headers.get("x-forwarded-for").and_then(|value| value.to_str().ok());
        let forwarded = said.and_then(|said| said.rsplit(',').next()).map(str::trim).filter(|said| !said.is_empty());
        let address = match (peer, forwarded) {
            (Some(peer), Some(forwarded)) if app.config.server.proxies.contains(&peer) => {
                forwarded.chars().take(64).collect()
            }
            (Some(peer), _) => peer.to_string(),
            (None, _) => String::new(),
        };
        let named = parts.headers.get(USER_AGENT).and_then(|value| value.to_str().ok()).unwrap_or_default();
        Self { address, browser: named.chars().take(LONGEST).collect() }
    }
}

impl FromRequestParts<Shared> for Caller {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, app: &Shared) -> Result<Self, Infallible> {
        Ok(Self::of(parts, app))
    }
}

/// What became of following a link.
#[derive(Debug)]
pub enum Door {
    /// The browser is let in to the document, by a cookie of this name and value for this long.
    Open { document: String, cookie: String, secret: String, life: u64 },
    /// The link asks a passphrase. `wrong` when one was tried and is not it.
    Asks { wrong: bool },
}

/// A link's reader, as a request's cookie names them.
pub(crate) struct Guest {
    pub linked: Linked,
    /// The secret in the cookie, and what the visit goes by among the open sockets.
    pub secret: String,
    pub visit: String,
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

fn passphrase_is(kept: &str, tried: &str) -> bool {
    let Some((salt, hash)) = kept.split_once('$') else { return false };
    let unhex = |text: &str| -> Option<Vec<u8>> {
        let digit = |at: usize| u8::from_str_radix(text.get(at..at + 2)?, 16).ok();
        (0..text.len() / 2).map(|pair| digit(2 * pair)).collect()
    };
    let (Some(salt), Some(hash)) = (unhex(salt), unhex(hash)) else { return false };
    // Every byte is looked at, so that how long this takes says nothing of where the two differ.
    let differs =
        stretched(tried, &salt).iter().zip(&hash).fold(hash.len() ^ 32, |seen, (a, b)| seen | usize::from(a ^ b));
    differs == 0
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
    app.telemetry.record(Event::LinkMade);
    blocking(move || app.make_link(&who, &id, &new).map(Json)).await
}

async fn revoke(
    State(app): State<Shared>,
    who: Signed,
    Path((id, link)): Path<(String, String)>,
) -> Answer<StatusCode> {
    app.telemetry.record(Event::LinkRevoked);
    blocking(move || app.revoke_link(&who, &id, &link)).await?;
    Ok(StatusCode::NO_CONTENT)
}

impl App {
    /// The links of the file of the document `id` in `root`, in the order they were made.
    pub(crate) fn links_of(&self, id: &str, root: &Root) -> Answer<Vec<Kept>> {
        let mut links: Vec<Kept> =
            self.store.under(&LINKS, &under(id, &root.key))?.into_iter().map(|(_, kept)| kept).collect();
        let order = |kept: &Kept| (kept.link.created, kept.nth, kept.link.id.clone());
        links.sort_by_key(order);
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
        let before = self.links_of(id, &found.root)?;
        if before.iter().filter(|kept| kept.live(now)).count() >= MOST {
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
        let (asks, nth) = (new.passphrase.as_deref().map(kept_passphrase), before.len() as u32);
        let kept = Kept { link: link.clone(), token: hashed(&token), asks, nth };
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

    /// The link the secret in an address leads to, while it works. Whatever else an address holds leads
    /// nowhere, and is told no more than that. The document's log is told of a link that has run out.
    fn follow(&self, token: &str, caller: &Caller) -> Answer<(Leads, Kept)> {
        let nowhere = || Refusal::new(Code::NotFound, "This link does not work, or no longer does.");
        // A server whose owner turned links off again follows none.
        let leads = self.store.get(&TOKENS, &hashed(token))?.filter(|_| self.config.links.enabled);
        let leads = leads.ok_or_else(nowhere)?;
        let key = format!("{}{}", under(&leads.document, &leads.root), leads.link);
        let kept = self.store.get(&LINKS, &key)?.ok_or_else(nowhere)?;
        if !kept.live(self.now()) {
            self.turned_away(&leads, &kept, caller);
            return Err(nowhere());
        }
        Ok((leads, kept))
    }

    /// Notes in the document's log that its link opened nothing for whoever tried it.
    fn turned_away(&self, leads: &Leads, kept: &Kept, caller: &Caller) {
        let reach = Reach::of_link(&leads.document, &leads.root, &kept.link, caller);
        self.noted(&reach, Action::Open, true);
    }

    /// Follows the link in an address for the browser `caller` tells of, with the passphrase it tried
    /// when it tried one.
    pub fn enter(&self, token: &str, tried: Option<&str>, caller: &Caller) -> Answer<Door> {
        let now = self.now();
        if !self.doors.spend(&caller.address, if tried.is_some() { TRY } else { 1 }, now) {
            return Err(Refusal::new(Code::Slow, "Too many tries from here. Wait a minute, and try again."));
        }
        let (leads, kept) = self.follow(token, caller)?;
        match tried {
            None if kept.asks.is_some() => return Ok(Door::Asks { wrong: false }),
            Some(tried) if !kept.admits(tried) => {
                self.turned_away(&leads, &kept, caller);
                return Ok(Door::Asks { wrong: true });
            }
            _ => {}
        }
        let secret = secret();
        let ends = kept.link.expires.map_or(now + STAYS, |expires| expires.min(now + STAYS));
        self.store.put(&VISITS, &hashed(&secret), &Visit { leads: leads.clone(), ends, trusted: false })?;
        self.sweep(now);
        let cookie = format!("{COOKIE}{}", leads.link);
        Ok(Door::Open { document: leads.document, cookie, secret, life: ends - now })
    }

    /// Clears away the visits that are over, now and then.
    fn sweep(&self, now: u64) {
        let last = self.doors.swept.load(Ordering::Relaxed);
        let due = now >= last.saturating_add(SWEEP);
        if !due || self.doors.swept.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_err() {
            return;
        }
        let over = |(key, visit): (String, Visit)| (visit.ends <= now).then_some(key);
        let over: Vec<String> =
            self.store.under(&VISITS, "").unwrap_or_default().into_iter().filter_map(over).collect();
        // What is left behind is cleared the next time.
        let _ = self.store.write(|batch| over.iter().try_for_each(|key| batch.remove(&VISITS, key)));
    }

    /// The link that let the browser of a request in to the document `id`, while the visit and the link
    /// both last.
    pub(crate) fn let_in(&self, headers: &HeaderMap, id: &str) -> Option<Guest> {
        let now = self.now();
        let named = cookies(headers).filter(|(name, _)| name.starts_with(COOKIE)).take(MOST_COOKIES);
        for (name, secret) in named.filter(|_| self.config.links.enabled) {
            let visit = hashed(secret);
            let Ok(Some(Visit { leads, ends, trusted })) = self.store.get(&VISITS, &visit) else { continue };
            if leads.document != id || ends <= now || name.strip_prefix(COOKIE) != Some(leads.link.as_str()) {
                continue;
            }
            let key = format!("{}{}", under(&leads.document, &leads.root), leads.link);
            let Ok(Some(kept)) = self.store.get(&LINKS, &key) else { continue };
            if kept.live(now) {
                let (document, root) = (leads.document, leads.root);
                let linked = Linked { link: kept.link, document, root, alone: false, ends, trusted };
                return Some(Guest { linked, secret: secret.to_owned(), visit });
            }
        }
        None
    }

    /// Whoever sits at the browser of the visit `visit` trusts its document, until the visit is over.
    pub(crate) fn trust_visit(&self, visit: &str) -> Answer<()> {
        Ok(self.store.write(|batch| match batch.get(&VISITS, visit)? {
            Some(seen) => batch.put(&VISITS, visit, &Visit { trusted: true, ..seen }),
            None => Ok(()),
        })?)
    }

    /// Counts a picture or a preview the server is asked to fetch. Whoever only a link let in may ask for
    /// so many a minute in one visit.
    pub(crate) fn fetching(&self, who: &Signed) -> Answer<()> {
        if who.alone() && !self.doors.spend(&format!("visit {}", who.session), 1, self.now()) {
            return Err(Refusal::new(Code::Slow, "Too many pictures and previews at once. Wait a minute."));
        }
        Ok(())
    }

    /// Where the file of a link's document is, while it is in the folder the link was made in.
    pub(crate) fn led(&self, linked: &Linked) -> Answer<Option<(Root, Place)>> {
        let folders = self.folders();
        let Some(rooted) = folders.iter().find(|rooted| rooted.root.key == linked.root) else { return Ok(None) };
        let found = self.library.find(&self.store, &[&rooted.root], &linked.document, self.now())?;
        found.map(|(_, place)| Ok((rooted.root.again()?, place))).transpose()
    }
}
