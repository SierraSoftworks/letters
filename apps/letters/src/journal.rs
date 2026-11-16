//! Unsaved work, kept through a crash.
//!
//! What a window holds beyond its file goes into a journal a moment after it is typed, in a folder on the
//! person's own machine. A save that finds everything in the file empties the journal. One that outlives
//! its window is found when the app next starts, and its document is opened again.
//!
//! Every record waits for the disk, so all journals are written on one thread of their own. A lock file
//! beside a journal says that a window has it, in this copy of the app or another.
//!
//! A journal is read after a crash, which its last record may have caused. A marker lies beside it while
//! it is read, and a journal whose marker is found later is set aside and not read again.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use letters_format::{Journal as Records, Mark, Saved, Unsaved, is_journal, journal_name};
use letters_model::Notes;

use crate::files::Folder;

/// The right to write one journal, for as long as this is held. The system lets go of it for a window
/// whose app has died.
#[derive(Debug)]
struct Lock {
    path: PathBuf,
    _file: fs::File,
}

impl Lock {
    fn take(folder: &Path, journal: &str) -> Option<Self> {
        let path = folder.join(journal).with_extension("lock");
        let held = || {
            let file = fs::OpenOptions::new().write(true).create(true).truncate(false).open(&path).ok()?;
            file.try_lock().ok()?;
            // The window that had it removes the file as it lets go, which may have been just now.
            path.exists().then_some(file)
        };
        let file = held().or_else(held)?;
        Some(Self { path, _file: file })
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // Removed while it is still held, so that the folder is empty when nothing is unsaved.
        let _ = fs::remove_file(&self.path);
    }
}

/// A journal that was being read when the app ended, moved out of the way so that no later launch ends at
/// it too. It is kept, and nothing reads it again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Aside {
    /// Where the journal is now.
    pub kept: PathBuf,
    /// The file of its document, if that was ever saved.
    pub home: Option<PathBuf>,
}

impl Aside {
    /// What to tell the person. `here` is whether the window that says it shows the document as its file
    /// has it.
    pub fn said(&self, here: bool) -> String {
        let home = std::env::home_dir();
        let inside = home.as_deref().and_then(|home| self.kept.strip_prefix(home).ok());
        let kept = inside.map_or_else(|| self.kept.display().to_string(), |inside| format!("~/{}", inside.display()));
        let name = self.home.as_deref().and_then(Path::file_stem).map(OsStr::to_string_lossy);
        let what = match (&name, here) {
            (Some(_), true) => "unsaved changes to this document, so it is shown as it was last saved".to_owned(),
            (Some(name), false) => format!("unsaved changes to {name}"),
            (None, _) => "a document that was never saved".to_owned(),
        };
        format!(
            "Letters closed unexpectedly while it was bringing back {what}. What could not be brought back is kept in {kept}."
        )
    }
}

/// The file that lies beside a journal for as long as it is read.
fn marker(journal: &Path) -> PathBuf {
    journal.with_extension("reading")
}

/// Sets the journal `name` aside if its marker is beside it: the app ended as it was read. Fails when it
/// cannot be moved, and it is then neither read nor written to.
fn ended(folder: &Path, name: &str) -> io::Result<Option<Aside>> {
    let journal = folder.join(name);
    let marker = marker(&journal);
    if !marker.exists() {
        return Ok(None);
    }
    // Its records are read, which ended nothing, and none of its edits are tried.
    let unsaved = Unsaved::read(&mut Folder::new(folder), name).ok().flatten();
    let home = unsaved.and_then(|unsaved| unsaved.home).map(PathBuf::from);
    let stem = journal.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    // One set aside before it keeps its name.
    let names = (1..100).map(|nth| match nth {
        1 => format!("{stem}.set-aside.journal"),
        nth => format!("{stem}.set-aside-{nth}.journal"),
    });
    let free = names.map(|name| folder.join(name)).find(|kept| !kept.exists());
    let kept = free.ok_or_else(|| io::Error::other("no name is free to set a journal aside under"))?;
    match fs::rename(&journal, &kept) {
        Ok(()) => {}
        // The marker outlived its journal, and there is nothing to set aside.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return fs::remove_file(&marker).map(|()| None),
        Err(error) => return Err(error),
    }
    let _ = fs::remove_file(&marker);
    eprintln!("letters: the app ended as it read a journal, which is set aside as {}", kept.display());
    Ok(Some(Aside { kept, home }))
}

/// A document's journal, held for one window: what an earlier session left in it, and the right to add.
#[derive(Debug)]
pub struct Claim {
    _lock: Lock,
    journal: PathBuf,
    unsaved: Option<Box<Unsaved>>,
    aside: Option<Aside>,
}

impl Claim {
    /// What a session that ended early left unsaved.
    pub fn unsaved(&self) -> Option<&Unsaved> {
        self.unsaved.as_deref()
    }

    /// The journal that was here and was set aside as this one was claimed, for the person to be told.
    pub fn aside(&self) -> Option<&Aside> {
        self.aside.as_ref()
    }

    /// Hands what an earlier session left unsaved to `replay`, which brings it back into a document.
    /// `None` when it left nothing. A marker lies beside the journal meanwhile: a replay that ends the
    /// app, or its thread, leaves it there, and whoever claims the journal next sets it aside.
    pub fn replay<T>(&self, replay: impl FnOnce(&Unsaved) -> T) -> Option<T> {
        let unsaved = self.unsaved.as_deref()?;
        let marker = marker(&self.journal);
        if let Err(error) = fs::write(&marker, []) {
            eprintln!("letters: a journal could not be marked as being read: {error}");
        }
        let replayed = replay(unsaved);
        let _ = fs::remove_file(&marker);
        Some(replayed)
    }

    /// Deletes the journal, whose document needs nothing from it.
    pub fn discard(self) {
        let _ = fs::remove_file(&self.journal);
    }
}

/// Claims the journal of the document `id` in `folder`. `None` when another window has it or the folder
/// cannot be used, and that window's work is then not journalled.
pub fn claim(folder: &Path, id: &str) -> Option<Claim> {
    fs::create_dir_all(folder).ok()?;
    let name = journal_name(id);
    let lock = Lock::take(folder, &name)?;
    let aside = ended(folder, &name).inspect_err(unmoved).ok()?;
    // A journal that cannot be read is a newer build's, and is not written over.
    let unsaved = Unsaved::read(&mut Folder::new(folder), &name).ok()?.map(Box::new);
    Some(Claim { _lock: lock, journal: folder.join(name), unsaved, aside })
}

fn unmoved(error: &io::Error) {
    eprintln!("letters: a journal the app ended at could not be set aside: {error}");
}

fn orphans(folder: &Path) -> Vec<Claim> {
    let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
    let names = entries.flatten().filter_map(|entry| entry.file_name().into_string().ok());
    let claim = |name: String| {
        let lock = Lock::take(folder, &name)?;
        let aside = ended(folder, &name).inspect_err(unmoved).ok()?;
        let claim = Claim { _lock: lock, journal: folder.join(&name), unsaved: None, aside };
        // Its document is opened as its file has it, with the word of what was set aside.
        if claim.aside.is_some() {
            return Some(claim);
        }
        match Unsaved::read(&mut Folder::new(folder), &name) {
            Ok(Some(unsaved)) => Some(Claim { unsaved: Some(Box::new(unsaved)), ..claim }),
            // Cut off as it was made, before it held anything.
            Ok(None) => {
                claim.discard();
                None
            }
            Err(_) => None,
        }
    };
    names.filter(|name| is_journal(name)).filter_map(claim).collect()
}

enum Message {
    Open {
        key: u64,
        claim: Claim,
        id: String,
        created: String,
        home: Option<String>,
    },
    Edits {
        key: u64,
        edits: Vec<u8>,
    },
    Asset {
        key: u64,
        name: String,
        bytes: Vec<u8>,
    },
    Notes {
        key: u64,
        notes: Notes,
    },
    /// A save has been handed everything journalled so far.
    Saving {
        key: u64,
    },
    /// That save found all of it in the file.
    Held {
        key: u64,
    },
    Moved {
        key: u64,
        home: String,
    },
    Close {
        key: u64,
        forget: bool,
    },
    Finish,
}

/// The journals of the open documents, and the thread that writes them.
pub struct Journals {
    folder: PathBuf,
    opened: u64,
    writer: Option<(Sender<Message>, JoinHandle<()>)>,
}

impl Journals {
    /// The journals of the person whose home folder is `home`, kept in `.letters/unsaved`.
    pub fn of(home: &Path) -> Self {
        Self::at(home.join(".letters").join("unsaved"))
    }

    pub fn at(folder: impl Into<PathBuf>) -> Self {
        Self { folder: folder.into(), opened: 0, writer: None }
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Claims the journals no window has, which sessions that ended early left behind.
    pub fn orphans(&self) -> Vec<Claim> {
        orphans(&self.folder)
    }

    /// Opens the journal a window claimed for its document, which is saved at `home` if it has a file.
    /// A document with no file has a journal that stands alone: its first edits are its whole history.
    pub fn open(&mut self, mut claim: Claim, id: &str, created: &str, home: Option<&Path>) -> Journal {
        let folder = &self.folder;
        // The thread starts with the first journal, which is after the first frame.
        let (sender, _) = self.writer.get_or_insert_with(|| {
            let (sender, messages) = mpsc::channel();
            let folder = Folder::new(folder);
            (sender, thread::spawn(move || write(folder, messages)))
        });
        self.opened += 1;
        let journal = Journal {
            key: self.opened,
            sender: sender.clone(),
            home: home.map(absolute),
            // What an earlier session left has been read by now. It is owed a save, which lets it go.
            owed: claim.unsaved.take().is_some(),
            covered: false,
            forget: false,
        };
        let home = journal.home.as_deref().map(told);
        journal.send(Message::Open { key: journal.key, claim, id: id.into(), created: created.into(), home });
        journal
    }

    /// Waits until everything the journals were given is on disk. They take nothing after this.
    pub fn finish(&mut self) {
        if let Some((sender, writer)) = self.writer.take() {
            let _ = sender.send(Message::Finish);
            let _ = writer.join();
        }
    }
}

impl Drop for Journals {
    fn drop(&mut self) {
        self.finish();
    }
}

/// A path as a journal is told it. A journal is read by a later session, which may start anywhere.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}

fn told(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// One open document's journal, as its window holds it. Dropping it closes the journal, which stays on
/// disk while it holds anything.
pub struct Journal {
    key: u64,
    sender: Sender<Message>,
    home: Option<PathBuf>,
    /// Whether it holds records that no save has been handed.
    owed: bool,
    /// Whether the save that is running was handed any.
    covered: bool,
    forget: bool,
}

impl Journal {
    fn send(&self, message: Message) {
        // A writer that has finished takes nothing more, as the app is on its way out.
        let _ = self.sender.send(message);
    }

    /// Records edits: what the editor's model holds beyond what the journal was last given.
    pub fn edits(&mut self, edits: Vec<u8>) {
        self.owed = true;
        self.send(Message::Edits { key: self.key, edits });
    }

    /// Records an asset no file holds yet, ahead of the edits that name it.
    pub fn asset(&mut self, name: &str, bytes: &[u8]) {
        self.owed = true;
        self.send(Message::Asset { key: self.key, name: name.into(), bytes: bytes.into() });
    }

    /// Records the document's notes as they stand, after a comment was written or changed.
    pub fn notes(&mut self, notes: &Notes) {
        self.owed = true;
        self.send(Message::Notes { key: self.key, notes: notes.clone() });
    }

    /// Notes that a save is being handed everything recorded so far.
    pub fn saving(&mut self) {
        self.covered |= std::mem::take(&mut self.owed);
        self.send(Message::Saving { key: self.key });
    }

    /// Notes how that save ended. Only one that `held`, which read the file and found everything in it,
    /// lets the records go: a file that was written is read back by the next save.
    pub fn saved(&mut self, held: bool) {
        let covered = std::mem::take(&mut self.covered);
        match held {
            true => self.send(Message::Held { key: self.key }),
            false => self.owed |= covered,
        }
    }

    /// Whether the journal holds anything that no save has found in the file, and so is owed one.
    pub fn owes(&self) -> bool {
        self.owed || self.covered
    }

    /// Records where the document's file is from now on.
    pub fn moved(&mut self, home: &Path) {
        let home = absolute(home);
        if self.home.as_ref() != Some(&home) {
            self.send(Message::Moved { key: self.key, home: told(&home) });
            self.home = Some(home);
        }
    }

    /// Has the journal deleted when it closes, for a document that is not worth a file.
    pub fn forget(&mut self) {
        self.forget = true;
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        self.send(Message::Close { key: self.key, forget: self.forget });
    }
}

/// A journal as the writing thread holds it.
struct Open {
    records: Records,
    /// Where the journal stood as the running save was handed its edits.
    mark: Option<Mark>,
    claim: Claim,
}

/// Writes what the windows send until they have finished.
fn write(mut folder: Folder, messages: Receiver<Message>) {
    let mut open: HashMap<u64, Open> = HashMap::new();
    for message in messages {
        let done = match message {
            Message::Open { key, claim, id, created, home } => {
                Records::open(&mut folder, &id, &created, home.as_deref()).map(|(records, _)| {
                    open.insert(key, Open { records, mark: None, claim });
                })
            }
            Message::Edits { key, edits } => {
                open.get_mut(&key).map_or(Ok(()), |journal| journal.records.edits(&mut folder, &edits))
            }
            Message::Asset { key, name, bytes } => {
                open.get_mut(&key).map_or(Ok(()), |journal| journal.records.asset(&mut folder, &name, &bytes))
            }
            Message::Notes { key, notes } => {
                open.get_mut(&key).map_or(Ok(()), |journal| journal.records.notes(&mut folder, &notes))
            }
            Message::Saving { key } => {
                if let Some(journal) = open.get_mut(&key) {
                    journal.mark = Some(journal.records.mark());
                }
                Ok(())
            }
            Message::Held { key } => {
                let marked = open.get_mut(&key).and_then(|journal| Some((journal.mark.take()?, journal)));
                marked.map_or(Ok(()), |(mark, journal)| journal.records.saved(&mut folder, mark, &Saved::Unchanged))
            }
            Message::Moved { key, home } => {
                open.get_mut(&key).map_or(Ok(()), |journal| journal.records.moved(&mut folder, &home))
            }
            Message::Close { key, forget } => {
                if let Some(journal) = open.remove(&key).filter(|_| forget) {
                    journal.claim.discard();
                }
                Ok(())
            }
            Message::Finish => break,
        };
        if let Err(error) = done {
            eprintln!("letters: unsaved work could not be journalled: {error}");
        }
    }
}
