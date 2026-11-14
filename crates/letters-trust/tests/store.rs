mod common;

use common::{HOME, Memory, SETTINGS, store};
use letters_trust::{HttpsUrl, Mode, Origin, Source, Trust, TrustError, TrustStore};

fn url(text: &str) -> HttpsUrl {
    HttpsUrl::parse(text).unwrap()
}

/// A second store over the same file, as another window would open it.
fn reopened(memory: &Memory) -> Trust {
    let mut store = TrustStore::new(memory.clone(), Some(HOME));
    store.load().expect("what was written loads");
    store.trust().clone()
}

#[test]
fn nothing_is_trusted_before_the_file_is_read_or_when_there_is_none() {
    let mut unread = TrustStore::new(Memory::holding(SETTINGS), Some(HOME));
    assert!(!unread.trust().trusts_document("doc-trusted"));
    unread.load().unwrap();
    assert!(unread.trust().trusts_document("doc-trusted"));

    let mut missing = TrustStore::new(Memory::default(), Some(HOME));
    missing.load().expect("a missing file is an empty one");
    assert_eq!(missing.trust().mode("doc-trusted", Origin::File("/home/ada/Documents/a.let")), Mode::Safe);
}

#[test]
fn what_the_user_trusts_is_written_and_read_back() {
    for settings in ["", SETTINGS, "[trust]\ndocuments = []", "trust = { folders = ['/srv/shared'] }"] {
        let (mut store, memory) = store(settings);
        let source = Source::parse("https://cdn.example.net/img/").unwrap();
        store.trust_document("doc-new").unwrap();
        store.trust_folder("/home/ada/Notes").unwrap();
        store.trust_folder("/mnt/archive/").unwrap();
        store.trust_source(&source).unwrap();

        for trust in [store.trust().clone(), reopened(&memory)] {
            assert!(trust.trusts_document("doc-new"), "{settings:?}");
            assert!(trust.trusts_folder("/home/ada/Notes/todo.let"), "{settings:?}");
            assert!(trust.trusts_folder("/mnt/archive/2020/old.let"), "{settings:?}");
            assert!(trust.trusts_source(&url("https://cdn.example.net/img/a.png")), "{settings:?}");
            assert!(trust.ignored().is_empty(), "{settings:?}: {:?}", trust.ignored());
        }
    }
}

#[test]
fn what_the_user_forgets_is_no_longer_trusted() {
    let (mut store, memory) = store(SETTINGS);
    store.forget_document("doc-trusted").unwrap();
    store.forget_folder("/home/ada/Documents/").unwrap();
    store.forget_source(&Source::parse("upload.wikimedia.org").unwrap()).unwrap();

    for trust in [store.trust().clone(), reopened(&memory)] {
        assert!(!trust.trusts_document("doc-trusted"));
        assert!(!trust.trusts_folder("/home/ada/Documents/a.let"));
        assert!(!trust.trusts_source(&url("https://upload.wikimedia.org/a.png")));
        // The rest stays.
        assert!(trust.trusts_folder("/srv/shared/a.let"));
        assert!(trust.trusts_source(&url("https://img.example.org/charts/q3.png")));
    }
}

#[test]
fn the_rest_of_the_settings_file_is_left_as_the_user_wrote_it() {
    let (mut store, memory) = store(SETTINGS);
    store.trust_document("doc-new").unwrap();
    store.trust_folder("/home/ada/Notes").unwrap();
    store.forget_source(&Source::parse("upload.wikimedia.org").unwrap()).unwrap();

    let text = memory.text();
    for kept in ["# Ada's settings", "theme = \"dark\"", "# work and family", "[spelling]\nlanguage = \"en-GB\""] {
        assert!(text.contains(kept), "{kept:?} is missing from:\n{text}");
    }
}

#[test]
fn a_folder_under_home_is_written_with_a_tilde() {
    let (mut store, memory) = store("");
    store.trust_folder("/home/ada/Notes/journal/").unwrap();
    store.trust_folder("/srv/shared").unwrap();
    assert_eq!(store.trust().folders(), ["~/Notes/journal", "/srv/shared"]);
    assert!(memory.text().contains("\"~/Notes/journal\""), "{}", memory.text());
}

#[test]
fn trusting_twice_or_forgetting_the_unknown_writes_nothing() {
    let (mut store, memory) = store(SETTINGS);
    store.trust_document("doc-trusted").unwrap();
    store.trust_folder("~/Documents/").unwrap();
    store.trust_folder("/home/ada/Documents").unwrap();
    store.trust_source(&Source::parse("https://UPLOAD.wikimedia.org").unwrap()).unwrap();
    store.forget_document("doc-unknown").unwrap();
    store.forget_folder("/nowhere/at/all").unwrap();
    assert_eq!(*memory.writes.borrow(), 0);
    assert_eq!(memory.text(), SETTINGS);

    let (mut empty, memory) = common::store("");
    empty.forget_document("doc-unknown").unwrap();
    assert_eq!(memory.text(), "", "forgetting does not create the table");
}

#[test]
fn a_change_starts_from_the_file_as_it_is_now() {
    let (mut store, memory) = store(SETTINGS);
    // Another window trusts a document, and the user edits another setting.
    memory.set(&SETTINGS.replace("\"doc-trusted\"", "\"doc-trusted\", \"doc-other\"").replace("dark", "light"));
    store.trust_document("doc-new").unwrap();

    assert!(memory.text().contains("theme = \"light\""));
    for id in ["doc-trusted", "doc-other", "doc-new"] {
        assert!(store.trust().trusts_document(id), "{id}");
    }
}

#[test]
fn a_file_that_is_not_toml_is_reported_and_left_alone() {
    let broken = "[trust\ndocuments = [\"doc-trusted\"]";
    let memory = Memory::holding(broken);
    let mut store = TrustStore::new(memory.clone(), Some(HOME));

    assert!(matches!(store.load(), Err(TrustError::Toml(_))));
    assert!(!store.trust().trusts_document("doc-trusted"));
    assert!(matches!(store.trust_document("doc-new"), Err(TrustError::Toml(_))));
    assert!(!store.trust().trusts_document("doc-new"));
    assert_eq!(memory.text(), broken);
}

#[test]
fn a_reload_that_fails_keeps_what_was_trusted() {
    let (mut store, memory) = store(SETTINGS);
    memory.set("not = [toml");
    assert!(store.load().is_err());
    assert!(store.trust().trusts_document("doc-trusted"));

    *memory.broken.borrow_mut() = true;
    assert!(matches!(store.load(), Err(TrustError::Store(_))));
    assert!(matches!(store.trust_document("doc-new"), Err(TrustError::Store(_))));
    assert!(store.trust().trusts_document("doc-trusted"));
    assert!(!store.trust().trusts_document("doc-new"));
}

#[test]
fn an_entry_that_cannot_be_read_trusts_nothing_and_is_kept() {
    let settings = r#"
[trust]
documents = ["doc-trusted", 7, ""]
folders = ["~/Documents", "relative/folder", "/"]
image_sources = ["https://upload.wikimedia.org/", "http://plain.example/", "https://192.168.1.1/"]
"#;
    let (mut store, memory) = store(settings);
    let trust = store.trust().clone();
    assert_eq!(trust.documents().collect::<Vec<_>>(), ["doc-trusted"]);
    assert_eq!(trust.folders(), ["~/Documents"]);
    assert_eq!(trust.sources(), [Source::parse("upload.wikimedia.org").unwrap()]);
    assert_eq!(trust.ignored().len(), 6, "{:?}", trust.ignored());
    assert!(!trust.trusts_folder("/etc/passwd"));

    store.trust_document("doc-new").unwrap();
    for kept in ["relative/folder", "http://plain.example/", "7"] {
        assert!(memory.text().contains(kept), "{kept:?} is missing from:\n{}", memory.text());
    }
}

#[test]
fn a_list_of_the_wrong_shape_is_ignored_and_never_overwritten() {
    for (settings, key) in [("trust = 'everything'", "trust"), ("[trust]\nfolders = '~/Documents'", "folders")] {
        let (mut store, memory) = store(settings);
        assert!(!store.trust().trusts_folder("/home/ada/Documents/a.let"), "{settings}");
        assert_eq!(store.trust().ignored(), [key], "{settings}");
        assert!(matches!(store.trust_folder("/home/ada/Notes"), Err(TrustError::Shape(shape)) if shape == key));
        assert_eq!(memory.text(), settings);
    }
}

#[test]
fn only_a_real_document_or_folder_can_be_trusted() {
    let (mut store, memory) = store(SETTINGS);
    assert!(matches!(store.trust_document(""), Err(TrustError::NoDocument)));
    for path in ["/", "relative/folder", "/home/ada/Documents/../..", "", r"C:\", "~eve/Documents"] {
        assert!(matches!(store.trust_folder(path), Err(TrustError::Folder(_))), "{path:?}");
    }
    assert_eq!(memory.text(), SETTINGS);
}

#[test]
fn a_list_keeps_its_layout_and_a_new_one_has_an_entry_a_line() {
    let cases = [
        // A new list: one entry a line, since it will grow.
        ("", 2),
        // A list on one line stays on it.
        ("[trust]\ndocuments = [\"doc-a\"]\n", 0),
        // A list written by hand, with and without a last comma, and with comments.
        ("[trust]\ndocuments = [\n  \"doc-a\", # the report\n  \"doc-b\"\n]\n", 3),
        ("[trust]\ndocuments = [\n  \"doc-a\",\n  \"doc-b\", # the plan\n]\n", 3),
        ("[trust]\ndocuments = [\n  \"doc-a\",\n  \"doc-b\" # the plan\n]\n", 3),
    ];
    for (settings, own_lines) in cases {
        let (mut store, memory) = store(settings);
        store.trust_document("doc-b").unwrap();
        store.trust_document("doc-c").unwrap();

        let text = memory.text();
        let entry = |line: &&str| line.trim_start().starts_with("\"doc-");
        assert_eq!(text.lines().filter(entry).count(), own_lines, "entries on their own lines in:\n{text}");
        assert!(text.lines().all(|line| !line.trim_start().starts_with(',')), "a stray comma in:\n{text}");
        for comment in settings.lines().filter_map(|line| line.split_once('#')) {
            assert!(text.contains(comment.1), "the comment {:?} is missing from:\n{text}", comment.1);
        }
        let trust = reopened(&memory);
        assert!(trust.trusts_document("doc-b") && trust.trusts_document("doc-c"), "{text}");
        assert!(trust.ignored().is_empty(), "{text}");
    }
}
