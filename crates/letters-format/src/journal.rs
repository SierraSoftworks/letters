//! The journal: a writer's own record of what the shared file is not yet known to hold.
//!
//! It lives in the writer's state folder on local disk, one file per document, never beside the document.
//! Every record is appended and flushed on its own, so a crash loses at most the record being written, and
//! the next session replays the rest. Records leave the journal only once a save has read them back from the
//! file: a save that was written may still lose a race, and a crash after that must not lose the edits.
//!
//! The file is `LETJ1\n` and then records: a kind byte, the payload's length as a little-endian `u32`, the
//! payload, and the first eight bytes of the SHA-256 of all three. A reader skips kinds it does not know and
//! stops at the first record that does not check out.

use std::collections::BTreeMap;

use letters_model::Notes;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::save::{Files, Saved};
use crate::{Error, hash};

const MAGIC: &[u8] = b"LETJ1\n";
/// Rises only for a breaking change.
const FORMAT: u64 = 1;
const SUFFIX: &str = ".journal";
const SUM: usize = 8;

/// What the journal is of: JSON with `format`, `id`, `created`, `home` and `whole`. The last one read stands.
const HEADER: u8 = b'H';
/// Edits, as `Model::save` or `Model::updates_since` wrote them.
const EDITS: u8 = b'U';
/// The notes as they stood, as `notes.json`.
const NOTES: u8 = b'N';
/// A new asset: its name's length as a little-endian `u16`, the name, and its bytes.
const ASSET: u8 = b'A';

/// The name of a document's journal in the state folder. It is made from a hash, since the id comes from a
/// file and may hold anything.
pub fn journal_name(id: &str) -> String {
    let digest = hash(id.as_bytes());
    let hex = digest.rsplit(':').next().unwrap_or_default();
    format!("{}{SUFFIX}", &hex[..32])
}

/// Whether a file in the state folder is a journal, for the shell that lists them when it starts.
pub fn is_journal(name: &str) -> bool {
    name.strip_suffix(SUFFIX).is_some_and(|stem| stem.len() == 32 && stem.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// A point in a journal: everything journalled before it. Take one when a save starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mark(u64);

/// The journal of one open document.
#[derive(Debug)]
pub struct Journal {
    name: String,
    about: About,
    /// The bytes before the first record that can leave: the magic and the header. `0` while there is no file.
    head: u64,
    /// The records in the file, in bytes, and how many bytes left the journal before them.
    len: u64,
    gone: u64,
}

/// What a journal says of its document.
#[derive(Clone, Debug, PartialEq)]
struct About {
    id: String,
    created: String,
    home: Option<String>,
    whole: bool,
}

impl Journal {
    /// Opens the journal of a document as the document is opened or made, and hands back what an earlier
    /// session left in it. Nothing is written until there is something to record.
    ///
    /// `home` is where the document is saved, in the shell's own words, or `None` while it has no file. A
    /// journal that starts without a home must be able to stand alone: its first [`Journal::edits`] are the
    /// whole history, as `Model::save` gives it.
    pub fn open(
        files: &mut impl Files,
        id: &str,
        created: &str,
        home: Option<&str>,
    ) -> Result<(Self, Option<Unsaved>), Error> {
        let name = journal_name(id);
        let about = About { id: id.into(), created: created.into(), home: home.map(Into::into), whole: home.is_none() };
        let mut journal = Self { name, about, head: 0, len: 0, gone: 0 };
        let Some(bytes) = files.read(&journal.name).map_err(folder)? else { return Ok((journal, None)) };
        // A file that does not start as a journal was cut off as it was made, and held nothing yet.
        let Some(read) = Read::new(&bytes)? else { return Ok((journal, None)) };
        if read.about.id != id {
            return Err(Error::OtherDocument(read.about.id));
        }
        if read.end < bytes.len() {
            // What follows a torn record could never be read, so the tear goes before anything is added.
            journal.replace(files, &bytes[..read.end])?;
        }
        journal.head = read.head as u64;
        journal.len = (read.end - read.head) as u64;
        journal.about = About { home: home.map(Into::into).or(read.about.home.clone()), ..read.about.clone() };
        if journal.about.home != read.about.home {
            journal.add(files, HEADER, &journal.about.to_json())?;
        }
        let unsaved = read.unsaved();
        Ok((journal, (!unsaved.is_empty()).then_some(unsaved)))
    }

    /// Records edits: what the editor's model holds beyond what it last gave the journal.
    pub fn edits(&mut self, files: &mut impl Files, history: &[u8]) -> Result<(), Error> {
        self.add(files, EDITS, history)
    }

    /// Records the notes as they stand, after one was added or changed.
    pub fn notes(&mut self, files: &mut impl Files, notes: &Notes) -> Result<(), Error> {
        self.add(files, NOTES, notes.to_json().as_bytes())
    }

    /// Records an asset that no file holds yet.
    pub fn asset(&mut self, files: &mut impl Files, name: &str, bytes: &[u8]) -> Result<(), Error> {
        let length = u16::try_from(name.len()).map_err(|_| Error::Files("an asset's name is too long".into()))?;
        let payload = [&length.to_le_bytes(), name.as_bytes(), bytes].concat();
        self.add(files, ASSET, &payload)
    }

    /// Records where the document is saved from now on: when a home is chosen for it, after a move, or
    /// when a conflict copy became its home.
    pub fn moved(&mut self, files: &mut impl Files, home: &str) -> Result<(), Error> {
        if self.about.home.as_deref() == Some(home) {
            return Ok(());
        }
        self.about.home = Some(home.into());
        // With nothing recorded yet there is no file to add to, and the first record will say the home. A
        // journal opened without one still starts from the whole history, whenever the home was chosen.
        if self.head == 0 { Ok(()) } else { self.add(files, HEADER, &self.about.to_json()) }
    }

    /// Everything journalled so far. Take it as a save is handed its edits, and give it to
    /// [`Journal::saved`] with how that save ended.
    pub fn mark(&self) -> Mark {
        Mark(self.gone + self.len)
    }

    /// Whether the journal holds nothing, so that no save is owed to it.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Tells the journal how a save ended. `mark` was taken as that save was handed its edits.
    ///
    /// Only [`Saved::Unchanged`] lets records go: the save read the file and found everything in it. A save
    /// that wrote the file has not read it back, so the shell saves once more when things are quiet, and that
    /// save empties the journal.
    pub fn saved(&mut self, files: &mut impl Files, mark: Mark, saved: &Saved) -> Result<(), Error> {
        let through = mark.0.saturating_sub(self.gone).min(self.len);
        if !matches!(saved, Saved::Unchanged) || through == 0 {
            return Ok(());
        }
        let bytes = files.read(&self.name).map_err(folder)?.unwrap_or_default();
        let kept = bytes.get((self.head + through) as usize..).unwrap_or_default().to_vec();
        // What stays builds on the file that was just read.
        self.about.whole = false;
        self.gone += through;
        self.len = kept.len() as u64;
        if kept.is_empty() {
            self.head = 0;
            return match files.remove(&self.name) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(folder(error)),
                _ => Ok(()),
            };
        }
        let head = self.start();
        self.head = head.len() as u64;
        self.replace(files, &[head, kept].concat())
    }

    /// The magic and the header, which every journal file starts with.
    fn start(&self) -> Vec<u8> {
        [MAGIC, &record(HEADER, &self.about.to_json())].concat()
    }

    fn add(&mut self, files: &mut impl Files, kind: u8, payload: &[u8]) -> Result<(), Error> {
        let record = record(kind, payload);
        if self.head == 0 {
            let head = self.start();
            self.head = head.len() as u64;
            files.write(&self.name, &[head, record.clone()].concat()).map_err(folder)?;
        } else {
            files.append(&self.name, &record).map_err(folder)?;
        }
        self.len += record.len() as u64;
        Ok(())
    }

    /// Puts `bytes` in place of the journal in one step, so that a crash leaves the old one or the new.
    fn replace(&self, files: &mut impl Files, bytes: &[u8]) -> Result<(), Error> {
        let temporary = format!(".{}.tmp", self.name);
        let replaced = files.write(&temporary, bytes).and_then(|()| files.rename(&temporary, &self.name));
        replaced.map_err(|error| {
            let _ = files.remove(&temporary);
            folder(error)
        })
    }
}

impl About {
    fn to_json(&self) -> Vec<u8> {
        let mut map = Map::new();
        map.insert("format".into(), FORMAT.into());
        map.insert("id".into(), self.id.as_str().into());
        map.insert("created".into(), self.created.as_str().into());
        if let Some(home) = &self.home {
            map.insert("home".into(), home.as_str().into());
        }
        map.insert("whole".into(), self.whole.into());
        Value::Object(map).to_string().into_bytes()
    }

    fn from_json(json: &[u8]) -> Result<Option<Self>, Error> {
        let Ok(Value::Object(map)) = serde_json::from_slice(json) else { return Ok(None) };
        match map.get("format").and_then(Value::as_u64) {
            Some(FORMAT) => {}
            Some(newer) if newer > FORMAT => return Err(Error::NewerFormat(newer as u32)),
            _ => return Ok(None),
        }
        let text = |key: &str| map.get(key).and_then(Value::as_str).map(str::to_owned);
        let (Some(id), Some(created)) = (text("id"), text("created")) else { return Ok(None) };
        let whole = map.get("whole").and_then(Value::as_bool).unwrap_or(false);
        Ok(Some(Self { id, created, home: text("home"), whole }))
    }
}

fn folder(error: std::io::Error) -> Error {
    Error::Files(error.to_string())
}

fn sum(kind: u8, length: [u8; 4], payload: &[u8]) -> [u8; SUM] {
    let digest = Sha256::new().chain_update([kind]).chain_update(length).chain_update(payload).finalize();
    digest[..SUM].try_into().expect("a SHA-256 digest has 32 bytes")
}

fn record(kind: u8, payload: &[u8]) -> Vec<u8> {
    let length = u32::try_from(payload.len()).expect("a record is far smaller than 4 GB").to_le_bytes();
    [&[kind][..], &length, payload, &sum(kind, length, payload)].concat()
}

/// A journal file as read: what it is about, and its sound records.
struct Read<'a> {
    about: About,
    /// Where the records after the first header start, and where the sound ones end.
    head: usize,
    end: usize,
    records: Vec<(u8, &'a [u8])>,
}

impl<'a> Read<'a> {
    /// `None` for a file that does not start as a journal does.
    fn new(bytes: &'a [u8]) -> Result<Option<Self>, Error> {
        let mut at = MAGIC.len();
        if !bytes.starts_with(MAGIC) {
            return Ok(None);
        }
        let next = |at: &mut usize| {
            let rest = &bytes[*at..];
            let kind = *rest.first()?;
            let length: [u8; 4] = rest.get(1..5)?.try_into().ok()?;
            let size = u32::from_le_bytes(length) as usize;
            let payload = rest.get(5..5 + size)?;
            let found = rest.get(5 + size..5 + size + SUM)?;
            (found == sum(kind, length, payload)).then(|| {
                *at += 5 + size + SUM;
                (kind, payload)
            })
        };
        let Some((HEADER, first)) = next(&mut at) else { return Ok(None) };
        let Some(mut about) = About::from_json(first)? else { return Ok(None) };
        let head = at;
        let mut records = Vec::new();
        while let Some((kind, payload)) = next(&mut at) {
            match kind {
                // A later header says where the document went. What the journal holds stays what it was.
                HEADER => {
                    if let Some(later) = About::from_json(payload)? {
                        about.home = later.home;
                    }
                }
                kind => records.push((kind, payload)),
            }
        }
        Ok(Some(Self { about, head, end: at, records }))
    }

    fn unsaved(&self) -> Unsaved {
        let About { id, created, home, whole } = self.about.clone();
        let mut unsaved = Unsaved { id, created, home, whole, ..Unsaved::default() };
        for &(kind, payload) in &self.records {
            match kind {
                EDITS => unsaved.edits.push(payload.to_vec()),
                // Only the newest notes matter, and notes a newer build wrote differently are left out.
                NOTES => {
                    let notes = std::str::from_utf8(payload).ok().and_then(|json| Notes::from_json(json).ok());
                    unsaved.notes = notes.or(unsaved.notes.take());
                }
                ASSET => {
                    let named = payload.split_at_checked(2).and_then(|(length, rest)| {
                        let length = u16::from_le_bytes([length[0], length[1]]) as usize;
                        let (name, bytes) = rest.split_at_checked(length)?;
                        Some((std::str::from_utf8(name).ok()?.to_owned(), bytes.to_vec()))
                    });
                    unsaved.assets.extend(named);
                }
                _ => {}
            }
        }
        unsaved
    }
}

/// What a journal holds from a session that ended before its edits were known to be in the file.
///
/// Give it to [`WorkingCopy::replay`](crate::WorkingCopy::replay) once the file at `home` is open, or to
/// [`WorkingCopy::recover`](crate::WorkingCopy::recover) when there is no file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Unsaved {
    pub id: String,
    /// RFC 3339.
    pub created: String,
    /// Where the document was saved, as the shell said it. `None` for one that never reached a file.
    pub home: Option<String>,
    /// Whether the journal holds the document's whole history, and so needs no file.
    pub whole: bool,
    pub(crate) edits: Vec<Vec<u8>>,
    pub(crate) notes: Option<Notes>,
    pub(crate) assets: BTreeMap<String, Vec<u8>>,
}

impl Unsaved {
    /// Reads a journal by its file name, for the shell that finds some as it starts. `None` when the file
    /// is missing or holds nothing.
    pub fn read(files: &mut impl Files, name: &str) -> Result<Option<Self>, Error> {
        let Some(bytes) = files.read(name).map_err(folder)? else { return Ok(None) };
        let unsaved = Read::new(&bytes)?.map(|read| read.unsaved());
        Ok(unsaved.filter(|unsaved| !unsaved.is_empty()))
    }

    fn is_empty(&self) -> bool {
        self.edits.is_empty() && self.notes.is_none() && self.assets.is_empty()
    }
}
