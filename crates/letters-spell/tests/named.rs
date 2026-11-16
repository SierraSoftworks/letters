//! Which dictionary serves a language, told from the names of the files alone.

use std::collections::BTreeMap;

use letters_spell::{best_dictionary, dictionary_language, served_dictionaries};

#[test]
fn a_name_says_the_language_it_is_for() {
    let cases = [
        ("de", Some("de")),
        ("de_DE", Some("de-DE")),
        ("pt-br", Some("pt-BR")),
        // A variant and a script say more than a language and a region, and are for the same.
        ("de_DE_frami", Some("de-DE")),
        ("sr-Latn-RS", Some("sr-RS")),
        // What is no language's name.
        ("hyph_de_DE", None),
        ("README", None),
        ("", None),
    ];
    for (name, language) in cases {
        assert_eq!(dictionary_language(name).as_deref(), language, "{name:?}");
    }
}

#[test]
fn the_dictionary_nearest_to_a_language_is_chosen() {
    let cases: [(&[&str], &str, Option<&str>); 11] = [
        // Its region's own.
        (&["de_AT", "de_CH", "de_DE"], "de-CH", Some("de_CH")),
        // Else the language's alone.
        (&["de_AT", "de", "de_DE"], "de-CH", Some("de")),
        // Else that of the country the language is named after.
        (&["de_AT", "de_DE"], "de-CH", Some("de_DE")),
        (&["de_AT", "de_DE"], "de", Some("de_DE")),
        // Else any.
        (&["pt_BR"], "pt-PT", Some("pt_BR")),
        // A plain name before a variant, and of two that serve alike the earlier.
        (&["de_DE_frami", "de_DE"], "de-DE", Some("de_DE")),
        (&["pt_BR", "pt_AO"], "pt", Some("pt_BR")),
        // A tag is read however it is written.
        (&["pt_BR", "pt_PT"], "PT_br", Some("pt_BR")),
        // None for another language, and none where no language is asked for.
        (&["de_DE"], "fr", None),
        (&["de_DE"], "", None),
        (&[], "de", None),
    ];
    for (names, language, best) in cases {
        let chosen = best_dictionary(names.iter().copied(), language).map(|place| names[place]);
        assert_eq!(chosen, best, "{language:?} among {names:?}");
    }
}

#[test]
fn a_servers_dictionary_is_taken_under_the_tag_the_speller_looks_for() {
    /// The languages a document names, what the server has, and each dictionary to ask for with the tag
    /// it is added under.
    type Case = (&'static [&'static str], &'static [&'static str], &'static [(&'static str, &'static str)]);
    let cases: [Case; 5] = [
        // A region the server lacks falls back to the language's dictionary, asked for once for both.
        (&["de", "de-CH"], &["de-DE"], &[("de-DE", "de")]),
        (&["de-CH"], &["de-DE", "de-CH"], &[("de-CH", "de-CH")]),
        (&["de", "de-CH"], &["de-DE", "de-CH"], &[("de-CH", "de-CH"), ("de-DE", "de")]),
        (&["pt-BR"], &["pt-PT", "pt-BR"], &[("pt-BR", "pt-BR")]),
        (&["fr"], &["de-DE"], &[]),
    ];
    for (named, held, expected) in cases {
        let held: Vec<String> = held.iter().map(|tag| (*tag).to_owned()).collect();
        let expected: BTreeMap<String, String> =
            expected.iter().map(|(tag, under)| ((*tag).to_owned(), (*under).to_owned())).collect();
        assert_eq!(served_dictionaries(named.iter().copied(), &held), expected, "{named:?} of {held:?}");
    }
}
