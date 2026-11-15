//! Saving to a shared folder without locks.
//!
//! Network filesystems make locks unreliable, so every writer saves the same way: read the file and merge what
//! is new, write a temporary sibling, and rename it over the original. A lost race is healed by the next save's
//! merge, and when the file cannot be replaced the work goes to a conflict copy beside it. Nothing is lost.

use std::io::{self, Cursor};

use crate::{Archive, Error, Manifest, Package, Plan};

/// One folder, as the shell lends it. Names are file names inside it.
///
/// Every call opens, reads or writes, and closes: nothing may hold a handle that would block another writer.
pub trait Files {
    /// What the file looks like from outside. `None` when there is no such file.
    fn stamp(&mut self, name: &str) -> io::Result<Option<Stamp>>;
    /// The whole file. `None` when there is no such file.
    fn read(&mut self, name: &str) -> io::Result<Option<Vec<u8>>>;
    /// Creates or replaces a file, and returns once its bytes are flushed to disk.
    fn write(&mut self, name: &str, bytes: &[u8]) -> io::Result<()>;
    /// Adds to the end of a file, creating it, and returns once the bytes are flushed to disk. A shell with
    /// real files appends in place: the journal calls this for every edit.
    fn append(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let mut whole = self.read(name)?.unwrap_or_default();
        whole.extend_from_slice(bytes);
        self.write(name, &whole)
    }
    /// Renames a file in one step, replacing whatever `to` held.
    fn rename(&mut self, from: &str, to: &str) -> io::Result<()>;
    fn remove(&mut self, name: &str) -> io::Result<()>;
    /// Called before each retry, counted from 1. The shell may wait here, since the core has no clock.
    fn wait(&mut self, _retry: u32) {}
}

/// A file's size and modification time. Two equal stamps are only a hint that nothing changed, and a stamp
/// never orders two files: content hashes and the merge do that.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub len: u64,
    /// As the filesystem reports it, in any unit.
    pub modified: u64,
}

/// A copy of the file as read from the folder.
pub type DiskCopy = Archive<Cursor<Vec<u8>>>;

/// The document in memory, as a save sees it.
pub trait Replica {
    /// The document's identity. A file holding another document is never merged or replaced.
    fn id(&self) -> &str;

    /// The id the document went by before it became a document of its own, while its file may still carry
    /// that id. Such a file is this document's from before: it is merged and replaced like its own.
    fn was(&self) -> Option<&str> {
        None
    }

    /// Takes in whatever the copy on disk holds that memory lacks. This is also where a writer learns which
    /// of its own journalled changes have reached the shared file.
    ///
    /// An error means the copy cannot be read as this document, and the save treats it as damaged. The
    /// exception is [`Error::Unmerged`]: the copy is sound but cannot be merged, so the save leaves it alone
    /// and writes a conflict copy.
    fn merge(&mut self, disk: &mut DiskCopy) -> Result<(), Error>;

    /// The document as it should be written now.
    fn package(&self) -> Package;

    /// Called once the file has been replaced, with what it now holds: the package as written, and the
    /// manifest, which also lists the entries carried over.
    fn saved(&mut self, _written: Package, _manifest: &Manifest) {}

    /// Whether to carry over an entry of the file on disk that the package does not hold. Keeping is the safe
    /// default: it preserves assets that were never loaded and entries that a newer build added.
    fn keeps(&self, _name: &str) -> bool {
        true
    }
}

/// One save of one file.
#[derive(Clone, Copy, Debug)]
pub struct Save<'a> {
    /// The file to replace, such as `report.let`.
    pub name: &'a str,
    /// Unique to this save, so that two writers never share a temporary file.
    pub token: &'a str,
    /// Tells a conflict copy or a damaged file apart from others, such as a date and a device name.
    pub label: &'a str,
    /// How often to try before writing a conflict copy.
    pub attempts: u32,
}

/// How a save ended. The document is safe on disk in every case.
#[derive(Debug)]
pub enum Saved {
    /// The file already held everything in memory, so nothing was written.
    Unchanged,
    /// The file was replaced.
    Written,
    /// The file could not be read as a document. It was moved to `aside` and written again from memory.
    Restored { aside: String },
    /// The file could not be replaced, for the reason in `cause`. The document went to `name` beside it.
    ConflictCopy { name: String, cause: Error },
}

/// Saves `replica` over the file `save` names.
///
/// Fails only when the document could not be written to the folder under any name.
pub fn save(files: &mut impl Files, replica: &mut impl Replica, save: &Save) -> Result<Saved, Error> {
    let name = save.name;
    let temporary = format!(".{name}.{}.tmp", save.token);
    let mut aside = None;
    // The last readable copy, which a conflict copy can still take unloaded entries from.
    let mut base: Option<DiskCopy> = None;
    let mut cause = Error::Files(String::new());

    let attempts = save.attempts.max(1);
    for attempt in 0..attempts {
        if attempt > 0 {
            files.wait(attempt);
        }
        let last = attempt + 1 == attempts;

        let (mut before, disk) = match files.stamp(name).and_then(|stamp| Ok((stamp, files.read(name)?))) {
            Ok(read) => read,
            Err(error) => {
                cause = folder(error);
                continue;
            }
        };
        base = None;
        if let Some(bytes) = disk {
            let (copy, merged) = open(bytes, replica);
            base = copy;
            match merged {
                Ok(()) => {}
                // Another document, a newer build's file, or changes we cannot merge are never ours to replace.
                Err(error @ (Error::OtherDocument(_) | Error::NewerFormat(_) | Error::Unmerged)) => {
                    cause = error;
                    break;
                }
                // A file that fails validation may still be syncing, so it gets every attempt to arrive.
                Err(error) if !last => {
                    cause = error;
                    continue;
                }
                Err(_) => {
                    let damaged = free_name(files, &sibling(name, &format!("damaged {}", save.label)));
                    if let Err(error) = files.rename(name, &damaged) {
                        cause = folder(error);
                        break;
                    }
                    aside = Some(damaged);
                    before = None;
                }
            }
        }

        let package = replica.package();
        let plan = Plan::new(&package, base.as_ref().map(Archive::manifest), |name| replica.keeps(name));
        if base.as_ref().is_some_and(|base| base.manifest() == &plan.manifest) {
            return Ok(Saved::Unchanged);
        }
        let mut bytes = Cursor::new(Vec::new());
        plan.write(&mut bytes, base.as_mut())?;

        // The stamp is checked as late as possible. A writer that slips in after it is merged by the next save.
        let replaced = files.write(&temporary, bytes.get_ref()).and_then(|()| {
            if files.stamp(name)? != before {
                return Err(io::Error::other("the file changed while it was being saved"));
            }
            files.rename(&temporary, name)
        });
        match replaced {
            Ok(()) => {
                let manifest = plan.manifest;
                replica.saved(package, &manifest);
                return Ok(aside.map_or(Saved::Written, |aside| Saved::Restored { aside }));
            }
            Err(error) => {
                let _ = files.remove(&temporary);
                cause = folder(error);
            }
        }
    }

    let package = replica.package();
    let plan = Plan::new(&package, base.as_ref().map(Archive::manifest), |name| replica.keeps(name));
    let mut bytes = Cursor::new(Vec::new());
    plan.write(&mut bytes, base.as_mut())?;
    let copy = free_name(files, &sibling(name, &format!("conflicted copy {}", save.label)));
    let written = files.write(&temporary, bytes.get_ref()).and_then(|()| files.rename(&temporary, &copy));
    if let Err(error) = written {
        let _ = files.remove(&temporary);
        return Err(folder(error));
    }
    Ok(Saved::ConflictCopy { name: copy, cause })
}

fn folder(error: io::Error) -> Error {
    Error::Files(error.to_string())
}

/// Opens what was read from disk and merges it into memory.
///
/// A copy of this document that could not be merged is handed back all the same: the conflict copy carries
/// over its unloaded entries.
fn open(bytes: Vec<u8>, replica: &mut impl Replica) -> (Option<DiskCopy>, Result<(), Error>) {
    let mut copy = match Archive::open(Cursor::new(bytes)) {
        Ok(copy) => copy,
        Err(error) => return (None, Err(error)),
    };
    let id = copy.manifest().id.as_str();
    if id != replica.id() && Some(id) != replica.was() {
        return (None, Err(Error::OtherDocument(id.to_owned())));
    }
    match replica.merge(&mut copy) {
        Ok(()) => (Some(copy), Ok(())),
        Err(Error::Unmerged) => (Some(copy), Err(Error::Unmerged)),
        Err(error) => (None, Err(error)),
    }
}

/// Splits `report.let` into `report` and `.let`.
fn split(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    }
}

/// `report (note).let` for `report.let`.
fn sibling(name: &str, note: &str) -> String {
    let (stem, extension) = split(name);
    format!("{stem} ({note}){extension}")
}

/// `name`, or its first numbered variant that no file has yet, so that an earlier copy is never replaced.
fn free_name(files: &mut impl Files, name: &str) -> String {
    let (stem, extension) = split(name);
    let stem = stem.strip_suffix(')').unwrap_or(stem);
    let mut numbered = (2..).map(|n| format!("{stem} {n}){extension}"));
    let mut candidate = name.to_owned();
    while matches!(files.stamp(&candidate), Ok(Some(_))) {
        candidate = numbered.next().expect("the numbers do not run out");
    }
    candidate
}

/// The file a conflict copy was made from, when `name` follows the pattern of one.
///
/// Recognises the copies Letters writes, the `conflicted copy` names of Dropbox and Nextcloud and the
/// `sync-conflict` names of Syncthing. A name is only a hint: a real conflict copy also holds the same
/// document id as its original.
pub fn conflict_copy_of(name: &str) -> Option<String> {
    let (stem, extension) = split(name);
    if let Some(at) = stem.find(".sync-conflict-") {
        return Some(format!("{}{extension}", &stem[..at]));
    }
    let marker = stem.rfind("conflicted copy")?;
    let open = stem[..marker].rfind(" (")?;
    (open > 0 && stem.ends_with(')')).then(|| format!("{}{extension}", &stem[..open]))
}
