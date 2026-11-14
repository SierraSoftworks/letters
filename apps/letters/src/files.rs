//! Opening and saving `.let` files.
//!
//! A file is read and closed at once, and saved the way `letters_format::save` lays down: merged with what is
//! on disk, written beside it and renamed over it. Saving runs off the UI thread: a [`Home`] and a document
//! are all it needs.

use std::fmt;
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use letters_format::{Archive, DOCUMENT, DiskCopy, Files, Package, Replica, Save, Saved, Stamp};
use letters_model::{Block, BlockId, BlockKind, Document, Style};

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

/// The style a new document starts with: the user's own default, kept in `.letters/style.json` in their home
/// folder as on the server, or the built-in style when they have none that can be read.
pub fn new_style(home_folder: Option<&Path>) -> Style {
    let json = home_folder.and_then(|home| fs::read_to_string(home.join(".letters").join("style.json")).ok());
    json.and_then(|json| Style::from_json(&json).ok()).unwrap_or_default()
}

/// Where a document lives on disk, and what was there when it was last read or written.
#[derive(Clone, Debug)]
pub struct Home {
    path: PathBuf,
    id: String,
    created: String,
    /// The hash of the `document.json` last read from or written to the file, or `None` if there was no file.
    seen: Option<String>,
    /// Whether the first save replaces whatever the path holds, because the user chose to.
    replace: bool,
    /// The style a new document is first written with. `None` once the file has a style of its own.
    style: Option<Style>,
}

impl Home {
    /// A home for a new document in `style`. The first save replaces whatever `path` holds now.
    pub fn new(path: impl Into<PathBuf>, style: Style) -> Self {
        let (id, created) = (platform::document_id(), platform::now_rfc3339());
        Self { path: path.into(), id, created, seen: None, replace: true, style: Some(style) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file name without its extension, which is the document's title.
    pub fn title(&self) -> String {
        self.path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// Writes the document and returns its home from now on, with how the save ended.
    ///
    /// If someone else changed the file since it was last read, their version is left alone and this one goes
    /// to a conflict copy beside it, which becomes the document's home.
    pub fn save(&self, document: &Document) -> Result<(Home, Saved), Error> {
        let name = self.path.file_name().unwrap_or_default().to_string_lossy();
        let mut folder = Folder(folder_of(&self.path).to_owned());
        if self.replace {
            match folder.remove(&name) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error.into()),
                _ => {}
            }
        }

        let mut package = Package {
            id: self.id.clone(),
            created: self.created.clone(),
            document: document.clone(),
            entries: Default::default(),
        };
        if let Some(style) = &self.style {
            package.set_style(style);
        }
        let written = letters_format::hash(document.to_json().as_bytes());
        let mut replica = Snapshot { package, seen: self.seen.clone() };
        let today = &platform::now_rfc3339()[..10];
        let save = Save { name: &name, token: &platform::block_id(), label: today, attempts: 3 };
        let saved = letters_format::save(&mut folder, &mut replica, &save)?;

        let path = match &saved {
            Saved::ConflictCopy { name, .. } => self.path.with_file_name(name),
            _ => self.path.clone(),
        };
        Ok((Home { path, seen: Some(written), replace: false, style: None, ..self.clone() }, saved))
    }
}

/// The document in memory at the moment of a save.
struct Snapshot {
    package: Package,
    seen: Option<String>,
}

impl Replica for Snapshot {
    fn id(&self) -> &str {
        &self.package.id
    }

    /// Until the model can merge, the only file memory may replace is the one it last saw.
    fn merge(&mut self, disk: &mut DiskCopy) -> Result<(), letters_format::Error> {
        if disk.manifest().hashes.get(DOCUMENT) == self.seen.as_ref() {
            return Ok(());
        }
        // The conflict copy this leads to should still be a whole document, so it takes the file's other
        // entries with it.
        let names: Vec<String> = disk.names().filter(|name| *name != DOCUMENT).map(str::to_owned).collect();
        for name in names {
            if let Some(bytes) = disk.read(&name)? {
                self.package.entries.entry(name).or_insert(bytes);
            }
        }
        Err(letters_format::Error::Unmerged)
    }

    fn package(&self) -> Package {
        self.package.clone()
    }
}

/// The folder a file is in, which for a bare file name is the current one.
fn folder_of(path: &Path) -> &Path {
    path.parent().filter(|folder| !folder.as_os_str().is_empty()).unwrap_or(Path::new("."))
}

/// A real folder, for `letters_format::save`.
struct Folder(PathBuf);

/// `None` for a file that is not there.
fn found<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

impl Files for Folder {
    fn stamp(&mut self, name: &str) -> io::Result<Option<Stamp>> {
        let Some(metadata) = found(fs::metadata(self.0.join(name)))? else { return Ok(None) };
        let modified = metadata.modified()?.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos() as u64);
        Ok(Some(Stamp { len: metadata.len(), modified }))
    }

    fn read(&mut self, name: &str) -> io::Result<Option<Vec<u8>>> {
        found(fs::read(self.0.join(name)))
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let mut file = fs::File::create(self.0.join(name))?;
        file.write_all(bytes)?;
        file.sync_all()
    }

    fn rename(&mut self, from: &str, to: &str) -> io::Result<()> {
        let (from, to) = (self.0.join(from), self.0.join(to));
        // The new file takes over the permissions of the one it replaces.
        if let Ok(replaced) = fs::metadata(&to) {
            fs::set_permissions(&from, replaced.permissions())?;
        }
        fs::rename(from, to)
    }

    fn remove(&mut self, name: &str) -> io::Result<()> {
        fs::remove_file(self.0.join(name))
    }

    fn wait(&mut self, retry: u32) {
        thread::sleep(Duration::from_millis(50) * retry);
    }
}

/// A document ready to show, and where its saves go.
#[derive(Clone, Debug)]
pub struct Opened {
    pub document: Document,
    pub style: Style,
    pub home: Home,
}

/// Reads what the first paint needs from a `.let` file: its document and its style.
///
/// A path with no file yet opens as a blank document in `new_style` that will be saved there.
pub fn open(path: &Path, new_style: impl FnOnce() -> Style) -> Result<Opened, Error> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        // A file that is missing from a folder that exists is a new document. A missing folder is a mistake.
        Err(error) if error.kind() == io::ErrorKind::NotFound && folder_of(path).is_dir() => {
            let style = new_style();
            let home = Home { replace: false, ..Home::new(path, style.clone()) };
            return Ok(Opened { document: blank(), style, home });
        }
        Err(error) => return Err(error.into()),
    };
    let mut archive = Archive::open(Cursor::new(bytes))?;
    let document = archive.document()?;
    // A style this build cannot read stays in the file untouched, and the built-in one is shown.
    let style = archive.style().unwrap_or_default();
    let manifest = archive.manifest();
    let seen = manifest.hashes.get(DOCUMENT).cloned();
    let home = Home {
        path: path.into(),
        id: manifest.id.clone(),
        created: manifest.created.clone(),
        seen,
        replace: false,
        style: None,
    };
    Ok(Opened { document, style, home })
}
