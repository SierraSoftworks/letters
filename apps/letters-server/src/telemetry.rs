//! What the server reports about its own use: that it runs and for how long, and how often its features
//! are asked for.
//!
//! Reports go to [`SERVER`] from release builds, unless the settings say otherwise:
//!
//! ```toml
//! [telemetry]
//! enabled = false
//! ```
//!
//! An [`Event`] is a name and at most one word out of a closed list, all written in this file. Nothing
//! of a document, a person, a group, a link, a space or an address has a field to go in, and the access
//! log is never read here. Beside an event go a random number made for this run and kept nowhere, the
//! version, the system's kind, its language and its time zone. No errors or panics are reported.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tracing_batteries::{Analytics, Session};

use crate::session::{Carrier, Signed};

/// Sierra Softworks' own analytics server, which keeps no address and no identifier.
pub const SERVER: &str = "https://analytics.sierrasoftworks.com";

/// The site every part of Letters reports under, and the one page of it that is the server's: a run is
/// one long visit to it.
pub const SITE: &str = "letters.app";
pub const PAGE: &str = "/server";

/// How a request reached the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Through {
    Browser,
    Desktop,
    /// A share link, by someone who did not sign in.
    Link,
}

impl Through {
    /// Only how `who` came is read, and nothing of who they are.
    pub const fn of(who: &Signed) -> Self {
        match (&who.link, who.carrier) {
            (Some(_), _) => Self::Link,
            (None, Carrier::Cookie) => Self::Browser,
            (None, Carrier::Bearer) => Self::Desktop,
        }
    }
}

/// Something that was asked of the server. The whole of what can be reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    SignedIn(Through),
    DocumentMade,
    /// Someone began to read or edit a document as others do.
    SyncJoined(Through),
    ShareChanged,
    LinkMade,
    LinkRevoked,
    ConflictsMerged,
}

impl Event {
    pub const fn name(self) -> &'static str {
        match self {
            Self::SignedIn(_) => "signed_in",
            Self::DocumentMade => "document_made",
            Self::SyncJoined(_) => "sync_joined",
            Self::ShareChanged => "share_changed",
            Self::LinkMade => "link_made",
            Self::LinkRevoked => "link_revoked",
            Self::ConflictsMerged => "conflicts_merged",
        }
    }

    /// The one thing said beside the name, where there is one.
    pub const fn property(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::SignedIn(through) | Self::SyncJoined(through) => Some((
                "through",
                match through {
                    Through::Browser => "browser",
                    Through::Desktop => "desktop",
                    Through::Link => "link",
                },
            )),
            _ => None,
        }
    }
}

/// Whether this run may report at all, whatever its settings say: a release build, with neither
/// `LETTERS_TELEMETRY=0` nor `DO_NOT_TRACK=1` around it. Builds for development and their tests stay quiet.
pub fn here() -> bool {
    let said = |name: &str, word: &str| std::env::var_os(name).is_some_and(|value| value == word);
    !cfg!(debug_assertions) && !said("LETTERS_TELEMETRY", "0") && !said("DO_NOT_TRACK", "1")
}

/// The reports of one run of the server. Until [`Telemetry::start`] there is no session, and every report
/// is dropped.
#[derive(Default)]
pub struct Telemetry {
    session: Mutex<Option<Session>>,
}

impl Telemetry {
    /// Begins the session, which says that a server started. Called on the runtime the server answers on:
    /// each report is sent as a task of its own there, and no request waits for one.
    pub fn start(&self, server: &str) {
        let battery =
            Analytics::new(server.to_owned()).with_hostname(SITE).with_initial_page(PAGE).with_panic_capture(false);
        let session =
            Session::new("letters-server", env!("CARGO_PKG_VERSION")).with_debug_builds().with_battery(battery);
        *self.session.lock().unwrap_or_else(PoisonError::into_inner) = Some(session);
    }

    pub fn record(&self, event: Event) {
        if let Some(session) = &*self.session.lock().unwrap_or_else(PoisonError::into_inner) {
            let property = event.property().map(|(key, value)| (key.to_owned(), value.to_owned()));
            session.record_event(event.name(), property.into_iter().collect::<HashMap<_, _>>());
        }
    }

    /// Ends the session, which says how long the server ran, waiting no longer than `within` for what is
    /// still on its way. A report that fails is lost without a word.
    pub async fn stop(&self, within: Duration) {
        let session = self.session.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(session) = session {
            let _ = tokio::time::timeout(within, tokio::task::spawn_blocking(move || session.shutdown())).await;
        }
    }
}
