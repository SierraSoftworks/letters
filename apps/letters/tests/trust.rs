//! The settings file the trust rules are kept in, and how a document's place decides its treatment.

use std::fs;
use std::path::PathBuf;

use letters::trust::{self, Settings};
use letters_trust::{Mode, Store};

/// A user's home folder for one test, removed when the test ends.
struct Home(PathBuf);

impl Home {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("letters-trust-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        // Paths are compared as the system resolves them, which on some systems is not as they were made.
        Self(fs::canonicalize(&path).unwrap())
    }

    fn settings(&self) -> PathBuf {
        self.0.join(".letters/settings.toml")
    }

    /// A file at `inside` the home folder, in folders made for it.
    fn file(&self, inside: &str) -> PathBuf {
        let path = self.0.join(inside);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"a document").unwrap();
        path
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_document_made_here_stays_trusted_wherever_it_is_kept() {
    let home = Home::new("made-here");
    let mut store = trust::store(&home.0);
    store.load().expect("no settings file is no trust, not a failure");
    let downloads = home.file("Downloads/Report.let");
    assert_eq!(trust::mode(store.trust(), "doc-1", None), Mode::Trusted, "a document with no file yet");
    assert_eq!(trust::mode(store.trust(), "doc-1", Some(&downloads)), Mode::Safe, "nothing vouches for it yet");

    store.trust_document("doc-1").unwrap();
    assert_eq!(trust::mode(store.trust(), "doc-1", Some(&downloads)), Mode::Trusted);
    assert_eq!(trust::mode(store.trust(), "doc-2", Some(&downloads)), Mode::Safe, "another document there");

    // Another window, or the next launch, reads the same file.
    let mut later = trust::store(&home.0);
    later.load().unwrap();
    assert_eq!(trust::mode(later.trust(), "doc-1", Some(&downloads)), Mode::Trusted);
}

#[test]
fn a_trusted_folder_vouches_for_what_really_lies_in_it() {
    let home = Home::new("folders");
    let (inside, outside) = (home.file("Documents/Letters/Plan.let"), home.file("Downloads/Plan.let"));
    let mut store = trust::store(&home.0);
    store.trust_folder(home.0.join("Documents").to_str().unwrap()).unwrap();

    assert_eq!(trust::mode(store.trust(), "doc", Some(&inside)), Mode::Trusted);
    assert_eq!(trust::mode(store.trust(), "doc", Some(&outside)), Mode::Safe);
    let gone = home.0.join("Documents/Gone.let");
    assert_eq!(trust::mode(store.trust(), "doc", Some(&gone)), Mode::Safe, "a path that leads nowhere");

    // A link in a trusted folder to a file outside it does not bring the file in.
    #[cfg(unix)]
    {
        let link = home.0.join("Documents/Linked.let");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert_eq!(trust::mode(store.trust(), "doc", Some(&link)), Mode::Safe);
    }
}

#[test]
fn the_rest_of_the_settings_file_is_left_as_it_was() {
    let home = Home::new("settings");
    fs::create_dir_all(home.settings().parent().unwrap()).unwrap();
    fs::write(home.settings(), "# Mine\n[editor]\nfont = \"Inter\"\n").unwrap();

    let mut store = trust::store(&home.0);
    store.load().unwrap();
    store.trust_document("doc-1").unwrap();

    let text = Settings::of(&home.0).read().unwrap().expect("the file is there");
    assert!(text.contains("# Mine") && text.contains("font = \"Inter\""), "{text}");
    assert!(text.contains("doc-1"), "{text}");
    let folder = fs::read_dir(home.settings().parent().unwrap()).unwrap();
    assert_eq!(folder.count(), 1, "nothing is left beside the file");
}
