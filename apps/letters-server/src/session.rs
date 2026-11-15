//! Who is asking: sessions, and the tokens that stand for them.
//!
//! A session is made when the identity provider vouches for a person, and holds who they are and their
//! groups as the provider gave them then. A browser carries its token in a cookie and the desktop app in a
//! header. The store keeps only a hash of each token, so that reading the store signs nobody in.

use axum::extract::{FromRequestParts, RawPathParams};
use axum::http::header::{AUTHORIZATION, COOKIE, ORIGIN};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method};
use letters_api::{Code, Link, Person, Token};
use serde::{Deserialize, Serialize};

use crate::error::{Answer, Refusal};
use crate::ids::{self, hashed, secret};
use crate::store::{Batch, StoreError, Table};
use crate::{App, Shared};

/// The cookie a browser's session is in.
pub const COOKIE_NAME: &str = "letters_session";
/// How long the desktop app's token works before it is renewed.
const TOKEN_LIFE: u64 = 60 * 60;

/// How a session's token travels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Carrier {
    /// In a browser's cookie, for as long as the sign-in lasts.
    Cookie,
    /// In the desktop app's header: short-lived, and renewed.
    Bearer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Names the sign-in through every token it is renewed to. Empty in a row an older build wrote.
    #[serde(default)]
    pub id: String,
    pub person: Person,
    pub groups: Vec<String>,
    pub carrier: Carrier,
    /// When the token stops working.
    pub expires: u64,
    /// When the sign-in ends, renewed or not.
    pub ends: u64,
    /// The hash of the renewal that trades for the next token.
    #[serde(default)]
    pub renewal: Option<String>,
}

/// Someone the server has seen sign in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen {
    pub person: Person,
    pub groups: Vec<String>,
    pub first: u64,
    pub last: u64,
}

/// By the hash of the token.
const SESSIONS: Table<Session> = Table::new("sessions");
/// By the hash of the renewal: the hash of the token it renews.
const RENEWALS: Table<String> = Table::new("renewals");
/// By [`Person::id`].
pub const PEOPLE: Table<Seen> = Table::new("people");

impl App {
    /// Starts a session for someone the identity provider vouched for, and notes that they were seen.
    pub fn sign_in(&self, person: Person, groups: Vec<String>, carrier: Carrier) -> Answer<Token> {
        let now = self.now();
        let ends = now + u64::from(self.config.auth.session_days) * 24 * 60 * 60;
        Ok(self.store.write(|batch| {
            let first = batch.get(&PEOPLE, &person.id)?.map_or(now, |seen| seen.first);
            batch.put(
                &PEOPLE,
                &person.id,
                &Seen { person: person.clone(), groups: groups.clone(), first, last: now },
            )?;
            let id = ids::hex(&ids::random::<16>());
            issue(batch, Session { id, person, groups, carrier, expires: 0, ends, renewal: None }, now)
        })?)
    }

    /// Trades a renewal for the next token. A renewal works once, and the token it renewed stops working.
    pub fn renew(&self, renewal: &str) -> Answer<Token> {
        let now = self.now();
        let token = self.store.write(|batch| {
            let renewal = hashed(renewal);
            let Some(key) = batch.get(&RENEWALS, &renewal)? else { return Ok(None) };
            batch.remove(&RENEWALS, &renewal)?;
            let Some(session) = batch.get(&SESSIONS, &key)? else { return Ok(None) };
            batch.remove(&SESSIONS, &key)?;
            if session.ends <= now {
                return Ok(None);
            }
            issue(batch, session, now).map(Some)
        })?;
        token.ok_or_else(Refusal::signed_out)
    }

    /// Ends the session `token` stands for, and the sockets that were opened with it.
    pub fn sign_out(&self, token: &str) -> Answer<()> {
        let key = hashed(token);
        let ended = self.store.write(|batch| {
            let ended = batch.get(&SESSIONS, &key)?;
            if let Some(Session { renewal: Some(renewal), .. }) = &ended {
                batch.remove(&RENEWALS, renewal)?;
            }
            batch.remove(&SESSIONS, &key)?;
            Ok(ended)
        })?;
        if let Some(ended) = ended {
            self.rooms.signed_out(&ended.named(&key));
        }
        Ok(())
    }

    /// The session `token` stands for, while it lasts.
    fn session(&self, token: &str) -> Answer<Session> {
        let session = self.store.get(&SESSIONS, &hashed(token))?.ok_or_else(Refusal::signed_out)?;
        let now = self.now();
        match (session.expires <= now, session.ends <= now) {
            (false, false) => Ok(session),
            (true, false) if session.renewal.is_some() => {
                Err(Refusal::new(Code::Expired, "The session's token has run out and must be renewed."))
            }
            _ => Err(Refusal::signed_out()),
        }
    }
}

impl Session {
    /// What the sign-in goes by among the open sockets. `key` is the hash of the token it has now, which
    /// stands in for a sign-in an older build started.
    fn named(&self, key: &str) -> String {
        if self.id.is_empty() { key.to_owned() } else { self.id.clone() }
    }
}

/// Writes a new token for `session`, and a renewal when its carrier renews.
fn issue(batch: &mut Batch, mut session: Session, now: u64) -> Result<Token, StoreError> {
    let (token, renew) = (secret(), secret());
    session.expires = match session.carrier {
        Carrier::Cookie => session.ends,
        Carrier::Bearer => session.ends.min(now + TOKEN_LIFE),
    };
    session.renewal = (session.carrier == Carrier::Bearer).then(|| hashed(&renew));
    if let Some(renewal) = &session.renewal {
        batch.put(&RENEWALS, renewal, &hashed(&token))?;
    }
    batch.put(&SESSIONS, &hashed(&token), &session)?;
    Ok(Token { token, expires: session.expires, renew, ends: session.ends, person: session.person })
}

/// The token a request carries: the app's header first, then a browser's cookie.
pub fn carried(headers: &HeaderMap) -> Option<(Carrier, &str)> {
    let text = |name| headers.get(name).and_then(|value| value.to_str().ok());
    if let Some(token) = text(AUTHORIZATION).and_then(|value| value.strip_prefix("Bearer ")) {
        return Some((Carrier::Bearer, token.trim()));
    }
    cookie(headers, COOKIE_NAME).map(|token| (Carrier::Cookie, token))
}

/// The cookies a request carries, each by its name.
pub fn cookies(headers: &HeaderMap) -> impl Iterator<Item = (&str, &str)> {
    let cookies = headers.get_all(COOKIE).into_iter().filter_map(|value| value.to_str().ok());
    cookies.flat_map(|line| line.split(';')).filter_map(|pair| pair.trim().split_once('='))
}

/// The value of the cookie `name` among the ones a request carries.
pub fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    cookies(headers).find(|(found, _)| *found == name).map(|(_, value)| value)
}

/// Whoever a request comes from, signed in. A request without a session is refused as signed out.
#[derive(Clone, Debug)]
pub struct Signed {
    pub person: Person,
    pub groups: Vec<String>,
    pub carrier: Carrier,
    /// The token itself, to end the session with.
    pub token: String,
    /// What the sign-in goes by through every token it is renewed to.
    pub session: String,
    /// When the sign-in ends, renewed or not.
    pub ends: u64,
    /// The link that let the request's browser in to the document the request is about. Only a
    /// [`Reader`] has one.
    pub link: Option<Linked>,
}

/// A link that let a browser in to its document.
#[derive(Clone, Debug)]
pub struct Linked {
    pub link: Link,
    /// The document it leads to, and the key of the root that document's file is in.
    pub document: String,
    pub root: String,
    /// Whether the link is all the request has, with nobody signed in.
    pub alone: bool,
    /// When the visit that let the browser in is over: the link's own end, or sooner.
    pub ends: u64,
    /// Whether the document was taken out of safe mode in this visit.
    pub trusted: bool,
}

/// Whoever a request about one document comes from: someone signed in, or someone a link let in to that
/// document. Only what is asked of a document itself takes a reader. Everything else asks for [`Signed`],
/// which a link never is.
#[derive(Clone, Debug)]
pub struct Reader(pub Signed);

impl Signed {
    pub fn read(app: &App, method: &Method, headers: &HeaderMap) -> Answer<Self> {
        let (carrier, token) = carried(headers).ok_or_else(Refusal::signed_out)?;
        let session = app.session(token)?;
        // A token is only good the way it was handed out: a cookie's value in a header is someone's copy.
        if session.carrier != carrier {
            return Err(Refusal::signed_out());
        }
        // A browser sends its cookie with whatever a page of another site makes it ask for. What changes
        // something must come from the server's own pages, which the browser says in `Origin`.
        let reads = matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
        if carrier == Carrier::Cookie && !reads && !same_origin(app, headers) {
            return Err(Refusal::forbidden("This request did not come from the server's own pages."));
        }
        let (named, ends) = (session.named(&hashed(token)), session.ends);
        Ok(Self {
            person: session.person,
            groups: session.groups,
            carrier,
            token: token.to_owned(),
            session: named,
            ends,
            link: None,
        })
    }

    /// Whether a link is all that stands for them: nobody is signed in, and they reach the link's
    /// document and nothing else.
    pub fn alone(&self) -> bool {
        self.link.as_ref().is_some_and(|linked| linked.alone)
    }
}

impl Reader {
    /// Whoever asks about the document `id`. Someone signed in asks as themselves, and a link their
    /// browser was let in by comes along for what they do not reach otherwise. With nobody signed in, a
    /// link's reader goes by the link's label.
    pub fn read(app: &App, method: &Method, headers: &HeaderMap, id: Option<&str>) -> Answer<Self> {
        let guest = id.and_then(|id| app.let_in(headers, id));
        match (Signed::read(app, method, headers), guest) {
            (Ok(mut who), guest) => {
                who.link = guest.map(|guest| guest.linked);
                Ok(Self(who))
            }
            (Err(refusal), Some(guest)) if matches!(refusal.code(), Code::SignedOut | Code::Expired) => {
                // The cookie of a link is sent along like any other, with whatever another site's page asks.
                let reads = matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
                if !reads && !same_origin(app, headers) {
                    return Err(Refusal::forbidden("This request did not come from the server's own pages."));
                }
                let (linked, ends) = (Linked { alone: true, ..guest.linked }, guest.linked.ends);
                Ok(Self(Signed {
                    // No person has an id like it: one the provider gives is trimmed, and this starts
                    // with a space.
                    person: Person {
                        id: format!(" link:{}", linked.link.id),
                        username: String::new(),
                        name: linked.link.label.clone(),
                    },
                    groups: Vec::new(),
                    carrier: Carrier::Cookie,
                    token: guest.secret,
                    session: guest.visit,
                    ends,
                    link: Some(linked),
                }))
            }
            (Err(refusal), _) => Err(refusal),
        }
    }
}

/// Whether the browser says the request was made by one of the server's own pages.
pub fn same_origin(app: &App, headers: &HeaderMap) -> bool {
    let origin = headers.get(ORIGIN).and_then(|value| value.to_str().ok());
    origin.is_some_and(|origin| origin.eq_ignore_ascii_case(&app.config.server.url))
}

impl FromRequestParts<Shared> for Signed {
    type Rejection = Refusal;

    async fn from_request_parts(parts: &mut Parts, app: &Shared) -> Result<Self, Refusal> {
        Self::read(app, &parts.method, &parts.headers)
    }
}

/// The document a request's address names, as the routes call it.
pub async fn named_document(parts: &mut Parts, app: &Shared) -> Option<String> {
    let params = RawPathParams::from_request_parts(parts, app).await.ok()?;
    params.iter().find(|(name, _)| *name == "id").map(|(_, id)| id.to_owned())
}

impl FromRequestParts<Shared> for Reader {
    type Rejection = Refusal;

    async fn from_request_parts(parts: &mut Parts, app: &Shared) -> Result<Self, Refusal> {
        let id = named_document(parts, app).await;
        Self::read(app, &parts.method, &parts.headers, id.as_deref())
    }
}
