//! The documents of a space: what its folders hold, and where each document is.
//!
//! A document is known by the id in its file, which a rename, a move or a sync tool leaves alone. The index
//! from ids to files is a cache: a file is read when its size or time changed, and a document that is not
//! where the index says is found again by a scan. Nothing here is the truth but the files.

use std::collections::{HashMap, HashSet};
use std::io::{BufReader, Cursor};
use std::sync::Mutex;

use letters_format::{Archive, Files, OPLOG, Replica, WorkingCopy, conflict_copy_of, file_name};
use letters_model::{Block, BlockId, BlockKind, Document, Model, NoteKind, Op, ReplicaId, Style};
use serde::{Deserialize, Serialize};

use letters_api::Code;

use crate::error::{Answer, Refusal};
use crate::ids;
use crate::spaces::{EXTENSION, Folder, Listed, Place, Root, name_ok, seconds};
use crate::store::{Batch, Store, StoreError, Table};

/// What a new document is called until its first heading names it.
pub const UNTITLED: &str = "Untitled";
/// How long a root is taken as scanned. A lost document is looked for again only after that, so asking
/// for ids that do not exist cannot keep the server walking its folders.
const SCAN_REST: u64 = 30;

/// What the index keeps of one file. It stands while the file's size and time are the ones here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Known {
    len: u64,
    modified: u64,
    /// `None` for a file that could not be read as a document.
    pub id: Option<String>,
    pub created: Option<String>,
    pub contributors: Vec<String>,
    /// The import notes nobody has settled.
    pub import_notes: u32,
}

/// A file that holds a document: a root, by its key, and the place in it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct At {
    root: String,
    path: String,
}

/// By root and place.
const FILES: Table<Known> = Table::new("files");
/// By document id: every file that holds the document.
const HOMES: Table<Vec<At>> = Table::new("homes");

/// What the trash remembers of a file in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thrown {
    /// Where the file was in its space.
    pub path: String,
    /// When it was thrown away.
    pub at: u64,
}

/// By root and the file's name in the trash. A file some other program put there has no row.
const THROWN: Table<Thrown> = Table::new("trash");

fn file_key(root: &Root, place: &Place) -> String {
    format!("{}\0{}", root.key, place.as_str())
}

/// One row of a folder: a name, and for a document what its file says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub place: Place,
    pub listed: Listed,
    pub known: Option<Known>,
    /// The file beside it that this one is a conflict copy of.
    pub copy_of: Option<Place>,
}

pub struct Library {
    /// When each root was last scanned, by its key.
    scanned: Mutex<HashMap<String, u64>>,
}

impl Default for Library {
    fn default() -> Self {
        Self::new()
    }
}

impl Library {
    pub fn new() -> Self {
        Self { scanned: Mutex::new(HashMap::new()) }
    }

    /// What the folder at `place` holds, read from the folder now.
    pub fn list(&self, store: &Store, root: &Root, place: &Place) -> Answer<Vec<Row>> {
        let folder = root.folder(place)?;
        let listed = folder.list()?;
        let mut rows = Vec::with_capacity(listed.len());
        let mut changed = Vec::new();
        for listed in listed {
            let at = place.child(&listed.name)?;
            let known = match listed.folder {
                true => None,
                false => Some(match store.get(&FILES, &file_key(root, &at))? {
                    Some(known) if (known.len, known.modified) == (listed.stamp.len, listed.stamp.modified) => known,
                    was => {
                        let now = read(&folder, &listed);
                        changed.push((at.clone(), was, now.clone()));
                        now
                    }
                }),
            };
            rows.push(Row { place: at, listed, known, copy_of: None });
        }
        if !changed.is_empty() {
            store.write(|batch| {
                changed.iter().try_for_each(|(at, was, now)| index(batch, root, at, was.as_ref(), Some(now)))
            })?;
        }
        // A conflict copy is one by its name and by holding the document of the file it names.
        let ids: HashMap<&str, Option<&str>> = rows
            .iter()
            .map(|row| (row.listed.name.as_str(), row.known.as_ref().and_then(|known| known.id.as_deref())))
            .collect();
        let copies: Vec<Option<Place>> = rows
            .iter()
            .map(|row| {
                let original = conflict_copy_of(&row.listed.name)?;
                let same = ids.get(original.as_str())?.is_some()
                    && ids.get(original.as_str()) == ids.get(row.listed.name.as_str());
                same.then(|| place.child(&original).ok()).flatten()
            })
            .collect();
        for (row, copy_of) in rows.iter_mut().zip(copies) {
            row.copy_of = copy_of;
        }
        Ok(rows)
    }

    /// Where the document `id` is among `roots`: which root, and the place in it. A document with conflict
    /// copies is found at its original.
    ///
    /// A document that is not where the index says was moved or renamed behind the server's back, by a
    /// sync tool or over a share, and is looked for in every root.
    pub fn find(&self, store: &Store, roots: &[&Root], id: &str, now: u64) -> Answer<Option<(usize, Place)>> {
        for rescan in [false, true] {
            // Every root is looked through, not only up to the first that was.
            let scan = |root: &&&Root| self.scan_rested(store, root, now).unwrap_or(false);
            if rescan && roots.iter().filter(scan).count() == 0 {
                break;
            }
            let mut homes: Vec<(bool, usize, Place)> = Vec::new();
            for at in store.get(&HOMES, id)?.unwrap_or_default() {
                let Some(root) = roots.iter().position(|root| root.key == at.root) else { continue };
                let Ok(place) = Place::parse(&at.path) else { continue };
                homes.push((conflict_copy_of(place.name()).is_some(), root, place));
            }
            homes.sort_by(|a, b| (a.0, a.1, a.2.as_str()).cmp(&(b.0, b.1, b.2.as_str())));
            for (_, root, place) in homes {
                if self.holds(store, roots[root], &place, id)? {
                    return Ok(Some((root, place)));
                }
            }
        }
        Ok(None)
    }

    /// Whether the file at `place` holds the document `id` now.
    fn holds(&self, store: &Store, root: &Root, place: &Place, id: &str) -> Answer<bool> {
        let (folder, name) = place.split();
        // A folder that is gone holds nothing.
        let Ok(rows) = self.list(store, root, &folder) else { return Ok(false) };
        let row = rows.iter().find(|row| row.listed.name == name);
        Ok(row.and_then(|row| row.known.as_ref()).is_some_and(|known| known.id.as_deref() == Some(id)))
    }

    /// Scans `root` unless it was scanned a moment ago. Says whether it was scanned.
    fn scan_rested(&self, store: &Store, root: &Root, now: u64) -> Answer<bool> {
        {
            let mut scanned = self.scanned.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if scanned.get(&root.key).is_some_and(|last| now < last + SCAN_REST) {
                return Ok(false);
            }
            scanned.insert(root.key.clone(), now);
        }
        self.scan(store, root).map(|()| true)
    }

    /// Reads every folder of `root`, so that the index knows each document there is, and forgets the files
    /// that are gone.
    pub fn scan(&self, store: &Store, root: &Root) -> Answer<()> {
        let mut seen = HashSet::new();
        let mut folders = vec![Place::default()];
        while let Some(place) = folders.pop() {
            // A folder that cannot be read, or went away meanwhile, holds nothing to find.
            let Ok(rows) = self.list(store, root, &place) else { continue };
            for row in rows {
                match row.listed.folder {
                    true => folders.push(row.place),
                    false => drop(seen.insert(file_key(root, &row.place))),
                }
            }
        }
        let prefix = format!("{}\0", root.key);
        let gone: Vec<(String, Known)> =
            store.under(&FILES, &prefix)?.into_iter().filter(|(key, _)| !seen.contains(key)).collect();
        if gone.is_empty() {
            return Ok(());
        }
        Ok(store.write(|batch| {
            for (key, was) in &gone {
                let place = Place::parse(&key[prefix.len()..]).unwrap_or_default();
                index(batch, root, &place, Some(was), None)?;
            }
            Ok(())
        })?)
    }

    /// Starts a document in the folder at `place` and says where it is. With no `name` it is "Untitled",
    /// or the first "Untitled 2" and so on that is free.
    pub fn create(
        &self,
        root: &Root,
        place: &Place,
        name: Option<&str>,
        style: Style,
        author: &str,
        now: u64,
    ) -> Answer<Place> {
        let mut folder = root.folder(place)?;
        let stem = match name {
            Some(name) => {
                file_name(name).ok_or_else(|| Refusal::invalid("That name cannot be used for a document."))?
            }
            None => UNTITLED.into(),
        };
        let replica = ReplicaId::new(&ids::block_id());
        let mut copy = WorkingCopy::new(ids::document_id(), ids::rfc3339(now), Document::default(), style, replica);
        // The first line is written as its maker's, which is what names them in the history.
        copy.sign(author, now);
        let line = Block::text(BlockId::new(ids::block_id()), BlockKind::Paragraph, "");
        copy.apply([Op::InsertBlock { parent: None, index: 0, block: line }])
            .map_err(|error| Refusal::new(Code::Internal, format!("A document could not be started: {error}.")))?;
        // Written under a hidden name first, so that the document appears whole or not at all.
        let hidden = format!(".{stem}.{}.new", ids::block_id());
        Files::write(&mut folder, &hidden, &packed(&copy)?)?;
        for n in 1.. {
            let file = if n == 1 { format!("{stem}{EXTENSION}") } else { format!("{stem} {n}{EXTENSION}") };
            match folder.move_to(&hidden, &folder, &file) {
                Ok(()) => return place.child(&file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && name.is_none() => {}
                Err(error) => {
                    let _ = Files::remove(&mut folder, &hidden);
                    return Err(error.into());
                }
            }
        }
        unreachable!("some number is free")
    }

    /// Opens the document at `place` to read.
    pub fn open(&self, root: &Root, place: &Place) -> Answer<Archive<BufReader<std::fs::File>>> {
        let (folder, name) = place.split();
        let file = root.folder(&folder)?.open(name)?;
        Ok(Archive::open(BufReader::new(file))?)
    }

    /// The whole file at `place`.
    pub fn bytes(&self, root: &Root, place: &Place) -> Answer<Vec<u8>> {
        let (folder, name) = place.split();
        let bytes = Files::read(&mut root.folder(&folder)?, name)?;
        bytes.ok_or_else(Refusal::not_found)
    }

    /// Moves the file or folder at `from` to `to`, in the same root or another. Nothing at `to` is replaced.
    pub fn relocate(&self, store: &Store, from: (&Root, &Place), to: (&Root, &Place)) -> Answer<()> {
        let ((from_folder, from_name), (to_folder, to_name)) = (from.1.split(), to.1.split());
        from.0.folder(&from_folder)?.move_to(from_name, &to.0.folder(&to_folder)?, to_name)?;
        self.forget(store, from.0, from.1)
    }

    /// Moves the document at `place` to its root's trash, which remembers where it was.
    pub fn trash(&self, store: &Store, root: &Root, place: &Place, now: u64) -> Answer<()> {
        let (folder, name) = place.split();
        let (folder, trash) = (root.folder(&folder)?, root.trash()?);
        let kept = moved_free(&folder, name, &trash, name)?;
        store.put(&THROWN, &format!("{}\0{kept}", root.key), &Thrown { path: place.as_str().to_owned(), at: now })?;
        self.forget(store, root, place)
    }

    /// The documents in the trash of `root`, the last thrown away first, each with what the trash
    /// remembers of it.
    pub fn thrown(&self, store: &Store, root: &Root) -> Answer<Vec<(Listed, Option<Thrown>)>> {
        let Some(trash) = root.thrown()? else { return Ok(Vec::new()) };
        let prefix = format!("{}\0", root.key);
        let mut remembered: HashMap<String, Thrown> = store.under(&THROWN, &prefix)?.into_iter().collect();
        let files = trash.list()?.into_iter().filter(|listed| !listed.folder);
        let mut thrown: Vec<(Listed, Option<Thrown>)> = files
            .map(|listed| {
                let was = remembered.remove(&format!("{prefix}{}", listed.name));
                (listed, was)
            })
            .collect();
        thrown.sort_by_key(|(listed, was)| std::cmp::Reverse(was.as_ref().map_or(seconds(listed.stamp), |was| was.at)));
        Ok(thrown)
    }

    /// Takes `file` out of the trash of `root` and puts it back where it was, or at the top of its space
    /// when that folder is gone. Says where it is now: under another name when its own was taken meanwhile.
    pub fn put_back(&self, store: &Store, root: &Root, file: &str) -> Answer<Place> {
        let trash = root.thrown()?.filter(|trash| name_ok(file) && trash.has(file)).ok_or_else(Refusal::not_found)?;
        let key = format!("{}\0{file}", root.key);
        let was = store.get(&THROWN, &key)?.and_then(|was| Place::parse(&was.path).ok());
        // The name it had: the trash may hold it under another, beside a file of the same name.
        let (place, name) = match &was {
            Some(was) if root.folder(&was.split().0).is_ok() => (was.split().0, was.name()),
            Some(was) => (Place::default(), was.name()),
            None => (Place::default(), file),
        };
        let kept = moved_free(&trash, file, &root.folder(&place)?, name)?;
        store.remove(&THROWN, &key)?;
        place.child(&kept)
    }

    /// Takes the file at `place` out of the index, once it is no longer there.
    fn forget(&self, store: &Store, root: &Root, place: &Place) -> Answer<()> {
        Ok(store.write(|batch| match batch.get(&FILES, &file_key(root, place))? {
            Some(was) => index(batch, root, place, Some(&was), None),
            None => Ok(()),
        })?)
    }
}

/// Moves the document `name` of `from` to `to` as `wanted`, or as the first of "`wanted` 2" and so on that
/// is free there. Says the name it has now.
fn moved_free(from: &Folder, name: &str, to: &Folder, wanted: &str) -> Answer<String> {
    let stem = wanted.strip_suffix(EXTENSION).unwrap_or(wanted);
    for n in 1.. {
        let kept = if n == 1 { wanted.to_owned() } else { format!("{stem} {n}{EXTENSION}") };
        match from.move_to(name, to, &kept) {
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            done => {
                done?;
                return Ok(kept);
            }
        }
    }
    unreachable!("some number is free")
}

/// Brings the index up to what the file at `place` now says: `now`, or nothing when the file is gone.
fn index(
    batch: &mut Batch,
    root: &Root,
    place: &Place,
    was: Option<&Known>,
    now: Option<&Known>,
) -> Result<(), StoreError> {
    let key = file_key(root, place);
    let at = At { root: root.key.clone(), path: place.as_str().to_owned() };
    let (old, new) = (was.and_then(|known| known.id.as_deref()), now.and_then(|known| known.id.as_deref()));
    if old != new {
        if let Some(old) = old {
            let mut homes = batch.get(&HOMES, old)?.unwrap_or_default();
            homes.retain(|home| *home != at);
            match homes.is_empty() {
                true => batch.remove(&HOMES, old)?,
                false => batch.put(&HOMES, old, &homes)?,
            }
        }
        if let Some(new) = new {
            let mut homes = batch.get(&HOMES, new)?.unwrap_or_default();
            if !homes.contains(&at) {
                homes.push(at);
                batch.put(&HOMES, new, &homes)?;
            }
        }
    }
    match now {
        Some(now) => batch.put(&FILES, &key, now),
        None => batch.remove(&FILES, &key),
    }
}

/// Reads what the index keeps of a file. One that cannot be read is known as such, and not read again
/// until it changes.
fn read(folder: &Folder, listed: &Listed) -> Known {
    let (len, modified) = (listed.stamp.len, listed.stamp.modified);
    let damaged = Known { len, modified, id: None, created: None, contributors: Vec::new(), import_notes: 0 };
    let Ok(file) = folder.open(&listed.name) else { return damaged };
    let Ok(mut archive) = Archive::open(BufReader::new(file)) else { return damaged };
    let manifest = archive.manifest();
    let (id, created) = (Some(manifest.id.clone()), Some(manifest.created.clone()));
    // Who wrote it is in the history. The replica only reads, so any id will do for it.
    let history = archive.read(OPLOG).ok().flatten();
    let model = history.and_then(|history| Model::load(&history, ReplicaId::from(0)).ok());
    let contributors = model.map(|model| model.authors().into_iter().filter(|name| !name.is_empty()).collect());
    let open = |notes: letters_model::Notes| {
        notes.notes.iter().filter(|note| note.kind == NoteKind::Import && !note.resolved).count() as u32
    };
    let import_notes = archive.notes().map(open).unwrap_or(0);
    Known { len, modified, id, created, contributors: contributors.unwrap_or_default(), import_notes }
}

/// A `.let` file as bytes, from a working copy: for a document no file holds yet.
pub fn packed(copy: &WorkingCopy) -> Answer<Vec<u8>> {
    let mut bytes = Cursor::new(Vec::new());
    letters_format::write(&mut bytes, &copy.package())?;
    Ok(bytes.into_inner())
}
