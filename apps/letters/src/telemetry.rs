//! What Letters reports about its own use: which features are used, how many sessions there are and how
//! long they last.
//!
//! Nothing of a document or a person can be said here. An [`Event`] is a name and values out of closed
//! lists, all written in this file: there is no field a name, a title, a path, an address or any typed
//! text could be put in. Errors and panics are not reported, since their messages can quote text.
//!
//! Reports go to [`SERVER`] from release builds alone. Beside what is said here, each carries a random
//! number made for this run and kept nowhere, the version, the system's kind, its language and its time
//! zone. The person turns it off in Settings, with `LETTERS_TELEMETRY=0`, with `DO_NOT_TRACK=1`, or with this in `~/.letters/settings.toml`:
//!
//! ```toml
//! [telemetry]
//! enabled = false
//! ```
//!
//! `noticed = true` beside it says that the landing card has told the person of the reports once.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread;
use std::time::Duration;

use std::io;
use std::path::Path;

use letters_trust::Store as _;
use letters_ui::{CopyFormat, Mode, Request};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::trust::Settings;
use tracing_batteries::{Analytics, Session};

/// Where reports go: Sierra Softworks' own analytics server, which keeps no address and no identifier.
pub const SERVER: &str = "https://analytics.sierrasoftworks.com";

/// The site every part of Letters reports under. The desktop app's pages are those under `/desktop`,
/// the server's under `/server`.
pub const SITE: &str = "letters.app";

/// Whether this run may report at all, whatever the person set: a release build, with neither
/// `LETTERS_TELEMETRY=0` nor `DO_NOT_TRACK=1` around it. Builds for development and their tests stay quiet.
pub fn here() -> bool {
    let said = |name: &str, word: &str| std::env::var_os(name).is_some_and(|value| value == word);
    !cfg!(debug_assertions) && !said("LETTERS_TELEMETRY", "0") && !said("DO_NOT_TRACK", "1")
}

/// How many reports may wait for the thread. More than that are dropped: a report is never worth a wait.
const WAITING: usize = 64;

/// A part of the window the person is in. The time in each is what a session's length is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Landing,
    Editor,
    History,
    Review,
    Print,
}

impl Page {
    pub const ALL: [Self; 5] = [Self::Landing, Self::Editor, Self::History, Self::Review, Self::Print];

    /// The page for what the window shows.
    pub const fn of(mode: Mode) -> Self {
        match mode {
            Mode::Landing => Self::Landing,
            Mode::Editor => Self::Editor,
            Mode::History => Self::History,
            Mode::Review => Self::Review,
            Mode::Print => Self::Print,
        }
    }

    pub const fn path(self) -> &'static str {
        match self {
            Self::Landing => "/desktop/landing",
            Self::Editor => "/desktop/editor",
            Self::History => "/desktop/history",
            Self::Review => "/desktop/review",
            Self::Print => "/desktop/print",
        }
    }
}

/// How a document came to be in a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    New,
    /// One the window listed: a recent document, or one of a folder.
    Listed,
    /// One the person went and found.
    Chosen,
}

/// How far a grant of trust reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Document,
    Folder,
    Host,
}

/// What the other side of a merge is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Merged {
    Copies,
    File,
    Original,
}

/// Something the person did. The whole of what can be reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    DocumentOpened(Source),
    DocumentImported,
    /// A copy was saved, as a document or as another kind of file.
    DocumentCopied(CopyFormat),
    DocumentPrinted,
    PictureAdded {
        linked: bool,
    },
    MergeAsked(Merged),
    TrustGranted(Scope),
    SignInStarted,
    ShareChanged,
    LinkMade,
    LinkRevoked,
    SettingsChanged,
}

impl Event {
    /// What a request of the window's says of how Letters is used. Only which request it is counts:
    /// whatever it carries, a name or a path or an address, is not looked at.
    pub const fn asked(request: &Request) -> Option<Self> {
        Some(match request {
            Request::SaveCopy { format, .. } | Request::SaveVersion { format, .. } => Self::DocumentCopied(*format),
            Request::Import { .. } => Self::DocumentImported,
            Request::Print => Self::DocumentPrinted,
            Request::Pictures => Self::PictureAdded { linked: false },
            Request::LinkedPicture(_) => Self::PictureAdded { linked: true },
            Request::MergeConflicts { confirmed: false } => Self::MergeAsked(Merged::Copies),
            Request::MergeFrom { confirmed: false, .. } => Self::MergeAsked(Merged::File),
            Request::MergeIntoOriginal { confirmed: false } => Self::MergeAsked(Merged::Original),
            Request::TrustDocument => Self::TrustGranted(Scope::Document),
            Request::TrustFolder => Self::TrustGranted(Scope::Folder),
            Request::TrustPictures(_) => Self::TrustGranted(Scope::Host),
            Request::SignIn { .. } => Self::SignInStarted,
            Request::SharePeople(_) => Self::ShareChanged,
            Request::MakeLink(_) => Self::LinkMade,
            Request::RevokeLink(_) => Self::LinkRevoked,
            Request::Settings(_) => Self::SettingsChanged,
            _ => return None,
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::DocumentOpened(_) => "document_opened",
            Self::DocumentImported => "document_imported",
            Self::DocumentCopied(_) => "document_copied",
            Self::DocumentPrinted => "document_printed",
            Self::PictureAdded { .. } => "picture_added",
            Self::MergeAsked(_) => "merge_asked",
            Self::TrustGranted(_) => "trust_granted",
            Self::SignInStarted => "sign_in_started",
            Self::ShareChanged => "share_changed",
            Self::LinkMade => "link_made",
            Self::LinkRevoked => "link_revoked",
            Self::SettingsChanged => "settings_changed",
        }
    }

    /// The one thing said beside the name, where there is one.
    pub const fn property(self) -> Option<(&'static str, &'static str)> {
        Some(match self {
            Self::DocumentOpened(source) => (
                "source",
                match source {
                    Source::New => "new",
                    Source::Listed => "listed",
                    Source::Chosen => "chosen",
                },
            ),
            Self::DocumentCopied(format) => (
                "format",
                match format {
                    CopyFormat::Letters => "letters",
                    CopyFormat::Clean => "clean",
                    CopyFormat::Word => "word",
                    CopyFormat::Pdf => "pdf",
                    CopyFormat::WebPage => "html",
                    CopyFormat::Markdown => "markdown",
                },
            ),
            Self::PictureAdded { linked } => ("kind", if linked { "linked" } else { "embedded" }),
            Self::MergeAsked(from) => (
                "from",
                match from {
                    Merged::Copies => "copies",
                    Merged::File => "file",
                    Merged::Original => "original",
                },
            ),
            Self::TrustGranted(scope) => (
                "scope",
                match scope {
                    Scope::Document => "document",
                    Scope::Folder => "folder",
                    Scope::Host => "host",
                },
            ),
            Self::DocumentImported
            | Self::DocumentPrinted
            | Self::SignInStarted
            | Self::ShareChanged
            | Self::LinkMade
            | Self::LinkRevoked
            | Self::SettingsChanged => return None,
        })
    }
}

/// Whether the settings file's text lets Letters report. Only a plain `enabled = false` under
/// `[telemetry]` turns it off.
pub fn allowed(settings: &str) -> bool {
    let Ok(file) = settings.parse::<DocumentMut>() else { return true };
    file.get("telemetry").and_then(|table| table.get("enabled")).and_then(|enabled| enabled.as_bool()) != Some(false)
}

/// Whether the settings file's text says that the person was told of the reports.
pub fn noticed(settings: &str) -> bool {
    let Ok(file) = settings.parse::<DocumentMut>() else { return false };
    file.get("telemetry").and_then(|table| table.get("noticed")).and_then(|noticed| noticed.as_bool()) == Some(true)
}

/// The settings file's text with `key` under `[telemetry]` set, and all else as its writer had it. `None`
/// for text that is no TOML, which nothing could be written into without losing what its writer meant.
pub fn written(settings: &str, key: &str, set: bool) -> Option<String> {
    let mut file = settings.parse::<DocumentMut>().ok()?;
    let table = file.entry("telemetry").or_insert_with(|| Item::Table(Table::new()));
    table.as_table_like_mut()?.insert(key, value(set));
    Some(file.to_string())
}

/// Keeps `key` under `[telemetry]` in the settings of the person whose home folder is `home`.
pub fn keep(home: &Path, key: &str, set: bool) -> io::Result<()> {
    let mut file = Settings::of(home);
    let text = file.read()?.unwrap_or_default();
    let text = written(&text, key, set).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no TOML"))?;
    file.write(&text)
}

enum Said {
    Event(Event),
    Page(Page),
    Allowed(bool),
    Stop(Sender<()>),
}

/// The reports of one run of Letters. Everything is handed to a thread of its own, so no call here waits
/// for the network, or for anything else.
pub struct Telemetry {
    said: Option<SyncSender<Said>>,
}

impl Telemetry {
    /// Reports nothing.
    pub const fn off() -> Self {
        Self { said: None }
    }

    /// Reports to the server at `server`, from a thread started here. With `allowed` unset nothing is
    /// sent until [`Telemetry::allow`] says otherwise.
    pub fn start(server: impl Into<String>, allowed: bool) -> Self {
        let server = server.into();
        let (said, waiting) = mpsc::sync_channel(WAITING);
        let started = thread::Builder::new().name("telemetry".into()).spawn(move || report(&server, allowed, &waiting));
        Self { said: started.ok().map(|_| said) }
    }

    pub fn record(&self, event: Event) {
        self.say(Said::Event(event));
    }

    /// The person is now in `page`.
    pub fn page(&self, page: Page) {
        self.say(Said::Page(page));
    }

    /// The person turned reporting on or off.
    pub fn allow(&self, allowed: bool) {
        self.say(Said::Allowed(allowed));
    }

    /// Ends the session, waiting no longer than `within` for what is still on its way.
    pub fn stop(self, within: Duration) {
        let (done, finished) = mpsc::channel();
        if let Some(said) = &self.said
            && said.try_send(Said::Stop(done)).is_ok()
        {
            let _ = finished.recv_timeout(within);
        }
    }

    fn say(&self, said: Said) {
        if let Some(waiting) = &self.said {
            // A full queue or a thread that is gone loses the report, and nothing else.
            let _ = waiting.try_send(said);
        }
    }
}

/// The thread's work: one session, fed until it is told to stop or its `Telemetry` is gone.
fn report(server: &str, allowed: bool, waiting: &Receiver<Said>) {
    // The sends run on this runtime's own thread, so they go on while this one waits for the next report.
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build() else {
        return;
    };
    let _entered = runtime.enter();
    let battery =
        Analytics::new(server.to_owned()).with_hostname(SITE).without_initial_page().with_panic_capture(false);
    let metadata = Session::new("letters", env!("CARGO_PKG_VERSION")).with_debug_builds();
    let session = metadata.with_battery(battery);
    let switch = session.enable();
    switch.store(allowed, Ordering::Relaxed);
    let mut stopped = None;
    while let Ok(said) = waiting.recv() {
        match said {
            Said::Event(event) => {
                let properties = event.property().map(|(key, value)| (key.to_owned(), value.to_owned()));
                session.record_event(event.name(), properties.into_iter().collect::<HashMap<_, _>>());
            }
            // The marker would put the page before back when dropped, and there is no page before.
            Said::Page(page) => std::mem::forget(session.record_new_page(page.path())),
            Said::Allowed(allowed) => switch.store(allowed, Ordering::Relaxed),
            Said::Stop(done) => {
                stopped = Some(done);
                break;
            }
        }
    }
    session.shutdown();
    if let Some(done) = stopped {
        let _ = done.send(());
    }
}
