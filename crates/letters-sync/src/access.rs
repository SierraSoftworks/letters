//! Who may send what. The server's host decides a session's role, and the room holds every message to it.

use std::fmt;

use letters_model::{Note, NoteKind, Notes};

/// What a session may do to the document. Each role may do all that the one before it may.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// Reads, and is seen reading.
    Reader,
    /// Also adds comments, replies to them and resolves them.
    Commenter,
    /// Also edits the text, the style, the versions and every note.
    Editor,
}

impl Role {
    /// Whether the role may change the document's history: its text, style and versions.
    pub fn edits(self) -> bool {
        self == Self::Editor
    }

    /// Whether the role may write comments.
    pub fn comments(self) -> bool {
        self >= Self::Commenter
    }
}

/// A session as the server's host admits it to a room. The host has already checked who this is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    pub role: Role,
    /// The name the others see beside this session's caret.
    pub name: String,
}

/// Why the server did not take a message, or ended a session. Each reads as a sentence for the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Refusal {
    /// The session may not open this document, or may no longer.
    NoAccess,
    /// The person's sign-in ended. Signing in again starts a new session.
    SignedOut,
    /// The document was moved or deleted on the server.
    Gone,
    /// The server no longer speaks this build's protocol.
    TooOld,
    /// The server speaks an older protocol than this build.
    TooNew,
    /// The file the server holds under this name is another document than the copy here.
    OtherDocument,
    /// An edit from a session that may not edit. The edit stays on the device that made it.
    ReadOnly,
    /// A comment from a session that may only read, or a change to a note that a commenter may not make.
    NoComments,
    /// A message that could not be read, or one that came before the handshake.
    Unreadable,
    /// The session's role changed. The device joins again at once, and is told its new role.
    AccessChanged,
    /// What one side has for the other does not fit in a frame. The device's work stays in its own
    /// file, which the server can take whole.
    TooLarge,
    /// A reason this build does not know, by its number on the wire.
    Other(u8),
}

impl Refusal {
    /// Whether opening the socket again as things are would end the same way, so that the client waits
    /// for the person instead of trying again.
    pub fn is_final(self) -> bool {
        use Refusal::*;
        matches!(self, NoAccess | SignedOut | Gone | TooOld | TooNew | OtherDocument | TooLarge)
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoAccess => "You do not have access to this document.",
            Self::SignedOut => "You are signed out. Sign in again to carry on.",
            Self::Gone => "The document was moved or deleted on the server.",
            Self::TooOld => "This version of Letters is too old for the server. Update Letters to carry on.",
            Self::TooNew => "The server is too old for this version of Letters.",
            Self::OtherDocument => "The server holds a different document under this name.",
            Self::ReadOnly => "You can read this document, but not change it.",
            Self::NoComments => "You can read this document, but not comment on it.",
            Self::Unreadable => "The server could not read what this device sent.",
            Self::AccessChanged => "Your access to this document changed.",
            Self::TooLarge => "The changes are too large to send in one piece.",
            Self::Other(_) => "The server ended the session.",
        })
    }
}

/// What a commenter may change in the notes the room holds, out of the notes their session sent: a new
/// comment whole, and of a comment that is held its new replies and whether it is resolved. Anything else
/// the same note says differently is left out, as a note reads differently on a device while edits travel.
///
/// Refused when it would add a note that is no comment, or answer or resolve one.
pub(crate) fn comments(held: &Notes, sent: &Notes) -> Result<Notes, Refusal> {
    let mut allowed = Notes::default();
    for note in &sent.notes {
        match held.notes.iter().find(|held| held.id == note.id) {
            None if note.kind == NoteKind::Comment => allowed.notes.push(note.clone()),
            None => return Err(Refusal::NoComments),
            Some(held) => {
                let adds =
                    note.resolved && !held.resolved || note.replies.iter().any(|reply| !held.replies.contains(reply));
                if adds && held.kind != NoteKind::Comment {
                    return Err(Refusal::NoComments);
                }
                if adds {
                    allowed.notes.push(Note { replies: note.replies.clone(), resolved: note.resolved, ..held.clone() });
                }
            }
        }
    }
    Ok(allowed)
}
