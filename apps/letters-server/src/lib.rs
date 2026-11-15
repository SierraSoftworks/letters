//! The Letters server: it maps the people an identity provider signs in onto folders of documents, and
//! keeps only the state that files cannot hold.
//!
//! - [`config`] is the settings file, and [`store`] the state directory.
//! - [`session`] says who is asking, [`oidc`] asks the identity provider who signs in, and [`auth`] is
//!   the way from one to the other.
//! - [`spaces`] is the only way to a file, and cannot leave a space's folder.
//! - [`library`] lists a space and finds a document by its id, wherever its file was moved to.
//! - [`api`] answers the interface `letters_api` lays down.
//! - [`pages`] are the server's own pages for a browser: the documents a person reaches, and one to read.
//!
//! A document's content lives in its `.let` file alone. Whatever else writes that file, over a share or
//! with a sync tool, is another writer like the server, and the save in `letters_format` merges with it.

pub mod api;
pub mod auth;
pub mod config;
pub mod error;
pub mod ids;
pub mod library;
pub mod oidc;
pub mod pages;
pub mod session;
pub mod spaces;
pub mod store;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use letters_api::{Role, Space};

use crate::config::Config;
use crate::error::{Answer, Refusal};
use crate::library::Library;
use crate::oidc::Provider;
use crate::session::Signed;
use crate::spaces::Root;
use crate::store::Store;

/// Tells the time, in seconds since 1970.
pub type Clock = Box<dyn Fn() -> u64 + Send + Sync>;

/// Everything a request is answered from.
pub struct App {
    pub config: Config,
    pub store: Store,
    pub library: Library,
    pub provider: Provider,
    clock: Clock,
}

pub type Shared = Arc<App>;

impl App {
    pub fn new(config: Config, store: Store) -> Self {
        let clock = || SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
        let provider = Provider::new(&config);
        Self { config, store, library: Library::new(), provider, clock: Box::new(clock) }
    }

    /// The same server telling another time, for tests of what runs out.
    pub fn with_clock(self, clock: Clock) -> Self {
        Self { clock, ..self }
    }

    pub fn now(&self) -> u64 {
        (self.clock)()
    }

    /// The spaces `who` reaches, in the order the settings list them.
    pub fn spaces(&self, who: &Signed) -> Vec<(&config::Space, Role)> {
        let reached = |space| Some((space, config::Space::role(space, &who.groups)?));
        self.config.spaces.iter().filter_map(reached).collect()
    }

    /// The space `id` as `who` reaches it. One they do not reach is not there for them.
    pub fn space(&self, who: &Signed, id: &str) -> Answer<(&config::Space, Role)> {
        self.spaces(who).into_iter().find(|(space, _)| space.id() == id).ok_or_else(Refusal::not_found)
    }

    /// Opens a space's folder for `who`. Their own is made when they first come to it.
    pub fn root(&self, who: &Signed, space: &config::Space) -> Answer<Root> {
        if space.personal() && !spaces::username_ok(&who.person.username) {
            return Err(Refusal::forbidden("The name you sign in with cannot name a folder."));
        }
        Ok(Root::open(&space.folder(&who.person.username), space.personal())?)
    }

    /// Every space `who` reaches whose folder can be opened now, as the interface tells it.
    pub fn reach(&self, who: &Signed) -> Vec<(Space, Root)> {
        let open = |(space, role): (&config::Space, Role)| Some((space.told(role), self.root(who, space).ok()?));
        self.spaces(who).into_iter().filter_map(open).collect()
    }
}

impl App {
    /// Reads every folder the server knows of into the index: the shared spaces, and the own space of
    /// everyone who has signed in. A folder that cannot be read is said and passed over.
    pub fn scan(&self) {
        let people = self.store.under(&session::PEOPLE, "").unwrap_or_default();
        let names: Vec<String> = people.into_iter().map(|(_, seen)| seen.person.username).collect();
        for space in &self.config.spaces {
            let folders: Vec<String> = match space.personal() {
                true => names.iter().filter(|name| spaces::username_ok(name)).map(|name| space.folder(name)).collect(),
                false => vec![space.folder("")],
            };
            for folder in folders {
                let scanned = Root::open(&folder, false).map_err(Refusal::from);
                if let Err(refusal) = scanned.and_then(|root| self.library.scan(&self.store, &root)) {
                    log(&format!("the space \"{}\" could not be scanned: {}", space.name, refusal.0));
                }
            }
        }
    }
}

/// Writes a line to the server's log, which is its standard error.
pub fn log(line: &str) {
    eprintln!("letters-server: {line}");
}
