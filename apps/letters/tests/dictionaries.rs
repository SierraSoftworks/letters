//! Finding and reading the system's dictionaries for the languages a document names.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use letters::dictionaries::{find, installed, languages, load, served};
use letters_model::{Block, BlockKind, Body, Document, Run};

/// Folders of dictionaries for one test, removed when the test ends.
struct Shelves(PathBuf);

impl Shelves {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("letters-dictionaries-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// The folders `inside`, as the search is given them. One that does not exist is searched like any other.
    fn folders(&self, inside: &[&str]) -> Vec<PathBuf> {
        inside.iter().map(|folder| self.0.join(folder)).collect()
    }

    /// Puts a dictionary called `name` in the folder `inside`, with these words.
    fn put(&self, inside: &str, name: &str, aff: &[u8], words: &[u8]) {
        let folder = self.0.join(inside);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(format!("{name}.aff")), aff).unwrap();
        fs::write(folder.join(format!("{name}.dic")), words).unwrap();
    }

    /// Puts dictionaries of one word each in the folder `inside`.
    fn stock(&self, inside: &str, names: &[&str]) {
        for name in names {
            self.put(inside, name, b"SET UTF-8\n", b"1\nwort\n");
        }
    }
}

impl Drop for Shelves {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_document_names_its_own_language_and_those_of_its_parts() {
    let block = |id: &str, lang: Option<&str>, runs: &[(&str, Option<&str>)]| {
        let run = |(text, lang): &(&str, Option<&str>)| Run {
            text: (*text).to_owned(),
            lang: lang.map(str::to_owned),
            ..Run::default()
        };
        let mut block = Block::text(id, BlockKind::Paragraph, "");
        block.body = Body::Text(runs.iter().map(run).collect());
        block.lang = lang.map(str::to_owned);
        block
    };
    let document = Document {
        language: "en-GB".into(),
        blocks: vec![
            block("b0", None, &[("Dear all, ", None), ("guten Tag", Some("de_ch")), (" and ", None)]),
            block("b1", Some("pt-BR"), &[("Bom dia", None), ("hello", Some("en-US")), ("?", Some("x"))]),
            block("b2", Some("fr"), &[("Bonjour", Some("fr"))]),
        ],
        ..Document::default()
    };
    // English is built in, and a tag that names no language asks for nothing.
    assert_eq!(Vec::from_iter(languages(&document)), ["de-CH", "fr", "pt-BR"]);
    assert!(languages(&Document { language: "en".into(), ..Document::default() }).is_empty());
    assert_eq!(Vec::from_iter(languages(&Document { language: "nl".into(), ..Document::default() })), ["nl"]);
}

#[test]
fn the_dictionary_nearest_to_a_language_is_found() {
    let shelves = Shelves::new("find");
    shelves.stock("own", &["de_AT", "sv_FI"]);
    shelves.stock("system", &["de_CH", "de_DE", "de_DE_frami", "de_AT", "pt_BR", "pt_PT", "nl", "nl_BE", "en_US"]);
    let folders = shelves.folders(&["missing", "own", "system"]);
    let cases = [
        ("de-CH", Some(("system", "de_CH")), "the region asked for"),
        ("de", Some(("system", "de_DE")), "the country the language is named after"),
        ("de-LU", Some(("system", "de_DE")), "the same, for a region with no dictionary"),
        ("de-AT", Some(("own", "de_AT")), "the person's own folder comes first"),
        ("pt_br", Some(("system", "pt_BR")), "a tag written as a file name"),
        ("nl-NL", Some(("system", "nl")), "the language alone before another region"),
        ("sv", Some(("own", "sv_FI")), "any region when there is nothing nearer"),
        ("fr", None, "no dictionary"),
        ("", None, "no language"),
    ];
    for (language, expected, why) in cases {
        let found = find(&folders, language);
        let expected = expected.map(|(folder, name)| shelves.0.join(folder).join(format!("{name}.aff")));
        assert_eq!(found.map(|found| found.aff), expected, "{language}: {why}");
    }
}

#[test]
fn a_servers_dictionary_is_taken_under_the_tag_the_speller_looks_for() {
    /// The languages a document names, what the server has, and each dictionary to fetch with the tag it
    /// is added under.
    type Case = (&'static [&'static str], &'static [&'static str], &'static [(&'static str, &'static str)]);
    let cases: [Case; 5] = [
        // A region the server lacks falls back to the language's dictionary, fetched once for both.
        (&["de", "de-CH"], &["de-DE"], &[("de-DE", "de")]),
        (&["de-CH"], &["de-DE", "de-CH"], &[("de-CH", "de-CH")]),
        (&["de", "de-CH"], &["de-DE", "de-CH"], &[("de-CH", "de-CH"), ("de-DE", "de")]),
        (&["pt-BR"], &["pt-PT", "pt-BR"], &[("pt-BR", "pt-BR")]),
        (&["fr"], &["de-DE"], &[]),
    ];
    for (named, held, expected) in cases {
        let named: BTreeSet<String> = named.iter().map(|tag| (*tag).to_owned()).collect();
        let held: Vec<String> = held.iter().map(|tag| (*tag).to_owned()).collect();
        let expected: BTreeMap<String, String> =
            expected.iter().map(|(tag, under)| ((*tag).to_owned(), (*under).to_owned())).collect();
        assert_eq!(served(&named, &held), expected, "{named:?} of {held:?}");
    }
}

#[test]
fn half_a_dictionary_is_not_one() {
    let shelves = Shelves::new("half");
    fs::write(shelves.0.join("fr_FR.aff"), "SET UTF-8\n").unwrap();
    fs::write(shelves.0.join("it_IT.dic"), "1\nciao\n").unwrap();
    for language in ["fr", "it"] {
        assert_eq!(find(&shelves.folders(&[""]), language), None, "{language}");
    }
}

#[test]
fn a_dictionary_is_read_whatever_its_encoding() {
    let shelves = Shelves::new("load");
    shelves.put("", "de_DE", b"SET UTF-8\n", "2\nStra\u{df}e\ngr\u{fc}n\n".as_bytes());
    // Latin-1, as older dictionaries are, and Cyrillic in the encoding older Russian ones use.
    shelves.put("", "fr_FR", b"SET ISO8859-1\n", b"2\n\xe9t\xe9\ngar\xe7on\n");
    shelves.put("", "ru_RU", b"SET KOI8-R\n", b"1\n\xcd\xc9\xd2\n");
    let cases =
        [("de", "de_DE", "Stra\u{df}e"), ("fr-CA", "fr_FR", "gar\u{e7}on"), ("ru", "ru_RU", "\u{43c}\u{438}\u{440}")];
    for (language, name, word) in cases {
        let (loaded, dictionary) = load(&shelves.folders(&[""]), language).expect("the dictionary is there");
        assert_eq!(loaded, name);
        assert!(dictionary.check(word), "{word} is in {name}");
        assert!(!dictionary.check("zzyzx"), "{name} does not take anything");
    }
}

#[test]
fn a_language_without_a_usable_dictionary_is_left_alone() {
    let shelves = Shelves::new("unusable");
    shelves.put("", "es_ES", b"SET UTF-8\nFLAG no-such-kind\n", b"1\nhola\n");
    for language in ["es", "it"] {
        assert!(load(&shelves.folders(&[""]), language).is_none(), "{language}");
    }
}

#[test]
fn the_languages_with_a_dictionary_are_listed_once_each() {
    let shelves = Shelves::new("installed");
    shelves.stock("own", &["de_DE", "de_DE_frami", "pt_BR", "en_ZA"]);
    shelves.stock("shared", &["de_DE", "fr", "notes"]);
    // Half a dictionary is none.
    fs::write(shelves.0.join("shared/nl_NL.aff"), b"SET UTF-8\n").unwrap();
    let listed = installed(&shelves.folders(&["own", "shared", "missing"]));
    // English is built in, so the system's is not offered a second time.
    assert_eq!(Vec::from_iter(listed), ["de-DE", "fr", "pt-BR"]);
}
