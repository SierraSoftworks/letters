//! The server's own pages, for a browser with no editor in it.

use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use letters_api::{Person, paths};

use crate::App;
use crate::auth::Desktop;
use crate::error::Refusal;

const STYLE: &str = "\
body{margin:0;min-height:100vh;display:grid;place-items:center;font:16px/1.5 system-ui,sans-serif;\
background:Canvas;color:CanvasText;color-scheme:light dark}\
main{max-width:26rem;padding:2rem}h1{font-size:1.25rem;margin:0 0 .5rem}p{margin:0 0 1rem}\
button{font:inherit;padding:.5rem 1rem;border-radius:.375rem;border:1px solid;cursor:pointer}\
button.quiet{border:0;padding:0;background:none;color:inherit;text-decoration:underline}\
small{opacity:.7}";

/// Text as it stands in HTML, in an attribute as between tags.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A page of the server's own: `body` inside its frame. Nothing on it is fetched or run, and a form on it
/// goes to this server, or to `beyond` when the page names one.
fn page(app: &App, status: StatusCode, title: &str, body: &str, beyond: Option<&str>) -> Response {
    let name = escape(&app.config.server.name);
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{} · {name}</title><style>{STYLE}</style></head><body><main>{body}</main></body></html>",
        escape(title)
    );
    let policy = format!(
        "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'{}{}; base-uri 'none'; frame-ancestors 'none'",
        if beyond.is_some() { " " } else { "" },
        beyond.unwrap_or_default()
    );
    let headers = [
        (CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8")),
        // What a page says holds for the person and the moment it was made for.
        (CACHE_CONTROL, HeaderValue::from_static("no-store")),
        (CONTENT_SECURITY_POLICY, HeaderValue::from_str(&policy).expect("a policy is plain text")),
        (X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
        // The address of a sign-in holds what ends it, and is nobody else's to see.
        (REFERRER_POLICY, HeaderValue::from_static("no-referrer")),
    ];
    (status, headers, html).into_response()
}

/// Tells the person what did not work, and offers the way to try again.
pub fn failed(app: &App, refusal: &Refusal) -> Response {
    let status = StatusCode::from_u16(refusal.0.code.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let title = "Signing in did not work";
    let body = format!(
        "<h1>{title}</h1><p>{}</p><p><a href=\"{}\">Sign in again</a></p>",
        escape(&refusal.0.message),
        paths::SIGN_IN_PAGE
    );
    page(app, status, title, &body, None)
}

/// Asks the person whether the Letters app that opened this page may be signed in as them. Agreeing sends
/// the browser to the app, which listens on this computer.
pub fn hand_over(app: &App, person: &Person, asked: &Desktop) -> Response {
    let title = "Sign in to Letters";
    let field =
        |name: &str, value: &str| format!("<input type=\"hidden\" name=\"{name}\" value=\"{}\">", escape(value));
    let body = format!(
        "<h1>Sign in to the Letters app?</h1>\
         <p>The Letters app on this computer asks to open your documents on {server} as <b>{name}</b>.</p>\
         <form method=\"post\" action=\"{here}\">{challenge}{port}{state}<p><button>Sign in to the app</button></p></form>\
         <p><small>Close this page if you did not just ask Letters to sign in.</small></p>\
         <form method=\"post\" action=\"{leave}\"><small>Not {name}? <button class=\"quiet\">Sign out</button></small></form>",
        server = escape(&app.config.server.name),
        name = escape(&person.name),
        here = paths::DESKTOP_PAGE,
        challenge = field("challenge", &asked.challenge),
        port = field("port", &asked.port.to_string()),
        state = field("state", &asked.state),
        leave = paths::SIGN_OUT_PAGE,
    );
    page(app, StatusCode::OK, title, &body, Some(&asked.listener()))
}
