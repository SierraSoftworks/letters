//! A folder in memory, with the ways a shared folder misbehaves.

use std::collections::BTreeMap;
use std::io;

use letters_format::{Files, Stamp};

#[derive(Default)]
pub struct Folder {
    pub files: BTreeMap<String, Vec<u8>>,
    /// When each file last changed, on a clock that ticks once per change.
    modified: BTreeMap<String, u64>,
    clock: u64,
    /// Renames of this file and onto it are refused, as a file held open on SMB refuses both.
    pub locked: Option<String>,
    /// Put in place of this file just after the next write: another writer slipping in mid-save.
    pub racing: Option<(String, Vec<u8>)>,
}

impl Folder {
    /// Puts a file in place as another writer would.
    pub fn put(&mut self, name: &str, bytes: Vec<u8>) {
        self.clock += 1;
        self.modified.insert(name.into(), self.clock);
        self.files.insert(name.into(), bytes);
    }
}

impl Files for Folder {
    fn stamp(&mut self, name: &str) -> io::Result<Option<Stamp>> {
        Ok(self.files.get(name).map(|bytes| Stamp { len: bytes.len() as u64, modified: self.modified[name] }))
    }

    fn read(&mut self, name: &str) -> io::Result<Option<Vec<u8>>> {
        Ok(self.files.get(name).cloned())
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        self.put(name, bytes.to_vec());
        if let Some((name, bytes)) = self.racing.take() {
            self.put(&name, bytes);
        }
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> io::Result<()> {
        if self.locked.as_deref().is_some_and(|locked| locked == to || locked == from) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the file is in use"));
        }
        let bytes = self.files.remove(from).ok_or(io::ErrorKind::NotFound)?;
        self.modified.remove(from);
        self.put(to, bytes);
        Ok(())
    }

    fn remove(&mut self, name: &str) -> io::Result<()> {
        self.modified.remove(name);
        self.files.remove(name).map(drop).ok_or(io::ErrorKind::NotFound.into())
    }
}
