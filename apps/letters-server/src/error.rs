//! Every refusal, as the interface words it.

use std::io;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use letters_api::{Code, Error};

use crate::store::StoreError;

/// A request that was refused. It is answered with its [`Error`] under the status of its code.
#[derive(Debug)]
pub struct Refusal(pub Error);

pub type Answer<T> = Result<T, Refusal>;

impl Refusal {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self(Error::new(code, message))
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
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.code.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(self.0)).into_response()
    }
}

/// A folder's refusal. What the person asked for decides the words, so only the kind is kept: the system's
/// own message names paths, which are never told.
impl From<io::Error> for Refusal {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => Self::not_found(),
            io::ErrorKind::AlreadyExists => Self::new(Code::NameTaken, "Something of that name is already there."),
            _ => {
                crate::log(&format!("a folder refused: {error}"));
                Self::new(Code::Unavailable, "The folder behind this space cannot be used just now.")
            }
        }
    }
}

impl From<StoreError> for Refusal {
    fn from(error: StoreError) -> Self {
        crate::log(&error.to_string());
        Self::new(Code::Internal, "The server could not read its own records.")
    }
}

impl From<letters_format::Error> for Refusal {
    fn from(error: letters_format::Error) -> Self {
        match error {
            letters_format::Error::Files(why) => {
                crate::log(&format!("a folder refused: {why}"));
                Self::new(Code::Unavailable, "The folder behind this space cannot be used just now.")
            }
            error => Self::new(Code::Invalid, format!("The file cannot be read as a document: {error}.")),
        }
    }
}
