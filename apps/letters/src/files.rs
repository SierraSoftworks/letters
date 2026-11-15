//! Opening and saving `.let` files.
//!
//! A file is read and closed at once, and saved the way `letters_format::save` lays down: merged with what is
//! on disk, written beside it and renamed over it. Saving runs off the UI thread: a [`Home`] and the
//! document's working copy are all it needs.

use std::fmt;
use std::fs;
use std::io::{self, BufReader, Cursor, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use letters_format::{Archive, Files, Save, Saved, Stamp, WorkingCopy};
use letters_model::{Block, BlockId, BlockKind, Document, Model, Notes, Prepared, ReplicaId, Revision, Style};

use crate::journal::{self, Claim};
use crate::platform;
use crate::strangers;

pub const EXTENSION: &str = "let";
/// What a stylesheet's file name ends in.
pub const STYLESHEET: &str = "letstyle";

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

impl Error {
    /// Why `what` was not saved, as its writer is to read it.
    pub fn unsaved(&self, what: &str) -> String {
        match self {
            Self::Io(error) => unwritten(what, error),
            Self::Format(error) => format!("{what} could not be saved{}.", unfit(error)),
        }
    }

    /// Why a file was not opened or read, to end a sentence a person reads: " because it is damaged", or
    /// nothing where there is no plain reason to give.
    pub fn unopened(&self) -> &'static str {
        match self {
            Self::Io(error) => unread(error),
            Self::Format(error) => unfit(error),
        }
    }
}

/// What is wrong with a file that was read, to end a sentence. The detail is for the log.
fn unfit(error: &letters_format::Error) -> &'static str {
    use letters_format::Error::{
        Document, HashMismatch, History, Manifest, Missing, NewerFormat, NotLetters, Notes, OtherDocument, Style,
        Unmerged, Zip,
    };
    eprintln!("letters: {error}");
    match error {
        Zip(_) | NotLetters | Missing(_) | Manifest(_) => " because it is damaged or is not a Letters document",
        NewerFormat(_) => " because a newer Letters wrote it",
        HashMismatch(_) | Document(_) | Style(_) | Notes(_) => " because it is damaged",
        History(_) => " because its history cannot be read",
        OtherDocument(_) => " because the file holds a different document",
        Unmerged => " because the file holds changes that Letters could not take in",
        // What a folder refused is said by the one who knows what it was asked.
        _ => "",
    }
}

/// Why the system would not read a file, to end a sentence a person reads: " because it is no longer
/// there", or nothing where there is no plain reason to give. As [`because`], for a file that is read.
pub fn unread(error: &io::Error) -> &'static str {
    use io::ErrorKind::{IsADirectory, NotADirectory, NotFound, PermissionDenied};
    match error.kind() {
        NotFound | NotADirectory => " because it is not there",
        PermissionDenied => " because Letters may not read it",
        IsADirectory => " because it is a folder",
        _ => {
            eprintln!("letters: a file could not be read: {error}");
            ""
        }
    }
}

/// Why the system would not write a file, make a folder or move one, to end a sentence a person reads:
/// " because the disk is full", or nothing where there is no plain reason to give. The system's own
/// words are never shown: where they are all there is, they go to the log.
pub fn because(error: &io::Error) -> &'static str {
    use io::ErrorKind::{
        AlreadyExists, InvalidFilename, NotADirectory, NotFound, PermissionDenied, QuotaExceeded, ReadOnlyFilesystem,
        StorageFull,
    };
    match error.kind() {
        NotFound | NotADirectory => " because that folder does not exist",
        PermissionDenied | ReadOnlyFilesystem => " because Letters may not write to that folder",
        StorageFull | QuotaExceeded => " because the disk is full",
        AlreadyExists => " because a file with that name already exists",
        InvalidFilename => " because that name cannot be used for a file",
        _ => {
            eprintln!("letters: a file could not be written: {error}");
            ""
        }
    }
}

/// Why `what` could not be saved, as a person is to read it: "The document could not be saved because the
/// disk is full." One wording for a save, a copy, an export and an import.
pub fn unwritten(what: &str, error: &io::Error) -> String {
    format!("{what} could not be saved{}.", because(error))
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

/// Keeps `json`, the text of a `style.json`, as the style the person's new documents start with.
pub fn keep_style(home_folder: &Path, json: &str) -> io::Result<()> {
    let folder = home_folder.join(".letters");
    fs::create_dir_all(&folder)?;
    // Written beside and renamed, so that a crash leaves the old default or the new one.
    let beside = folder.join("style.json.new");
    fs::write(&beside, json)?;
    fs::rename(beside, folder.join("style.json"))
}

/// Writes a stylesheet where the person chose, and returns where that is: a name typed without the ending
/// gets it, and then replaces nothing, since nobody was asked about the file of that name.
pub fn write_stylesheet(chosen: &Path, json: &str) -> io::Result<PathBuf> {
    if chosen.extension().is_some_and(|ending| ending.eq_ignore_ascii_case(STYLESHEET)) {
        return fs::write(chosen, json).map(|()| chosen.to_owned());
    }
    let mut path = chosen.as_os_str().to_owned();
    path.push(format!(".{STYLESHEET}"));
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
    file.write_all(json.as_bytes()).map(|()| path.into())
}

/// The working copy of a new document in `style`, which no file holds yet.
pub fn new_copy(document: Document, style: Style) -> WorkingCopy {
    let replica = ReplicaId::new(&platform::block_id());
    WorkingCopy::new(platform::document_id(), platform::now_rfc3339(), document, style, replica)
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

    /// The home of a document in the file at `path`, which is there already and holds that document, as
    /// the original of a conflict copy does. A save merges with what the file holds.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), written: true }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the document is in the file, which a home that was only chosen has yet to be written.
    pub fn written(&self) -> bool {
        self.written
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
        let mut folder = Folder::new(folder_of(&self.path));
        let today = &platform::now_rfc3339()[..10];
        let save = Save { name: &name, token: &platform::block_id(), label: today, attempts: 3 };
        let saved =
            letters_format::save(&mut folder, copy, &save).map_err(|error| match (error, folder.failed.take()) {
                // The save keeps only the words of what the folder refused. The folder knows what it was.
                (letters_format::Error::Files(_), Some(failed)) => Error::Io(failed),
                (error, _) => Error::Format(error),
            })?;
        let path = match &saved {
            Saved::ConflictCopy { name, .. } => self.path.with_file_name(name),
            _ => self.path.clone(),
        };
        Ok((Home { path, written: true }, saved))
    }

    /// One save from an editor: takes its `edits` and its `notes` as they stand into `copy`, writes the
    /// file, and says what the file held that the editor, at `revision`, has not seen.
    ///
    /// The files at `others` are merged into the document on the way: conflict copies, or a file the
    /// person picked. They are only read, and one that cannot be merged does not hold the save back.
    pub fn exchange(
        &self,
        copy: &mut WorkingCopy,
        edits: &[u8],
        notes: Notes,
        revision: &Revision,
        others: &[PathBuf],
    ) -> Result<Exchange, Error> {
        copy.edit(edits)?;
        // The editor's notes have followed its text and say what was resolved, so they stand for the copy's.
        let ours = (copy.model().revision(), notes.clone());
        copy.notes = notes;
        // Merged after the editor's own words are in, so that what clashes with them is found and noted.
        let merged = others.iter().map(|path| Merged { path: path.clone(), result: merge(copy, path) }).collect();
        let (home, saved) = self.save(copy)?;
        let news = (copy.model().revision() != ours.0).then(|| copy.model().updates_since(revision));
        let notes = (copy.notes != ours.1).then(|| copy.notes.clone());
        Ok(Exchange { home, news, notes, held: matches!(saved, Saved::Unchanged), merged })
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

/// How one save from an editor ended.
#[derive(Debug)]
pub struct Exchange {
    /// The document's home from now on.
    pub home: Home,
    /// What the file held that the editor has not seen. The editor merges it.
    pub news: Option<Vec<u8>>,
    /// The notes as the file's own changed them. The editor merges them.
    pub notes: Option<Notes>,
    /// Whether the file was read and found to hold everything, so that nothing was written. Only this
    /// tells a journal that its records are in the file.
    pub held: bool,
    /// How each file that was to be merged fared, in the order asked.
    pub merged: Vec<Merged>,
}

/// A file that was to be merged into a document, and whether it was. The file is as it was either way.
#[derive(Debug)]
pub struct Merged {
    pub path: PathBuf,
    pub result: Result<(), Error>,
}

/// Merges the document in the file at `path` into `copy`.
fn merge(copy: &mut WorkingCopy, path: &Path) -> Result<(), Error> {
    let mut archive = Archive::open(BufReader::new(fs::File::open(path)?))?;
    // Another file's history is a stranger's, whoever's this document is: it is tried where it can hurt
    // nothing before the document takes it in.
    let was = (copy.wary, copy.check);
    (copy.wary, copy.check) = (true, strangers::check);
    let merged = copy.merge_from(&mut archive).map(drop).map_err(Error::from);
    (copy.wary, copy.check) = was;
    merged
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

/// A real folder, for `letters_format::save` and for journals.
pub(crate) struct Folder {
    path: PathBuf,
    /// What the system last refused here, for whoever asks why a save failed.
    failed: Option<io::Error>,
}

impl Folder {
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), failed: None }
    }

    /// Notes a refusal on its way out.
    fn noted<T>(&mut self, done: io::Result<T>) -> io::Result<T> {
        if let Err(error) = &done {
            self.failed = Some(io::Error::new(error.kind(), error.to_string()));
        }
        done
    }
}

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
        let stamp = |path: &Path| {
            let Some(metadata) = found(fs::metadata(path))? else { return Ok(None) };
            let modified = metadata.modified()?.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos() as u64);
            Ok(Some(Stamp { len: metadata.len(), modified }))
        };
        let stamp = stamp(&self.path.join(name));
        self.noted(stamp)
    }

    fn read(&mut self, name: &str) -> io::Result<Option<Vec<u8>>> {
        let read = found(fs::read(self.path.join(name)));
        self.noted(read)
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let write = |path: &Path| {
            let mut file = fs::File::create(path)?;
            file.write_all(bytes)?;
            file.sync_all()
        };
        let written = write(&self.path.join(name));
        self.noted(written)
    }

    fn append(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let append = |path: &Path| {
            let mut file = fs::OpenOptions::new().append(true).create(true).open(path)?;
            file.write_all(bytes)?;
            file.sync_data()
        };
        let appended = append(&self.path.join(name));
        self.noted(appended)
    }

    fn rename(&mut self, from: &str, to: &str) -> io::Result<()> {
        let (from, to) = (self.path.join(from), self.path.join(to));
        let rename = || {
            // The new file takes over the permissions of the one it replaces.
            if let Ok(replaced) = fs::metadata(&to) {
                fs::set_permissions(&from, replaced.permissions())?;
            }
            fs::rename(&from, &to)
        };
        let renamed = rename();
        self.noted(renamed)
    }

    fn remove(&mut self, name: &str) -> io::Result<()> {
        let removed = fs::remove_file(self.path.join(name));
        self.noted(removed)
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
    /// `None` for a document that never reached a file, as one found in a journal may not have.
    pub home: Option<Home>,
    pub rest: Rest,
}

/// What opening leaves for after the first paint: the file's history, and whatever else a save needs.
pub struct Rest(Source);

enum Source {
    /// The file as it was read.
    File(Vec<u8>),
    /// No file yet.
    New(Box<WorkingCopy>),
    /// A document that already took in what its journal held.
    Kept(Box<WorkingCopy>, Claim),
}

impl fmt::Debug for Rest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            Source::File(_) => "Rest(File)",
            Source::New(_) => "Rest(New)",
            Source::Kept(..) => "Rest(Kept)",
        })
    }
}

/// What saving a document needs: its working copy, and the history for the editor that shows it.
pub struct Working {
    pub copy: WorkingCopy,
    /// For `Editor::attach`. `None` when the editor's own history is already the document's.
    pub history: Option<Prepared>,
    /// What a journal held that the file did not, for the editor to merge once the history is attached.
    pub news: Option<Vec<u8>>,
    /// The document's journal, when one is kept and no other window has it.
    pub claim: Option<Claim>,
    /// The revision the journal's next edits build on. `None` for a new document, whose journal starts
    /// with its whole history.
    pub journalled: Option<Revision>,
}

impl Rest {
    /// The rest of a new `document` in `style`, which is nothing to read.
    pub fn new(document: &Document, style: &Style) -> Self {
        Self(Source::New(Box::new(new_copy(document.clone(), style.clone()))))
    }

    /// Reads it. `editor` is the replica the document's editor was made with, and `journals` the folder
    /// unsaved work is journalled in, if it is. `wary` says of the file's document, by its id, whether
    /// nobody vouches for it: its history is then a stranger's, which is tried in a process of its own
    /// before a model loads it and left out if it would do harm, and the copy stays wary of what reaches
    /// it later.
    ///
    /// This is the slow part of opening a long document, so it runs on another thread once the document
    /// is on screen.
    pub fn read(
        self,
        editor: ReplicaId,
        journals: Option<&Path>,
        wary: impl FnOnce(&str) -> bool,
    ) -> Result<Working, Error> {
        let claim = |copy: &WorkingCopy| journals.and_then(|folder| journal::claim(folder, &copy.id));
        let (mut copy, kept) = match self.0 {
            Source::File(bytes) => {
                let mut archive = Archive::open(Cursor::new(bytes))?;
                let replica = ReplicaId::new(&platform::block_id());
                let copy = match wary(&archive.manifest().id) {
                    true => WorkingCopy::open_checked(&mut archive, replica, strangers::check)?,
                    false => WorkingCopy::open(&mut archive, replica)?,
                };
                (copy, None)
            }
            Source::New(copy) => {
                let claim = claim(&copy);
                return Ok(Working { copy: *copy, history: None, news: None, claim, journalled: None });
            }
            Source::Kept(copy, claim) => (*copy, Some(claim)),
        };
        // The editor takes the history as the copy holds it, so a document changed outside Letters is one
        // change in both and not one in each.
        let model = copy.model();
        let shown = Model::new(model.doc().clone(), editor).with_style(model.style().clone());
        let history = shown.prepare(Some(&model.save())).map_err(letters_format::Error::History)?;
        let mut news = None;
        let claim = kept.or_else(|| {
            // A journal may wait for this file though no window opened for it: its file was gone when the
            // app started, and has turned up since. The editor shows the file, so it merges what that adds.
            claim(&copy).filter(|claim| {
                let Some(unsaved) = claim.unsaved() else { return true };
                let file = copy.model().revision();
                let replayed = copy.replay(unsaved);
                news = (copy.model().revision() != file).then(|| copy.model().updates_since(&file));
                // A journal that builds on another file stays as it is, and this window keeps none.
                replayed
                    .inspect_err(|error| eprintln!("letters: unsaved work could not be brought back: {error}"))
                    .is_ok()
            })
        });
        let journalled = Some(copy.model().revision());
        Ok(Working { history: Some(history), news, claim, journalled, copy })
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
            return Ok(Opened { document, style, home: Some(Home::new(path)), rest });
        }
        Err(error) => return Err(error.into()),
    };
    let mut archive = Archive::open(Cursor::new(&bytes))?;
    let document = archive.document()?;
    // A style this build cannot read stays in the file untouched, and the built-in one is shown.
    let style = archive.style().unwrap_or_default();
    let home = Some(Home { path: path.into(), written: true });
    Ok(Opened { document, style, home, rest: Rest(Source::File(bytes)) })
}

/// Reads one entry of the document at `path`, such as a picture it holds. `None` when the file has no
/// such entry, or cannot be read: only the entry asked for is taken from the disk.
pub fn entry(path: &Path, name: &str) -> Option<Vec<u8>> {
    let mut archive = Archive::open(BufReader::new(fs::File::open(path).ok()?)).ok()?;
    archive.read(name).ok()?
}

/// Opens the document of a journal that a session left behind when it ended early.
///
/// `None` when the document's file turns out to hold everything, and the journal goes. `None` too when the
/// journal builds on a file that is not where it was: it then waits for that file to be opened.
pub fn reopen(claim: Claim) -> Option<Opened> {
    let unsaved = claim.unsaved()?;
    let replica = || ReplicaId::new(&platform::block_id());
    let on_file = unsaved.home.as_deref().map(Path::new).and_then(|path| {
        let mut archive = Archive::open(Cursor::new(fs::read(path).ok()?)).ok()?;
        let mut copy = WorkingCopy::open(&mut archive, replica()).ok()?;
        // A copy made by hand that became a document of its own, and never reached its file as one: the
        // file still says it is the original. A file that is neither refuses the journal.
        if copy.id != unsaved.id {
            copy.fork(unsaved.id.clone());
        }
        let added = copy.replay(unsaved).ok()?;
        Some((Home { path: path.into(), written: true }, copy, added))
    });
    let (home, copy) = match on_file {
        Some((_, _, false)) => {
            claim.discard();
            return None;
        }
        Some((home, copy, true)) => (Some(home), copy),
        // With no file to add to, the journal has to hold the whole document.
        None => (None, WorkingCopy::recover(unsaved, replica()).ok()??),
    };
    let (document, style) = (copy.model().doc().clone(), copy.model().style().clone());
    Some(Opened { document, style, home, rest: Rest(Source::Kept(Box::new(copy), claim)) })
}
