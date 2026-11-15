//! Realtime sync: how a document that is open on several devices stays one document.
//!
//! Every device holds a full replica and trades what it lacks with the server over one WebSocket per
//! document. A frame is one binary message of the socket and holds one [`ClientMessage`] or
//! [`ServerMessage`], at most [`MAX_FRAME`] bytes long.
//!
//! - [`Client`] is a device's side: it says when to open the socket, what to send and what to merge, and
//!   keeps what was written offline until the server holds it.
//! - [`Room`] is the server's side of one open document: its replica in memory, a journal of every change
//!   on local disk, and the file, rewritten through [`letters_format::save`] when the edits pause.
//! - [`Role`] says what a session may send, and the room refuses the rest with a [`Refusal`].
//!
//! # What travels
//!
//! - **The history.** Edits, the style and version marks are one CRDT history. Each side tells the other
//!   its [`Extent`](letters_model::Extent) in the handshake and is sent only what it lacks, so a device
//!   that was away catches up by what changed. Only an [`Role::Editor`] may send any.
//! - **Notes.** Comments and the other notes of `notes.json` travel whole, one message for the notes that
//!   changed, and join as [`Notes::merge`](letters_model::Notes::merge) joins two files. A
//!   [`Role::Commenter`] may add comments, reply to them and resolve them, and nothing else.
//! - **Presence.** Who is here and where their caret is. It is passed on and never kept: not in the
//!   journal, not in the file.
//!
//! Pictures do not travel here. A history names its assets by hash, and a device fetches the ones it
//! lacks from the server, and hands over new ones, apart from the socket.
//!
//! # What meets
//!
//! Edits from sessions meet in the room as the history merges them, character by character, and no one
//! is asked anything. What another writer put into the file meets them in the room's save, as two files
//! meet: a word that both changed reads as the later change left it, and a conflict note keeps both for
//! someone to pick from. The note and the settled text go to every session.
//!
//! Nothing in this crate opens a socket, reads a file or asks the time. The shell and the server's host
//! bring all three.

mod access;
mod client;
mod presence;
mod room;
mod wire;

pub use access::{Access, Refusal, Role};
pub use client::{Client, Event, Link, Local, Progress};
pub use presence::{Passage, Peer};
pub use room::{LOOK_EVERY, Outbound, Room, SAVE_AFTER, SAVE_WITHIN};
pub use wire::{ClientMessage, Hello, MAX_FRAME, PROTOCOL, ServerMessage, SessionId, Welcome, WireError};

/// A session that has sent nothing for this long sends a [`ClientMessage::Ping`], in milliseconds.
pub const PING_AFTER: u64 = 20_000;
/// A side that has heard nothing for this long takes the socket for dead and closes it, in milliseconds.
pub const SILENT_AFTER: u64 = 60_000;
