//! Words, text and suggestions through the public interface, with the bundled English.

use std::collections::BTreeSet;

use letters_spell::{Dictionary, Speller};

fn words(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|word| word.to_string()).collect()
}

/// The words `speller` flags in `text`, for a document in `language` that lists `document` words.
fn flagged<'a>(speller: &Speller, language: &str, document: &[&str], text: &'a str) -> Vec<&'a str> {
    let document = words(document);
    let checker = speller.checker(language, &document).expect("the language has a dictionary");
    checker.misspellings(text).into_iter().map(|range| &text[range]).collect()
}

/// A dictionary of a few words with no rules, standing in for one the shell loads. A real one uses every
/// letter of its language, which the last entry does here.
fn tiny(list: &[&str]) -> Dictionary {
    let dic = format!("{}\n{}\nabcdefghijklmnopqrstuvwxyz\n", list.len() + 1, list.join("\n"));
    Dictionary::parse("SET UTF-8\n", &dic).unwrap()
}

#[test]
fn each_english_follows_its_own_spelling() {
    let cases = [
        ("en-US", "color", true),
        ("en-US", "colour", false),
        ("en", "color", true),
        ("en-GB", "colour", true),
        ("en-GB", "color", false),
        // British English is written both ways.
        ("en-GB", "organise", true),
        ("en-GB", "organize", true),
        ("en-AU", "colour", true),
        ("en-CA", "colour", true),
        ("en-CA", "organize", true),
        // A region without its own dictionary follows British spelling.
        ("en-NZ", "colour", true),
        // Tags are read loosely: any case, Hunspell's underscores, a script.
        ("EN_gb", "colour", true),
        ("en-Latn-GB", "colour", true),
    ];
    let (speller, none) = (Speller::new(), BTreeSet::new());
    for (language, word, correct) in cases {
        let checker = speller.checker(language, &none).expect(language);
        assert_eq!(checker.check(word), correct, "{word} in {language}");
    }
}

#[test]
fn words_are_judged_as_a_writer_expects() {
    let cases = [
        ("hello", true),
        ("Hello", true),
        ("HELLO", true),
        ("helo", false),
        ("London", true),
        ("london", false),
        ("don't", true),
        ("don’t", true),
        ("children's", true),
        ("café", true),
        ("naïve", true),
        ("recieve", false),
    ];
    let (speller, none) = (Speller::new(), BTreeSet::new());
    let checker = speller.checker("en-GB", &none).unwrap();
    for (word, correct) in cases {
        assert_eq!(checker.check(word), correct, "{word}");
    }
}

#[test]
fn a_language_without_a_dictionary_is_left_unchecked() {
    let (speller, none) = (Speller::new(), BTreeSet::new());
    for language in ["fr", "de-DE", "und", "x-klingon", "", "english"] {
        assert!(!speller.supports(language), "{language:?}");
        assert!(speller.checker(language, &none).is_none(), "{language:?}");
    }
    assert!(speller.supports("en-GB"));
}

#[test]
fn corrections_include_the_intended_word() {
    let cases = [
        ("en-US", "recieve", "receive"),
        ("en-US", "definately", "definitely"),
        ("en-US", "Wendsday", "Wednesday"),
        ("en-GB", "colur", "colour"),
        ("en-US", "colur", "color"),
    ];
    let (speller, none) = (Speller::new(), BTreeSet::new());
    for (language, typed, intended) in cases {
        let suggestions = speller.checker(language, &none).unwrap().suggest(typed);
        assert!(suggestions.iter().any(|s| s == intended), "{typed} in {language} gave {suggestions:?}");
    }
}

#[test]
fn document_and_user_words_are_accepted_on_top_of_the_dictionary() {
    let cases = [
        // (document words, user words, word, accepted)
        (&[][..], &[][..], "Okafor", false),
        (&["Okafor"], &[], "Okafor", true),
        (&[], &["Okafor"], "Okafor", true),
        // A listed word covers its possessive and its upper-case form, but a name stays capitalised.
        (&["Okafor"], &[], "Okafor's", true),
        (&["Okafor"], &[], "Okafor’s", true),
        (&["Okafor"], &[], "OKAFOR", true),
        (&["Okafor"], &[], "okafor", false),
        // A word listed in lower case can start a sentence.
        (&[], &["zorbl"], "Zorbl", true),
        (&[], &["zorbl"], "ZORBL", true),
        (&["O’Leary"], &[], "O'Leary", true),
        (&["Okafor"], &[], "Okafro", false),
    ];
    for (document, user, word, accepted) in cases {
        let mut speller = Speller::new();
        speller.set_user_words(user);
        let document = words(document);
        let checker = speller.checker("en-GB", &document).unwrap();
        assert_eq!(checker.check(word), accepted, "{word} with {document:?} and {user:?}");
    }
}

#[test]
fn the_users_word_file_is_read_line_by_line() {
    let mut speller = Speller::new();
    speller.set_user_words("Okafor\n\n  zorbl  \n".lines());
    assert_eq!(speller.user_words(), &words(&["Okafor", "zorbl"]));
}

#[test]
fn a_listed_word_close_to_the_typed_one_is_suggested_first() {
    let mut speller = Speller::new();
    speller.set_user_words(["Okafor"]);
    let document = words(&["Harland"]);
    let checker = speller.checker("en-GB", &document).unwrap();
    assert_eq!(checker.suggest("Harlnad").first().map(String::as_str), Some("Harland"));
    assert_eq!(checker.suggest("Okafro").first().map(String::as_str), Some("Okafor"));
    assert!(!checker.suggest("recieve").iter().any(|s| s == "Harland" || s == "Okafor"));
}

#[test]
fn only_misspelled_prose_is_flagged_in_text() {
    let cases: [(&str, &[&str]); 16] = [
        ("The quick brwn fox jumps ovr the lazy dog.", &["brwn", "ovr"]),
        ("Nothing wrong here, is there?", &[]),
        // Offsets are bytes, also after text that takes several of them.
        ("“Café” fiancée, naïve wrods…", &["wrods"]),
        ("It's the dogs' dinner, and I don’t 'mind'.", &[]),
        // Addresses, and words with digits or inner punctuation, are not prose.
        ("Write to ben@exampel.com or see https://exampel.com/pth and www.exampel.com.", &[]),
        ("Version 2b of report_finl.pdf covers the B12 and e.g. the 3rd qtr.", &["qtr"]),
        ("Points x and y lie on a line.", &[]),
        // A hyphenated word is flagged by its wrong part.
        ("A well-known and well-knwn fact.", &["knwn"]),
        ("A non-trivial, state-of-the-art answer.", &[]),
        ("The Wi-Fi is down.", &["Wi", "Fi"]),
        // Words in letters English does not use are someone else's to judge.
        ("He said λόγος, then слово, then 言葉, in Łódź.", &[]),
        ("Upper case is checked too: NASA, INTRODUCTOIN.", &["INTRODUCTOIN"]),
        ("Harland wrote to Harland's bank.", &["Harland", "Harland's"]),
        ("", &[]),
        ("   \n\t ", &[]),
        ("teh", &["teh"]),
    ];
    let speller = Speller::new();
    for (text, expected) in cases {
        assert_eq!(flagged(&speller, "en-GB", &[], text), expected, "{text}");
    }
}

#[test]
fn listed_words_clear_their_flags_in_text() {
    let speller = Speller::new();
    assert_eq!(flagged(&speller, "en-GB", &["Harland"], "Harland wrote to Harland's bank."), [""; 0]);
    assert_eq!(flagged(&speller, "en-GB", &["Wi-Fi"], "The Wi-Fi is down."), [""; 0]);
}

#[test]
fn an_added_dictionary_serves_its_language() {
    let mut speller = Speller::new();
    speller.add_dictionary("nl_NL", tiny(&["hallo", "wereld"]));
    // Any region of the language is served, rather than left unchecked.
    for language in ["nl-NL", "nl", "nl-BE"] {
        assert_eq!(flagged(&speller, language, &[], "hallo wereld helo"), ["helo"], "{language}");
    }
    assert!(!speller.supports("de"));
}

#[test]
fn an_added_dictionary_is_preferred_for_its_exact_tag() {
    let mut speller = Speller::new();
    speller.add_dictionary("nl", tiny(&["hallo"]));
    speller.add_dictionary("nl-BE", tiny(&["allee"]));
    speller.add_dictionary("en-GB", tiny(&["blorp"]));
    assert_eq!(flagged(&speller, "nl-BE", &[], "hallo allee"), ["hallo"]);
    assert_eq!(flagged(&speller, "nl-NL", &[], "hallo allee"), ["allee"]);
    assert_eq!(flagged(&speller, "en-GB", &[], "blorp colour"), ["colour"]);
    assert_eq!(flagged(&speller, "en-AU", &[], "blorp colour"), ["blorp"]);
    // Adding again for the same tag replaces.
    speller.add_dictionary("nl-be", tiny(&["hallo"]));
    assert_eq!(flagged(&speller, "nl-BE", &[], "hallo allee"), ["allee"]);
}

#[test]
fn a_broken_dictionary_is_refused() {
    let error = Dictionary::parse("SET UTF-8\n", "not a count\nhallo\n").unwrap_err();
    assert!(!error.to_string().is_empty());
}

#[test]
fn a_speller_can_be_shared_with_another_thread() {
    fn shared<T: Send + Sync>() {}
    shared::<Speller>();
}
