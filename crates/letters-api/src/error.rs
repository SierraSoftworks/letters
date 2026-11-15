use std::fmt;

use serde::{Deserialize, Serialize};

/// Why a request was refused. The status line says the same in HTTP's terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    /// `400`: the request cannot be read, or asks for what cannot be: a name no folder takes, a path that
    /// leaves its space.
    Invalid,
    /// `401`: there is no session, or it has ended: sign in again.
    SignedOut,
    /// `401`: the token has run out: renew it, and ask again.
    Expired,
    /// `403`: the session's role does not allow this.
    Forbidden,
    /// `404`: nothing is there, or nothing this session may know of.
    NotFound,
    /// `409`: a file or folder of that name is in the way.
    NameTaken,
    /// `409`: the document changed since the request was made up: read it again.
    Stale,
    /// `413`: the body is larger than [`Limits`](crate::Limits) allows.
    TooLarge,
    /// `422`: the address may not be fetched: the server's trusted sources and the fetch rules refuse it.
    Untrusted,
    /// `429`: too many requests: wait before the next.
    Slow,
    /// `502`: what the server asked of another host failed: the identity provider, or a page to fetch.
    Upstream,
    /// `503`: the folder behind a space cannot be reached just now.
    Unavailable,
    /// `500`: the server's own fault.
    Internal,
    /// A code this build does not know. Go by the status.
    #[serde(other)]
    Unknown,
}

impl Code {
    /// The HTTP status a refusal with this code is sent under.
    pub fn status(self) -> u16 {
        match self {
            Self::Invalid => 400,
            Self::SignedOut | Self::Expired => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::NameTaken | Self::Stale => 409,
            Self::TooLarge => 413,
            Self::Untrusted => 422,
            Self::Slow => 429,
            Self::Upstream => 502,
            Self::Unavailable => 503,
            Self::Internal | Self::Unknown => 500,
        }
    }
}

/// The body of every refusal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Error {
    pub code: Code,
    /// One sentence a person can be shown.
    pub message: String,
}

impl Error {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}
