//! Keeping the user's trust in a file the shell lends.

use std::{fmt, io};

use crate::file::{self, DOCUMENTS, FOLDERS, IMPORTED, SOURCES};
use crate::path::Path;
use crate::{Source, Trust};

/// The user's settings file, as the shell lends it: a file on the desktop, a row per user on the server.
///
/// The file may hold other settings too. Only its `[trust]` table is read and changed here.
pub trait Store {
    /// The file's text. `None` when there is no file yet.
    fn read(&mut self) -> io::Result<Option<String>>;
    /// Replaces the file's text.
    fn write(&mut self, text: &str) -> io::Result<()>;
}

#[derive(Debug)]
pub enum TrustError {
    /// The store could not read or write the file.
    Store(String),
    /// The file is not TOML. It is left as it is, and nothing in it is trusted.
    Toml(String),
    /// The file's `trust` table, or this list in it, holds something else.
    Shape(&'static str),
    /// A document needs an id to be trusted.
    NoDocument,
    /// Not a folder that can be trusted: a relative path, one that climbs with `..`, or a whole disk.
    Folder(String),
}

impl fmt::Display for TrustError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(message) => write!(f, "the settings file could not be read or written: {message}"),
            Self::Toml(message) => write!(f, "the settings file is not valid TOML: {message}"),
            Self::Shape("trust") => write!(f, "`trust` in the settings file is not a table"),
            Self::Shape(key) => write!(f, "`trust.{key}` in the settings file is not a list"),
            Self::NoDocument => write!(f, "the document has no id"),
            Self::Folder(path) => write!(f, "`{path}` is not a folder that can be trusted"),
        }
    }
}

impl std::error::Error for TrustError {}

/// What the user trusts, kept in step with the settings file.
pub struct TrustStore<S> {
    store: S,
    home: Option<String>,
    trust: Trust,
}

impl<S: Store> TrustStore<S> {
    /// Trusts nothing until [`load`](Self::load) has read the file. `home` is the user's home folder.
    pub fn new(store: S, home: Option<&str>) -> Self {
        Self { store, home: home.map(String::from), trust: Trust::empty(home) }
    }

    /// Reads the file again. On an error what was trusted before stays, which at first is nothing.
    pub fn load(&mut self) -> Result<(), TrustError> {
        let text = self.read()?;
        self.trust = Trust::parse(&text, self.home.as_deref())?;
        Ok(())
    }

    pub fn trust(&self) -> &Trust {
        &self.trust
    }

    /// "Trust this document". Also called for a document the user creates, which starts trusted.
    pub fn trust_document(&mut self, id: &str) -> Result<(), TrustError> {
        if id.is_empty() {
            return Err(TrustError::NoDocument);
        }
        self.change(DOCUMENTS, true, |list| file::add(list, id, |entry| entry == id))
    }

    pub fn forget_document(&mut self, id: &str) -> Result<(), TrustError> {
        self.change(DOCUMENTS, false, |list| file::remove(list, |entry| entry == id))
    }

    /// Records that the user made this document from another file, as an import does. From then on no
    /// folder vouches for it, wherever it is kept: it is trusted once the user trusts the document itself.
    pub fn import_document(&mut self, id: &str) -> Result<(), TrustError> {
        if id.is_empty() {
            return Err(TrustError::NoDocument);
        }
        self.change(IMPORTED, true, |list| file::add(list, id, |entry| entry == id))
    }

    /// "Trust this folder", and every folder inside it. `path` is absolute.
    pub fn trust_folder(&mut self, path: &str) -> Result<(), TrustError> {
        let folder = self.folder(path)?;
        let written = folder.written(self.home.as_deref());
        let home = self.home.clone();
        let is = |entry: &str| Path::parse(entry, home.as_deref()).is_some_and(|entry| entry.is(&folder));
        self.change(FOLDERS, true, |list| file::add(list, &written, is))
    }

    pub fn forget_folder(&mut self, path: &str) -> Result<(), TrustError> {
        let folder = self.folder(path)?;
        let home = self.home.clone();
        let is = |entry: &str| Path::parse(entry, home.as_deref()).is_some_and(|entry| entry.is(&folder));
        self.change(FOLDERS, false, |list| file::remove(list, is))
    }

    /// "Always load from this host", or a source added in settings.
    pub fn trust_source(&mut self, source: &Source) -> Result<(), TrustError> {
        let is = |entry: &str| Source::parse(entry).is_ok_and(|entry| entry == *source);
        self.change(SOURCES, true, |list| file::add(list, &source.to_string(), is))
    }

    pub fn forget_source(&mut self, source: &Source) -> Result<(), TrustError> {
        let is = |entry: &str| Source::parse(entry).is_ok_and(|entry| entry == *source);
        self.change(SOURCES, false, |list| file::remove(list, is))
    }

    fn folder(&self, path: &str) -> Result<Path, TrustError> {
        Path::parse(path, self.home.as_deref()).filter(Path::is_folder).ok_or_else(|| TrustError::Folder(path.into()))
    }

    fn read(&mut self) -> Result<String, TrustError> {
        let text = self.store.read().map_err(|error| TrustError::Store(error.to_string()))?;
        Ok(text.unwrap_or_default())
    }

    fn change(
        &mut self,
        key: &'static str,
        create: bool,
        change: impl FnOnce(&mut toml_edit::Array),
    ) -> Result<(), TrustError> {
        // Start from the file as it is now: another window, or the user, may have changed it since.
        let before = self.read()?;
        let after = file::edit(&before, key, create, change)?;
        if after != before {
            self.store.write(&after).map_err(|error| TrustError::Store(error.to_string()))?;
        }
        self.trust = Trust::parse(&after, self.home.as_deref())?;
        Ok(())
    }
}
