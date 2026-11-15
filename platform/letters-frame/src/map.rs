//! A font file read as the system pages it in, which no safe call does.

use std::fs::File;
use std::io;
use std::path::Path;

use memmap2::Mmap;

/// A file's bytes, in memory only as far as they are read. The mapping lasts as long as this does.
pub struct Mapped(Mmap);

impl AsRef<[u8]> for Mapped {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Maps one of the system's font files for reading, so that a face costs the memory of the glyphs that
/// are drawn and not of its whole file: an emoji face runs to hundreds of megabytes on some systems.
///
/// For font files only. A mapping shows the file as it is on disk, so its bytes hold still only while
/// nothing writes into the file. The system's fonts are installed and updated by putting a new file in
/// the old one's place, which leaves a mapping of the old one whole. A file cut short in place under a
/// mapping would end the app when the missing part is read.
pub fn map_font(path: &Path) -> io::Result<Mapped> {
    let file = File::open(path)?;
    // SAFETY: the mapping is private and read-only, and it outlives the file handle by design of mmap.
    // That its bytes do not change rests on how font files are replaced, as said above. The font readers
    // check every offset against the length they are given, which a mapping keeps.
    unsafe { Mmap::map(&file) }.map(Mapped)
}
