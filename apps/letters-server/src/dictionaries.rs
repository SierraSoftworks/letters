//! Spelling dictionaries for languages other than English, which the editor has built in.
//!
//! The server's owner puts Hunspell dictionaries into [`FOLDER`] of the state directory: an `.aff` and a
//! `.dic` file of one name, such as `de_DE`, as a system keeps them. The editor in a browser cannot read
//! a system's own, so it asks here for the language a document or a passage names.
//!
//! Like the faces that back the fonts up, they are sent to whoever asks. They are word lists anyone may
//! have, and a link's visitor has no session beyond the link's document to be known by. Each file is sent
//! as UTF-8 whatever encoding its `SET` line names, in brotli where the browser reads that, and under an
//! `ETag` of its contents: a browser keeps it, and asks only whether it changed.

use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path as FilePath, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_ENCODING, CONTENT_TYPE, ETAG, IF_NONE_MATCH, VARY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use letters_api::paths;
use sha2::{Digest, Sha256};

use crate::ids::hex;
use crate::pages::editor::takes_brotli;
use crate::{App, Shared};

/// The folder of the state directory the dictionaries are in.
pub const FOLDER: &str = "dictionaries";
/// A file larger than this is not read: no word list is worth that much memory.
const LARGEST: u64 = 64 << 20;
/// How hard a file is pressed, once for as long as it stays the same.
const QUALITY: u32 = 9;

/// The parts of a language tag or of a dictionary's file name that choose a dictionary. The desktop app
/// reads a system's dictionaries by the same rule (`apps/letters/src/dictionaries.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Tag {
    /// In lower case.
    language: String,
    /// In upper case.
    region: Option<String>,
    /// Whether the name says more than a language and a region, as `de_DE_frami` and `sr-Latn-RS` do.
    more: bool,
}

impl Tag {
    /// Reads a BCP 47 tag such as `pt-BR`, or a file name such as `pt_BR`.
    fn parse(name: &str) -> Option<Self> {
        let alphabetic = |part: &str| part.bytes().all(|b| b.is_ascii_alphabetic());
        let mut parts = name.trim().split(['-', '_']);
        let language = parts.next().filter(|part| matches!(part.len(), 2 | 3) && alphabetic(part))?;
        let rest: Vec<&str> = parts.collect();
        let region = rest.iter().find(|part| part.len() == 2 && alphabetic(part));
        Some(Self {
            language: language.to_ascii_lowercase(),
            region: region.map(|region| region.to_ascii_uppercase()),
            more: rest.len() > usize::from(region.is_some()),
        })
    }

    fn written(&self) -> String {
        match &self.region {
            Some(region) => format!("{}-{region}", self.language),
            None => self.language.clone(),
        }
    }
}

/// The dictionaries in `folder`: each one's tag, and the name its two files share. English is left out:
/// the editor has it built in.
fn held(folder: &FilePath) -> Vec<(Tag, String)> {
    let mut held = Vec::new();
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let aff = entry.path();
        let whole = aff.extension().is_some_and(|ending| ending == "aff") && aff.with_extension("dic").is_file();
        let name = aff.file_stem().and_then(|name| name.to_str()).filter(|_| whole);
        let named = name.and_then(|name| Some((Tag::parse(name)?, name.to_owned())));
        held.extend(named.filter(|(tag, _)| tag.language != "en"));
    }
    // A folder is read in no order.
    held.sort_by(|a, b| a.1.cmp(&b.1));
    held
}

/// The name of the dictionary in `folder` that best serves `language`, a tag such as `de` or `de-CH`: one
/// for its region, else one for the language alone, else that of the country the language is named
/// after, else any. A plain name is preferred to a variant such as `de_DE_frami`.
fn best(folder: &FilePath, language: &str) -> Option<String> {
    let wanted = Tag::parse(language)?;
    let rank = |tag: &Tag| match &tag.region {
        region if *region == wanted.region => 0,
        None => 1,
        Some(region) if region.eq_ignore_ascii_case(&wanted.language) => 2,
        Some(_) => 3,
    };
    let serves = held(folder).into_iter().filter(|(tag, _)| tag.language == wanted.language);
    serves.min_by_key(|(tag, name)| (rank(tag), tag.more, name.clone())).map(|(_, name)| name)
}

/// The text of a dictionary's file. An older dictionary is in the encoding the `SET` line of its `.aff`
/// names, and Hunspell reads one that names none as Latin-1.
fn decoded(aff: &[u8], file: Vec<u8>) -> String {
    let file = match String::from_utf8(file) {
        Ok(text) => return text,
        Err(not) => not.into_bytes(),
    };
    let set = aff.split(|byte| *byte == b'\n').find_map(|line| line.strip_prefix(b"SET"));
    let label = set.map(|label| String::from_utf8_lossy(label).trim().to_ascii_lowercase()).unwrap_or_default();
    // Hunspell's names for encodings are nearly the web's.
    let label = match label.strip_prefix("microsoft-").unwrap_or(&label) {
        label if label.starts_with("tis620") => "tis-620",
        label => label,
    };
    let encoding = encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::WINDOWS_1252);
    encoding.decode_without_bom_handling(&file).0.into_owned()
}

/// What tells that a file is still the one that was read: how long it is, and when it was written.
type Stamp = (u64, Option<SystemTime>);

fn stamp(path: &FilePath) -> Option<Stamp> {
    let said = std::fs::metadata(path).ok()?;
    (said.is_file() && said.len() <= LARGEST).then(|| (said.len(), said.modified().ok()))
}

/// One file of a dictionary, ready to send.
struct Packed {
    /// Of its own file, and of the `.aff` its encoding was read from.
    stamps: (Stamp, Stamp),
    /// The `ETag`: its text's hash, in quotes.
    named: String,
    /// Its text as UTF-8, in brotli.
    brotli: Bytes,
}

/// The files that were sent before, each as it was then.
#[derive(Default)]
pub struct Dictionaries {
    packed: Mutex<HashMap<PathBuf, Arc<Packed>>>,
}

impl Dictionaries {
    /// The file at `path`, whose encoding the `.aff` beside it names. It is read and pressed once, and
    /// again when either file changed.
    fn file(&self, path: &FilePath) -> Option<Arc<Packed>> {
        let aff = path.with_extension("aff");
        let stamps = (stamp(path)?, stamp(&aff)?);
        let kept = |packed: &Arc<Packed>| packed.stamps == stamps;
        let had = self.packed.lock().unwrap_or_else(PoisonError::into_inner).get(path).filter(|had| kept(had)).cloned();
        if had.is_some() {
            return had;
        }
        let text = decoded(&std::fs::read(&aff).ok()?, std::fs::read(path).ok()?);
        let named = format!("\"{}\"", &hex(&Sha256::digest(text.as_bytes()))[..32]);
        let mut brotli = Vec::new();
        let mut press = brotli::CompressorReader::new(text.as_bytes(), 1 << 16, QUALITY, 22);
        press.read_to_end(&mut brotli).ok()?;
        let packed = Arc::new(Packed { stamps, named, brotli: brotli.into() });
        self.packed.lock().unwrap_or_else(PoisonError::into_inner).insert(path.to_owned(), packed.clone());
        Some(packed)
    }
}

pub fn routes() -> Router<Shared> {
    Router::new().route(paths::DICTIONARIES, get(list)).route("/api/v1/dictionaries/{tag}/{part}", get(file))
}

async fn list(State(app): State<Shared>) -> Json<Vec<String>> {
    Json(tokio::task::spawn_blocking(move || app.dictionaries()).await.unwrap_or_default())
}

async fn file(State(app): State<Shared>, Path((tag, part)): Path<(String, String)>, headers: HeaderMap) -> Response {
    let nothing = || StatusCode::NOT_FOUND.into_response();
    if !matches!(part.as_str(), "aff" | "dic") {
        return nothing();
    }
    let found = tokio::task::spawn_blocking(move || {
        let folder = app.config.server.state.join(FOLDER);
        let name = best(&folder, &tag)?;
        app.dictionaries.file(&folder.join(format!("{name}.{part}")))
    });
    let Ok(Some(packed)) = found.await else { return nothing() };
    // Asked for anew each time, and answered with nothing where the browser's copy is still the file.
    let named = packed.named.clone();
    let kept = || [(ETAG, named.clone()), (CACHE_CONTROL, "public, no-cache".into()), (VARY, "accept-encoding".into())];
    let has = headers.get_all(IF_NONE_MATCH).iter().filter_map(|value| value.to_str().ok());
    if has.flat_map(|value| value.split(',')).any(|tag| tag.trim().trim_start_matches("W/") == named) {
        return (StatusCode::NOT_MODIFIED, kept()).into_response();
    }
    let text = [(CONTENT_TYPE, "text/plain; charset=utf-8"), (X_CONTENT_TYPE_OPTIONS, "nosniff")];
    if takes_brotli(&headers) {
        return (kept(), text, [(CONTENT_ENCODING, "br")], packed.brotli.clone()).into_response();
    }
    let whole = tokio::task::spawn_blocking(move || {
        let mut whole = Vec::new();
        brotli_decompressor::Decompressor::new(&packed.brotli[..], 1 << 16).read_to_end(&mut whole).map(|_| whole)
    });
    match whole.await {
        Ok(Ok(whole)) => (kept(), text, whole).into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

impl App {
    /// The languages the server has a dictionary for, as tags such as `de-DE`, each once.
    pub fn dictionaries(&self) -> Vec<String> {
        let tags: BTreeSet<String> =
            held(&self.config.server.state.join(FOLDER)).into_iter().map(|(tag, _)| tag.written()).collect();
        tags.into_iter().collect()
    }
}
