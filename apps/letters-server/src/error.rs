//! Every refusal, as the interface words it.

use std::io;

use axum::Json;
use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use letters_api::{Code, Error};

use crate::store::StoreError;

/// A request that was refused. It is answered with its [`Error`] under the status of its code, and with
/// the seconds to wait before asking again, where the refusal knows them.
#[derive(Debug)]
pub struct Refusal(pub Error, Option<u32>);

pub type Answer<T> = Result<T, Refusal>;

impl Refusal {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self(Error::new(code, message), None)
    }

    /// Too much was asked at once. In `seconds` it is worth asking again, which the answer says as
    /// `Retry-After`, for a page that asks by itself.
    pub fn slow(seconds: u64, message: impl Into<String>) -> Self {
        Self(Error::new(Code::Slow, message), Some(u32::try_from(seconds.max(1)).unwrap_or(u32::MAX)))
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(Code::Invalid, message)
    }

    /// Nothing is there, or nothing the session may know of: the two are told the same way, so that an
    /// answer never says what exists.
    pub fn not_found() -> Self {
        Self::new(Code::NotFound, "There is nothing here, or nothing you can open.")
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(Code::Forbidden, message)
    }

    pub fn signed_out() -> Self {
        Self::new(Code::SignedOut, "Sign in to carry on.")
    }

    pub fn code(&self) -> Code {
        self.0.code
    }

    /// The seconds to wait before asking again, where the refusal knows them.
    pub fn wait(&self) -> Option<u32> {
        self.1
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let Self(error, wait) = self;
        let status = StatusCode::from_u16(error.code.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut response = (status, Json(error)).into_response();
        if let Some(seconds) = wait {
            response.headers_mut().insert(RETRY_AFTER, HeaderValue::from(seconds));
        }
        response
    }
}

/// A folder's refusal. What the person asked for decides the words, so only the kind is kept: the system's
/// own message names paths, which are never told.
impl From<io::Error> for Refusal {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => Self::not_found(),
            io::ErrorKind::AlreadyExists => {
                Self::new(Code::NameTaken, "A document or folder with that name already exists.")
            }
            // Only a folder's move says so: a document is copied across.
            io::ErrorKind::CrossesDevices => {
                Self::invalid("A folder cannot be moved to a space on another disk. Move its documents instead.")
            }
            _ => {
                crate::logs::warn(&format!("a folder refused: {error}"));
                Self::new(Code::Unavailable, "The folder behind this space cannot be used just now.")
            }
        }
    }
}

impl From<StoreError> for Refusal {
    fn from(error: StoreError) -> Self {
        crate::logs::warn(&error.to_string());
        Self::new(Code::Internal, "The server could not read its own records.")
    }
}

impl From<letters_format::Error> for Refusal {
    fn from(error: letters_format::Error) -> Self {
        match error {
            letters_format::Error::Files(why) => {
                crate::logs::warn(&format!("a folder refused: {why}"));
                Self::new(Code::Unavailable, "The folder behind this space cannot be used just now.")
            }
            error => Self::new(Code::Invalid, format!("The file cannot be read as a document: {error}.")),
        }
    }
}
