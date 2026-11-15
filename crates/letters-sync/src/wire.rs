//! The messages, and their bytes.
//!
//! A frame is a kind byte and then that kind's fields in the order they are declared. A number is a
//! little-endian `u32` or `u64`, and text or bytes are their length as a `u32` and then the bytes. A
//! reader skips a kind it does not know and ignores bytes after the fields it knows, so a later build may
//! add kinds and fields. [`PROTOCOL`] rises only when an older build could no longer follow.

use std::fmt;
use std::str::FromStr;

use letters_model::{Extent, Notes, Revision, Style};

use crate::access::{Refusal, Role};
use crate::presence::{Passage, Peer};

/// The protocol this build speaks, as [`Hello`] and [`Welcome`] state it.
pub const PROTOCOL: u32 = 1;

/// The longest frame either side sends or takes, in bytes: what a server takes as one message unless it
/// was set up otherwise. The history of a long document is sent whole to a device that has none of it,
/// and fits many times over.
pub const MAX_FRAME: usize = 8 << 20;

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

// A device's kinds count up from 1 and the server's from 65, so neither side takes the other's for its own.
const HELLO: u8 = 1;
const EDITS: u8 = 2;
const NOTES: u8 = 3;
const STYLE: u8 = 4;
const SELECTION: u8 = 5;
const PING: u8 = 6;

const WELCOME: u8 = 65;
const HELD: u8 = 66;
const REFUSED: u8 = 67;
const EDITS_ELSEWHERE: u8 = 68;
const NOTES_ELSEWHERE: u8 = 69;
const JOINED: u8 = 70;
const SELECTION_ELSEWHERE: u8 = 71;
const LEFT: u8 = 72;
const PONG: u8 = 73;
const GOODBYE: u8 = 74;

impl ClientMessage {
    /// The frame that carries this message.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Hello(hello) => {
                let mut frame = Writer::new(HELLO);
                frame.u32(hello.protocol);
                frame.text(&hello.document);
                frame.bytes(&hello.extent.to_bytes());
                frame.option(hello.revision.as_ref(), |frame, revision| frame.text(&revision.to_string()));
                frame.0
            }
            Self::Edits { seq, updates } => {
                let mut frame = Writer::new(EDITS);
                frame.u64(*seq);
                frame.bytes(updates);
                frame.0
            }
            Self::Notes { seq, notes } => {
                let mut frame = Writer::new(NOTES);
                frame.u64(*seq);
                frame.text(&notes.to_json());
                frame.0
            }
            Self::Style { seq, style } => {
                let mut frame = Writer::new(STYLE);
                frame.u64(*seq);
                frame.text(&style.to_json());
                frame.0
            }
            Self::Selection(selection) => {
                let mut frame = Writer::new(SELECTION);
                frame.option(selection.as_ref(), Writer::passage);
                frame.0
            }
            Self::Ping => Writer::new(PING).0,
        }
    }

    /// Reads a frame. `None` for a kind of message this build does not know, which is skipped.
    pub fn decode(frame: &[u8]) -> Result<Option<Self>, WireError> {
        let (kind, mut frame) = Reader::open(frame)?;
        Ok(Some(match kind {
            HELLO => Self::Hello(Hello {
                protocol: frame.u32()?,
                document: frame.text()?.to_owned(),
                extent: frame.extent()?,
                revision: frame.option(|frame| frame.parsed("a revision"))?,
            }),
            EDITS => Self::Edits { seq: frame.u64()?, updates: frame.bytes()?.to_vec() },
            NOTES => Self::Notes { seq: frame.u64()?, notes: frame.notes()? },
            STYLE => {
                let (seq, style) = (frame.u64()?, Style::from_json(frame.text()?));
                Self::Style { seq, style: style.map_err(|_| WireError::Malformed("a style"))? }
            }
            SELECTION => Self::Selection(frame.option(Reader::passage)?),
            PING => Self::Ping,
            _ => return Ok(None),
        }))
    }
}

impl ServerMessage {
    /// The frame that carries this message.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Welcome(welcome) => {
                let mut frame = Writer::new(WELCOME);
                frame.u32(welcome.protocol);
                frame.u32(welcome.session.0);
                frame.u8(role_byte(welcome.role));
                frame.text(&welcome.document);
                frame.text(&welcome.created);
                frame.bytes(&welcome.updates);
                frame.bytes(&welcome.extent.to_bytes());
                frame.text(&welcome.notes.to_json());
                frame.u32(welcome.peers.len() as u32);
                for peer in &welcome.peers {
                    // Each with its length, so that a later build may tell more of a peer.
                    let mut inner = Writer(Vec::new());
                    inner.peer(peer);
                    frame.bytes(&inner.0);
                }
                frame.0
            }
            Self::Held { seq } => {
                let mut frame = Writer::new(HELD);
                frame.u64(*seq);
                frame.0
            }
            Self::Refused { seq, why } => {
                let mut frame = Writer::new(REFUSED);
                frame.u64(*seq);
                frame.u8(refusal_byte(*why));
                frame.0
            }
            Self::Edits { updates } => {
                let mut frame = Writer::new(EDITS_ELSEWHERE);
                frame.bytes(updates);
                frame.0
            }
            Self::Notes { notes } => {
                let mut frame = Writer::new(NOTES_ELSEWHERE);
                frame.text(&notes.to_json());
                frame.0
            }
            Self::Joined(peer) => {
                let mut frame = Writer::new(JOINED);
                frame.peer(peer);
                frame.0
            }
            Self::Selection { session, selection } => {
                let mut frame = Writer::new(SELECTION_ELSEWHERE);
                frame.u32(session.0);
                frame.option(selection.as_ref(), Writer::passage);
                frame.0
            }
            Self::Left { session } => {
                let mut frame = Writer::new(LEFT);
                frame.u32(session.0);
                frame.0
            }
            Self::Pong => Writer::new(PONG).0,
            Self::Goodbye { why } => {
                let mut frame = Writer::new(GOODBYE);
                frame.u8(refusal_byte(*why));
                frame.0
            }
        }
    }

    /// Reads a frame. `None` for a kind of message this build does not know, which is skipped.
    pub fn decode(frame: &[u8]) -> Result<Option<Self>, WireError> {
        let (kind, mut frame) = Reader::open(frame)?;
        Ok(Some(match kind {
            WELCOME => Self::Welcome(Welcome {
                protocol: frame.u32()?,
                session: SessionId(frame.u32()?),
                role: role(frame.u8()?),
                document: frame.text()?.to_owned(),
                created: frame.text()?.to_owned(),
                updates: frame.bytes()?.to_vec(),
                extent: frame.extent()?,
                notes: frame.notes()?,
                peers: {
                    let count = frame.u32()?;
                    let peer = |_| Reader(frame.bytes()?).peer();
                    (0..count).map(peer).collect::<Result<_, _>>()?
                },
            }),
            HELD => Self::Held { seq: frame.u64()? },
            REFUSED => Self::Refused { seq: frame.u64()?, why: refusal(frame.u8()?) },
            EDITS_ELSEWHERE => Self::Edits { updates: frame.bytes()?.to_vec() },
            NOTES_ELSEWHERE => Self::Notes { notes: frame.notes()? },
            JOINED => Self::Joined(frame.peer()?),
            SELECTION_ELSEWHERE => {
                Self::Selection { session: SessionId(frame.u32()?), selection: frame.option(Reader::passage)? }
            }
            LEFT => Self::Left { session: SessionId(frame.u32()?) },
            PONG => Self::Pong,
            GOODBYE => Self::Goodbye { why: refusal(frame.u8()?) },
            _ => return Ok(None),
        }))
    }
}

fn role_byte(role: Role) -> u8 {
    match role {
        Role::Reader => 0,
        Role::Commenter => 1,
        Role::Editor => 2,
    }
}

/// A role this build does not know may do the least.
fn role(byte: u8) -> Role {
    match byte {
        1 => Role::Commenter,
        2 => Role::Editor,
        _ => Role::Reader,
    }
}

/// Every refusal this build knows, with its number on the wire. A number is never given to another.
const REFUSALS: [(u8, Refusal); 11] = [
    (1, Refusal::NoAccess),
    (2, Refusal::SignedOut),
    (3, Refusal::Gone),
    (4, Refusal::TooOld),
    (5, Refusal::TooNew),
    (6, Refusal::OtherDocument),
    (7, Refusal::ReadOnly),
    (8, Refusal::NoComments),
    (9, Refusal::Unreadable),
    (10, Refusal::AccessChanged),
    (11, Refusal::TooLarge),
];

fn refusal_byte(why: Refusal) -> u8 {
    match why {
        Refusal::Other(byte) => byte,
        why => REFUSALS.iter().find(|(_, known)| *known == why).map_or(0, |(byte, _)| *byte),
    }
}

fn refusal(byte: u8) -> Refusal {
    REFUSALS.iter().find(|(known, _)| *known == byte).map_or(Refusal::Other(byte), |(_, why)| *why)
}

/// A frame as it is written: the kind, then each field.
struct Writer(Vec<u8>);

impl Writer {
    fn new(kind: u8) -> Self {
        Self(vec![kind])
    }

    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn bytes(&mut self, bytes: &[u8]) {
        // A field longer than a frame may be makes a frame that no reader takes, which is the right end.
        self.u32(u32::try_from(bytes.len()).unwrap_or(u32::MAX));
        self.0.extend_from_slice(bytes);
    }

    fn text(&mut self, text: &str) {
        self.bytes(text.as_bytes());
    }

    fn option<T>(&mut self, value: Option<T>, write: impl FnOnce(&mut Self, T)) {
        self.u8(u8::from(value.is_some()));
        if let Some(value) = value {
            write(self, value);
        }
    }

    fn passage(&mut self, passage: &Passage) {
        self.text(&passage.anchor.to_string());
        self.text(&passage.head.to_string());
    }

    fn peer(&mut self, peer: &Peer) {
        self.u32(peer.session.0);
        self.text(&peer.name);
        self.u8(peer.colour);
        self.option(peer.selection.as_ref(), Self::passage);
    }
}

/// What is left of a frame as it is read.
struct Reader<'a>(&'a [u8]);

type Read<T> = Result<T, WireError>;

impl<'a> Reader<'a> {
    /// The frame's kind, and its fields.
    fn open(frame: &'a [u8]) -> Read<(u8, Self)> {
        if frame.len() > MAX_FRAME {
            return Err(WireError::TooLong);
        }
        let (kind, fields) = frame.split_first().ok_or(WireError::Malformed("an empty frame"))?;
        Ok((*kind, Self(fields)))
    }

    fn take(&mut self, len: usize) -> Read<&'a [u8]> {
        let (taken, rest) = self.0.split_at_checked(len).ok_or(WireError::Malformed("the frame ends early"))?;
        self.0 = rest;
        Ok(taken)
    }

    fn u8(&mut self) -> Read<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Read<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four bytes were taken")))
    }

    fn u64(&mut self) -> Read<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("eight bytes were taken")))
    }

    fn bytes(&mut self) -> Read<&'a [u8]> {
        let len = self.u32()?;
        self.take(len as usize)
    }

    fn text(&mut self) -> Read<&'a str> {
        std::str::from_utf8(self.bytes()?).map_err(|_| WireError::Malformed("text that is not UTF-8"))
    }

    /// Text that reads as a `T`, such as a revision or a stable position.
    fn parsed<T: FromStr>(&mut self, what: &'static str) -> Read<T> {
        self.text()?.parse().map_err(|_| WireError::Malformed(what))
    }

    fn option<T>(&mut self, read: impl FnOnce(&mut Self) -> Read<T>) -> Read<Option<T>> {
        match self.u8()? {
            0 => Ok(None),
            _ => read(self).map(Some),
        }
    }

    fn extent(&mut self) -> Read<Extent> {
        Extent::from_bytes(self.bytes()?).map_err(|_| WireError::Malformed("an extent"))
    }

    fn notes(&mut self) -> Read<Notes> {
        Notes::from_json(self.text()?).map_err(|_| WireError::Malformed("notes"))
    }

    fn passage(&mut self) -> Read<Passage> {
        Ok(Passage { anchor: self.parsed("a position")?, head: self.parsed("a position")? })
    }

    fn peer(&mut self) -> Read<Peer> {
        Ok(Peer {
            session: SessionId(self.u32()?),
            name: self.text()?.to_owned(),
            colour: self.u8()?,
            selection: self.option(Self::passage)?,
        })
    }
}
