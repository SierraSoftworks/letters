//! Where a share link leads: the page that lets its reader in, after the passphrase when the link asks
//! one. It says nothing of the document, names no referrer to wherever its reader goes next, and asks not
//! to be indexed.

use axum::extract::{Form, Path, State};
use axum::http::header::{LOCATION, REFERRER_POLICY, RETRY_AFTER, SET_COOKIE};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use letters_api::{Code, paths};
use serde::Deserialize;

use super::view::Card;
use super::{card, status};
use crate::Shared;
use crate::api::blocking;
use crate::auth::cookie;
use crate::links::{Caller, Door};

/// What the passphrase form sends.
#[derive(Deserialize)]
pub struct Tried {
    #[serde(default)]
    passphrase: String,
}

/// `GET /l/{token}`: follows the link.
pub async fn follow(State(app): State<Shared>, caller: Caller, Path(token): Path<String>) -> Response {
    knock(app, caller, token, None).await
}

/// `POST /l/{token}`: follows the link with a passphrase. The form comes from a page that names no
/// referrer, so the browser says nothing of where it came from, and nothing is asked of that: whoever
/// sends it holds the link already.
pub async fn tried(
    State(app): State<Shared>,
    caller: Caller,
    Path(token): Path<String>,
    Form(tried): Form<Tried>,
) -> Response {
    knock(app, caller, token, Some(tried.passphrase)).await
}

async fn knock(app: Shared, caller: Caller, token: String, tried: Option<String>) -> Response {
    let door = blocking({
        let (app, token) = (app.clone(), token.clone());
        move || app.enter(&token, tried.as_deref(), &caller)
    });
    let mut response = match door.await {
        Ok(Door::Open { document, cookie: name, secret, life }) => {
            let let_in = cookie(&app, &name, &secret, "/", life);
            let next = [(LOCATION, paths::editor_page(&document))];
            (StatusCode::SEE_OTHER, next, [(SET_COOKIE, let_in)]).into_response()
        }
        Ok(Door::Asks { wrong }) => {
            let (status, title) = (if wrong { StatusCode::FORBIDDEN } else { StatusCode::OK }, "A passphrase is asked");
            card(&app, status, title, Card::Passphrase { action: paths::link_page(&token), wrong }, None).await
        }
        Err(refusal) => {
            let (title, message) = match refusal.code() {
                Code::Slow => ("Too many tries", "Wait a minute, and follow the link again."),
                Code::NotFound => (
                    "This link does not work",
                    "It has run out or was revoked. Whoever sent it to you can send another.",
                ),
                _ => ("This link did not open", refusal.0.message.as_str()),
            };
            let said = Card::Failed { title: title.into(), message: message.into(), again: None };
            let mut response = card(&app, status(&refusal), title, said, None).await;
            if let Some(seconds) = refusal.wait() {
                response.headers_mut().insert(RETRY_AFTER, HeaderValue::from(seconds));
            }
            response
        }
    };
    let headers = response.headers_mut();
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(HeaderName::from_static("x-robots-tag"), HeaderValue::from_static("noindex, nofollow"));
    response
}
