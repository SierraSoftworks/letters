//! What a Letters server and the shells say to each other over HTTP: the addresses, the bodies and the
//! refusals. The server answers with these types and the shells read them, so neither can drift.
//!
//! - [`paths`] names every address.
//! - [`Server`] is what a shell reads first, with no session: that this is a Letters server, and its limits.
//! - [`Token`] and [`Person`] are a session. A browser holds it in a cookie, the desktop app as a bearer.
//! - [`Space`], [`Listing`] and [`Entry`] are the documents a person reaches, and [`Opened`] one of them
//!   ready to edit.
//! - [`Shares`], [`Link`] and [`Access`] are who else reaches a document, and who did.
//! - [`Error`] is every refusal: a [`Code`] to act on and a sentence to show.
//!
//! Bodies are JSON. A reader ignores a field it does not know, and a field that says nothing is left out,
//! so either side can be the newer one. Times are seconds since 1970, but a document's `created`, which is
//! the RFC 3339 text its manifest holds. A document's own parts (`document.json`, `style.json`,
//! `notes.json`) travel as the JSON they are, unread here.

mod documents;
mod error;
pub mod paths;
mod session;
mod sharing;

pub use documents::{
    Entry, EntryKind, Fetch, Listing, Move, NewDocument, NewFolder, Opened, Preview, Rename, Seen, Space, SpaceKind,
    Tag, Trashed, Trust,
};
pub use error::{Code, Error};
pub use session::{Appearance, Exchange, Limits, Person, Renewal, Server, Settings, Token};
pub use sharing::{Access, Action, Allows, CreatedLink, Grant, Grantee, Link, NewLink, Role, Shares, Via, Who};

/// The version of this interface. It rises only for a change an older shell could not live with, and a
/// server names every version it still answers in [`Server::protocols`].
pub const PROTOCOL: u32 = 1;

/// The media type of every body but a file's.
pub const JSON: &str = "application/json";
