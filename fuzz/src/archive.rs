//! Zip files as a script spells them out: a list of named entries. A zip guards every entry with a
//! checksum, so a fuzzer that flipped the bytes of a file would only ever find the checksum. Spelled out as
//! entries, an input is always a zip that opens, and what it holds is the fuzzer's to change.

use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::input::{Script, put};

const ENTRIES: usize = 24;
const NAME: usize = 255;
const CONTENT: usize = 48 * 1024;

pub type Entries = Vec<(String, Vec<u8>)>;

/// The entries the rest of a script spells out.
pub fn entries(script: &mut Script) -> Entries {
    let mut entries = Entries::new();
    while !script.done() && entries.len() < ENTRIES {
        entries.push((script.text(NAME), script.bytes(CONTENT).to_vec()));
    }
    entries
}

/// A zip of `entries`, without those whose names a zip cannot hold.
pub fn zipped(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in entries {
        if zip.start_file(name.as_str(), stored).is_ok() {
            zip.write_all(content).expect("memory takes any write");
        }
    }
    zip.finish().expect("memory takes any write").into_inner()
}

/// The entries of a real file, for a seed.
pub fn unzipped(file: &[u8]) -> Entries {
    let mut zip = ZipArchive::new(Cursor::new(file)).expect("a seed is a zip");
    let mut entries = Entries::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).expect("a seed's entries read");
        let mut content = Vec::new();
        entry.read_to_end(&mut content).expect("a seed's entries read");
        let name = entry.name().expect("a seed's entries have names").into_owned();
        entries.push((name, content));
    }
    entries
}

/// Entries as [`entries`] reads them.
pub fn spelled(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, content) in entries {
        put(&mut out, NAME, name.as_bytes());
        put(&mut out, CONTENT, content);
    }
    out
}
