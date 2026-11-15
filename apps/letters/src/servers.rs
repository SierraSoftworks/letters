//! The servers the person is signed in to on this device.
//!
//! Their addresses are listed in `.letters/servers.json` in the person's home folder, so that the app
//! knows where to look when it opens. The sign-ins themselves are in the system's keychain and nowhere
//! else: the file names servers, and nothing of who the person is there.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    write(home, &servers)
}

/// Lists the server at `origin` no longer, as when the person signed out of it.
pub fn forget(home: &Path, origin: &str) -> io::Result<()> {
    let mut servers = listed(home);
    let before = servers.len();
    servers.retain(|server| server != origin);
    if servers.len() == before { Ok(()) } else { write(home, &servers) }
}

fn write(home: &Path, servers: &[String]) -> io::Result<()> {
    let file = file(home);
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)?;
    }
    // Written beside and renamed, so that a crash leaves a whole list or the last one.
    let beside = file.with_extension("new");
    fs::write(&beside, serde_json::to_vec(servers)?)?;
    fs::rename(&beside, &file)
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
