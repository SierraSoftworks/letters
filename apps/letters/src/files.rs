//! Opening and saving `.let` files.
//!
//! A file is read and closed at once, and saved the way `letters_format::save` lays down: merged with what is
//! on disk, written beside it and renamed over it. Saving runs off the UI thread: a [`Home`] and the
//! document's working copy are all it needs.

use std::fmt;
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use letters_format::{Archive, Files, Save, Saved, Stamp, WorkingCopy};
use letters_model::{Block, BlockId, BlockKind, Document, Model, Prepared, ReplicaId, Revision, Style};

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

/// The working copy of a new document in `style`, which no file holds yet.
pub fn new_copy(document: Document, style: Style) -> WorkingCopy {
    let replica = ReplicaId::new(&platform::block_id());
    let mut copy = WorkingCopy::new(platform::document_id(), platform::now_rfc3339(), document, replica);
    copy.style = style;
    copy
}

/// Where a document lives on disk.
#[derive(Clone, Debug)]
pub struct Home {
    path: PathBuf,
    /// Whether the document was ever read from the path or written to it.
    written: bool,
}

impl Home {
    /// A home for a document that has no file yet. A file that turns up at `path` before the first save is
    /// left alone, as one changed by someone else is.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), written: false }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file name without its extension, which is the document's title.
    pub fn title(&self) -> String {
        self.path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// Writes `copy` to the file and returns the document's home from now on, with how the save ended.
    ///
    /// What someone else wrote to the file since it was last read is merged into `copy` first. A file that
    /// cannot be merged is left alone: the document goes to a conflict copy beside it, which becomes its home.
    pub fn save(&self, copy: &mut WorkingCopy) -> Result<(Home, Saved), Error> {
        let name = self.path.file_name().unwrap_or_default().to_string_lossy();
        let mut folder = Folder(folder_of(&self.path).to_owned());
        let today = &platform::now_rfc3339()[..10];
        let save = Save { name: &name, token: &platform::block_id(), label: today, attempts: 3 };
        let saved = letters_format::save(&mut folder, copy, &save)?;
        let path = match &saved {
            Saved::ConflictCopy { name, .. } => self.path.with_file_name(name),
            _ => self.path.clone(),
        };
        Ok((Home { path, written: true }, saved))
    }

    /// One save from an editor: takes its `edits` into `copy`, writes the file, and returns the home with
    /// what the file held that the editor, at `revision`, has not seen. The editor merges that.
    pub fn exchange(
        &self,
        copy: &mut WorkingCopy,
        edits: &[u8],
        revision: &Revision,
    ) -> Result<(Home, Option<Vec<u8>>), Error> {
        copy.edit(edits)?;
        let ours = copy.model().revision();
        let (home, _) = self.save(copy)?;
        let news = (copy.model().revision() != ours).then(|| copy.model().updates_since(revision));
        Ok((home, news))
    }

    /// Moves the document's file to `to` and returns its home there. Whatever `to` holds is never replaced:
    /// the move fails with `AlreadyExists` instead. No save of this home may be running.
    pub fn move_to(&self, to: &Path) -> io::Result<Home> {
        match self.written {
            true => relocate(&self.path, to)?,
            // Nothing is on disk yet, so there is only a path to change.
            false if to.symlink_metadata().is_ok() => return Err(io::ErrorKind::AlreadyExists.into()),
            false => {}
        }
        Ok(Home { path: to.to_owned(), ..self.clone() })
    }
}

/// Moves a file to a path that must be free. A file already at `to` is never replaced, as a plain rename
/// would: the move fails with `AlreadyExists` instead.
pub fn relocate(from: &Path, to: &Path) -> io::Result<()> {
    // A change of letter case alone names the same file where the file system ignores case.
    let same_name = from.file_name().zip(to.file_name()).is_some_and(|(from, to)| from.eq_ignore_ascii_case(to));
    if same_name && from != to && fs::canonicalize(from).is_ok_and(|file| fs::canonicalize(to).ok() == Some(file)) {
        return fs::rename(from, to);
    }
    // A link fails where a file exists, and appears whole or not at all.
    match fs::hard_link(from, to) {
        Ok(()) => return fs::remove_file(from),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Err(error),
        // Another disk, or a file system without links: copy instead.
        Err(_) => {}
    }
    let mut copy = fs::OpenOptions::new().write(true).create_new(true).open(to)?;
    let copied = fs::File::open(from)
        .and_then(|mut file| {
            io::copy(&mut file, &mut copy)?;
            copy.set_permissions(file.metadata()?.permissions())?;
            copy.sync_all()
        })
        .and_then(|()| fs::remove_file(from));
    if copied.is_err() {
        // The original is still whole, so the half-made copy must not pass for the document.
        let _ = fs::remove_file(to);
    }
    copied
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
#[derive(Debug)]
pub struct Opened {
    pub document: Document,
    pub style: Style,
    pub home: Home,
    pub rest: Rest,
}

/// What opening leaves for after the first paint: the file's history, and whatever else a save needs.
pub struct Rest(Source);

enum Source {
    /// The file as it was read.
    File(Vec<u8>),
    /// No file yet.
    New(Box<WorkingCopy>),
}

impl fmt::Debug for Rest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            Source::File(_) => "Rest(File)",
            Source::New(_) => "Rest(New)",
        })
    }
}

/// What saving a document needs: its working copy, and the history for the editor that shows it.
pub struct Working {
    pub copy: WorkingCopy,
    /// For `Editor::attach`. `None` when the editor's own history is already the document's.
    pub history: Option<Prepared>,
}

impl Rest {
    /// The rest of a new `document` in `style`, which is nothing to read.
    pub fn new(document: &Document, style: &Style) -> Self {
        Self(Source::New(Box::new(new_copy(document.clone(), style.clone()))))
    }

    /// Reads it. `editor` is the replica the document's editor was made with.
    ///
    /// This is the slow part of opening a long document, so it runs on another thread once the document
    /// is on screen.
    pub fn read(self, editor: ReplicaId) -> Result<Working, Error> {
        let bytes = match self.0 {
            Source::File(bytes) => bytes,
            Source::New(copy) => return Ok(Working { copy: *copy, history: None }),
        };
        let mut archive = Archive::open(Cursor::new(bytes))?;
        let copy = WorkingCopy::open(&mut archive, ReplicaId::new(&platform::block_id()))?;
        // The editor takes the history as the copy holds it, so a document changed outside Letters is one
        // change in both and not one in each.
        let model = copy.model();
        let history = Model::prepare(model.doc().clone(), Some(&model.save()), editor);
        Ok(Working { history: Some(history.map_err(letters_format::Error::History)?), copy })
    }
}

/// Reads what the first paint needs from a `.let` file: its document and its style.
///
/// A path with no file yet opens as a blank document in `new_style` that will be saved there.
pub fn open(path: &Path, new_style: impl FnOnce() -> Style) -> Result<Opened, Error> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        // A file that is missing from a folder that exists is a new document. A missing folder is a mistake.
        Err(error) if error.kind() == io::ErrorKind::NotFound && folder_of(path).is_dir() => {
            let (document, style) = (blank(), new_style());
            let rest = Rest::new(&document, &style);
            return Ok(Opened { document, style, home: Home::new(path), rest });
        }
        Err(error) => return Err(error.into()),
    };
    let mut archive = Archive::open(Cursor::new(&bytes))?;
    let document = archive.document()?;
    // A style this build cannot read stays in the file untouched, and the built-in one is shown.
    let style = archive.style().unwrap_or_default();
    Ok(Opened { document, style, home: Home { path: path.into(), written: true }, rest: Rest(Source::File(bytes)) })
}
