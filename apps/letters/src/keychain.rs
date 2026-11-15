//! A person's sign-in to a Letters server, kept by the system's keychain.
//!
//! A sign-in opens every document its person may open on that server. So it is kept where the system
//! keeps passwords and nowhere else: not in the settings, not in a file of Letters' own, and never in
//! anything that is printed or logged. Nothing here is on the way to a first window: the keychain is
//! asked only when a server is.

use std::fmt;
use std::sync::Arc;

use keyring_core::{Entry, Error};

/// The name the system's keychain files Letters' sign-ins under.
const SERVICE: &str = "Letters";

/// Why a sign-in was not kept or could not be read. It never says anything of the sign-in itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unkept {
    /// The system has no keychain to ask, as a Linux desktop without a Secret Service has none.
    NoKeychain,
    /// The keychain is locked, or will not let Letters in.
    Locked,
    /// The keychain failed. The reason is its own wording, for whoever debugs.
    Failed(String),
}

impl fmt::Display for Unkept {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoKeychain => write!(f, "this system has no keychain to keep a sign-in in"),
            Self::Locked => write!(f, "the system's keychain is locked"),
            Self::Failed(_) => write!(f, "the system's keychain could not be used"),
        }
    }
}

impl std::error::Error for Unkept {}

impl From<Error> for Unkept {
    fn from(error: Error) -> Self {
        match error {
            Error::NoDefaultStore => Self::NoKeychain,
            Error::NoStorageAccess(_) => Self::Locked,
            // These hold what was kept, which is not to be written out anywhere.
            Error::BadEncoding(_) | Error::BadDataFormat(..) | Error::Ambiguous(_) => {
                Self::Failed("what was kept could not be read".to_owned())
            }
            other => Self::Failed(other.to_string()),
        }
    }
}

/// The keychain sign-ins are kept in. One for the whole app.
#[derive(Debug)]
pub struct Keychain(());

impl Keychain {
    /// The system's own: Keychain on macOS, Credential Manager on Windows, the Secret Service elsewhere.
    /// Where there is none, a sign-in lasts only as long as the app is open.
    pub fn system() -> Result<Self, Unkept> {
        match keyring::Entry::store_status() {
            Ok(()) => Ok(Self(())),
            Err(Error::NoStorageAccess(_)) => Err(Unkept::Locked),
            Err(_) => Err(Unkept::NoKeychain),
        }
    }

    /// One that keeps sign-ins in memory until the app ends, in place of the system's. For tests, which
    /// must not write in the keychain of whoever runs them.
    pub fn in_memory() -> Result<Self, Unkept> {
        let store: Arc<keyring_core::CredentialStore> = keyring_core::mock::Store::new()?;
        keyring_core::set_default_store(store);
        Ok(Self(()))
    }

    /// The sign-in kept for the server at `server`, its address as `https://host`. `None` when nobody
    /// signed in there, or signed out since.
    pub fn kept(&self, server: &str) -> Result<Option<String>, Unkept> {
        match Entry::new(SERVICE, server)?.get_password() {
            Ok(sign_in) => Ok(Some(sign_in)),
            Err(Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Keeps `sign_in` for the server, in place of any kept for it before.
    pub fn keep(&self, server: &str, sign_in: &str) -> Result<(), Unkept> {
        Ok(Entry::new(SERVICE, server)?.set_password(sign_in)?)
    }

    /// Forgets the sign-in kept for the server, as signing out does. Forgetting one that is not kept is
    /// no failure.
    pub fn forget(&self, server: &str) -> Result<(), Unkept> {
        match Entry::new(SERVICE, server)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}
