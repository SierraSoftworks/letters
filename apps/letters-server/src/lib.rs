//! The Letters server: it maps the people an identity provider signs in onto folders of documents, and
//! keeps only the state that files cannot hold.
//!
//! - [`config`] is the settings file, and [`store`] the state directory.
//! - [`session`] says who is asking, [`oidc`] asks the identity provider who signs in, and [`auth`] is
//!   the way from one to the other.
//! - [`spaces`] is the only way to a file, and cannot leave a space's folder.
//! - [`library`] lists a space and finds a document by its id, wherever its file was moved to.
//! - [`api`] answers the interface `letters_api` lays down.
//! - [`home`] is what a person set for themselves, kept as files in their own space.
//! - [`import`] makes a document of a Markdown or a Word file.
//! - [`shares`] is who reaches a document besides those its space lets in, and [`links`] are the
//!   addresses that let in whoever holds one.
//! - [`fetch`] is what the server fetches for a document from elsewhere: a linked picture, a link's page.
//! - [`seen`] is where each person read a document up to.
//! - [`rooms`] are the documents that are open now, and [`socket`] is each device's line to one.
//! - [`logs`] is what the server says of itself, on its standard error.
//! - [`pages`] are the server's own pages for a browser: the documents a person reaches, one to read, and
//!   one in the editor, which is compiled in when the server is built with it.
//!
//! A document's content lives in its `.let` file alone. Whatever else writes that file, over a share or
//! with a sync tool, is another writer like the server, and the save in `letters_format` merges with it.

pub mod access;
pub mod api;
pub mod auth;
pub mod config;
pub mod error;
pub mod fetch;
pub mod home;
pub mod ids;
pub mod import;
pub mod library;
pub mod links;
pub mod logs;
pub mod oidc;
pub mod pages;
pub mod rooms;
pub mod seen;
pub mod session;
pub mod shares;
pub mod socket;
pub mod spaces;
pub mod store;
pub mod telemetry;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::serve::ListenerExt;
use letters_api::{Person, Role, Space};

use crate::access::Log;
use crate::config::Config;
use crate::error::{Answer, Refusal};
use crate::fetch::{Fetches, Roots};
use crate::home::Homes;
use crate::library::Library;
use crate::links::Doors;
use crate::oidc::Provider;
use crate::pages::editor::Editor;
use crate::rooms::Rooms;
use crate::session::Signed;
use crate::spaces::Root;
use crate::store::Store;
use crate::telemetry::Telemetry;

/// Tells the time, in seconds since 1970.
pub type Clock = Box<dyn Fn() -> u64 + Send + Sync>;

/// Everything a request is answered from.
pub struct App {
    pub config: Config,
    pub store: Store,
    pub library: Library,
    pub provider: Provider,
    pub rooms: Rooms,
    /// The browser's editor, when the server was built with one. Without it a browser reads.
    pub editor: Option<Editor>,
    pub fetches: Fetches,
    /// What following share links has cost each address of late.
    pub doors: Doors,
    /// What the access logs noted of late.
    pub log: Log,
    pub telemetry: Telemetry,
    homes: Homes,
    clock: Clock,
}

pub type Shared = Arc<App>;

impl App {
    pub fn new(config: Config, store: Store) -> Self {
        let clock = || SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
        let (provider, rooms) = (Provider::new(&config), Rooms::new(&config.server.state, store.clone()));
        let (editor, fetches) = (Editor::compiled(), Fetches::new(Roots::System));
        let (library, doors, log) = (Library::new(), Doors::default(), Log::default());
        let (telemetry, homes, clock) = (Telemetry::default(), Homes::default(), Box::new(clock));
        Self { config, store, library, provider, rooms, editor, fetches, doors, log, telemetry, homes, clock }
    }

    /// The same server taking another word on who a site is, for tests against a site of their own.
    pub fn with_roots(self, roots: Roots) -> Self {
        Self { fetches: Fetches::new(roots), ..self }
    }

    /// The same server whose rooms hold fewer bytes of pictures that wait for their edit, for tests of
    /// too many.
    pub fn with_waiting(self, most: usize) -> Self {
        Self { rooms: self.rooms.holding(most), ..self }
    }

    /// The same server with another editor for its browsers, or with none.
    pub fn with_editor(self, editor: Option<Editor>) -> Self {
        Self { editor, ..self }
    }

    /// The same server telling another time, for tests of what runs out.
    pub fn with_clock(self, clock: Clock) -> Self {
        Self { clock, ..self }
    }

    pub fn now(&self) -> u64 {
        (self.clock)()
    }

    /// The spaces `who` reaches, in the order the settings list them. Whoever only a link let in reaches
    /// none: not even a space of their own, which everyone who signs in has.
    pub fn spaces(&self, who: &Signed) -> Vec<(&config::Space, Role)> {
        let reached = |space| Some((space, config::Space::role(space, &who.groups)?));
        self.config.spaces.iter().filter(|_| !who.alone()).filter_map(reached).collect()
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

/// A folder the server knows of, open.
pub(crate) struct Rooted<'a> {
    pub space: &'a config::Space,
    /// Whose it is, when it is one person's own.
    pub whose: Option<Person>,
    pub root: Root,
}

impl App {
    /// Every folder the server knows of: the shared spaces, and the own space of everyone who has signed
    /// in. One that cannot be opened is said and passed over, and so is the own space of someone who
    /// has not come to it yet, which is no fault.
    pub(crate) fn folders(&self) -> Vec<Rooted<'_>> {
        let people = self.store.under(&session::PEOPLE, "").unwrap_or_default();
        let people: Vec<Person> = people.into_iter().map(|(_, seen)| seen.person).collect();
        let mut roots = Vec::new();
        for space in &self.config.spaces {
            let own = people.iter().filter(|person| spaces::username_ok(&person.username));
            let folders: Vec<(String, Option<Person>)> = match space.personal() {
                true => own.map(|person| (space.folder(&person.username), Some(person.clone()))).collect(),
                false => vec![(space.folder(""), None)],
            };
            for (folder, whose) in folders {
                match Root::open(&folder, false) {
                    Ok(root) => roots.push(Rooted { space, whose, root }),
                    Err(error) if whose.is_some() && error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => logs::warn(&format!("the space \"{}\" could not be opened: {error}", space.name)),
                }
            }
        }
        roots
    }

    /// Every folder the server knows of that can be opened now.
    pub fn roots(&self) -> Vec<Root> {
        self.folders().into_iter().map(|rooted| rooted.root).collect()
    }

    /// Reads every folder the server knows of into the index. One that cannot be read is said and passed
    /// over.
    pub fn scan(&self) {
        for Rooted { space, root, .. } in self.folders() {
            if let Err(refusal) = self.library.scan(&self.store, &root) {
                logs::warn(&format!("the space \"{}\" could not be scanned: {}", space.name, refusal.0));
            }
        }
    }
}

/// Serves on `listener` until `stop` resolves. What an earlier run left in its journals goes to its files
/// before anyone is let in. When the server is told to stop, every session ends and every open document
/// is saved first, and then the requests that are being answered are answered.
pub async fn serve(
    app: Shared,
    listener: tokio::net::TcpListener,
    stop: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let recovering = app.clone();
    tokio::task::spawn_blocking(move || recovering.rooms.recover(&recovering)).await?;
    // The index is brought up to the folders while the first requests are already answered.
    let scanning = app.clone();
    tokio::task::spawn_blocking(move || scanning.scan());
    if app.config.telemetry.enabled && telemetry::here() {
        app.telemetry.start(telemetry::SERVER);
    }
    let (closing, reporting) = (app.clone(), app.clone());
    let stopped = async move {
        stop.await;
        if tokio::task::spawn_blocking(move || closing.rooms.close()).await.is_err() {
            logs::warn("the open documents could not all be saved as the server stopped: their journals keep the rest");
        }
    };
    // A frame that relays what someone typed is small, and is not held back to be sent with the next.
    let listener = listener.tap_io(|socket| drop(socket.set_nodelay(true)));
    // Each request is told the address it came from, which following a link is counted by.
    let routes = api::router(app).into_make_service_with_connect_info::<std::net::SocketAddr>();
    let served = axum::serve(listener, routes).with_graceful_shutdown(stopped).await;
    // Last, and briefly: everything is saved by now.
    reporting.telemetry.stop(std::time::Duration::from_secs(1)).await;
    served
}
