use serde::{Deserialize, Serialize};

/// What someone may do with a document. Each role may do all that the one before it may.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Reader,
    /// Reads, and writes comments.
    Commenter,
    Editor,
    /// Edits, and says who else may. A document in a person's own space is theirs, and one in a shared
    /// space belongs to the space's editors.
    Owner,
}

impl Role {
    pub fn comments(self) -> bool {
        self >= Self::Commenter
    }

    pub fn edits(self) -> bool {
        self >= Self::Editor
    }

    pub fn shares(self) -> bool {
        self == Self::Owner
    }
}

/// Who a document is shared with.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Grantee {
    /// Someone who has signed in to this server, by their [`Person::id`](crate::Person::id).
    Person { id: String, name: String },
    /// Everyone in a group the identity provider names.
    Group { name: String },
}

/// One row of a document's people. An owner's row cannot be taken away.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub to: Grantee,
    pub role: Role,
}

/// What a link lets its reader take away. The server withholds the exports a link does not allow. Not
/// showing a copy command is a deterrent only, since anything displayed can be captured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allows {
    #[serde(default)]
    pub download: bool,
    #[serde(default)]
    pub copy: bool,
    #[serde(default)]
    pub print: bool,
}

/// A link to make. Everything but the label is fixed from here on: another setting is another link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewLink {
    /// Who it is for, such as "Accountant". The access log names the link by it.
    pub label: String,
    /// [`Role::Owner`] is refused.
    pub role: Role,
    /// When it stops working. `None` is never.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<u64>,
    /// Asked of whoever opens the link. The server keeps only a hash of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
    #[serde(default)]
    pub allows: Allows,
}

/// A link as its owner sees it afterwards. Its address is not part of it: the server keeps only a hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    /// Names the link to revoke it. It opens nothing.
    pub id: String,
    pub label: String,
    pub role: Role,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<u64>,
    /// Whether a passphrase is asked.
    #[serde(default)]
    pub passphrase: bool,
    #[serde(default)]
    pub allows: Allows,
    pub created: u64,
    /// When it was revoked. A revoked link stays listed, so that the access log can still name it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked: Option<u64>,
}

/// A link just made: the one time its address is told.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedLink {
    pub link: Link,
    /// The whole address to hand on.
    pub url: String,
}

/// Who reaches a document besides those its space lets in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shares {
    #[serde(default)]
    pub people: Vec<Grant>,
    #[serde(default)]
    pub links: Vec<Link>,
}

/// Who an access was by.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Who {
    Person {
        id: String,
        name: String,
    },
    /// A link's reader, named by the link's label.
    Link {
        id: String,
        label: String,
    },
}

/// What was done, or asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// The document was opened to read.
    Open,
    /// A session of edits began.
    Edit,
    /// A comment was written.
    Comment,
    /// The file or an export of it was taken.
    Download,
    Print,
    /// An action this build does not know.
    #[serde(other)]
    Other,
}

/// One row of a document's access log: an access through a share.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Access {
    pub time: u64,
    pub who: Who,
    pub action: Action,
    /// The role the access was made under.
    pub role: Role,
    /// Whether the server refused it.
    #[serde(default)]
    pub refused: bool,
    /// The network address it came from.
    pub address: String,
    /// The browser or app, as it named itself.
    pub browser: String,
}
