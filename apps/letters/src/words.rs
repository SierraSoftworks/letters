//! The person's own dictionary: one word a line in `.letters/dictionary.txt`, right in every document.
//!
//! Every window of every running copy of the app shares the file, so a window reads it again when it has
//! changed, and a word is added by appending a line.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What told one reading of the file from the next: when it was written, and how long it is.
pub type Stamp = Option<(SystemTime, u64)>;

#[derive(Clone, Debug)]
pub struct Words {
    path: PathBuf,
}

impl Words {
    /// The dictionary of the user whose home folder is `home`, beside their default style.
    pub fn of(home: &Path) -> Self {
        Self::at(home.join(".letters").join("dictionary.txt"))
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The words, when the file is not as it was at `seen`. No file is no words, which is no news at first.
    pub fn changed(&self, seen: &mut Stamp) -> Option<Vec<String>> {
        let stamp = fs::metadata(&self.path).ok().and_then(|file| Some((file.modified().ok()?, file.len())));
        if stamp == *seen {
            return None;
        }
        *seen = stamp;
        let text = fs::read_to_string(&self.path).unwrap_or_default();
        Some(text.lines().map(str::trim).filter(|word| !word.is_empty()).map(str::to_owned).collect())
    }

    /// Adds a word, unless it is there already.
    pub fn learn(&self, word: &str) -> io::Result<()> {
        let word = word.trim();
        if word.is_empty() || word.contains(['\n', '\r']) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a word"));
        }
        let known = fs::read_to_string(&self.path).unwrap_or_default();
        if known.lines().any(|line| line.trim() == word) {
            return Ok(());
        }
        if let Some(folder) = self.path.parent() {
            fs::create_dir_all(folder)?;
        }
        // A file someone edited by hand may not end its last line.
        let gap = if known.is_empty() || known.ends_with('\n') { "" } else { "\n" };
        let mut file = fs::OpenOptions::new().append(true).create(true).open(&self.path)?;
        file.write_all(format!("{gap}{word}\n").as_bytes())
    }
}
