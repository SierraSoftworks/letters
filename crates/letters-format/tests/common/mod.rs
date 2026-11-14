//! Fixtures shared by the container, corpus and save tests.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{self, Cursor, Read, Write};

use letters_format::{Archive, Files, Package, Stamp, write};
use letters_model::{Block, BlockKind, Document};

pub const ID: &str = "8f14e45f-ceea-467f-a0e6-5b1d3c2a9f00";

pub fn paragraph(id: &str, text: &str) -> Block {
    Block::text(id, BlockKind::Paragraph, text)
}

/// A small document with an asset and some history.
pub fn package() -> Package {
    let document = Document { blocks: vec![paragraph("p", "Hello")], ..Document::default() };
    let entries = [("assets/0a1b.png", vec![0x89, b'P', b'N', b'G']), ("history/oplog", vec![1, 2, 3])];
    Package {
        id: ID.into(),
        created: "2026-11-14T20:00:00Z".into(),
        document,
        entries: entries.into_iter().map(|(name, bytes)| (name.to_string(), bytes)).collect(),
    }
}

pub fn bytes(package: &Package) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    write(&mut out, package).unwrap();
    out.into_inner()
}

pub fn open(file: &[u8]) -> Archive<Cursor<Vec<u8>>> {
    Archive::open(Cursor::new(file.to_vec())).unwrap()
}

/// Rebuilds a file uncompressed, passing each entry through `edit`, which may change or drop it.
pub fn rezip(file: &[u8], edit: impl Fn(&str, Vec<u8>) -> Option<Vec<u8>>) -> Vec<u8> {
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut zip = zip::ZipArchive::new(Cursor::new(file)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let name = entry.name().unwrap().to_string();
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        if let Some(content) = edit(&name, content) {
            out.start_file(name, stored).unwrap();
            out.write_all(&content).unwrap();
        }
    }
    out.finish().unwrap().into_inner()
}

type Event = Box<dyn FnOnce(&mut Folder)>;

/// A folder in memory, with the ways a shared folder misbehaves.
#[derive(Default)]
pub struct Folder {
    pub files: BTreeMap<String, Vec<u8>>,
    /// When each file last changed, on a clock that ticks once per change.
    modified: BTreeMap<String, u64>,
    clock: u64,
    /// Renames onto this name are refused, as a file held open on SMB refuses them.
    pub locked: Option<String>,
    /// Refuses every write and rename.
    pub read_only: bool,
    /// Runs once, just after the next write: another writer slipping in mid-save.
    pub after_write: Option<Event>,
    /// Runs once, at the next wait between attempts: a sync tool finishing its work.
    pub at_wait: Option<Event>,
    pub waits: u32,
}

impl Folder {
    /// Puts a file in place as another writer would.
    pub fn put(&mut self, name: &str, bytes: Vec<u8>) {
        self.clock += 1;
        self.modified.insert(name.into(), self.clock);
        self.files.insert(name.into(), bytes);
    }

    pub fn names(&self) -> Vec<&str> {
        self.files.keys().map(String::as_str).collect()
    }

    /// How many changes the folder has seen.
    pub fn changes(&self) -> u64 {
        self.clock
    }

    fn refuse<T>(&self, what: &str) -> io::Result<T> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, what.to_string()))
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
        if self.read_only {
            return self.refuse("the folder is read-only");
        }
        self.put(name, bytes.to_vec());
        if let Some(event) = self.after_write.take() {
            event(self);
        }
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> io::Result<()> {
        if self.read_only || self.locked.as_deref() == Some(to) {
            return self.refuse("the file is in use");
        }
        let bytes = self.files.remove(from).ok_or(io::ErrorKind::NotFound)?;
        self.modified.remove(from);
        self.put(to, bytes);
        Ok(())
    }

    fn remove(&mut self, name: &str) -> io::Result<()> {
        self.files.remove(name).map(drop).ok_or(io::ErrorKind::NotFound.into())
    }

    fn wait(&mut self, _retry: u32) {
        self.waits += 1;
        if let Some(event) = self.at_wait.take() {
            event(self);
        }
    }
}
