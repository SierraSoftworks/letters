//! What the person trusts, kept in the `[trust]` table of `~/.letters/settings.toml`.
//!
//! The rules are `letters_trust`'s. The shell lends it the file, and says where each document came from.

use std::path::{Path, PathBuf};
use std::{fs, io};

use letters_trust::{Gate, Mode, Origin, Store, Trust, TrustStore};

/// The settings file. Whatever else it holds is left as its writer had it.
#[derive(Clone, Debug)]
pub struct Settings {
    path: PathBuf,
}

impl Settings {
    /// The settings of the user whose home folder is `home`, beside their recent documents.
    pub fn of(home: &Path) -> Self {
        Self::at(home.join(".letters").join("settings.toml"))
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl Store for Settings {
    fn read(&mut self) -> io::Result<Option<String>> {
        match fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn write(&mut self, text: &str) -> io::Result<()> {
        // Written beside the file and moved over it, so that a crash leaves the old settings or the new.
        let beside = self.path.with_extension("toml.new");
        if let Some(folder) = self.path.parent() {
            fs::create_dir_all(folder)?;
        }
        fs::write(&beside, text)?;
        fs::rename(&beside, &self.path)
    }
}

pub type Trusted = TrustStore<Settings>;

/// What the user whose home folder is `home` trusts: nothing, until it is loaded.
pub fn store(home: &Path) -> Trusted {
    // Documents are asked about by their resolved paths, so `~` stands for the resolved home.
    let real = fs::canonicalize(home).unwrap_or_else(|_| home.to_owned());
    TrustStore::new(Settings::of(home), real.to_str())
}

/// How a document is treated. One made here, which has no `path` yet, is trusted. One read from a file is
/// trusted by its id, or by a trusted folder it lies in.
pub fn mode(trust: &Trust, id: &str, path: Option<&Path>) -> Mode {
    let Some(path) = path else { return trust.mode(id, Origin::Created) };
    // Folders are matched by name alone, so links are followed first.
    let real = fs::canonicalize(path).ok();
    match real.as_deref().and_then(Path::to_str) {
        Some(path) => trust.mode(id, Origin::File(path)),
        // No folder can vouch for a path that cannot be resolved or written as text.
        None => trust.mode(id, Origin::Outside),
    }
}

/// What a document may fetch, link to and merge, by the same rules as [`mode`].
pub fn gate<'a>(trust: &'a Trust, id: &str, path: Option<&Path>) -> Gate<'a> {
    let Some(path) = path else { return trust.gate(id, Origin::Created) };
    let real = fs::canonicalize(path).ok();
    match real.as_deref().and_then(Path::to_str) {
        Some(path) => trust.gate(id, Origin::File(path)),
        None => trust.gate(id, Origin::Outside),
    }
}
