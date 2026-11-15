//! What the person trusts, kept in the `[trust]` table of `~/.letters/settings.toml`.
//!
//! The rules are `letters_trust`'s. The shell lends it the file, and says where each document came from.

use std::path::{Path, PathBuf};
use std::{fs, io};

use letters_trust::{Gate, Mode, Origin, Source, Store, Trust, TrustError, TrustStore, UrlError};
use letters_ui::{TrustedFolder, TrustedSource};

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

/// Records the import of the file at `source` as the document `id`, before the document is written.
///
/// What an import holds came from the file, so the folder the file lies in vouches for it, not the folder
/// the document is saved to. From a trusted folder the document is trusted wherever it is kept. From
/// anywhere else it opens in safe mode until the person trusts the document itself.
pub fn import(trust: &mut Trusted, id: &str, source: &Path) -> Result<Mode, TrustError> {
    let real = fs::canonicalize(source).ok();
    match real.as_deref().and_then(Path::to_str).is_some_and(|path| trust.trust().trusts_folder(path)) {
        true => trust.trust_document(id).map(|()| Mode::Trusted),
        false => trust.import_document(id).map(|()| Mode::Safe),
    }
}

/// The folders the person trusts, for a window's Settings to list: each as the settings file writes it,
/// which is under `~` where it can be, and is what [`TrustStore::forget_folder`] takes back.
pub fn folders(trust: &Trust) -> Vec<TrustedFolder> {
    trust.folders().iter().map(|folder| TrustedFolder { path: folder.clone(), shown: folder.clone() }).collect()
}

/// The sources linked pictures load from, for a window's Settings to list. All are the person's own: only
/// a server has an administrator's.
pub fn sources(trust: &Trust) -> Vec<TrustedSource> {
    trust.sources().iter().map(|source| TrustedSource { source: source.to_string(), fixed: false }).collect()
}

/// The source a person typed in Settings, or why it is none, as they are to read it.
pub fn source(typed: &str) -> Result<Source, String> {
    Source::parse(typed).map_err(|error| {
        match error {
            UrlError::NotHttps => {
                "A source must start with https://, so that nobody on the way sees what is asked for."
            }
            UrlError::Credentials => "A source cannot hold a user name or a password.",
            UrlError::Private | UrlError::Malformed => {
                "A source is a host such as img.example.org, or an address that starts with https://."
            }
        }
        .to_owned()
    })
}

/// Why a change to what the person trusts was not kept, to end a sentence they read, or nothing where
/// there is no plain reason to give. The store's own words are for the log.
pub fn unkept(error: &TrustError) -> &'static str {
    eprintln!("letters: what is trusted could not be changed: {error}");
    match error {
        TrustError::Store(_) => " because ~/.letters/settings.toml could not be read or written",
        TrustError::Toml(_) | TrustError::Shape(_) => {
            " because ~/.letters/settings.toml holds something Letters cannot read"
        }
        TrustError::Folder(_) => " because a whole disk cannot be trusted, only a folder on it",
        TrustError::NoDocument => "",
    }
}
