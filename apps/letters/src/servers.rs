//! The servers the person is signed in to on this device.
//!
//! Their addresses are listed in `.letters/servers.json` in the person's home folder, so that the app
//! knows where to look when it opens. The sign-ins themselves are in the system's keychain and nowhere
//! else: the file names servers, and nothing of who the person is there.
//!
//! The documents of those servers that were open here lately are listed beside it, in
//! `.letters/servers-recent.json`, for the list of recent documents. Signing out of a server takes its
//! documents off that list.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use crate::keychain::Keychain;
use crate::remote::Roots;
use crate::server::Client;
use crate::session::Session;

fn file(home: &Path) -> PathBuf {
    home.join(".letters").join("servers.json")
}

/// The addresses of the servers a sign-in was kept for, each as `https://host`. A list that cannot be
/// read is an empty one.
pub fn listed(home: &Path) -> Vec<String> {
    fs::read(file(home)).ok().and_then(|json| serde_json::from_slice(&json).ok()).unwrap_or_default()
}

/// Lists the server at `origin`, whose sign-in the keychain keeps from now on.
pub fn remember(home: &Path, origin: &str) -> io::Result<()> {
    let mut servers = listed(home);
    if servers.iter().any(|server| server == origin) {
        return Ok(());
    }
    servers.push(origin.to_owned());
    write(&file(home), &json!(servers))
}

/// Lists the server at `origin` no longer, as when the person signed out of it, nor the documents of it
/// that were open lately.
pub fn forget(home: &Path, origin: &str) -> io::Result<()> {
    let mut servers = listed(home);
    let before = servers.len();
    servers.retain(|server| server != origin);
    if servers.len() != before {
        write(&file(home), &json!(servers))?;
    }
    let mut recent = lately(home);
    let before = recent.len();
    recent.retain(|lately| !lately.address.strip_prefix(origin).is_some_and(|rest| rest.starts_with('/')));
    if recent.len() == before { Ok(()) } else { write_lately(home, &recent) }
}

fn write(file: &Path, list: &Value) -> io::Result<()> {
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)?;
    }
    // Written beside and renamed, so that a crash leaves a whole list or the last one.
    let beside = file.with_extension("new");
    fs::write(&beside, serde_json::to_vec(list)?)?;
    fs::rename(&beside, file)
}

/// How many documents of servers the list of recent ones remembers.
const LATELY: usize = 20;

/// A server's document that was open on this device lately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lately {
    /// The address of the document's page, which opens it: `https://host/d/ID`.
    pub address: String,
    pub name: String,
    /// Where the server keeps it, as it is to read: `letters.example.org / Family / House`.
    pub folder: String,
    /// When it was last opened here, as RFC 3339.
    pub opened: String,
}

fn lately_file(home: &Path) -> PathBuf {
    home.join(".letters").join("servers-recent.json")
}

/// The documents of servers that were open here lately, the newest first. A list that cannot be read is an
/// empty one.
pub fn lately(home: &Path) -> Vec<Lately> {
    let entry = |entry: &Value| {
        let text = |key: &str| entry[key].as_str().map(str::to_owned);
        Some(Lately {
            address: text("address")?,
            name: text("name")?,
            folder: text("folder")?,
            opened: text("opened")?,
        })
    };
    let read = fs::read(lately_file(home)).ok().and_then(|json| serde_json::from_slice::<Value>(&json).ok());
    read.as_ref().and_then(Value::as_array).map(|list| list.iter().filter_map(entry).collect()).unwrap_or_default()
}

/// Puts a document first among those that were open lately, in place of what was listed at its address.
pub fn opened(home: &Path, document: Lately) -> io::Result<()> {
    let mut recent = lately(home);
    recent.retain(|lately| lately.address != document.address);
    recent.insert(0, document);
    recent.truncate(LATELY);
    write_lately(home, &recent)
}

fn write_lately(home: &Path, recent: &[Lately]) -> io::Result<()> {
    let entry = |lately: &Lately| json!({ "address": lately.address, "name": lately.name, "folder": lately.folder, "opened": lately.opened });
    write(&lately_file(home), &recent.iter().map(entry).collect())
}

/// The sessions kept from when the app last ran, one for each listed server whose sign-in the keychain
/// still has and that has not ended. Nothing is asked of any server, and of the keychain only if a
/// server is listed.
pub fn kept(home: &Path, keychain: impl Fn() -> Option<Keychain>, roots: &Roots) -> Vec<Arc<Session>> {
    let listed = listed(home);
    let Some(keychain) = Some(()).filter(|()| !listed.is_empty()).and_then(|()| keychain()) else { return Vec::new() };
    let session = |origin: &String| {
        let server = Client::new(origin, roots).ok()?;
        Session::kept(server, keychain).ok().flatten().filter(|session| !session.ended()).map(Arc::new)
    };
    listed.iter().filter_map(session).collect()
}
