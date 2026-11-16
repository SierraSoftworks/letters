//! What the person last saw of each document, kept on this device.
//!
//! `.letters/seen.json` in the person's home folder lists, by a document's id, the revision it read at when
//! they last saved or closed it here. A window is told it when the document opens, and says who else wrote
//! since. It is kept outside the documents: a document is everyone's, and this is one person's.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};

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

/// Tells a server what its person last saw of a document, off the caller's thread and one telling at a
/// time, so that their account has it for the next device they open the document on.
#[derive(Debug, Default)]
pub struct Telling {
    /// What was last sent, whether it arrived or not.
    sent: Option<Revision>,
    /// The telling that is on its way, which ends with whether the server took it.
    last: Option<JoinHandle<bool>>,
    /// Whether the server did not take what was last sent, and whether that was told once more.
    refused: bool,
    again: bool,
}

impl Telling {
    /// Starts from what the server itself `said` the person saw, which it need not be told again.
    pub fn after(said: Option<Revision>) -> Self {
        Self { sent: said, ..Self::default() }
    }

    /// Tells that the document was seen as it reads at `revision`, with `tell`, which is handed the
    /// revision as text and says whether the server took it.
    ///
    /// A save's telling is left out while another is on its way, since the next one carries more, and
    /// what the server did not take waits for a new revision. `closing` is the last chance: the telling
    /// then follows the one on its way, and what the server did not take is told once more.
    pub fn tell(&mut self, revision: &Revision, closing: bool, tell: impl FnOnce(String) -> bool + Send + 'static) {
        let before = match self.last.take() {
            Some(last) if last.is_finished() => {
                self.refused = !last.join().unwrap_or(false);
                None
            }
            Some(last) if !closing => return self.last = Some(last),
            before => before,
        };
        if closing && self.refused && !std::mem::replace(&mut self.again, true) {
            self.sent = None;
        }
        if self.sent.as_ref() == Some(revision) {
            return self.last = before;
        }
        self.refused = false;
        let seen = revision.to_string();
        self.last = Some(thread::spawn(move || {
            // In the order they were seen, so that the later one stands.
            let _ = before.map(JoinHandle::join);
            tell(seen)
        }));
        self.sent = Some(revision.clone());
    }

    /// Whether a telling is on its way.
    pub fn on_its_way(&self) -> bool {
        self.last.as_ref().is_some_and(|last| !last.is_finished())
    }

    /// The telling that is on its way, for whoever gives it a moment before the app ends.
    pub fn leave(&mut self) -> Option<JoinHandle<bool>> {
        self.last.take()
    }
}
