//! Who is asking: sessions, and the tokens that stand for them.
//!
//! A session is made when the identity provider vouches for a person, and holds who they are and their
//! groups as the provider gave them then. A browser carries its token in a cookie and the desktop app in a
//! header. The store keeps only a hash of each token, so that reading the store signs nobody in.

use axum::extract::FromRequestParts;
use axum::http::header::{AUTHORIZATION, COOKIE, ORIGIN};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method};
use letters_api::{Code, Person, Token};
use serde::{Deserialize, Serialize};

use crate::error::{Answer, Refusal};
use crate::ids::{hashed, secret};
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
            issue(batch, Session { person, groups, carrier, expires: 0, ends, renewal: None }, now)
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

    /// Ends the session `token` stands for.
    pub fn sign_out(&self, token: &str) -> Answer<()> {
        Ok(self.store.write(|batch| {
            let key = hashed(token);
            if let Some(Session { renewal: Some(renewal), .. }) = batch.get(&SESSIONS, &key)? {
                batch.remove(&RENEWALS, &renewal)?;
            }
            batch.remove(&SESSIONS, &key)
        })?)
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
    let cookies = headers.get_all(COOKIE).into_iter().filter_map(|value| value.to_str().ok());
    let mut pairs = cookies.flat_map(|line| line.split(';')).filter_map(|pair| pair.trim().split_once('='));
    pairs.find(|(name, _)| *name == COOKIE_NAME).map(|(_, token)| (Carrier::Cookie, token))
}

/// Whoever a request comes from, signed in. A request without a session is refused as signed out.
#[derive(Clone, Debug)]
pub struct Signed {
    pub person: Person,
    pub groups: Vec<String>,
    pub carrier: Carrier,
    /// The token itself, to end the session with.
    pub token: String,
}

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
        Ok(Self { person: session.person, groups: session.groups, carrier, token: token.to_owned() })
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
