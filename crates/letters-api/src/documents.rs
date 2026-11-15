use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Allows, Role};

/// Whose a space is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceKind {
    /// The person's own, made when they first signed in.
    Personal,
    /// A folder the server's owner named, which groups of people reach.
    Shared,
}

/// A named folder of documents. Where it is kept is the server's business and is never told.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Space {
    /// Names the space in an address.
    pub id: String,
    pub name: String,
    pub kind: SpaceKind,
    /// What the session may do with the documents in it.
    pub role: Role,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Folder,
    Document,
}

/// What a file table marks an entry with beside its name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Tag {
    /// A copy a sync tool left when two devices wrote at once, of the document at this path in the same
    /// space. It waits to be merged.
    ConflictCopy { of: String },
    /// Import notes nobody has settled yet.
    ImportNotes { count: u32 },
    /// Who has it open to edit now, by the names they sign with.
    Editing { people: Vec<String> },
    /// Shared with this many people or groups beyond its space. Told to whoever says who it is shared with.
    Shared { with: u32 },
    /// This many links lead to it.
    Links { count: u32 },
    /// The file could not be read as a document.
    Damaged,
    /// A tag this build does not know.
    #[serde(other)]
    Unknown,
}

/// A folder or a document in a space.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub kind: EntryKind,
    /// The name to show: a document's without its `.let`.
    pub name: String,
    /// The [`Space::id`] it is in. `None` for a document that reaches the session only through a share,
    /// which is opened by its id alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    /// Where it is in its space: the names of its folders and its own file name, joined by `/`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// A document's id, which every copy of it shares and a rename or a move leaves alone. `None` for a
    /// folder, and for a file that could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// When the document was made, as its manifest records it in RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// When its file was last written.
    pub modified: u64,
    /// What the session may do with it.
    pub role: Role,
    /// Everyone who has edited the document, read from its history.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contributors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
}

/// What a folder holds: its folders first, then its documents, each by name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    pub space: Space,
    /// The folder listed, as [`Entry::path`] writes it. Empty for the space itself.
    #[serde(default)]
    pub path: String,
    pub entries: Vec<Entry>,
}

/// A document to start, sent to [`paths::documents`](crate::paths::documents).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewDocument {
    /// The folder to start it in, as [`Entry::path`] writes it.
    #[serde(default)]
    pub folder: String,
    /// `None` is "Untitled", until the document's first heading names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A folder to make, sent to [`paths::folders`](crate::paths::folders).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewFolder {
    #[serde(default)]
    pub parent: String,
    pub name: String,
}

/// A new name for a document or a folder, as it is shown. Where a file of that name is in the way the
/// answer is [`Code::NameTaken`](crate::Code::NameTaken).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rename {
    pub name: String,
}

/// Another folder for a document or a folder, in its own space or another the session may edit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Move {
    pub space: String,
    #[serde(default)]
    pub folder: String,
}

/// How a document's remote loads are treated for this session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    Trusted,
    /// It reads and edits as normal. Remote pictures, links and merges are held back until the person
    /// trusts it.
    Safe,
}

/// A document ready to edit: what a shell needs besides the file to open it and join its room.
///
/// A page the server composes for a browser carries it beside `document.json`, with `style` and `notes`
/// filled in, so that the editor starts without a request. The desktop app asks for it at
/// [`paths::document`](crate::paths::document) and for the file at [`paths::file`](crate::paths::file).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Opened {
    pub id: String,
    /// As the manifest records it, in RFC 3339.
    pub created: String,
    /// The name to show.
    pub name: String,
    /// Whether the name is the document's own. One started without a name is called "Untitled" until
    /// its first heading names it, which a shell does at [`paths::rename`](crate::paths::rename).
    #[serde(default = "yes")]
    pub named: bool,
    /// Where it is, for a session that reaches it through a space: see [`Entry::space`] and [`Entry::path`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    pub role: Role,
    /// The name this session's edits are signed with.
    pub author: String,
    /// The address of the document's socket, from the server's root. The replica that joins is new for
    /// every session, and is kept nowhere.
    pub socket: String,
    pub trust: Trust,
    /// What may be taken away. Everything, but for a link's reader.
    pub allows: Allows,
    /// The document's `style.json`. In a page only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<Value>,
    /// The document's `notes.json`. In a page only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<Value>,
}

fn yes() -> bool {
    true
}

/// An address for the server to fetch on a document's behalf: a page a browser may not read itself, sent
/// to [`paths::fetch_preview`](crate::paths::fetch_preview), or a picture, sent to
/// [`paths::fetch_picture`](crate::paths::fetch_picture) and answered with its bytes.
///
/// The server fetches only what its own trusted sources and the fetch rules of safe mode allow, and sends
/// nothing of the session along.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fetch {
    pub url: String,
    /// The person asked for this one themselves. Otherwise only a trusted source is fetched, as when a
    /// document opens.
    #[serde(default)]
    pub asked: bool,
}

/// What the page behind a link says of itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preview {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The whole address of the page's picture, to fetch in turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
}
