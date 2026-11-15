//! Signing in: a browser at the identity provider, and the desktop app through a browser.
//!
//! A browser that asks for [`paths::SIGN_IN_PAGE`] is sent to the provider and comes back to
//! [`paths::CALLBACK_PAGE`], where its session starts. The desktop app opens a browser at
//! [`paths::DESKTOP_PAGE`]: the person signs in there as they would anyway and agrees, and the browser
//! carries a code of the server's own to the app, which only the app that asked can trade for a session.
//! So the provider knows one client and one address to come back to, whatever the device.

use axum::extract::rejection::{FormRejection, QueryRejection};
use axum::extract::{Form, Query, State};
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, HeaderValue, Method};
use axum::response::{AppendHeaders, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use letters_api::{Code, Exchange, Person, Token, paths};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::api::blocking;
use crate::error::{Answer, Refusal};
use crate::ids::{base64url, hashed, secret};
use crate::oidc::{Sent, form};
use crate::session::{self, Carrier, Signed};
use crate::store::Table;
use crate::telemetry::{Event, Through};
use crate::{App, Shared, log, pages};

/// How long a person has at the provider before the sign-in must be started again.
const SIGN_IN_TIME: u64 = 10 * 60;
/// How many sign-ins may be under way at once. Starting one takes no session, so their number is held.
const MOST_PENDING: usize = 10_000;
/// How long the desktop app has to trade its code.
const HANDOVER_TIME: u64 = 60;
/// The cookie that ties a sign-in to the browser that started it.
const STATE_COOKIE: &str = "letters_signin";
/// Where that cookie is sent: the pages of a sign-in and no others.
const STATE_PATH: &str = "/auth";

/// A sign-in whose person is at the provider.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Pending {
    sent: Sent,
    /// Where on the server the person was going.
    back: String,
    expires: u64,
}

/// A person on their way from a browser to the desktop app.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Handover {
    person: Person,
    groups: Vec<String>,
    /// The hash of the secret the app made, which it shows when it trades the code.
    challenge: String,
    expires: u64,
}

/// By the hash of the state.
const PENDING: Table<Pending> = Table::new("sign-ins");
/// By the hash of the code.
const HANDOVERS: Table<Handover> = Table::new("handovers");

pub fn routes() -> Router<Shared> {
    Router::new()
        .route(paths::SIGN_IN_PAGE, get(sign_in))
        .route(paths::CALLBACK_PAGE, get(callback))
        .route(paths::SIGN_OUT_PAGE, post(sign_out))
        .route(paths::DESKTOP_PAGE, get(desktop).post(hand_over))
        .route(paths::EXCHANGE, post(exchange))
}

/// A cookie that only this server is sent, out of reach of a page's scripts. A `life` of 0 takes it away.
pub fn cookie(app: &App, name: &str, value: &str, path: &str, life: u64) -> HeaderValue {
    // A browser keeps a `Secure` cookie from a server it reached without TLS, which only one on this
    // machine may be.
    let secure = if app.config.server.url.starts_with("https://") { "; Secure" } else { "" };
    let cookie = format!("{name}={value}; Path={path}; Max-Age={life}; HttpOnly; SameSite=Lax{secure}");
    HeaderValue::from_str(&cookie).expect("a cookie of the server's own making is plain text")
}

/// The cookie a session's token is kept in, for as long as the sign-in lasts.
fn session_cookie(app: &App, token: &Token) -> HeaderValue {
    cookie(app, session::COOKIE_NAME, &token.token, "/", token.ends.saturating_sub(app.now()))
}

/// The session's cookie, taken away.
pub fn no_session_cookie(app: &App) -> HeaderValue {
    cookie(app, session::COOKIE_NAME, "", "/", 0)
}

/// `to` when it is a place on this server, and the front page when it could lead anywhere else.
pub fn local(to: Option<&str>) -> String {
    let inside = |to: &&str| to.starts_with('/') && !to.starts_with("//") && !to.contains(['\\', '\r', '\n']);
    to.filter(inside).unwrap_or(paths::HOME_PAGE).to_owned()
}

/// The browser's own session, when it has one.
pub fn browser(app: &App, method: &Method, headers: &HeaderMap) -> Answer<Signed> {
    Signed::read(app, method, headers).and_then(|who| match who.carrier {
        Carrier::Cookie => Ok(who),
        Carrier::Bearer => Err(Refusal::signed_out()),
    })
}

#[derive(Deserialize)]
struct Start {
    #[serde(default)]
    then: Option<String>,
}

async fn sign_in(State(app): State<Shared>, headers: HeaderMap, Query(start): Query<Start>) -> Response {
    let back = local(start.then.as_deref());
    // Someone who is signed in has nothing to do at the provider.
    if browser(&app, &Method::GET, &headers).is_ok() {
        return Redirect::to(&back).into_response();
    }
    let began = blocking({
        let app = app.clone();
        move || app.begin(back)
    });
    match began.await {
        Ok((address, state)) => {
            let state = cookie(&app, STATE_COOKIE, &state, STATE_PATH, SIGN_IN_TIME);
            ([(SET_COOKIE, state)], Redirect::to(&address)).into_response()
        }
        Err(refusal) => pages::failed(&app, &refusal).await,
    }
}

/// What the provider sends a browser back with.
#[derive(Deserialize)]
struct Back {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

async fn callback(State(app): State<Shared>, headers: HeaderMap, Query(back): Query<Back>) -> Response {
    let started = session::cookie(&headers, STATE_COOKIE).map(str::to_owned);
    let finished = blocking({
        let app = app.clone();
        move || app.finish(back, started)
    });
    let spent = (SET_COOKIE, cookie(&app, STATE_COOKIE, "", STATE_PATH, 0));
    match finished.await {
        Ok((token, to)) => {
            app.telemetry.record(Event::SignedIn(Through::Browser));
            let session = (SET_COOKIE, session_cookie(&app, &token));
            (AppendHeaders([spent, session]), Redirect::to(&to)).into_response()
        }
        Err(refusal) => (AppendHeaders([spent]), pages::failed(&app, &refusal).await).into_response(),
    }
}

/// Ends a browser's session from one of the server's pages, and shows the front page.
async fn sign_out(State(app): State<Shared>, headers: HeaderMap) -> Response {
    match browser(&app, &Method::POST, &headers) {
        Ok(who) => {
            let ended = blocking({
                let app = app.clone();
                move || app.sign_out(&who.token)
            });
            match ended.await {
                Ok(()) => ([(SET_COOKIE, no_session_cookie(&app))], Redirect::to(paths::HOME_PAGE)).into_response(),
                Err(refusal) => pages::failed(&app, &refusal).await,
            }
        }
        // Nobody was signed in, or the request is another site's: either way there is nothing to end.
        Err(_) => Redirect::to(paths::HOME_PAGE).into_response(),
    }
}

/// What the desktop app asks for when it opens a browser: see [`paths::desktop_sign_in`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Desktop {
    pub challenge: String,
    pub port: u16,
    pub state: String,
}

impl Desktop {
    /// Whether this is what an app sends: the hash of a secret, a port a program may listen on, and a
    /// state that goes into an address as it is.
    fn sound(&self) -> bool {
        let plain = |text: &str| text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        let state = !self.state.is_empty() && self.state.len() <= 128 && plain(&self.state);
        self.challenge.len() == 43 && plain(&self.challenge) && self.port >= 1024 && state
    }

    /// Where the app listens for the browser.
    pub fn listener(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

fn not_the_app() -> Refusal {
    Refusal::invalid("This is not an address Letters opens to sign in. Start again from the app.")
}

async fn desktop(
    State(app): State<Shared>,
    headers: HeaderMap,
    asked: Result<Query<Desktop>, QueryRejection>,
) -> Response {
    let asked = match asked {
        Ok(Query(asked)) if asked.sound() => asked,
        _ => return pages::failed(&app, &not_the_app()).await,
    };
    match browser(&app, &Method::GET, &headers) {
        Ok(who) => pages::hand_over(&app, &who.person, &asked).await,
        // The person signs in first, and comes back here to agree.
        Err(_) => {
            let here = paths::desktop_sign_in(&asked.challenge, asked.port, &asked.state);
            Redirect::to(&paths::sign_in_page(&here)).into_response()
        }
    }
}

/// The person agreed: the browser goes to the app with a code for it.
async fn hand_over(
    State(app): State<Shared>,
    headers: HeaderMap,
    asked: Result<Form<Desktop>, FormRejection>,
) -> Response {
    let asked = match asked {
        Ok(Form(asked)) if asked.sound() => asked,
        _ => return pages::failed(&app, &not_the_app()).await,
    };
    let who = match browser(&app, &Method::POST, &headers) {
        Ok(who) => who,
        Err(refusal) => return pages::failed(&app, &refusal).await,
    };
    let handed = blocking({
        let (app, asked) = (app.clone(), asked.clone());
        move || app.hand_over(who, asked.challenge)
    });
    match handed.await {
        Ok(code) => {
            let query = form(&[("code", &code), ("state", &asked.state)]);
            Redirect::to(&format!("{}/?{query}", asked.listener())).into_response()
        }
        Err(refusal) => pages::failed(&app, &refusal).await,
    }
}

async fn exchange(State(app): State<Shared>, Json(exchange): Json<Exchange>) -> Answer<Json<Token>> {
    let reporting = app.clone();
    let exchanged = blocking(move || app.exchange(&exchange).map(Json)).await;
    exchanged.inspect(|_| reporting.telemetry.record(Event::SignedIn(Through::Desktop)))
}

impl App {
    /// Starts a sign-in that comes back to `back`: where to send the browser, and the state to tie it to.
    fn begin(&self, back: String) -> Answer<(String, String)> {
        let now = self.now();
        let (address, sent) = self.provider.begin(now)?;
        let state = sent.state.clone();
        let room = self.store.write(|batch| {
            let mut pending = 0;
            batch.prune(&PENDING, "", |_, pending_one: &Pending| {
                let lasts = now < pending_one.expires;
                pending += usize::from(lasts);
                lasts
            })?;
            if pending >= MOST_PENDING {
                return Ok(false);
            }
            batch.put(&PENDING, &hashed(&state), &Pending { sent, back, expires: now + SIGN_IN_TIME })?;
            Ok(true)
        })?;
        match room {
            true => Ok((address, state)),
            false => Err(Refusal::slow(3 * 60, "Too many sign-ins are under way. Try again in a few minutes.")),
        }
    }

    /// Ends a sign-in whose browser came back from the provider: the session, and where the person was
    /// going. `started` is the state the browser itself holds.
    fn finish(&self, back: Back, started: Option<String>) -> Answer<(Token, String)> {
        let now = self.now();
        let stale = || Refusal::invalid("This sign-in was started in another browser, or too long ago. Start again.");
        // Coming back with someone else's state would sign this browser in as them.
        let state = back.state.filter(|state| Some(state) == started.as_ref()).ok_or_else(stale)?;
        // A sign-in comes back once.
        let key = hashed(&state);
        let pending = self.store.write(|batch| {
            let pending = batch.get(&PENDING, &key)?;
            batch.remove(&PENDING, &key)?;
            Ok(pending)
        })?;
        let pending = pending.filter(|pending| now < pending.expires).ok_or_else(stale)?;
        if let Some(error) = back.error {
            let said = back.error_description.unwrap_or_default();
            log(&format!("the identity provider ended a sign-in with \"{error}\": {said}"));
            return Err(Refusal::forbidden(match error.as_str() {
                "access_denied" => "You were not signed in at the identity provider.",
                _ => "The identity provider could not sign you in. Try again, or ask whoever runs this server.",
            }));
        }
        let code = back.code.ok_or_else(stale)?;
        let (person, groups) = self.provider.finish(&code, &pending.sent, now)?;
        Ok((self.sign_in(person, groups, Carrier::Cookie)?, pending.back))
    }

    /// Makes the code that carries `who` from their browser to the app that showed `challenge`.
    fn hand_over(&self, who: Signed, challenge: String) -> Answer<String> {
        let now = self.now();
        let code = secret();
        let handover = Handover { person: who.person, groups: who.groups, challenge, expires: now + HANDOVER_TIME };
        self.store.write(|batch| {
            batch.prune(&HANDOVERS, "", |_, handover: &Handover| now < handover.expires)?;
            batch.put(&HANDOVERS, &hashed(&code), &handover)
        })?;
        Ok(code)
    }

    /// Trades the app's code for its session. A code works once, and only for whoever holds the secret
    /// behind the challenge it was made for: a program that only overheard the code holds nothing.
    fn exchange(&self, exchange: &Exchange) -> Answer<Token> {
        let now = self.now();
        let key = hashed(&exchange.code);
        let handover = self.store.write(|batch| {
            let handover = batch.get(&HANDOVERS, &key)?;
            batch.remove(&HANDOVERS, &key)?;
            Ok(handover)
        })?;
        let shown = base64url(&Sha256::digest(exchange.verifier.as_bytes()));
        match handover.filter(|handover| now < handover.expires && handover.challenge == shown) {
            Some(handover) => self.sign_in(handover.person, handover.groups, Carrier::Bearer),
            None => {
                Err(Refusal::new(Code::SignedOut, "The sign-in was not finished in time. Start again from the app."))
            }
        }
    }
}
