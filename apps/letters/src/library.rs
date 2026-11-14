//! Where documents live on this machine: the folder new ones go to, the ones opened lately, and a place on
//! disk for a document that has none yet.
//!
//! Nobody is asked where to save. A new document gets its file once it has content, named after its first
//! Heading 1, and is renamed or moved from the top bar afterwards. Nothing here ever replaces a file.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use letters_format::{file_name, suggested_name};
use letters_model::{Block, BlockId, BlockKind, BodyShape, Document};
use serde_json::{Value, json};

use crate::files::{EXTENSION, Home};

/// The name of a document whose writer has given it none, in a heading or otherwise.
pub const UNTITLED: &str = "Untitled";
/// How many documents the recents list remembers.
const REMEMBERED: usize = 50;
/// How much of the folder tree is offered, so a huge one cannot hold up the window.
const FOLDERS: usize = 500;
const DEPTH: u8 = 8;

/// A document opened lately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    pub path: PathBuf,
    /// When it was last opened or saved somewhere new, as RFC 3339.
    pub opened: String,
}

impl Recent {
    /// The file name without its extension, which is the document's title.
    pub fn name(&self) -> String {
        title(&self.path)
    }
}

/// A folder a document can be moved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub path: PathBuf,
    pub name: String,
    /// How far below the top of the tree it sits.
    pub depth: u8,
}

fn title(path: &Path) -> String {
    path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default()
}

fn folder_of(path: &Path) -> PathBuf {
    path.parent().map(Path::to_owned).unwrap_or_default()
}

/// One user's documents: the folder they are kept under, and a small file remembering what was used lately.
///
/// The file is read at each question and rewritten at each change, since every window of every running
/// copy of the app shares it.
#[derive(Clone, Debug)]
pub struct Library {
    root: PathBuf,
    memory: PathBuf,
}

impl Library {
    /// The library of the user whose home folder is `home`: documents under `Documents/Letters`, and the
    /// memory beside their default style in `.letters`.
    pub fn of(home: &Path) -> Self {
        Self::at(home.join("Documents").join("Letters"), home.join(".letters").join("recent.json"))
    }

    pub fn at(root: impl Into<PathBuf>, memory: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), memory: memory.into() }
    }

    /// The top of the folder tree documents are kept in. It need not exist before the first save.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn remembered(&self) -> (Option<PathBuf>, Vec<Recent>) {
        let json = fs::read_to_string(&self.memory).ok();
        let memory: Value = json.and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default();
        let recent = |entry: &Value| {
            Some(Recent { path: entry["path"].as_str()?.into(), opened: entry["opened"].as_str()?.to_owned() })
        };
        let recents = memory["recent"].as_array().map(|list| list.iter().filter_map(recent).collect());
        (memory["folder"].as_str().map(PathBuf::from), recents.unwrap_or_default())
    }

    /// Losing the memory costs a list of shortcuts, so a failure to write it stops nothing.
    fn remember(&self, folder: Option<&Path>, recents: &[Recent]) {
        let entry = |recent: &Recent| Some(json!({ "path": recent.path.to_str()?, "opened": recent.opened }));
        let memory = json!({
            "folder": folder.and_then(Path::to_str),
            "recent": recents.iter().filter_map(entry).collect::<Vec<_>>(),
        });
        let beside = self.memory.with_extension("json.new");
        let written = fs::create_dir_all(folder_of(&self.memory))
            .and_then(|()| fs::write(&beside, format!("{memory:#}\n")))
            .and_then(|()| fs::rename(&beside, &self.memory));
        if let Err(error) = written {
            eprintln!("letters: could not remember recent documents: {error}");
        }
    }

    /// The documents opened lately that are still where they were, newest first.
    pub fn recents(&self) -> Vec<Recent> {
        self.remembered().1.into_iter().filter(|recent| recent.path.is_file()).collect()
    }

    /// The folder a new document goes to: the one last used, or the root.
    pub fn default_folder(&self) -> PathBuf {
        self.remembered().0.filter(|folder| folder.is_dir()).unwrap_or_else(|| self.root.clone())
    }

    /// Notes that the document at `path` was opened, or first saved there, at `now`.
    pub fn opened(&self, path: &Path, now: &str) {
        self.moved(path, path, now);
    }

    /// Notes that the document at `from` is at `to` from `now` on.
    pub fn moved(&self, from: &Path, to: &Path, now: &str) {
        let Ok(to) = std::path::absolute(to) else { return };
        let from = std::path::absolute(from).unwrap_or_else(|_| from.to_owned());
        let (_, mut recents) = self.remembered();
        recents.retain(|recent| recent.path != from && recent.path != to);
        recents.insert(0, Recent { path: to.clone(), opened: now.to_owned() });
        recents.truncate(REMEMBERED);
        self.remember(Some(&folder_of(&to)), &recents);
    }

    /// A folder as a window names it: its place under the root as `Work / Clients`, or its whole path when
    /// it is somewhere else.
    pub fn shown(&self, folder: &Path) -> String {
        let name = |part: &std::ffi::OsStr| part.to_string_lossy().into_owned();
        match folder.strip_prefix(&self.root) {
            Ok(inside) if inside.as_os_str().is_empty() => self.root.file_name().map(name).unwrap_or_default(),
            Ok(inside) => inside.iter().map(name).collect::<Vec<_>>().join(" / "),
            Err(_) => folder.display().to_string(),
        }
    }

    /// The folders a document in `current` can be moved to, as a tree in reading order: the root and
    /// everything under it, after `current` itself when that is somewhere else.
    pub fn folders(&self, current: &Path) -> Vec<Folder> {
        fn name(path: &Path) -> String {
            path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned()
        }
        fn descend(folder: &Path, depth: u8, out: &mut Vec<Folder>) {
            out.push(Folder { path: folder.to_owned(), name: name(folder), depth });
            if depth == DEPTH {
                return;
            }
            let entries = fs::read_dir(folder).into_iter().flatten().flatten();
            // A link to a folder is not followed, so the tree cannot loop.
            let mut inside: Vec<PathBuf> = entries
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
                .map(|entry| entry.path())
                .collect();
            inside.sort_by_key(|path| name(path).to_lowercase());
            for folder in inside {
                if out.len() < FOLDERS {
                    descend(&folder, depth + 1, out);
                }
            }
        }
        let mut folders = Vec::new();
        if !current.as_os_str().is_empty() && !current.starts_with(&self.root) {
            folders.push(Folder { path: current.to_owned(), name: name(current), depth: 0 });
        }
        descend(&self.root, 0, &mut folders);
        folders
    }
}

/// A path in `folder` for a document called `name` that nothing occupies: `name.let`, or `name 2.let` and
/// so on when that is taken.
pub fn free_path(folder: &Path, name: &str) -> PathBuf {
    let path = |n: u32| {
        let name = if n == 1 { name.to_owned() } else { format!("{name} {n}") };
        folder.join(format!("{name}.{EXTENSION}"))
    };
    (1..).map(path).find(|path| path.symlink_metadata().is_err()).expect("some number is free")
}

/// Whether a document holds anything worth a file.
pub fn has_content(document: &Document) -> bool {
    document.walk().any(holds)
}

/// Whether a block is more than somewhere to type.
fn holds(block: &Block) -> bool {
    match block.kind.shape() {
        BodyShape::Text => !block.plain_text().trim().is_empty(),
        BodyShape::Blocks => false,
        BodyShape::None => true,
    }
}

/// Why a document was not renamed or moved. Shown to its writer as it reads.
#[derive(Debug)]
pub enum Refused {
    /// Nothing of what was typed can be a file name.
    NoName,
    /// Another document has that name in that folder, and it is never replaced.
    Taken(String),
    Io(io::Error),
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoName => f.write_str("A document needs a name."),
            Self::Taken(name) => write!(f, "There is already a document called \u{201c}{name}\u{201d} in that folder."),
            Self::Io(error) => write!(f, "The document could not be moved: {error}."),
        }
    }
}

impl std::error::Error for Refused {}

impl From<io::Error> for Refused {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Where one document is kept, from before it has a file.
#[derive(Clone, Debug)]
pub struct Place {
    home: Option<Home>,
    /// The name its writer typed before there was a file.
    typed: Option<String>,
    /// The folder its writer chose before there was a file.
    chosen: Option<PathBuf>,
    /// Whether the name is still the first Heading 1's to give. It gives one once.
    naming: bool,
}

impl Default for Place {
    fn default() -> Self {
        Self::new()
    }
}

impl Place {
    /// The place of a new document: nowhere yet.
    pub fn new() -> Self {
        Self { home: None, typed: None, chosen: None, naming: true }
    }

    /// The place of a document that was opened from its file.
    pub fn at(home: Home) -> Self {
        Self { home: Some(home), typed: None, chosen: None, naming: false }
    }

    pub fn home(&self) -> Option<&Home> {
        self.home.as_ref()
    }

    pub fn path(&self) -> Option<&Path> {
        self.home.as_ref().map(Home::path)
    }

    /// The document's name: its file's, or the one typed for the file to come.
    pub fn name(&self) -> String {
        match (&self.home, &self.typed) {
            (Some(home), _) => home.title(),
            (None, Some(typed)) => typed.clone(),
            (None, None) => UNTITLED.into(),
        }
    }

    /// The folder the document is in, or will first be saved to.
    pub fn folder(&self, library: &Library) -> PathBuf {
        match (&self.home, &self.chosen) {
            (Some(home), _) => folder_of(home.path()),
            (None, Some(chosen)) => chosen.clone(),
            (None, None) => library.default_folder(),
        }
    }

    /// Takes the home a save returned, which is a conflict copy when someone else changed the file.
    pub fn saved(&mut self, home: Home) {
        self.home = Some(home);
    }

    /// Whether the document still has no name of its own, so its first Heading 1 may give it one.
    pub fn naming(&self) -> bool {
        self.naming
    }

    /// Gets the place ready for a save of `document`, and returns where it goes. `None` while the document
    /// has no file and is not to have one yet.
    ///
    /// A new document takes its name once: what its writer typed in the bar, or its first Heading 1 when
    /// the `caret` is no longer in it, with a number after a name that is taken. A title still being typed
    /// is not written. Other text is, as Untitled, and that file is renamed when the name is taken.
    pub fn settle(
        &mut self,
        document: &Document,
        caret: Option<&BlockId>,
        library: &Library,
    ) -> io::Result<Option<&Home>> {
        let heading = document.walk().find(|block| block.kind == BlockKind::Heading { level: 1 });
        let titling = heading.filter(|heading| self.naming && caret == Some(&heading.id));
        let name = if titling.is_some() { None } else { suggested_name(document).filter(|_| self.naming) };
        match &self.home {
            None if !has_content(document) => return Ok(None),
            // Nothing but the title: its file comes when it has its name.
            None if titling.is_some_and(|title| !document.walk().any(|block| block.id != title.id && holds(block))) => {
                return Ok(None);
            }
            None => {
                let folder = self.folder(library);
                fs::create_dir_all(&folder)?;
                let name = self.typed.take().or(name);
                self.naming &= name.is_none();
                let name = name.unwrap_or_else(|| UNTITLED.into());
                self.home = Some(Home::new(free_path(&folder, &name)));
            }
            Some(home) => {
                if let Some(name) = name {
                    self.naming = false;
                    // A move that fails leaves the document as it is, Untitled and whole.
                    let path = free_path(&folder_of(home.path()), &name);
                    if let Some(home) =
                        Some(home).filter(|home| home.title() != name).and_then(|home| home.move_to(&path).ok())
                    {
                        self.home = Some(home);
                    }
                }
            }
        }
        Ok(self.home.as_ref())
    }

    /// Gives the document the name its writer typed. A name another document has there is refused.
    ///
    /// No save of this document may be running.
    pub fn rename(&mut self, typed: &str, library: &Library) -> Result<(), Refused> {
        let name = file_name(typed).ok_or(Refused::NoName)?;
        self.put(&self.folder(library), &name)?;
        if self.home.is_none() {
            self.typed = Some(name);
        }
        self.naming = false;
        Ok(())
    }

    /// Moves the document to `folder`, keeping its name. A folder holding a document of that name refuses.
    ///
    /// No save of this document may be running.
    pub fn move_to(&mut self, folder: &Path) -> Result<(), Refused> {
        if !folder.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "the folder is not there").into());
        }
        match (&self.home, self.typed.clone()) {
            (Some(home), _) => self.put(folder, &home.title())?,
            (None, Some(typed)) => self.put(folder, &typed)?,
            (None, None) => {}
        }
        if self.home.is_none() {
            self.chosen = Some(folder.to_owned());
        }
        Ok(())
    }

    /// Moves the file to `name` in `folder`, or checks that a file to come could go there.
    fn put(&mut self, folder: &Path, name: &str) -> Result<(), Refused> {
        let path = folder.join(format!("{name}.{EXTENSION}"));
        let taken = |error: io::Error| match error.kind() {
            io::ErrorKind::AlreadyExists => Refused::Taken(name.to_owned()),
            _ => Refused::Io(error),
        };
        match &self.home {
            Some(home) if home.path() == path => {}
            Some(home) => self.home = Some(home.move_to(&path).map_err(taken)?),
            None if path.symlink_metadata().is_ok() => return Err(Refused::Taken(name.to_owned())),
            None => {}
        }
        Ok(())
    }
}
