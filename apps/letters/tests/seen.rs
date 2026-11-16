//! What the person last saw of each document, which every window shares through one file.

use std::fs;
use std::path::PathBuf;

use letters::seen::Seen;
use letters_model::Revision;

/// A file of what was seen for one test, in a folder that is removed when the test ends.
struct Shelf(PathBuf);

impl Shelf {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("letters-seen-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }

    fn file(&self) -> PathBuf {
        self.0.join(".letters/seen.json")
    }

    /// What was seen, as a window of any running copy of the app finds it.
    fn seen(&self) -> Seen {
        Seen::of(&self.0)
    }
}

impl Drop for Shelf {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn revision(text: &str) -> Revision {
    text.parse().expect("a revision as it is written")
}

#[test]
fn what_was_seen_of_a_document_is_kept_for_the_next_window() {
    let shelf = Shelf::new("kept");
    assert_eq!(shelf.seen().last("rates"), None, "a document never had here has no last visit");

    shelf.seen().saw("rates", &revision("3@7"));
    shelf.seen().saw("house", &revision("9@4,2@7"));
    shelf.seen().saw("rates", &revision("5@7"));

    assert_eq!(shelf.seen().last("rates"), Some(revision("5@7")), "the latest visit stands");
    assert_eq!(shelf.seen().last("house"), Some(revision("9@4,2@7")));
    assert_eq!(shelf.seen().last("other"), None);
}

#[test]
fn a_file_that_cannot_be_read_is_no_last_visit_and_is_written_anew() {
    let shelf = Shelf::new("unreadable");
    fs::create_dir_all(shelf.file().parent().unwrap()).unwrap();
    for held in ["not JSON", r#"{"document": "rates"}"#, r#"[{"document": "rates", "revision": "nonsense"}, 7]"#] {
        fs::write(shelf.file(), held).unwrap();
        assert_eq!(shelf.seen().last("rates"), None, "{held}");
    }
    shelf.seen().saw("rates", &revision("1@2"));
    assert_eq!(shelf.seen().last("rates"), Some(revision("1@2")));
}

#[test]
fn only_the_documents_seen_lately_are_remembered() {
    let shelf = Shelf::new("many");
    let seen = shelf.seen();
    for document in 0..520 {
        seen.saw(&format!("document-{document}"), &revision("1@2"));
    }
    assert_eq!(seen.last("document-0"), None, "the oldest drop off");
    assert_eq!(seen.last("document-519"), Some(revision("1@2")));
    assert!(seen.last("document-100").is_some(), "hundreds stay");
}
