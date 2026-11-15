//! The `[trust]` table of the user's settings file.
//!
//! ```toml
//! [trust]
//! documents = ["0a6f1c2e"]
//! folders = ["~/Documents", "~/Notes"]
//! image_sources = ["https://upload.wikimedia.org/", "https://img.example.org/charts/"]
//! imported = ["5c1d9e07"]
//! ```
//!
//! `imported` lists the documents the user made from other files. Their content came from elsewhere, so
//! no folder vouches for one: it is trusted when the user trusts the document itself.

use std::collections::BTreeSet;

use toml_edit::{Array, DocumentMut, Item, Table, Value};

use crate::path::Path;
use crate::{HttpsUrl, Source, TrustError};

pub(crate) const DOCUMENTS: &str = "documents";
pub(crate) const FOLDERS: &str = "folders";
pub(crate) const SOURCES: &str = "image_sources";
pub(crate) const IMPORTED: &str = "imported";

/// What one user trusts. The default trusts nothing.
#[derive(Clone, Debug, Default)]
pub struct Trust {
    documents: BTreeSet<String>,
    folders: Vec<String>,
    sources: Vec<Source>,
    imported: BTreeSet<String>,
    home: Option<String>,
    ignored: Vec<String>,
}

impl Trust {
    /// Reads the settings file's text. `home` is the user's home folder, which `~` in a folder stands for.
    ///
    /// An entry that cannot be read trusts nothing and is listed in [`ignored`](Self::ignored). The rest of
    /// the file still counts.
    pub fn parse(text: &str, home: Option<&str>) -> Result<Self, TrustError> {
        let document = document(text)?;
        let mut trust = Self::empty(home);
        let Some(table) = document.get("trust") else {
            return Ok(trust);
        };
        let Some(table) = table.as_table_like() else {
            trust.ignored.push("trust".into());
            return Ok(trust);
        };
        for key in [DOCUMENTS, FOLDERS, SOURCES, IMPORTED] {
            let Some(item) = table.get(key) else { continue };
            let Some(array) = item.as_array() else {
                trust.ignored.push(key.into());
                continue;
            };
            for value in array {
                if !value.as_str().is_some_and(|entry| trust.take(key, entry)) {
                    trust.ignored.push(format!("{key}: {}", value.to_string().trim()));
                }
            }
        }
        Ok(trust)
    }

    /// Trusts nothing, and knows where `~` is.
    pub(crate) fn empty(home: Option<&str>) -> Self {
        Self { home: home.map(String::from), ..Self::default() }
    }

    /// Takes one entry of a list. `false` when it cannot be read.
    fn take(&mut self, key: &str, entry: &str) -> bool {
        match key {
            DOCUMENTS if !entry.is_empty() => drop(self.documents.insert(entry.into())),
            IMPORTED if !entry.is_empty() => drop(self.imported.insert(entry.into())),
            FOLDERS if Path::parse(entry, self.home.as_deref()).is_some_and(|path| path.is_folder()) => {
                self.folders.push(entry.into());
            }
            SOURCES => match Source::parse(entry) {
                Ok(source) => self.sources.push(source),
                Err(_) => return false,
            },
            _ => return false,
        }
        true
    }

    pub fn documents(&self) -> impl Iterator<Item = &str> {
        self.documents.iter().map(String::as_str)
    }

    /// The trusted folders, as the file writes them.
    pub fn folders(&self) -> &[String] {
        &self.folders
    }

    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    /// Entries of the file that could not be read, for the settings screen to show.
    pub fn ignored(&self) -> &[String] {
        &self.ignored
    }

    pub fn trusts_document(&self, id: &str) -> bool {
        self.documents.contains(id)
    }

    /// Whether the user made this document from another file. A folder does not vouch for such a one.
    pub fn imported(&self, id: &str) -> bool {
        self.imported.contains(id)
    }

    /// Whether a file or folder lies inside a trusted folder. `path` is absolute, with links and `..`
    /// resolved by the shell: the comparison is by name alone.
    pub fn trusts_folder(&self, path: &str) -> bool {
        let home = self.home.as_deref();
        let Some(path) = Path::parse(path, home) else {
            return false;
        };
        self.folders.iter().filter_map(|folder| Path::parse(folder, home)).any(|folder| folder.contains(&path))
    }

    pub fn trusts_source(&self, url: &HttpsUrl) -> bool {
        self.sources.iter().any(|source| source.allows(url))
    }
}

fn document(text: &str) -> Result<DocumentMut, TrustError> {
    text.parse().map_err(|error: toml_edit::TomlError| TrustError::Toml(error.to_string()))
}

/// Changes one list of the `[trust]` table and returns the file's new text. Everything else in the file,
/// comments included, stays as the user wrote it.
///
/// `create` makes the table and the list when they are missing. Without it a missing list is left missing.
pub(crate) fn edit(
    text: &str,
    key: &'static str,
    create: bool,
    change: impl FnOnce(&mut Array),
) -> Result<String, TrustError> {
    let mut document = document(text)?;
    if !create && document.get("trust").and_then(|table| table.get(key)).is_none() {
        return Ok(text.into());
    }
    let table = document.entry("trust").or_insert(Item::Table(Table::new()));
    let table = table.as_table_like_mut().ok_or(TrustError::Shape("trust"))?;
    let list = table.entry(key).or_insert(Item::Value(Value::Array(Array::new())));
    change(list.as_array_mut().ok_or(TrustError::Shape(key))?);
    Ok(document.to_string())
}

/// Adds an entry unless `is` finds it already there.
pub(crate) fn add(list: &mut Array, entry: &str, is: impl Fn(&str) -> bool) {
    if list.iter().any(|value| value.as_str().is_some_and(&is)) {
        return;
    }
    // Follow the list's own layout: one entry a line when it is written that way, and for a new list.
    let indent = match list.iter().last() {
        None => Some("\n    ".to_string()),
        Some(last) => {
            let before = last.decor().prefix().and_then(|prefix| prefix.as_str()).unwrap_or_default();
            before.rfind('\n').map(|at| before[at..].to_string())
        }
    };
    let Some(indent) = indent else {
        return list.push(entry);
    };
    // What trails the last entry, a comment perhaps, stays on its line and ahead of the new entry.
    let mut before = String::new();
    if let Some(last) = list.iter_mut().last() {
        before.push_str(last.decor().suffix().and_then(|suffix| suffix.as_str()).unwrap_or_default().trim_end());
        last.decor_mut().set_suffix("");
    }
    before.push_str(list.trailing().as_str().unwrap_or_default().trim_end());
    let mut value = Value::from(entry);
    value.decor_mut().set_prefix(format!("{before}{indent}"));
    list.push_formatted(value);
    list.set_trailing("\n");
    list.set_trailing_comma(true);
}

pub(crate) fn remove(list: &mut Array, is: impl Fn(&str) -> bool) {
    list.retain(|value| !value.as_str().is_some_and(&is));
}
