//! The working copy of a document: what a writer holds in memory, and how it takes in another copy.
//!
//! Every copy of a document carries its history, so any two of them merge: the file a save is about to
//! replace, a conflict copy a sync tool left behind, or a file the user picks. The history merges as the CRDT
//! it is, and the style with it. Notes merge by id, and assets are kept from both sides.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io::{Read, Seek};

use letters_model::{Change, Document, Model, Note, Notes, Op, OpError, ReplicaId, Style};

use crate::journal::Unsaved;
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
    /// Add a note with [`Self::add_note`], which ties it to its text.
    pub notes: Notes,
    /// What to write besides the document, its style, notes and history: assets the file does not hold yet.
    /// One is dropped from memory once the file is seen to hold it, and the save then carries it over
    /// without loading it.
    pub entries: BTreeMap<String, Vec<u8>>,
    /// The hashes of the file as last read or written, so that an untouched file is not read again.
    seen: BTreeMap<String, String>,
    /// The style as it was when the file's `style.json` was found to be one this build cannot read, as a
    /// newer build's may be. The entry is carried over as it is until the style is changed here.
    unread_style: Option<Style>,
    /// Whether the file's `notes.json` is one this build cannot read. It is carried over as it is.
    unread_notes: bool,
    /// The history as the file last held it, read or written. Written back as it is while the model holds
    /// nothing more, so that writers with the same edits write the same bytes.
    history: Option<Vec<u8>>,
}

impl WorkingCopy {
    /// A new document that no file holds yet.
    pub fn new(
        id: impl Into<String>,
        created: impl Into<String>,
        document: Document,
        style: Style,
        replica: ReplicaId,
    ) -> Self {
        Self {
            id: id.into(),
            created: created.into(),
            model: Model::new(document, replica).with_style(style),
            notes: Notes::default(),
            entries: BTreeMap::new(),
            seen: BTreeMap::new(),
            unread_style: None,
            unread_notes: false,
            history: None,
        }
    }

    /// Reads a file for editing. `replica` is this session's id, fresh from the shell.
    ///
    /// The document and the style are the contract, so they win over the history. What another tool has
    /// changed in them goes into the history as one more change, and a file without a readable history
    /// starts one.
    pub fn open<R: Read + Seek>(file: &mut Archive<R>, replica: ReplicaId) -> Result<Self, Error> {
        let Theirs { model, mut notes, history, unread_style, unread_notes } = Theirs::read(file, replica, true)?;
        notes.pin(&model);
        let manifest = file.manifest();
        Ok(Self {
            id: manifest.id.clone(),
            created: manifest.created.clone(),
            unread_style: unread_style.then(|| model.style().clone()),
            unread_notes,
            model,
            notes,
            entries: BTreeMap::new(),
            seen: manifest.hashes.clone(),
            history,
        })
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    /// The document's style, which lives in its history.
    pub fn style(&self) -> &Style {
        self.model.style()
    }

    /// Names who writes here and tells the time, in seconds since 1970, for a writer with no editor of its
    /// own. Edits made from here on carry both, and a conflict shows them beside each version.
    pub fn sign(&mut self, author: &str, time: u64) {
        self.model.set_author(author);
        self.model.set_time(time);
    }

    /// Changes the style here, for a writer with no editor of its own. Fields left as they are stay open to
    /// what other writers make of them.
    pub fn set_style(&mut self, style: Style) -> Result<Change, OpError> {
        self.apply([Op::ReplaceStyle { style }])
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
    /// read. Fails with [`Error::Unmerged`] when its history shares nothing with this one, or when it holds a
    /// style or notes this build cannot read.
    pub fn merge_from<R: Read + Seek>(&mut self, other: &mut Archive<R>) -> Result<Change, Error> {
        // A style or notes that cannot be read would be left behind, in a copy the user may then throw away.
        let change = self.take_in(Theirs::read(other, self.model.replica(), false)?, false)?;
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

    /// Merges the history and notes of another copy. `home` says it is the file this document is saved to,
    /// whose unreadable entries are remembered so that they are carried over.
    fn take_in(&mut self, theirs: Theirs, home: bool) -> Result<Change, Error> {
        let Theirs { model, mut notes, history, unread_style, unread_notes } = theirs;
        if (unread_style || unread_notes) && !home {
            return Err(Error::Unmerged);
        }
        // What an outside change added is not in the bytes the file holds.
        let level = |history: &&Vec<u8>| model.compare(history) == Ok(Some(Ordering::Equal));
        let history = history.filter(|history| level(&history));
        let merged = history.clone().unwrap_or_else(|| model.save());
        // Histories with nothing in common would merge into both documents, one after the other.
        if !self.model.related(&merged).map_err(Error::History)? {
            return Err(Error::Unmerged);
        }
        notes.pin(&model);
        let restyled = self.unread_style.as_ref().is_some_and(|carried| carried != self.model.style());
        // Only copies that each hold edits the other lacks can have changed the same words.
        let apart = self.model.compare(&merged).map_err(Error::History)?.is_none();
        let apart = apart.then(|| (self.model.revision(), model.revision()));

        self.notes.pin(&self.model);
        let mut change = self.model.merge(&merged).map_err(Error::History)?;
        self.notes.merge(&notes);
        let mut raised = Vec::new();
        if let Some((ours, theirs)) = apart {
            // Each block the two changed in the same words reads as the later side left it from here on.
            let (settled, conflicts) = self.model.settle(&ours, &theirs).map_err(Error::History)?;
            change.absorb(settled);
            for conflict in conflicts {
                let [earlier, later] = [1 - conflict.later, conflict.later].map(|side| conflict.sides[side].version());
                raised.push(Note::conflict(conflict.block, [earlier, later]));
            }
        }
        // With none raised too: a note that came in can be newer than one held for the same block.
        self.notes.raise(raised);
        self.notes.follow(&self.model);
        if home {
            // What the merge made of the style came from other writers, and is no change made here.
            self.unread_style = (unread_style && !restyled).then(|| self.model.style().clone());
            self.unread_notes = unread_notes;
        }
        if history.is_some() {
            self.history = history;
        }
        Ok(change)
    }
}

impl WorkingCopy {
    /// Takes in what a journal kept from a session that ended before its edits were known to be in the
    /// file. Returns whether it held anything this copy lacked, which is then owed a save.
    ///
    /// Fails with [`Error::Unmerged`] when the journal's edits build on a history this copy does not hold.
    pub fn replay(&mut self, unsaved: &Unsaved) -> Result<bool, Error> {
        if unsaved.id != self.id {
            return Err(Error::OtherDocument(unsaved.id.clone()));
        }
        let held = |model: &Model, edits: &[u8]| {
            let held = model.compare(edits).map_err(Error::History)?;
            Ok::<_, Error>(matches!(held, Some(Ordering::Greater | Ordering::Equal)))
        };
        let mut added = false;
        self.notes.pin(&self.model);
        for edits in &unsaved.edits {
            if !held(&self.model, edits)? {
                self.model.merge(edits).map_err(Error::History)?;
                added = true;
            }
        }
        // Edits that wait for others the copy never saw are not in the document.
        for edits in &unsaved.edits {
            if !held(&self.model, edits)? {
                return Err(Error::Unmerged);
            }
        }
        if let Some(notes) = &unsaved.notes {
            added |= self.notes.merge(notes);
        }
        self.notes.follow(&self.model);
        for (name, bytes) in &unsaved.assets {
            if !self.entries.contains_key(name) && !self.seen.contains_key(name) {
                self.entries.insert(name.clone(), bytes.clone());
                added = true;
            }
        }
        Ok(added)
    }

    /// The document a journal holds on its own, for one that never reached a file or whose file is gone.
    /// `None` when the journal only adds to a file.
    pub fn recover(unsaved: &Unsaved, replica: ReplicaId) -> Result<Option<Self>, Error> {
        let Some(first) = unsaved.edits.first().filter(|_| unsaved.whole) else { return Ok(None) };
        let mut copy = Self {
            id: unsaved.id.clone(),
            created: unsaved.created.clone(),
            model: Model::load(first, replica).map_err(Error::History)?,
            notes: Notes::default(),
            entries: BTreeMap::new(),
            seen: BTreeMap::new(),
            unread_style: None,
            unread_notes: false,
            history: None,
        };
        copy.replay(unsaved)?;
        Ok(Some(copy))
    }
}

impl Replica for WorkingCopy {
    fn id(&self) -> &str {
        &self.id
    }

    fn merge(&mut self, disk: &mut DiskCopy) -> Result<(), Error> {
        let held = &disk.manifest().hashes;
        if held != &self.seen {
            self.take_in(Theirs::read(disk, self.model.replica(), false)?, true)?;
            self.seen = disk.manifest().hashes.clone();
        }
        // Notes this build cannot read are other people's words, which a note added here must not replace.
        if self.unread_notes && !self.notes.notes.is_empty() {
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
        if self.unread_style.as_ref() != Some(self.model.style()) {
            package.set_style(self.model.style());
        }
        if !(self.unread_notes && self.notes.notes.is_empty()) {
            package.set_notes(&self.notes);
        }
        package.entries.insert(OPLOG.into(), history);
        package
    }

    fn saved(&mut self, mut written: Package, manifest: &Manifest) {
        self.seen = manifest.hashes.clone();
        if written.entries.contains_key(STYLE) {
            self.unread_style = None;
        }
        self.unread_notes &= !written.entries.contains_key(NOTES);
        self.history = written.entries.remove(OPLOG);
    }
}

/// A file as read for merging: its history brought up to its document and style, and its notes.
struct Theirs {
    model: Model,
    notes: Notes,
    /// The history as the file holds it, when it could be read.
    history: Option<Vec<u8>>,
    /// Whether the style or the notes are ones this build cannot read. Such notes are given as none, and
    /// such a style as the history has it.
    unread_style: bool,
    unread_notes: bool,
}

impl Theirs {
    /// Reads `file`. `opening` says it is read for editing and not for merging: a history that cannot be
    /// read then starts over, and the built-in style stands in for one that cannot be read.
    ///
    /// Sharing the reader's replica id is safe: a copy that is only read writes nothing under it, and an
    /// outside change is written under an id of its own.
    fn read<R: Read + Seek>(file: &mut Archive<R>, replica: ReplicaId, opening: bool) -> Result<Self, Error> {
        let document = file.document()?;
        let (style, notes) = (file.style(), file.notes());
        let (unread_style, unread_notes) =
            (matches!(style, Err(Error::Style(_))), matches!(notes, Err(Error::Notes(_))));
        let style = if unread_style { opening.then(Style::default) } else { Some(style?) };
        let notes = if unread_notes { Notes::default() } else { notes? };

        let history = file.read(OPLOG)?;
        let loaded = match history.as_deref().map(|bytes| Model::load(bytes, replica)) {
            Some(Err(_)) if !opening => return Err(Error::Unmerged),
            loaded => loaded.and_then(Result::ok),
        };
        let (model, history) = match loaded {
            Some(mut model) => {
                // A history that was just loaded shows its newest revision, which can be rewritten.
                let _ = model.rewrite(document);
                if let Some(style) = style {
                    let _ = model.rewrite_style(style);
                }
                (model, history)
            }
            None => {
                let model = Model::new(document, replica);
                (style.into_iter().fold(model, Model::with_style), None)
            }
        };
        Ok(Self { model, notes, history, unread_style, unread_notes })
    }
}
