//! Opening and saving `.let` files.
//!
//! A file is read and closed at once and replaced by rename, so no handle is ever held open and a reader never
//! sees half a save. Saving runs off the UI thread: a [`Home`] and a document are all it needs.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use letters_format::{Archive, DOCUMENT, Package};
use letters_model::{Block, BlockId, BlockKind, Document};

use crate::platform;

pub const EXTENSION: &str = "let";

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Format(letters_format::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Format(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<letters_format::Error> for Error {
    fn from(error: letters_format::Error) -> Self {
        Self::Format(error)
    }
}

/// A blank page: one empty paragraph to put the caret in.
pub fn blank() -> Document {
    Document {
        blocks: vec![Block::text(BlockId::new(platform::block_id()), BlockKind::Paragraph, "")],
        ..Document::default()
    }
}

/// Where a document lives on disk, and what was there when it was last read or written.
#[derive(Clone, Debug)]
pub struct Home {
    path: PathBuf,
    id: String,
    created: String,
    /// The file's bytes as last read or written, or `None` if it did not exist. A file that no longer matches
    /// was changed by someone else.
    seen: Option<Arc<[u8]>>,
    /// The file whose other entries (style, notes, assets, history) every save carries over.
    carried: Option<Arc<[u8]>>,
}

impl Home {
    /// A home for a new document. The first save replaces whatever `path` holds now.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let seen = fs::read(&path).ok().map(Arc::from);
        Self { path, id: platform::document_id(), created: platform::now_rfc3339(), seen, carried: None }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file name without its extension, which is the document's title.
    pub fn title(&self) -> String {
        self.path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// Writes the document and returns its home from now on.
    ///
    /// If someone else changed the file since it was last read, their version is left alone and this one goes
    /// to a conflict copy beside it, which becomes the document's home.
    pub fn save(&self, document: &Document) -> Result<Home, Error> {
        let on_disk = fs::read(&self.path).ok();
        let changed = on_disk.is_some_and(|bytes| self.seen.as_deref() != Some(bytes.as_slice()));
        let path = if changed { conflict_copy(&self.path) } else { self.path.clone() };

        let mut entries = BTreeMap::new();
        if let Some(carried) = &self.carried {
            let mut archive = Archive::open(Cursor::new(&carried[..]))?;
            let names: Vec<String> = archive.names().filter(|name| *name != DOCUMENT).map(str::to_owned).collect();
            for name in names {
                if let Some(bytes) = archive.read(&name)? {
                    entries.insert(name, bytes);
                }
            }
        }
        let package =
            Package { id: self.id.clone(), created: self.created.clone(), document: document.clone(), entries };
        let mut out = Cursor::new(Vec::new());
        letters_format::write(&mut out, &package)?;
        let bytes: Arc<[u8]> = out.into_inner().into();

        replace(&path, &bytes)?;
        let seen = Some(bytes);
        Ok(Home { path, seen: seen.clone(), carried: seen, ..self.clone() })
    }
}

/// Reads a `.let` file: its document, for the first paint, and the home that later saves go back to.
pub fn open(path: &Path) -> Result<(Document, Home), Error> {
    let bytes: Arc<[u8]> = fs::read(path)?.into();
    let mut archive = Archive::open(Cursor::new(&bytes[..]))?;
    let document = archive.document()?;
    let manifest = archive.manifest();
    let seen = Some(bytes.clone());
    let home = Home {
        path: path.into(),
        id: manifest.id.clone(),
        created: manifest.created.clone(),
        seen,
        carried: Some(bytes),
    };
    Ok((document, home))
}

/// The first free name of the form sync tools give a conflict copy, so that it is recognised as one.
fn conflict_copy(path: &Path) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let extension = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let today = platform::now_rfc3339();
    let today = &today[..10];
    (1..)
        .map(|n| match n {
            1 => format!("{stem} (conflicted copy {today}){extension}"),
            n => format!("{stem} (conflicted copy {today} {n}){extension}"),
        })
        .map(|name| path.with_file_name(name))
        .find(|candidate| !candidate.exists())
        .expect("some numbered name is free")
}

/// Replaces `path` with `bytes` in one step: a flushed temporary sibling, renamed over it.
fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", platform::block_id()));
    let write = || {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        file.write_all(bytes)?;
        if let Ok(existing) = fs::metadata(path) {
            file.set_permissions(existing.permissions())?;
        }
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    };
    write().inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}
