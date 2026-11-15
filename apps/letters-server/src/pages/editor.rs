//! The editor in a browser: the page a document is sent in, and the files that page runs.
//!
//! The browser's build of the editor is part of the program when the program was built with one, each
//! file in brotli. Its files are served under an address that names their contents, so a browser fetches
//! each once and keeps it until the editor is another. The page is made for each document and is never
//! kept. The large faces that back the fonts up are not part of the program: they are read from
//! [`FALLBACK`] in the state directory, where the server's owner may put them.

use std::borrow::Cow;
use std::io::Read;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header::{
    ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_TYPE, VARY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use letters_api::{Code, paths};
use sha2::{Digest, Sha256};

use super::{Visitor, guarded, refused};
use crate::api::blocking;
use crate::error::Refusal;
use crate::ids::hex;
use crate::{App, Shared};

/// The files the build compiled in: each one's name, its bytes, and whether those are brotli.
const COMPILED: &[(&str, &[u8], bool)] = include!(concat!(env!("OUT_DIR"), "/editor.rs"));
/// The folder of the state directory, and of the editor's address, that the backup faces are in.
pub const FALLBACK: &str = "fallback";
/// The address changes when the bytes do.
const KEPT: &str = "public, max-age=31536000, immutable";

/// One file of the editor.
pub struct File {
    /// Its name under the editor's address, with a `/` after each folder.
    pub name: Cow<'static, str>,
    pub bytes: Bytes,
    /// Whether `bytes` are the file in brotli.
    pub brotli: bool,
}

/// The browser's build of the editor.
pub struct Editor {
    /// Where its files are: an address that ends in a slash.
    pub base: String,
    files: Vec<File>,
}

impl Editor {
    /// The editor this program was built with, if it was built with one.
    pub fn compiled() -> Option<Self> {
        let file = |&(name, bytes, brotli): &(&'static str, &'static [u8], bool)| File {
            name: name.into(),
            bytes: Bytes::from_static(bytes),
            brotli,
        };
        (!COMPILED.is_empty()).then(|| Self::of(COMPILED.iter().map(file).collect()))
    }

    /// An editor of these files.
    pub fn of(files: Vec<File>) -> Self {
        let mut contents = Sha256::new();
        for file in &files {
            contents.update(file.name.as_bytes());
            contents.update((file.bytes.len() as u64).to_be_bytes());
            contents.update(&file.bytes);
        }
        Self { base: format!("/assets/{}/", &hex(&contents.finalize())[..12]), files }
    }
}

/// `GET /d/{id}`: the document in the editor. A server built with no editor shows it to read.
pub async fn page(State(app): State<Shared>, Visitor(who): Visitor, Path(id): Path<String>) -> Response {
    let Some(base) = app.editor.as_ref().map(|editor| editor.base.clone()) else {
        return Redirect::to(&paths::read_page(&id)).into_response();
    };
    let written = blocking({
        let (app, who) = (app.clone(), who.clone());
        move || {
            let (document, opened) = app.page(&who, &id)?;
            Ok(letters_web::page::served(&document, &opened, &base))
        }
    });
    match written.await {
        Ok(html) => (StatusCode::OK, guarded(&policy(&app)), html).into_response(),
        Err(refusal) => refused(&app, Some(&who), &refusal).await,
    }
}

/// What the page may load: the editor's files, and the pictures and frames the editor makes itself. It
/// reaches this server alone, by request and by the document's socket.
fn policy(app: &App) -> String {
    // A browser may read `'self'` as the page's own scheme alone, and the socket has another: `wss` for
    // `https`, and `ws` for the `http` of a server on this machine.
    let socket = app.config.server.url.replacen("http", "ws", 1);
    format!(
        "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'unsafe-inline'; \
         img-src 'self' blob: data:; font-src 'self'; connect-src 'self' {socket}; frame-src blob:; \
         base-uri 'self'; form-action 'none'; frame-ancestors 'none'"
    )
}

/// `GET /assets/{version}/{*file}`.
pub async fn file(
    State(app): State<Shared>,
    Path((version, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let nothing = || StatusCode::NOT_FOUND.into_response();
    let Some(editor) = app.editor.as_ref().filter(|editor| editor.base == format!("/assets/{version}/")) else {
        return nothing();
    };
    let found = [(CONTENT_TYPE, kind(&name)), (CACHE_CONTROL, KEPT), (X_CONTENT_TYPE_OPTIONS, "nosniff")];
    if let Some(face) = name.strip_prefix(FALLBACK).and_then(|face| face.strip_prefix('/')) {
        // A name alone: nothing in it leads out of the folder.
        let plain = |c: char| c.is_ascii_alphanumeric() || "-_.".contains(c);
        if face.is_empty() || face.starts_with('.') || !face.chars().all(plain) {
            return nothing();
        }
        let path = app.config.server.state.join(FALLBACK).join(face);
        return match tokio::task::spawn_blocking(move || std::fs::read(path)).await {
            Ok(Ok(bytes)) => (found, bytes).into_response(),
            _ => nothing(),
        };
    }
    let Some(file) = editor.files.iter().find(|file| file.name == name) else { return nothing() };
    if !file.brotli {
        return (found, file.bytes.clone()).into_response();
    }
    // A cache on the way keeps the two forms apart.
    let either = (VARY, "accept-encoding");
    if takes_brotli(&headers) {
        return (found, [either, (CONTENT_ENCODING, "br")], file.bytes.clone()).into_response();
    }
    let packed = file.bytes.clone();
    let whole = blocking(move || {
        let mut whole = Vec::new();
        let read = brotli_decompressor::Decompressor::new(&packed[..], 1 << 16).read_to_end(&mut whole);
        read.map_err(|_| Refusal::new(Code::Internal, "The server failed while answering."))?;
        Ok(whole)
    });
    match whole.await {
        Ok(whole) => (found, [either], whole).into_response(),
        Err(refusal) => refusal.into_response(),
    }
}

/// Whether the browser says it reads brotli. Most say so only to a server they reach over TLS.
fn takes_brotli(headers: &HeaderMap) -> bool {
    let said = headers.get_all(ACCEPT_ENCODING).iter().filter_map(|value| value.to_str().ok());
    said.flat_map(|value| value.split(',')).any(|coding| {
        let mut parts = coding.split(';').map(str::trim);
        let named = parts.next().is_some_and(|name| name.eq_ignore_ascii_case("br"));
        let weight = |part: &str| part.strip_prefix("q=").and_then(|weight| weight.parse::<f32>().ok());
        named && !parts.filter_map(weight).any(|weight| weight <= 0.0)
    })
}

/// What a file is, by the ending of its name.
fn kind(name: &str) -> &'static str {
    match name.rsplit_once('.').map_or("", |(_, ending)| ending) {
        "js" => "text/javascript; charset=utf-8",
        // A browser compiles the module as it arrives only under this type.
        "wasm" => "application/wasm",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
