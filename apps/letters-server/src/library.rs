//! The documents of a space: what its folders hold, and where each document is.
//!
//! A document is known by the id in its file, which a rename, a move or a sync tool leaves alone. The index
//! from ids to files is a cache: a file is read when its size or time changed, and a document that is not
//! where the index says is found again by a scan. Nothing here is the truth but the files.

use std::collections::{HashMap, HashSet};
use std::io::{BufReader, Cursor, Read, Seek};
use std::sync::Mutex;

use letters_format::{
    ASSETS, Archive, Files, HISTORY, NOTES, OPLOG, Reading, Replica, Save, Saved, WorkingCopy, conflict_copy_of,
    file_name, hash,
};
use letters_model::{Block, BlockId, BlockKind, Document, Model, NoteKind, Op, ReplicaId, Style};
use serde::{Deserialize, Serialize};

use letters_api::Code;

use crate::error::{Answer, Refusal};
use crate::ids;
use crate::spaces::{EXTENSION, Folder, Listed, Place, Root, name_ok, seconds};
use crate::store::{Batch, Store, StoreError, Table};

/// What a new document is called until its first heading names it.
pub const UNTITLED: &str = "Untitled";

/// Whether `name` is what a document with no name of its own is called: "Untitled", or "Untitled 2" and
/// so on. The name alone says so, since other writers of a folder start documents the same way.
pub fn untitled(name: &str) -> bool {
    match name.strip_prefix(UNTITLED) {
        Some("") => true,
        Some(rest) => {
            rest.strip_prefix(' ').is_some_and(|count| !count.is_empty() && count.bytes().all(|c| c.is_ascii_digit()))
        }
        None => false,
    }
}
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
    /// The file's history, when it is known to be sound.
    #[serde(default)]
    sound: Option<Sound>,
}

/// A history that need not be checked again: it passed the check, or the server itself wrote it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Sound {
    /// The hash of the history's bytes, as the server worked it out from the bytes it read or wrote. What
    /// a file's manifest says of them is never taken for it.
    history: String,
    /// The check it stands for, as [`Model::check_name`] names it. Another check knows nothing of it.
    by: String,
}

impl Sound {
    fn of(history: &[u8]) -> Self {
        Self { history: hash(history), by: Model::check_name() }
    }

    fn is(&self, history: &[u8]) -> bool {
        self.by == Model::check_name() && self.history == hash(history)
    }
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
                        let now = read(&folder, &listed, was.as_ref());
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

    /// The file at `place` as a document of its own, with nothing of how it came to be. It is what a link
    /// hands whoever it let in, and what the desktop app's clean copy holds:
    ///
    /// - An id and a date of its own. With the original's, a merge of the two would bring the history back.
    /// - No history, and so no earlier text and no names of who wrote what.
    /// - No notes: comments, import notes and conflicts.
    /// - A picture only where the document shows it. The file may hold others that its history names.
    pub fn clean(&self, root: &Root, place: &Place, now: u64) -> Answer<Vec<u8>> {
        let mut package = self.open(root, place)?.package()?;
        (package.id, package.created) = (ids::document_id(), ids::rfc3339(now));
        // A preview no link names is kept for the history's sake, which goes.
        package.document.prune_previews();
        let shown = package.document.assets();
        let kept = |name: &str| match name {
            NOTES => false,
            name if name.starts_with(HISTORY) => false,
            name if name.starts_with(ASSETS) => shown.contains(name),
            _ => true,
        };
        package.entries.retain(|name, _| kept(name));
        let mut bytes = Cursor::new(Vec::new());
        letters_format::write(&mut bytes, &package)?;
        Ok(bytes.into_inner())
    }

    /// Merges `theirs`, another copy of the document `id`, into its file at `place`. Nothing in the file is
    /// replaced: the two meet as two writers of one folder do, and merging the same copy again changes
    /// nothing.
    ///
    /// The copy was read before this, where the document was found then. A file that is not there any
    /// more, or is another document's, is left alone, and the answer is [`moved`].
    pub fn merge<R: Read + Seek>(
        &self,
        store: &Store,
        root: &Root,
        place: &Place,
        id: &str,
        theirs: Reading<'_, R>,
        now: u64,
    ) -> Answer<()> {
        let (folder, name) = place.split();
        let gone = |error: std::io::Error| match Refusal::from(error) {
            refusal if refusal.code() == Code::NotFound => moved(),
            refusal => refusal,
        };
        let mut folder = root.folder(&folder).map_err(gone)?;
        let mut ours = Archive::open(BufReader::new(folder.open(name).map_err(gone)?))?;
        if ours.manifest().id != id {
            return Err(moved());
        }
        // The file is not the server's own writing either: its history is checked before it is loaded,
        // unless the index knows it.
        let mut copy = opened(store, root, place, &mut ours)?;
        copy.merge_read(theirs).map_err(unmerged)?;
        let (token, day) = (ids::block_id(), ids::rfc3339(now));
        let label = format!("{} server", day.get(..10).unwrap_or(&day));
        let save = Save { name, token: &token, label: &label, attempts: 3 };
        // A copy beside the file holds the document as safely as the file would.
        match letters_format::save(&mut folder, &mut copy, &save)? {
            Saved::ConflictCopy { name: beside, cause } => crate::log(&format!(
                "\"{name}\" could not be written ({cause}), and what was merged went to \"{beside}\""
            )),
            Saved::Written | Saved::Restored { .. } => wrote(store, root, place, &copy),
            Saved::Unchanged => {}
        }
        Ok(())
    }

    /// Moves the file or folder at `from` to `to`, in the same root or another. Nothing at `to` is replaced.
    pub fn relocate(&self, store: &Store, from: (&Root, &Place), to: (&Root, &Place)) -> Answer<()> {
        let ((from_folder, from_name), (to_folder, to_name)) = (from.1.split(), to.1.split());
        from.0.folder(&from_folder)?.move_to(from_name, &to.0.folder(&to_folder)?, to_name)?;
        self.forget(store, from.0, from.1)?;
        self.carry(store, from, Some(to))
    }

    /// Moves the document or folder at `place` to its root's trash, which remembers where it was.
    pub fn trash(&self, store: &Store, root: &Root, place: &Place, now: u64) -> Answer<()> {
        let (folder, name) = place.split();
        let (folder, trash) = (root.folder(&folder)?, root.trash()?);
        let kept = moved_free(&folder, name, &trash, name)?;
        store.put(&THROWN, &format!("{}\0{kept}", root.key), &Thrown { path: place.as_str().to_owned(), at: now })?;
        self.forget(store, root, place)?;
        self.carry(store, (root, place), None)
    }

    /// The documents and folders in the trash of `root`, the last thrown away first, each with what the
    /// trash remembers of it.
    pub fn thrown(&self, store: &Store, root: &Root) -> Answer<Vec<(Listed, Option<Thrown>)>> {
        let Some(trash) = root.thrown()? else { return Ok(Vec::new()) };
        let prefix = format!("{}\0", root.key);
        let mut remembered: HashMap<String, Thrown> = store.under(&THROWN, &prefix)?.into_iter().collect();
        let mut thrown: Vec<(Listed, Option<Thrown>)> = (trash.list()?.into_iter())
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
        // The documents of a folder that is back are in no index: the next one asked for by its id is
        // looked for at once.
        self.scanned.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&root.key);
        place.child(&kept)
    }

    /// Takes the file at `place` out of the index, once it is no longer there.
    fn forget(&self, store: &Store, root: &Root, place: &Place) -> Answer<()> {
        Ok(store.write(|batch| match batch.get(&FILES, &file_key(root, place))? {
            Some(was) => index(batch, root, place, Some(&was), None),
            None => Ok(()),
        })?)
    }

    /// Takes what the index holds of the files in the folder that was at `from` along to `to`, where the
    /// folder is now, or out of the index when the folder went to the trash. A document in a folder that
    /// moved is then found by its id at once, where the next scan would find it only later.
    fn carry(&self, store: &Store, from: (&Root, &Place), to: Option<(&Root, &Place)>) -> Answer<()> {
        let prefix = format!("{}/", file_key(from.0, from.1));
        let inside = |folder: &Place, rest: &str| Place::parse(&format!("{}/{rest}", folder.as_str())).ok();
        let mut moved = Vec::new();
        for (key, known) in store.under(&FILES, &prefix)? {
            let rest = &key[prefix.len()..];
            // A row that names no place is found out by the next scan.
            let Some(was) = inside(from.1, rest) else { continue };
            moved.push((was, to.and_then(|(root, folder)| Some((root, inside(folder, rest)?))), known));
        }
        Ok(store.write(|batch| {
            for (was, now, known) in &moved {
                index(batch, from.0, was, Some(known), None)?;
                if let Some((root, now)) = now {
                    let stale = batch.get(&FILES, &file_key(root, now))?;
                    index(batch, root, now, stale.as_ref(), Some(known))?;
                }
            }
            Ok(())
        })?)
    }
}

/// Moves the document or folder `name` of `from` to `to` as `wanted`, or as the first of "`wanted` 2" and
/// so on that is free there. Says the name it has now.
fn moved_free(from: &Folder, name: &str, to: &Folder, wanted: &str) -> Answer<String> {
    let (stem, ending) = match wanted.strip_suffix(EXTENSION) {
        Some(stem) => (stem, EXTENSION),
        None => (wanted, ""),
    };
    for n in 1.. {
        let kept = if n == 1 { wanted.to_owned() } else { format!("{stem} {n}{ending}") };
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
fn read(folder: &Folder, listed: &Listed, was: Option<&Known>) -> Known {
    let (len, modified) = (listed.stamp.len, listed.stamp.modified);
    let damaged =
        Known { len, modified, id: None, created: None, contributors: Vec::new(), import_notes: 0, sound: None };
    let Ok(file) = folder.open(&listed.name) else { return damaged };
    let Ok(mut archive) = Archive::open(BufReader::new(file)) else { return damaged };
    let manifest = archive.manifest();
    let (id, created) = (Some(manifest.id.clone()), Some(manifest.created.clone()));
    let open = |notes: letters_model::Notes| {
        notes.notes.iter().filter(|note| note.kind == NoteKind::Import && !note.resolved).count() as u32
    };
    let import_notes = archive.notes().map(open).unwrap_or(0);
    let history = archive.read(OPLOG).ok().flatten();
    // The history the file had before was checked then, and says the same of who wrote it.
    let same = |was: &&Known| was.sound.as_ref().zip(history.as_deref()).is_some_and(|(sound, now)| sound.is(now));
    if let Some(was) = was.filter(same) {
        let (contributors, sound) = (was.contributors.clone(), was.sound.clone());
        return Known { len, modified, id, created, contributors, import_notes, sound };
    }
    // Who wrote it is in the history, which is checked first: a folder holds whatever was put there. The
    // replica only reads, so any id will do for it.
    let history = history.filter(|history| Model::check(history).is_ok());
    let sound = history.as_deref().map(Sound::of);
    let model = history.and_then(|history| Model::load(&history, ReplicaId::from(0)).ok());
    let contributors = model.map(|model| model.authors().into_iter().filter(|name| !name.is_empty()).collect());
    Known { len, modified, id, created, contributors: contributors.unwrap_or_default(), import_notes, sound }
}

/// Opens the document in `archive`, which is the file at `place`, for the server to write on. A folder
/// holds whatever was put there, so the history is checked before it is loaded, unless the index knows
/// these very bytes to be sound: a long history is then read once, where the check reads it twice more.
pub(crate) fn opened<R: Read + Seek>(
    store: &Store,
    root: &Root,
    place: &Place,
    archive: &mut Archive<R>,
) -> Result<WorkingCopy, letters_format::Error> {
    let replica = ReplicaId::new(&ids::block_id());
    let known = store.get(&FILES, &file_key(root, place)).ok().flatten().and_then(|known| known.sound);
    let sound = match known {
        Some(sound) => archive.read(OPLOG)?.is_some_and(|history| sound.is(&history)),
        None => false,
    };
    if !sound {
        return WorkingCopy::open_wary(archive, replica);
    }
    let mut copy = WorkingCopy::open(archive, replica)?;
    // What is merged into it from here on is tried first, as with a copy that was opened warily.
    copy.wary = true;
    Ok(copy)
}

/// Takes into the index what the server itself just wrote to the file at `place`, as the copy it wrote
/// tells it. The file is then not read to learn what the server knows: a long history would be checked and
/// loaded again after every save. A fault here is none: the next listing reads the file.
pub(crate) fn wrote(store: &Store, root: &Root, place: &Place, copy: &WorkingCopy) {
    let (folder, name) = place.split();
    let stamp = root.folder(&folder).ok().and_then(|mut folder| Files::stamp(&mut folder, name).ok().flatten());
    let Some(stamp) = stamp else { return };
    let open = copy.notes.notes.iter().filter(|note| note.kind == NoteKind::Import && !note.resolved);
    // The history as the save wrote it, which the copy keeps: what the server wrote it need not check.
    let history = copy.package().entries.remove(OPLOG);
    let known = Known {
        len: stamp.len,
        modified: stamp.modified,
        id: Some(copy.id.clone()),
        created: Some(copy.created.clone()),
        contributors: copy.model().authors().into_iter().filter(|name| !name.is_empty()).collect(),
        import_notes: open.count() as u32,
        sound: history.as_deref().map(Sound::of),
    };
    let key = file_key(root, place);
    let _ = store.write(|batch| {
        let was = batch.get(&FILES, &key)?;
        index(batch, root, place, was.as_ref(), Some(&known))
    });
}

/// How a copy that could not be merged into a document is refused.
pub(crate) fn unmerged(error: letters_format::Error) -> Refusal {
    match error {
        letters_format::Error::Unmerged => Refusal::invalid("That file shares nothing with the document."),
        letters_format::Error::History(_) => Refusal::invalid(HARMFUL),
        error => error.into(),
    }
}

/// What a copy is refused with when its document is not where it was found any more: a copy is read
/// before its document is held, and the document may be moved or thrown away meanwhile. Asked again, the
/// merge finds the document where it is now.
pub(crate) fn moved() -> Refusal {
    Refusal::new(Code::Stale, "The document was moved or deleted while the file was being read. Try again.")
}

/// What is said of a copy whose history cannot be read, or closed the room that tried it.
pub(crate) const HARMFUL: &str = "That file's history cannot be merged.";

/// A `.let` file as bytes, from a working copy: for a document no file holds yet.
pub fn packed(copy: &WorkingCopy) -> Answer<Vec<u8>> {
    let mut bytes = Cursor::new(Vec::new());
    letters_format::write(&mut bytes, &copy.package())?;
    Ok(bytes.into_inner())
}
