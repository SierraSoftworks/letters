//! The working copy of a document: what a writer holds in memory, and how it takes in another copy.
//!
//! Every copy of a document carries its history, so any two of them merge: the file a save is about to
//! replace, a conflict copy a sync tool left behind, or a file the user picks. The history merges as the CRDT
//! it is, notes merge by id, and assets are kept from both sides.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io::{Read, Seek};

use letters_model::{Change, Document, Model, Note, Notes, Op, OpError, ReplicaId, Style};

use crate::save::{DiskCopy, Replica};
use crate::{Archive, DOCUMENT, Error, HISTORY, Manifest, NOTES, Package, STYLE};

/// The entry that holds the document's history, as [`Model::save`] writes it.
pub const OPLOG: &str = "history/oplog";

/// A document in memory between saves. It is the [`Replica`] that [`save`](crate::save) writes.
pub struct WorkingCopy {
    pub id: String,
    /// RFC 3339.
    pub created: String,
    model: Model,
    pub style: Style,
    /// Add a note with [`Self::add_note`], which ties it to its text.
    pub notes: Notes,
    /// What to write besides the document, its style, notes and history: assets the file does not hold yet.
    /// One is dropped from memory once the file is seen to hold it, and the save then carries it over
    /// without loading it.
    pub entries: BTreeMap<String, Vec<u8>>,
    /// The hashes of the file as last read or written, so that an untouched file is not read again.
    seen: BTreeMap<String, String>,
    /// The style the file last held, which tells a change made here from one made by another writer.
    seen_style: Style,
    /// Whether the file's style or notes are ones this build cannot read, as a newer build's may be. Such an
    /// entry is carried over as it is.
    unread: Unread,
    /// The history as the file last held it, read or written. Written back as it is while the model holds
    /// nothing more, so that writers with the same edits write the same bytes.
    history: Option<Vec<u8>>,
}

impl WorkingCopy {
    /// A new document that no file holds yet.
    pub fn new(id: impl Into<String>, created: impl Into<String>, document: Document, replica: ReplicaId) -> Self {
        Self {
            id: id.into(),
            created: created.into(),
            model: Model::new(document, replica),
            style: Style::default(),
            notes: Notes::default(),
            entries: BTreeMap::new(),
            seen: BTreeMap::new(),
            seen_style: Style::default(),
            unread: Unread::default(),
            history: None,
        }
    }

    /// Reads a file for editing. `replica` is this session's id, fresh from the shell.
    ///
    /// The document is the contract, so it wins over the history. A `document.json` that another tool has
    /// changed goes into the history as one more change, and a file without a readable history starts one.
    pub fn open<R: Read + Seek>(file: &mut Archive<R>, replica: ReplicaId) -> Result<Self, Error> {
        let document = file.document()?;
        let history = file.read(OPLOG)?;
        let loaded = history.as_deref().and_then(|bytes| Model::load(bytes, replica).ok());
        let (model, history) = match loaded {
            Some(mut model) => {
                // A history that was just loaded shows its newest revision, which can be rewritten.
                let _ = model.rewrite(document);
                (model, history)
            }
            None => (Model::new(document, replica), None),
        };
        let (style, mut notes, unread) = Unread::read(file)?;
        notes.pin(&model);
        let manifest = file.manifest();
        Ok(Self {
            id: manifest.id.clone(),
            created: manifest.created.clone(),
            model,
            seen_style: style.clone(),
            style,
            notes,
            entries: BTreeMap::new(),
            seen: manifest.hashes.clone(),
            unread,
            history,
        })
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    /// Edits the document here, for a writer with no editor of its own. Notes follow their text.
    pub fn apply(&mut self, ops: impl IntoIterator<Item = Op>) -> Result<Change, OpError> {
        self.notes.pin(&self.model);
        let change = self.model.apply(ops)?;
        self.notes.follow(&self.model);
        Ok(change)
    }

    /// Adds a note, tied to the text it covers now so that it follows that text.
    pub fn add_note(&mut self, mut note: Note) {
        note.anchor.pin(&self.model);
        self.notes.notes.push(note);
    }

    /// Takes in edits made on another replica of this session, such as the editor's, as [`Model::save`] or
    /// [`Model::updates_since`] wrote them. Notes follow their text.
    pub fn edit(&mut self, history: &[u8]) -> Result<Change, Error> {
        self.notes.pin(&self.model);
        let change = self.model.merge(history).map_err(Error::History)?;
        self.notes.follow(&self.model);
        Ok(change)
    }

    /// Merges another file into this document: a conflict copy, or a file the user picked.
    ///
    /// Nothing is lost from either side, and merging the same file again changes nothing. The file is only
    /// read. Fails with [`Error::Unmerged`] when its history does not lead to its document or does not share
    /// a start with this one, which a merge cannot reconcile.
    pub fn merge_from<R: Read + Seek>(&mut self, other: &mut Archive<R>) -> Result<Change, Error> {
        let (change, unread) = self.take_in(other)?;
        if unread != Unread::default() {
            // Its style or notes would be left behind, and a merged copy is one the user may throw away.
            return Err(Error::Unmerged);
        }
        // The save writes over this document's own file, so what only the other file holds comes into memory.
        let core = |name: &str| [DOCUMENT, STYLE, NOTES].contains(&name) || name.starts_with(HISTORY);
        let names: Vec<String> = other.names().filter(|name| !core(name)).map(str::to_owned).collect();
        for name in names {
            if !self.entries.contains_key(&name) && !self.seen.contains_key(&name) {
                let bytes = other.read(&name)?.ok_or_else(|| Error::Missing(name.clone()))?;
                self.entries.insert(name, bytes);
            }
        }
        Ok(change)
    }

    /// Merges the history and notes of another copy, and settles on a style. Says what of it could not be read.
    fn take_in<R: Read + Seek>(&mut self, other: &mut Archive<R>) -> Result<(Change, Unread), Error> {
        let document = other.document()?;
        let replica = self.model.replica();
        // Their copy is only read here, so sharing this replica's id with it writes nothing under that id.
        let (theirs, history) = match other.read(OPLOG)? {
            Some(history) => {
                let theirs = Model::load(&history, replica).map_err(|_| Error::Unmerged)?;
                // A document that its history does not lead to was changed by another tool. Until the model
                // can record such a change, merging the history would silently drop it.
                if theirs.doc() != &document {
                    return Err(Error::Unmerged);
                }
                (theirs, Some(history))
            }
            None => {
                // A file without a history merges only if this history began from that very document.
                let theirs = Model::new(document, replica);
                let held = self.model.compare(&theirs.save()).map_err(Error::History)?;
                if !matches!(held, Some(Ordering::Greater | Ordering::Equal)) {
                    return Err(Error::Unmerged);
                }
                (theirs, None)
            }
        };
        let (style, mut notes, unread) = Unread::read(other)?;
        notes.pin(&theirs);

        self.notes.pin(&self.model);
        let change = match &history {
            Some(history) => self.model.merge(history).map_err(Error::History)?,
            None => Change::default(),
        };
        self.notes.merge(&notes);
        self.notes.follow(&self.model);

        // The style is not in the history yet, so there is no telling which of two changes came last. A
        // change made by the other writer is taken unless this one changed the style too.
        if !unread.style {
            if style != self.seen_style && self.style == self.seen_style {
                self.style = style.clone();
            }
            self.seen_style = style;
        }
        if history.is_some() {
            self.history = history;
        }
        Ok((change, unread))
    }
}

impl Replica for WorkingCopy {
    fn id(&self) -> &str {
        &self.id
    }

    fn merge(&mut self, disk: &mut DiskCopy) -> Result<(), Error> {
        let held = &disk.manifest().hashes;
        if held != &self.seen {
            (_, self.unread) = self.take_in(disk)?;
            self.seen = disk.manifest().hashes.clone();
        }
        // Notes this build cannot read are other people's words, which a note added here must not replace.
        if self.unread.notes && !self.notes.notes.is_empty() {
            return Err(Error::Unmerged);
        }
        // A save that lost a race is replaced by a file without its new assets, so they stay in memory until
        // the file is read back with them in it.
        let seen = &self.seen;
        self.entries.retain(|name, _| !seen.contains_key(name));
        Ok(())
    }

    fn package(&self) -> Package {
        let unchanged = |history: &&Vec<u8>| self.model.compare(history) == Ok(Some(Ordering::Equal));
        let history = self.history.as_ref().filter(unchanged).cloned().unwrap_or_else(|| self.model.save());
        let mut package = Package {
            id: self.id.clone(),
            created: self.created.clone(),
            document: self.model.doc().clone(),
            entries: self.entries.clone(),
        };
        // An entry left out is carried over from the file as it is. A style changed here states the whole
        // style, so it replaces one that could not be read.
        if !(self.unread.style && self.style == self.seen_style) {
            package.set_style(&self.style);
        }
        if !(self.unread.notes && self.notes.notes.is_empty()) {
            package.set_notes(&self.notes);
        }
        package.entries.insert(OPLOG.into(), history);
        package
    }

    fn saved(&mut self, mut written: Package, manifest: &Manifest) {
        self.seen = manifest.hashes.clone();
        self.seen_style = self.style.clone();
        self.unread.style &= !written.entries.contains_key(STYLE);
        self.unread.notes &= !written.entries.contains_key(NOTES);
        self.history = written.entries.remove(OPLOG);
    }
}

/// Which entries of a file this build could not read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Unread {
    style: bool,
    notes: bool,
}

impl Unread {
    /// The style and notes of `file`. One that cannot be read is given as the default, and marked.
    fn read<R: Read + Seek>(file: &mut Archive<R>) -> Result<(Style, Notes, Self), Error> {
        let (style, notes) = (file.style(), file.notes());
        let unread =
            Self { style: matches!(style, Err(Error::Style(_))), notes: matches!(notes, Err(Error::Notes(_))) };
        let style = if unread.style { Style::default() } else { style? };
        let notes = if unread.notes { Notes::default() } else { notes? };
        Ok((style, notes, unread))
    }
}
