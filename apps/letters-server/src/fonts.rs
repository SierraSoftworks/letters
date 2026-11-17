//! The Google Fonts library for whoever writes in a browser: the list of its families, and a face of one.
//!
//! A page cannot ask the library for a file layout can read, and should not ask at all: the server asks,
//! so that the library hears of the server alone. Nothing is kept here. The page stores the face in the
//! document, and no reader asks for it again.

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use letters_api::{Code, Role, paths};
use letters_fonts::library;
use ureq::Agent;
use ureq::config::AutoHeaderValue;
use ureq::tls::{RootCerts, TlsConfig};

use crate::Shared;
use crate::api::blocking;
use crate::error::{Answer, Refusal};
use crate::session::Reader;

/// The most a font file of the library may weigh. Its largest, for Chinese, are some tens of megabytes.
const MOST: u64 = 64 << 20;
/// How long the library has to answer.
const WITHIN: Duration = Duration::from_secs(20);

/// Fetches what is at an address of the library, or nothing.
pub type Fetch = Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route(paths::FONTS, get(list))
        .route("/api/v1/documents/{id}/fonts/{family}/{weight}/{slant}", get(face))
}

async fn list() -> Response {
    let headers = [(CONTENT_TYPE, "text/plain; charset=utf-8"), (CACHE_CONTROL, "public, max-age=86400")];
    (headers, library::listing()).into_response()
}

async fn face(
    State(app): State<Shared>,
    Reader(who): Reader,
    Path((id, family, weight, slant)): Path<(String, String, u16, String)>,
) -> Answer<Response> {
    // Whoever only reads fetches nothing: what a document is set in is in its file.
    let allowed = app.clone();
    blocking(move || allowed.locate(&who, &id)?.must(Role::Editor)).await?;
    let italic = match slant.as_str() {
        "italic" => true,
        "upright" => false,
        _ => return Err(Refusal::not_found()),
    };
    // Only a face the family has is asked for, so nothing a page writes reaches the library as it is.
    let family = library::find(&family).filter(|has| has.weight(weight) == weight && (!italic || has.italic));
    let family = family.ok_or_else(Refusal::not_found)?;
    let _turn = app.fetches.turn().await?;
    let bytes = blocking(move || {
        let fetch = &app.fonts;
        let sheet = fetch(&family.stylesheet(weight, italic));
        let address = sheet.as_deref().and_then(|sheet| std::str::from_utf8(sheet).ok()).and_then(library::face);
        let bytes = address.and_then(|address| fetch(address)).filter(|bytes| letters_fonts::is_font(bytes));
        bytes.ok_or_else(|| Refusal::new(Code::Upstream, "The font could not be fetched from Google Fonts."))
    })
    .await?;
    let headers =
        [(CONTENT_TYPE, "font/ttf"), (CACHE_CONTROL, "private, max-age=86400"), (X_CONTENT_TYPE_OPTIONS, "nosniff")];
    Ok((headers, bytes).into_response())
}

/// The library over the network: its two hosts and no other, with no redirect followed.
pub fn web() -> Fetch {
    let config = Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        // Asked as no browser, the library names a whole TrueType file.
        .user_agent("Letters")
        .accept_encoding(AutoHeaderValue::None)
        .timeout_global(Some(WITHIN))
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .build();
    let agent: Agent = config.into();
    Arc::new(move |address: &str| {
        let host = address.strip_prefix("https://")?.split('/').next()?;
        if host != library::STYLESHEETS && host != library::FACES {
            return None;
        }
        let mut response = agent.get(address).call().ok()?;
        let mut bytes = Vec::new();
        response.body_mut().as_reader().take(MOST + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() as u64 <= MOST).then_some(bytes)
    })
}
