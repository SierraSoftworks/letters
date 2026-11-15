//! Conflict copies: the files a sync tool leaves beside a document when two devices saved it at once.
//!
//! The shell finds them here and the window offers to merge them. A copy is only read until the person
//! asks for the merge, and goes to the system's trash once the document's own file holds what it held.

use std::cmp::Ordering;
use std::ffi::OsStr;
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use letters_format::{Archive, OPLOG, conflict_copy_of};
use letters_model::Model;

/// A file beside a document that is named as a sync tool names a conflict copy of it, and whose manifest
/// says it is the same document.
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
        let mut archive = Archive::open(BufReader::new(fs::File::open(&path).ok()?)).ok()?;
        (archive.manifest().id == id).then_some(())?;
        let history = archive.read(OPLOG).ok()??;
        Some(Copy { path, history })
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
