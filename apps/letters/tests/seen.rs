//! What the person last saw of each document, which every window shares through one file.

use std::fs;
use std::path::PathBuf;

use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use letters::seen::{Seen, Telling};
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

/// A server that notes what it is told, and takes it unless it `refuses` at the time.
#[derive(Clone, Default)]
struct Account {
    heard: Arc<Mutex<Vec<String>>>,
    refuses: Arc<Mutex<bool>>,
}

impl Account {
    fn tell(&self, telling: &mut Telling, seen: &str, closing: bool) {
        let account = self.clone();
        telling.tell(&revision(seen), closing, move |seen| {
            account.heard.lock().unwrap().push(seen);
            !*account.refuses.lock().unwrap()
        });
    }

    /// What the server was told so far, once the telling that is on its way has ended.
    fn heard(&self, telling: &mut Telling) -> Vec<String> {
        if let Some(last) = telling.leave() {
            last.join().unwrap();
        }
        self.heard.lock().unwrap().clone()
    }
}

#[test]
fn a_server_is_told_what_it_does_not_know_and_once() {
    let account = Account::default();
    let mut telling = Telling::after(Some(revision("1@2")));
    account.tell(&mut telling, "1@2", false);
    assert!(account.heard(&mut telling).is_empty(), "the server said so itself");

    for (seen, closing) in [("3@2", false), ("3@2", false), ("5@2", false), ("5@2", true)] {
        account.tell(&mut telling, seen, closing);
        account.heard(&mut telling);
    }
    assert_eq!(account.heard(&mut telling), ["3@2", "5@2"]);
}

#[test]
fn what_a_server_did_not_take_is_told_once_more_at_closing() {
    let account = Account::default();
    let mut telling = Telling::default();
    *account.refuses.lock().unwrap() = true;
    account.tell(&mut telling, "3@2", false);
    settle(&telling);
    // A save does not tell it again, and closing does, once.
    account.tell(&mut telling, "3@2", false);
    settle(&telling);
    account.tell(&mut telling, "3@2", true);
    settle(&telling);
    account.tell(&mut telling, "3@2", true);
    assert_eq!(account.heard(&mut telling), ["3@2", "3@2"]);
}

/// Waits until no telling is on its way.
fn settle(telling: &Telling) {
    let started = std::time::Instant::now();
    while telling.on_its_way() {
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "the telling never ended");
        std::thread::yield_now();
    }
}

#[test]
fn the_last_word_at_closing_follows_the_telling_that_is_on_its_way() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let (go, wait) = channel::<()>();
    let mut telling = Telling::default();
    let tell = |heard: &Arc<Mutex<Vec<String>>>, held: Option<Receiver<()>>| {
        let heard = heard.clone();
        move |seen: String| {
            // The first is held up on its way, as by a slow server.
            if let Some(held) = held {
                let _ = held.recv();
            }
            heard.lock().unwrap().push(seen);
            true
        }
    };
    telling.tell(&revision("3@2"), false, tell(&heard, Some(wait)));
    // A save while it is on its way is left out, and closing is not.
    telling.tell(&revision("4@2"), false, tell(&heard, None));
    telling.tell(&revision("5@2"), true, tell(&heard, None));
    go.send(()).unwrap();
    telling.leave().unwrap().join().unwrap();
    assert_eq!(*heard.lock().unwrap(), ["3@2", "5@2"]);
}
