//! Which dictionary serves a language, told from the names of the files alone.

use letters_spell::{best_dictionary, dictionary_language};

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
