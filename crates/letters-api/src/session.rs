use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What [`paths::SERVER`](crate::paths::SERVER) answers to anyone: that a Letters server is here, and what
/// it holds its callers to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    /// Always [`Server::PRODUCT`], which tells a Letters server from whatever else answers JSON here.
    pub product: String,
    /// The server's name, as its people know it.
    pub name: String,
    /// The build that answers, such as `0.1.0`.
    pub version: String,
    /// Every [`PROTOCOL`](crate::PROTOCOL) the server answers.
    pub protocols: Vec<u32>,
    /// Whether documents can be shared by link here. It is off unless the server's owner turns it on.
    #[serde(default)]
    pub links: bool,
    /// How many days a row of a document's access log is kept. 0 from a server that does not say.
    #[serde(default)]
    pub log_days: u32,
    #[serde(default)]
    pub limits: Limits,
}

impl Server {
    pub const PRODUCT: &str = "letters-server";

    /// Whether a shell built for [`PROTOCOL`](crate::PROTOCOL) can talk to this server.
    pub fn speaks(&self) -> bool {
        self.product == Self::PRODUCT && self.protocols.contains(&crate::PROTOCOL)
    }
}

/// The most a server takes, in bytes. A larger body is refused as [`Code::TooLarge`](crate::Code::TooLarge).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// A whole `.let` file.
    pub document: u64,
    /// One asset of a document: a picture.
    pub asset: u64,
    /// One message on a document's socket.
    pub message: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { document: 256 << 20, asset: 32 << 20, message: 8 << 20 }
    }
}

/// Someone the identity provider signed in.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Person {
    /// Stable for the person's life at the provider. Not for show.
    pub id: String,
    /// The name they sign in with, which names their own space.
    pub username: String,
    /// The name to show, and the one their edits are signed with unless they set another.
    pub name: String,
}

/// What the desktop app sends to [`paths::EXCHANGE`](crate::paths::EXCHANGE) once the browser has come
/// back to it: the code the server handed its loopback address, and the verifier whose hash it began with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exchange {
    pub code: String,
    pub verifier: String,
}

/// What the desktop app sends to [`paths::RENEW`](crate::paths::RENEW).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Renewal {
    pub renew: String,
}

/// A session for the desktop app. `token` goes in an `Authorization: Bearer` header on every request and
/// on a socket's upgrade.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub token: String,
    /// When `token` stops working. Renew before then.
    pub expires: u64,
    /// Trades for a new `Token` at [`paths::RENEW`](crate::paths::RENEW), once: the answer holds the next.
    pub renew: String,
    /// When `renew` stops working, and the person signs in through the browser again.
    pub ends: u64,
    pub person: Person,
}

/// Whether the person reads on a light or a dark window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    Light,
    Dark,
}

/// What a person set for themselves, kept by the server so that every device has it. Each is `None` or
/// empty until they set it. A `PUT` replaces the whole, but for the words of their dictionary.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// The name their edits are signed with, in place of [`Person::name`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The language of their new documents, as a BCP 47 tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// `None` follows the device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<Appearance>,
    /// The accent as sRGB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<[u8; 3]>,
    /// The words they taught the spell checker. A `PUT` leaves them as they are, whatever it sends here:
    /// they change a word at a time ([`paths::word`](crate::paths::word)), so that a device with an old
    /// list cannot bring back a word another one took out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dictionary: Vec<String>,
    /// The `style.json` their new documents start with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_style: Option<Value>,
}
