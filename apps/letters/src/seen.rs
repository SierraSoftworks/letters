//! What the person last saw of each document, kept on this device.
//!
//! `.letters/seen.json` in the person's home folder lists, by a document's id, the revision it read at when
//! they last saved or closed it here. A window is told it when the document opens, and says who else wrote
//! since. It is kept outside the documents: a document is everyone's, and this is one person's.

use std::fs;
use std::path::{Path, PathBuf};

use letters_model::Revision;
use serde_json::{Value, json};

/// How many documents are remembered. A document that drops off the end opens with no news once.
const REMEMBERED: usize = 500;

/// The file is read at each question and rewritten at each change, since every window of every running
/// copy of the app shares it.
#[derive(Clone, Debug)]
pub struct Seen {
    file: PathBuf,
}

impl Seen {
    /// What the user whose home folder is `home` has seen, beside their recent documents.
    pub fn of(home: &Path) -> Self {
        Self::at(home.join(".letters").join("seen.json"))
    }

    pub fn at(file: impl Into<PathBuf>) -> Self {
        Self { file: file.into() }
    }

    /// The documents and what was seen of each, the latest first. A list that cannot be read is an empty one.
    fn listed(&self) -> Vec<(String, String)> {
        let entry =
            |entry: &Value| Some((entry["document"].as_str()?.to_owned(), entry["revision"].as_str()?.to_owned()));
        let read = fs::read(&self.file).ok().and_then(|json| serde_json::from_slice::<Value>(&json).ok());
        read.as_ref().and_then(Value::as_array).map(|list| list.iter().filter_map(entry).collect()).unwrap_or_default()
    }

    /// The revision the document of this `id` read at when the person last had it here. `None` for one
    /// they never had here.
    pub fn last(&self, id: &str) -> Option<Revision> {
        let listed = self.listed();
        listed.iter().find(|(document, _)| document == id).and_then(|(_, revision)| revision.parse().ok())
    }

    /// Notes that the person has seen the document of this `id` as it reads at `revision`. Losing this
    /// costs one note of what others wrote, so a failure to write it stops nothing.
    pub fn saw(&self, id: &str, revision: &Revision) {
        let mut listed = self.listed();
        listed.retain(|(document, _)| document != id);
        listed.insert(0, (id.to_owned(), revision.to_string()));
        listed.truncate(REMEMBERED);
        let entry = |(document, revision): &(String, String)| json!({ "document": document, "revision": revision });
        let list: Value = listed.iter().map(entry).collect();
        // Written beside and renamed, so that a crash leaves a whole list or the last one.
        let beside = self.file.with_extension("json.new");
        let written = fs::create_dir_all(self.file.parent().unwrap_or(Path::new("")))
            .and_then(|()| fs::write(&beside, format!("{list:#}\n")))
            .and_then(|()| fs::rename(&beside, &self.file));
        if let Err(error) = written {
            eprintln!("letters: could not remember what was seen of a document: {error}");
        }
    }
}
