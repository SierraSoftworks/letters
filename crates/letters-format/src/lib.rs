//! The `.let` container: a zip holding one document, its style, notes, assets and history.
//!
//! - [`Archive`] opens a file and reads its parts on demand, so first paint touches only what it needs.
//! - [`Package`] is everything that goes into a file, and [`write`] turns it into bytes.
//!
//! Nothing here touches the filesystem or the clock. The shell supplies readers, writers, ids and dates.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{Read, Seek, Write};

use letters_model::{Document, JsonError};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// The media type of a `.let` file, stored in its first entry.
pub const MIMETYPE: &str = "application/vnd.letters.document+zip";
/// The container version this build reads and writes.
pub const FORMAT: u32 = 1;

pub const MANIFEST: &str = "manifest.json";
pub const DOCUMENT: &str = "document.json";
pub const STYLE: &str = "style.json";
pub const NOTES: &str = "notes.json";
/// Images and attachments, named by content hash.
pub const ASSETS: &str = "assets/";
/// Private to Letters. A file without it is still a complete document.
pub const HISTORY: &str = "history/";

#[derive(Debug)]
pub enum Error {
    /// Not a zip, or a damaged one.
    Zip(String),
    /// A zip, but not a `.let` file.
    NotLetters,
    /// The file needs a newer build.
    NewerFormat(u32),
    Missing(String),
    /// An entry's content does not match the hash its manifest records.
    HashMismatch(String),
    Manifest(String),
    Document(JsonError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zip(message) => write!(f, "the file is damaged or is not a zip: {message}"),
            Self::NotLetters => f.write_str("the file is not a Letters document"),
            Self::NewerFormat(v) => write!(f, "the file uses format {v}, which is newer than this build reads"),
            Self::Missing(name) => write!(f, "the file has no {name}"),
            Self::HashMismatch(name) => write!(f, "{name} does not match its recorded hash"),
            Self::Manifest(message) => write!(f, "{MANIFEST}: {message}"),
            Self::Document(error) => write!(f, "{DOCUMENT}: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<zip::result::ZipError> for Error {
    fn from(error: zip::result::ZipError) -> Self {
        Self::Zip(error.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Zip(error.to_string())
    }
}

/// The JSON Schemas of the public entries, as published with this format version.
pub mod schema {
    pub const MANIFEST: &str = include_str!("../schema/manifest.schema.json");
    pub const DOCUMENT: &str = include_str!("../schema/document.schema.json");
    pub const STYLE: &str = include_str!("../schema/style.schema.json");
    pub const NOTES: &str = include_str!("../schema/notes.schema.json");
}

/// `manifest.json`: what the file is, and the hash of each entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    /// The document's identity, shared by every copy of it.
    pub id: String,
    /// When the document was created, as RFC 3339.
    pub created: String,
    /// Entry name to `sha256:<hex>`, for every entry but the mimetype and the manifest.
    pub hashes: BTreeMap<String, String>,
}

impl Manifest {
    fn from_json(bytes: &[u8]) -> Result<Self, Error> {
        let bad = |message: &str| Error::Manifest(message.into());
        let value: Value = serde_json::from_slice(bytes).map_err(|e| Error::Manifest(e.to_string()))?;
        match value["format"].as_u64() {
            Some(format) if format == u64::from(FORMAT) => {}
            Some(format) if format > u64::from(FORMAT) => return Err(Error::NewerFormat(format as u32)),
            _ => return Err(bad("expected a format version")),
        }
        let text = |key: &str| value[key].as_str().map(str::to_owned).ok_or_else(|| bad(&format!("expected {key}")));
        let hashes = value["hashes"].as_object().ok_or_else(|| bad("expected hashes"))?;
        let hashes = hashes.iter().filter_map(|(name, hash)| Some((name.clone(), hash.as_str()?.to_owned()))).collect();
        Ok(Self { id: text("id")?, created: text("created")?, hashes })
    }

    fn to_json(&self) -> String {
        let mut map = Map::new();
        map.insert("format".into(), FORMAT.into());
        map.insert("id".into(), self.id.as_str().into());
        map.insert("created".into(), self.created.as_str().into());
        map.insert("hashes".into(), self.hashes.iter().map(|(k, v)| (k.clone(), Value::from(v.as_str()))).collect());
        let mut out = serde_json::to_string_pretty(&Value::Object(map)).expect("a JSON value always serialises");
        out.push('\n');
        out
    }
}

/// The hash a manifest records for `bytes`.
pub fn hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("sha256:{hex}")
}

/// An open `.let` file. Entries are read, and checked against the manifest, only when asked for.
pub struct Archive<R> {
    zip: ZipArchive<R>,
    manifest: Manifest,
}

impl<R: Read + Seek> Archive<R> {
    pub fn open(reader: R) -> Result<Self, Error> {
        let mut zip = ZipArchive::new(reader)?;
        if raw(&mut zip, "mimetype")?.as_deref() != Some(MIMETYPE.as_bytes()) {
            return Err(Error::NotLetters);
        }
        let manifest = raw(&mut zip, MANIFEST)?.ok_or_else(|| Error::Missing(MANIFEST.into()))?;
        Ok(Self { manifest: Manifest::from_json(&manifest)?, zip })
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The names of every entry the manifest lists.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.manifest.hashes.keys().map(String::as_str)
    }

    /// Reads one entry. `None` when the file does not have it.
    pub fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, Error> {
        let Some(expected) = self.manifest.hashes.get(name) else { return Ok(None) };
        let bytes = raw(&mut self.zip, name)?.ok_or_else(|| Error::Missing(name.into()))?;
        if &hash(&bytes) != expected {
            return Err(Error::HashMismatch(name.into()));
        }
        Ok(Some(bytes))
    }

    pub fn document(&mut self) -> Result<Document, Error> {
        let bytes = self.read(DOCUMENT)?.ok_or_else(|| Error::Missing(DOCUMENT.into()))?;
        let json = String::from_utf8(bytes).map_err(|_| Error::Zip(format!("{DOCUMENT} is not UTF-8")))?;
        Document::from_json(&json).map_err(Error::Document)
    }

    /// Reads the whole file.
    pub fn package(&mut self) -> Result<Package, Error> {
        let document = self.document()?;
        let names: Vec<String> = self.names().filter(|name| *name != DOCUMENT).map(str::to_owned).collect();
        let mut entries = BTreeMap::new();
        for name in names {
            let bytes = self.read(&name)?.expect("the manifest lists it");
            entries.insert(name, bytes);
        }
        Ok(Package { id: self.manifest.id.clone(), created: self.manifest.created.clone(), document, entries })
    }
}

fn raw<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<Option<Vec<u8>>, Error> {
    let mut file = match zip.by_name(name) {
        Ok(file) => file,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// Everything that goes into a `.let` file.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub id: String,
    /// RFC 3339.
    pub created: String,
    pub document: Document,
    /// Every other entry by name: [`STYLE`], [`NOTES`], and those under [`ASSETS`] and [`HISTORY`].
    pub entries: BTreeMap<String, Vec<u8>>,
}

/// Writes a `.let` file. The same package always produces the same bytes.
pub fn write<W: Write + Seek>(writer: W, package: &Package) -> Result<(), Error> {
    let document = package.document.to_json().into_bytes();
    let mut entries: BTreeMap<&str, &[u8]> = package.entries.iter().map(|(k, v)| (k.as_str(), v.as_slice())).collect();
    entries.insert(DOCUMENT, &document);
    let hashes = entries.iter().map(|(name, bytes)| (name.to_string(), hash(bytes))).collect();
    let manifest = Manifest { id: package.id.clone(), created: package.created.clone(), hashes }.to_json();

    // A fixed timestamp keeps the output reproducible.
    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    let deflated = stored.compression_method(CompressionMethod::Deflated);

    let mut zip = ZipWriter::new(writer);
    // The mimetype comes first and uncompressed, so the file can be recognised from its first bytes.
    zip.start_file("mimetype", stored)?;
    zip.write_all(MIMETYPE.as_bytes())?;
    zip.start_file(MANIFEST, deflated)?;
    zip.write_all(manifest.as_bytes())?;
    for (name, bytes) in entries {
        zip.start_file(name, deflated)?;
        zip.write_all(bytes)?;
    }
    zip.finish()?;
    Ok(())
}
