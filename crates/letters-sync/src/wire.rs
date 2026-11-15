//! The messages, and their bytes.
//!
//! A frame is a kind byte and then that kind's fields in the order they are declared. A number is a
//! little-endian `u32` or `u64`, and text or bytes are their length as a `u32` and then the bytes. A
//! reader skips a kind it does not know and ignores bytes after the fields it knows, so a later build may
//! add kinds and fields. [`PROTOCOL`] rises only when an older build could no longer follow.

use std::fmt;

use letters_model::{Extent, Notes, Revision, Style};

use crate::access::{Refusal, Role};
use crate::presence::{Passage, Peer};

/// The protocol this build speaks, as [`Hello`] and [`Welcome`] state it.
pub const PROTOCOL: u32 = 1;

/// The longest frame either side sends or takes, in bytes. The history of a long document is sent whole
/// to a device that has none of it, and fits many times over.
pub const MAX_FRAME: usize = 32 << 20;

/// One connection to a room. The room numbers them, and others know a person's caret by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub u32);

/// What a device sends.
///
/// `seq` numbers what the device has handed over, and never falls. The server answers each message that
/// carries one with [`ServerMessage::Held`] or [`ServerMessage::Refused`].
#[derive(Clone, Debug, PartialEq)]
pub enum ClientMessage {
    /// The first message on every socket.
    Hello(Hello),
    /// Edits the server lacks, as [`Model::updates_for`](letters_model::Model::updates_for) wrote them.
    Edits { seq: u64, updates: Vec<u8> },
    /// The notes that were added or changed here, each whole. The server joins them into its own.
    Notes { seq: u64, notes: Notes },
    /// The document's style as it was set here. Fields that were left alone stay open to other writers.
    Style { seq: u64, style: Style },
    /// Where this person's caret is now. `None` takes it off the page.
    Selection(Option<Passage>),
    /// Sent after [`PING_AFTER`](crate::PING_AFTER) with nothing else to say. Answered with a pong.
    Ping,
}

/// How a device opens a session.
#[derive(Clone, Debug, PartialEq)]
pub struct Hello {
    pub protocol: u32,
    /// The id of the document the device means, as its manifest has it. Empty for a device that holds
    /// no copy yet and takes whichever document the socket leads to.
    pub document: String,
    /// What the device's copy holds, so that the server sends only the rest.
    pub extent: Extent,
    /// The revision the device's copy shows, for the server to tell what was written apart. `None`
    /// for a device that holds no copy yet.
    pub revision: Option<Revision>,
}

/// What the server sends.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerMessage {
    /// The answer to [`Hello`].
    Welcome(Welcome),
    /// Everything the device sent up to `seq` is kept on the server's disk.
    Held { seq: u64 },
    /// The message numbered `seq` was not taken. The session goes on.
    Refused { seq: u64, why: Refusal },
    /// Edits made elsewhere: by another session, or in the file by another writer.
    Edits { updates: Vec<u8> },
    /// Notes that were added or changed elsewhere, each whole.
    Notes { notes: Notes },
    /// Someone opened the document.
    Joined(Peer),
    /// Someone's caret moved, or left the page.
    Selection { session: SessionId, selection: Option<Passage> },
    /// Someone closed the document.
    Left { session: SessionId },
    /// The answer to [`ClientMessage::Ping`].
    Pong,
    /// The session is over, and the server closes the socket.
    Goodbye { why: Refusal },
}

/// How the server opens a session.
#[derive(Clone, Debug, PartialEq)]
pub struct Welcome {
    pub protocol: u32,
    /// This session, as the others know it.
    pub session: SessionId,
    pub role: Role,
    /// The document's id and when it was created, as RFC 3339, for a device that starts its copy here.
    pub document: String,
    pub created: String,
    /// What the device lacks of the history, as [`Model::updates_for`](letters_model::Model::updates_for)
    /// wrote it.
    pub updates: Vec<u8>,
    /// What the server holds, so that the device sends only the rest.
    pub extent: Extent,
    /// Every note the server holds.
    pub notes: Notes,
    /// Everyone else who has the document open.
    pub peers: Vec<Peer>,
}

/// Why a frame could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireError {
    /// Longer than [`MAX_FRAME`].
    TooLong,
    /// The frame ends before its fields do, or a field does not hold what it should.
    Malformed(&'static str),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong => f.write_str("the frame is longer than a frame may be"),
            Self::Malformed(what) => write!(f, "the frame cannot be read: {what}"),
        }
    }
}

impl std::error::Error for WireError {}

impl ClientMessage {
    /// The frame that carries this message.
    pub fn encode(&self) -> Vec<u8> {
        todo!()
    }

    /// Reads a frame. `None` for a kind of message this build does not know, which is skipped.
    pub fn decode(frame: &[u8]) -> Result<Option<Self>, WireError> {
        let _ = frame;
        todo!()
    }
}

impl ServerMessage {
    /// The frame that carries this message.
    pub fn encode(&self) -> Vec<u8> {
        todo!()
    }

    /// Reads a frame. `None` for a kind of message this build does not know, which is skipped.
    pub fn decode(frame: &[u8]) -> Result<Option<Self>, WireError> {
        let _ = frame;
        todo!()
    }
}
