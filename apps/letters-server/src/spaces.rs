//! Spaces: the named folders documents live in, and the only way to their files.
//!
//! Every file is reached through a [`Root`], which opens beneath its folder and nowhere else: a `..`, an
//! absolute path or a symlink that leads out is refused by the system call itself, not by a check that
//! could be raced. A [`Place`] is held to plain visible names on top of that, so that a refusal has words
//! and the server's own hidden files stay out of reach.

use std::io::{self, Read, Write};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use letters_api::{Role, Space, SpaceKind};
use letters_format::{Files, Stamp};

use crate::config::{self, USERNAME};
use crate::error::{Answer, Refusal};

/// What a document's file name ends in.
pub const EXTENSION: &str = ".let";
/// Where a space keeps what was thrown away. Hidden, as every name with a leading dot is.
pub const TRASH: &str = ".letters-trash";
/// Where a person's own space keeps what they set for themselves, as the desktop app does in a home folder.
pub const KEPT: &str = ".letters";

/// Whether `name` can name a file or a folder in a space: one visible name that every file system takes.
/// A leading dot is how sync tools, the trash and half-written files hide, so none is reachable by name.
pub fn name_ok(name: &str) -> bool {
    let plain = !name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\'));
    !name.is_empty() && name.len() <= 255 && !name.starts_with('.') && plain
}

/// Where something is in a space: the names of its folders and its own, joined by `/`. Empty for the
/// space itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Place(String);

impl Place {
    pub fn parse(text: &str) -> Answer<Self> {
        let text = text.trim_matches('/');
        if !text.is_empty() && !text.split('/').all(name_ok) {
            return Err(Refusal::invalid("That is not a place in a space."));
        }
        Ok(Self(text.to_owned()))
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// What `name` in this folder is called.
    pub fn child(&self, name: &str) -> Answer<Self> {
        if !name_ok(name) {
            return Err(Refusal::invalid("That name cannot be used for a file."));
        }
        Ok(Self(if self.is_root() { name.to_owned() } else { format!("{}/{name}", self.0) }))
    }

    /// The folder this is in, and its own name. The space itself is in itself and has none.
    pub fn split(&self) -> (Self, &str) {
        match self.0.rsplit_once('/') {
            Some((folder, name)) => (Self(folder.to_owned()), name),
            None => (Self::default(), &self.0),
        }
    }

    pub fn name(&self) -> &str {
        self.split().1
    }
}

/// The name a document is shown by: its file's, without the ending.
pub fn shown(file: &str) -> &str {
    file.strip_suffix(EXTENSION).unwrap_or(file)
}

impl config::Space {
    /// What `groups` may do in the space. `None` when they do not reach it.
    pub fn role(&self, groups: &[String]) -> Option<Role> {
        if self.personal() {
            return Some(Role::Owner);
        }
        let holds = |listed: &[String]| listed.iter().any(|group| groups.contains(group));
        let roles = [(Role::Editor, &self.editors), (Role::Commenter, &self.commenters), (Role::Reader, &self.readers)];
        roles.into_iter().find(|(_, listed)| holds(listed)).map(|(role, _)| role)
    }

    /// The space's folder for the person who signs in as `username`.
    pub fn folder(&self, username: &str) -> String {
        self.path.replace(USERNAME, username)
    }

    pub fn told(&self, role: Role) -> Space {
        let kind = if self.personal() { SpaceKind::Personal } else { SpaceKind::Shared };
        Space { id: self.id(), name: self.name.clone(), kind, role }
    }
}

/// Whether a name a person signs in with can stand in a personal space's path as one folder's name.
pub fn username_ok(username: &str) -> bool {
    let plain = username.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '@' | '+'));
    name_ok(username) && plain
}

/// A space's folder, open. Nothing above it can be reached through it.
pub struct Root {
    /// The folder's path as the settings give it. The document index names the root by it.
    pub key: String,
    dir: Dir,
}

impl Root {
    /// Opens the folder at `path`, which the server's owner chose. A person's own is made first if `make`
    /// says so.
    pub fn open(path: &str, make: bool) -> io::Result<Self> {
        if make {
            std::fs::create_dir_all(path)?;
        }
        Ok(Self { key: path.to_owned(), dir: Dir::open_ambient_dir(path, ambient_authority())? })
    }

    /// The same folder, open once more: for whoever keeps it longer than the request that opened it.
    pub fn again(&self) -> io::Result<Self> {
        Ok(Self { key: self.key.clone(), dir: self.dir.try_clone()? })
    }

    /// The folder at `place`.
    pub fn folder(&self, place: &Place) -> io::Result<Folder> {
        let dir = if place.is_root() { self.dir.try_clone() } else { self.dir.open_dir(place.as_str()) };
        Ok(Folder { dir: dir? })
    }

    /// The trash, made if it is not there.
    pub fn trash(&self) -> io::Result<Folder> {
        match self.dir.create_dir(TRASH) {
            Err(error) if error.kind() != io::ErrorKind::AlreadyExists => Err(error),
            _ => Ok(Folder { dir: self.dir.open_dir(TRASH)? }),
        }
    }

    /// The trash, when something was ever thrown away here. Looking makes nothing.
    pub fn thrown(&self) -> io::Result<Option<Folder>> {
        Ok(found(self.dir.open_dir(TRASH))?.map(|dir| Folder { dir }))
    }

    /// The folder of what the space's person set for themselves. It is made first if `make` says so, and
    /// is `None` when it is not there.
    pub fn kept(&self, make: bool) -> io::Result<Option<Folder>> {
        if make
            && let Err(error) = self.dir.create_dir(KEPT)
            && error.kind() != io::ErrorKind::AlreadyExists
        {
            return Err(error);
        }
        Ok(found(self.dir.open_dir(KEPT))?.map(|dir| Folder { dir }))
    }
}

/// What a folder holds under one name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub folder: bool,
    pub stamp: Stamp,
}

/// One folder of a space.
pub struct Folder {
    dir: Dir,
}

impl Folder {
    /// The folders and the documents here, by name. Hidden names are left out, and so are symlinks, which
    /// could show one folder inside itself, and whatever is not a document.
    pub fn list(&self) -> io::Result<Vec<Listed>> {
        let mut out = Vec::new();
        for entry in self.dir.entries()? {
            let entry = entry?;
            let Ok(name) = entry.file_name().into_string() else { continue };
            let kind = entry.file_type()?;
            let wanted = kind.is_dir() || (kind.is_file() && name.ends_with(EXTENSION) && name.len() > EXTENSION.len());
            if !name_ok(&name) || !wanted {
                continue;
            }
            // Gone between the listing and the look: not here, then.
            let Ok(metadata) = entry.metadata() else { continue };
            out.push(Listed { name, folder: kind.is_dir(), stamp: stamp(&metadata) });
        }
        // By the name shown, so that "Plan" comes before "Plan (conflicted copy)".
        out.sort_by_cached_key(|listed| (!listed.folder, shown(&listed.name).to_lowercase()));
        Ok(out)
    }

    pub fn has(&self, name: &str) -> bool {
        self.dir.symlink_metadata(name).is_ok()
    }

    pub fn make_folder(&self, name: &str) -> io::Result<()> {
        self.dir.create_dir(name)
    }

    /// The start of the file `name`, `most` bytes of it at most: for a small file that a person may have
    /// written by hand, at any length.
    pub fn start(&self, name: &str, most: u64) -> io::Result<Option<Vec<u8>>> {
        let Some(file) = found(self.dir.open(name))? else { return Ok(None) };
        let mut bytes = Vec::new();
        file.take(most).read_to_end(&mut bytes)?;
        Ok(Some(bytes))
    }

    /// The file to read from, with nothing read yet: a manifest is found without the pictures beside it.
    pub fn open(&self, name: &str) -> io::Result<std::fs::File> {
        Ok(self.dir.open(name)?.into_std())
    }

    /// Moves `name` to `to` as `new`, which must be free there: nothing is ever replaced.
    pub fn move_to(&self, name: &str, to: &Folder, new: &str) -> io::Result<()> {
        // A link fails where a file exists, and appears whole or not at all. Folders cannot be linked, and
        // some file systems link nothing: those are renamed once the name is seen to be free.
        match self.dir.hard_link(name, &to.dir, new) {
            Ok(()) => return self.dir.remove_file(name),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Err(error),
            Err(_) => {}
        }
        if to.has(new) {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        match self.dir.rename(name, &to.dir, new) {
            // A folder is not copied: half of it on each disk would be worse than where it is.
            Err(error) if error.kind() == io::ErrorKind::CrossesDevices && !self.dir.metadata(name)?.is_dir() => {
                self.copy_to(name, to, new)
            }
            done => done,
        }
    }

    /// Moves a file to another disk: copied whole, then removed here.
    fn copy_to(&self, name: &str, to: &Folder, new: &str) -> io::Result<()> {
        let mut options = OpenOptions::new();
        let mut copy = to.dir.open_with(new, options.write(true).create_new(true))?;
        let copied = self
            .dir
            .open(name)
            .and_then(|mut file| {
                io::copy(&mut file, &mut copy)?;
                copy.sync_all()
            })
            .and_then(|()| self.dir.remove_file(name));
        if copied.is_err() {
            // The original is still whole, so the half-made copy must not pass for the document.
            let _ = to.dir.remove_file(new);
        }
        copied
    }
}

fn stamp(metadata: &cap_std::fs::Metadata) -> Stamp {
    let modified = metadata.modified().ok().map(|time| time.into_std());
    let since = modified.and_then(|time| time.duration_since(UNIX_EPOCH).ok());
    Stamp { len: metadata.len(), modified: since.map_or(0, |since| since.as_nanos() as u64) }
}

/// When a stamp says its file was last written, in seconds since 1970.
pub fn seconds(stamp: Stamp) -> u64 {
    stamp.modified / 1_000_000_000
}

/// `None` for a file that is not there.
fn found<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

impl Files for Folder {
    fn stamp(&mut self, name: &str) -> io::Result<Option<Stamp>> {
        Ok(found(self.dir.metadata(name))?.map(|metadata| stamp(&metadata)))
    }

    fn read(&mut self, name: &str) -> io::Result<Option<Vec<u8>>> {
        let Some(mut file) = found(self.dir.open(name))? else { return Ok(None) };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(Some(bytes))
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let mut file = self.dir.create(name)?;
        file.write_all(bytes)?;
        file.sync_all()
    }

    fn append(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let mut options = OpenOptions::new();
        let mut file = self.dir.open_with(name, options.append(true).create(true))?;
        file.write_all(bytes)?;
        file.sync_data()
    }

    fn rename(&mut self, from: &str, to: &str) -> io::Result<()> {
        // The new file takes over the permissions of the one it replaces.
        if let Ok(replaced) = self.dir.metadata(to) {
            self.dir.set_permissions(from, replaced.permissions())?;
        }
        self.dir.rename(from, &self.dir, to)
    }

    fn remove(&mut self, name: &str) -> io::Result<()> {
        self.dir.remove_file(name)
    }

    fn wait(&mut self, retry: u32) {
        thread::sleep(Duration::from_millis(50) * retry);
    }
}
