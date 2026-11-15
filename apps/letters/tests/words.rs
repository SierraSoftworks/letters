//! The person's own dictionary, which every window shares through one file.

use std::fs;
use std::path::PathBuf;

use letters::platform::Desktop;
use letters::words::Words;
use letters_editor::Platform;

/// A dictionary file for one test, in a folder that is removed when the test ends.
struct Shelf(PathBuf);

impl Shelf {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("letters-words-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }

    fn words(&self) -> Words {
        Words::of(&self.0)
    }

    /// The words as a window that has read nothing yet finds them.
    fn read(&self) -> Vec<String> {
        self.words().changed(&mut None).unwrap_or_default()
    }
}

impl Drop for Shelf {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_learned_word_reaches_every_window_once() {
    let shelf = Shelf::new("learned");
    let (words, mut seen) = (shelf.words(), None);
    assert_eq!(words.changed(&mut seen), None, "no file is no news");

    for word in ["Kgalagadi", "fynbos", " Kgalagadi "] {
        words.learn(word).unwrap();
    }
    assert_eq!(words.changed(&mut seen), Some(vec!["Kgalagadi".to_owned(), "fynbos".to_owned()]));
    assert_eq!(words.changed(&mut seen), None);
    // Another window, or another copy of the app, reads the same file.
    assert_eq!(shelf.read(), ["Kgalagadi", "fynbos"]);

    fs::remove_dir_all(&shelf.0).unwrap();
    assert_eq!(words.changed(&mut seen), Some(vec![]), "a dictionary that has gone takes its words with it");
}

#[test]
fn a_dictionary_edited_by_hand_keeps_its_words() {
    let shelf = Shelf::new("by-hand");
    fs::create_dir_all(shelf.0.join(".letters")).unwrap();
    fs::write(shelf.0.join(".letters/dictionary.txt"), "braai\n\n  lekker  \nrooibos").unwrap();
    assert_eq!(shelf.read(), ["braai", "lekker", "rooibos"]);

    shelf.words().learn("stoep").unwrap();
    assert_eq!(shelf.read(), ["braai", "lekker", "rooibos", "stoep"]);
}

#[test]
fn what_is_not_a_word_is_not_kept() {
    let shelf = Shelf::new("refused");
    for word in ["", "   ", "two\nlines"] {
        assert!(shelf.words().learn(word).is_err(), "{word:?} was kept");
    }
    assert!(shelf.read().is_empty());
}

#[test]
fn a_word_the_editor_learns_is_kept_when_the_shell_says_where() {
    let shelf = Shelf::new("platform");
    // Without a place to keep it the word lasts as long as its window, and nothing fails.
    Desktop::new().learn_word("veld");
    assert!(shelf.read().is_empty());

    Desktop::new().with_words(shelf.words()).learn_word("veld");
    assert_eq!(shelf.read(), ["veld"]);
}
