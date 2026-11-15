//! Conflict copies: the files a sync tool leaves beside a document when two devices saved it at once.
//!
//! The shell finds them here and the window offers to merge them. A copy is only read until the person
//! asks for the merge, and goes to the system's trash once the document's own file holds what it held.
//!
//! A file copied by hand is no conflict copy. It says it is its original until it is first written in,
//! and [`twin`] finds the original it then stops being.

use std::cmp::Ordering;
use std::ffi::OsStr;
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use letters_format::{Archive, OPLOG, conflict_copy_of};
use letters_model::{Model, ReplicaId};

use crate::files::EXTENSION;

/// One of two files that are the same document by their manifests, where one is named as a sync tool names
/// a conflict copy of the other.
#[derive(Clone, Debug)]
pub struct Copy {
    pub path: PathBuf,
    /// The copy's history, as its file held it.
    history: Vec<u8>,
}

impl Copy {
    /// Whether the copy holds edits that `model` lacks. One that holds none is no conflict, and one whose
    /// history cannot be read may still be on its way from another device.
    pub fn adds_to(&self, model: &Model) -> bool {
        matches!(model.compare(&self.history), Ok(None | Some(Ordering::Less)))
    }

    /// Whether the file and `model` began as one document, so that the two merge into one and not into
    /// both, one after the other.
    pub fn related_to(&self, model: &Model) -> bool {
        model.related(&self.history).unwrap_or(false)
    }
}

/// What lies beside a document's file that it could be merged with.
#[derive(Clone, Debug, Default)]
pub struct Beside {
    /// The conflict copies of the document, in the order of their names.
    pub copies: Vec<Copy>,
    /// The file the document is itself a conflict copy of.
    pub original: Option<Copy>,
    /// The file the document was copied from by hand, when that was looked for.
    pub twin: Option<PathBuf>,
}

/// Looks beside the file at `home`, which holds the document `id`, for its conflict copies and for the
/// file it is a conflict copy of. This reads the folder and each file whose name fits, so it is for a
/// thread of its own.
///
/// With `known`, the paths of other documents the person has, the file it was copied from by hand is
/// looked for too, which reads every document beside it.
pub fn look(home: &Path, id: &str, known: Option<&[PathBuf]>) -> Beside {
    let twin = known.and_then(|known| twin(home, id, known));
    Beside { copies: beside(home, id), original: original(home, id), twin }
}

/// Whether a file's name is the one a sync tool gives a conflict copy.
fn copy_by_name(path: &Path) -> bool {
    path.file_name().and_then(OsStr::to_str).is_some_and(|name| conflict_copy_of(name).is_some())
}

/// The file that the one at `home`, which holds the document `id`, was copied from by hand: another file
/// that says it is the same document, beside it or among the `known` paths, where this one is the copy
/// of the two.
///
/// Told apart from the rest by what the files are, since the two are the same byte for byte until one is
/// written in. A conflict copy carries a sync tool's name for one, and neither it nor its original is
/// taken for a duplicate through the other. A file that was moved or renamed has left nothing behind
/// where it was. Of two that remain, the one that was carried on is the original: the one whose history
/// holds all of the other's and more. Where the two read alike, it is the one the system says was made
/// first. When a file was made says nothing once it was saved again, since a save puts a new file in
/// its place.
pub fn twin(home: &Path, id: &str, known: &[PathBuf]) -> Option<PathBuf> {
    if copy_by_name(home) {
        return None;
    }
    let own = fs::canonicalize(home).ok()?;
    let folder = own.parent()?;
    let beside = fs::read_dir(folder).into_iter().flatten().flatten().map(|entry| entry.path());
    let document = |path: &PathBuf| path.extension().is_some_and(|ending| ending.eq_ignore_ascii_case(EXTENSION));
    let elsewhere = known.iter().filter(|path| path.parent() != Some(folder)).cloned();
    let others = beside.filter(document).chain(elsewhere).filter_map(|path| fs::canonicalize(path).ok());
    let mut twins =
        others.filter(|real| *real != own && !copy_by_name(real)).filter_map(|real| read(real, id)).peekable();
    // Its own history is read only when there is a file to tell it from.
    twins.peek()?;
    // Nobody may vouch for this file, and its history is loaded here whatever the window does with it.
    let history = Some(read(own.clone(), id)?.history).filter(|history| crate::strangers::stands(history))?;
    let model = Model::load(&history, ReplicaId::new("twin")).ok()?;
    let made = |path: &Path| fs::metadata(path).and_then(|file| file.created()).ok();
    let born = made(&own);
    let copied = |twin: &Copy| match model.compare(&twin.history) {
        Ok(Some(Ordering::Greater)) | Err(_) => false,
        // Where either date is missing, or the two are one, the file that is written in first is the copy.
        Ok(Some(Ordering::Equal)) => !born.zip(made(&twin.path)).is_some_and(|(own, other)| own < other),
        // One that was left behind, or that both were written in, is no longer its original.
        Ok(Some(Ordering::Less) | None) => true,
    };
    twins.find(copied).map(|twin| twin.path)
}

/// Reads the file at `path` as one side of a conflict, if it holds the document `id`.
fn read(path: PathBuf, id: &str) -> Option<Copy> {
    let mut archive = Archive::open(BufReader::new(fs::File::open(&path).ok()?)).ok()?;
    (archive.manifest().id == id).then_some(())?;
    let history = archive.read(OPLOG).ok()??;
    Some(Copy { path, history })
}

/// The file that the one at `home`, which holds the document `id`, is a conflict copy of: the one beside
/// it that its name points to, if that holds the same document.
pub fn original(home: &Path, id: &str) -> Option<Copy> {
    let named = conflict_copy_of(home.file_name()?.to_str()?)?;
    read(home.with_file_name(named), id)
}

/// The conflict copies of the document `id` that lie beside its file at `home`, in the order of their
/// names. This reads the folder and each file named like a copy, so it is for a thread of its own.
///
/// A name alone proves nothing: a file that is named like a copy and is another document, or cannot be
/// read yet, is left out.
pub fn beside(home: &Path, id: &str) -> Vec<Copy> {
    let Some(own) = home.file_name().and_then(OsStr::to_str) else { return Vec::new() };
    let folder = home.parent().filter(|folder| !folder.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
    let copy = |path: PathBuf| {
        let name = path.file_name()?.to_str()?;
        (conflict_copy_of(name).as_deref() == Some(own)).then_some(())?;
        read(path, id)
    };
    let mut copies: Vec<Copy> = entries.flatten().filter_map(|entry| copy(entry.path())).collect();
    copies.sort_by(|a, b| a.path.cmp(&b.path));
    copies
}

/// Moves a copy that was merged to the system's trash, from where the person can still take it back.
pub fn discard(path: &Path) -> Result<(), String> {
    #[cfg_attr(not(target_os = "macos"), expect(unused_mut))]
    let mut trash = trash::TrashContext::default();
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        // Asked of the system itself. Through Finder, the person would first be asked to let Letters
        // control it.
        trash.set_delete_method(DeleteMethod::NsFileManager);
    }
    trash.delete(path).map_err(|error| error.to_string())
}
