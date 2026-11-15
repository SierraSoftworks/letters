mod common;

use common::Scratch;
use letters::settings;
use letters_ui::{Appearance, Settings};

/// Everything a person can set, set.
fn chosen() -> Settings {
    Settings {
        appearance: Appearance::Dark,
        accent: Some([0x1f, 0x8a, 0x5b]),
        name: Some("Ada Lovelace".into()),
        language: Some("en-GB".into()),
    }
}

#[test]
fn a_settings_file_says_what_was_set() {
    let nothing = Settings::default();
    let cases = [
        ("[writer]\nname = \"Ada Lovelace\"\n", Settings { name: Some("Ada Lovelace".into()), ..nothing.clone() }),
        ("[writer]\nname = \"  Ada  \"\n", Settings { name: Some("Ada".into()), ..nothing.clone() }),
        ("[writer]\nlanguage = \"de-CH\"\n", Settings { language: Some("de-CH".into()), ..nothing.clone() }),
        ("[appearance]\ntheme = \"light\"\n", Settings { appearance: Appearance::Light, ..nothing.clone() }),
        ("[appearance]\naccent = \"#1F8A5B\"\n", Settings { accent: Some([0x1f, 0x8a, 0x5b]), ..nothing.clone() }),
        (
            "[appearance]\ntheme = \"dark\"\naccent = \"#1f8a5b\"\n\n[writer]\nname = \"Ada Lovelace\"\nlanguage = \"en-GB\"\n",
            chosen(),
        ),
        // What makes no sense is as if it had never been set.
        ("[appearance]\ntheme = \"sepia\"\naccent = \"green\"\n", nothing.clone()),
        ("[appearance]\ntheme = 7\naccent = \"#1f8a\"\n", nothing.clone()),
        ("[writer]\nname = \"\"\nlanguage = 7\n", nothing.clone()),
        ("name = \"Ada\"\ntheme = \"dark\"\n", nothing.clone()),
        ("[trust]\nfolders = []\n", nothing.clone()),
        ("not toml [", nothing.clone()),
        ("", nothing),
    ];
    for (file, expected) in cases {
        assert_eq!(settings::read(file), expected, "{file:?}");
    }
}

#[test]
fn what_is_written_reads_back_and_leaves_the_rest_of_the_file_alone() {
    let before = "# Mine.\n[trust]\nfolders = [\"~/Documents\"] # the family's\n\n[writer]\nname = \"Someone else\"\n";
    let after = settings::written(before, &chosen()).expect("a settings file");
    assert_eq!(settings::read(&after), chosen());
    for kept in ["# Mine.", "[trust]", "folders = [\"~/Documents\"] # the family's"] {
        assert!(after.contains(kept), "{kept:?} is gone from {after:?}");
    }

    // What is set no longer goes from the file, and the file reads as one nobody set anything in.
    let cleared = settings::written(&after, &Settings::default()).expect("a settings file");
    assert_eq!(settings::read(&cleared), Settings::default());
    assert!(cleared.contains("folders = [\"~/Documents\"] # the family's"), "{cleared:?}");
    assert!(!cleared.contains("Ada"), "{cleared:?}");
}

#[test]
fn a_file_that_is_no_toml_is_refused_and_not_written_over() {
    assert!(settings::written("not toml [", &chosen()).is_err());

    let home = Scratch::new("settings-broken");
    let file = home.0.join(".letters/settings.toml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "not toml [").unwrap();
    assert!(settings::keep(&home.0, &chosen()).is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "not toml [");
    assert_eq!(settings::load(&home.0), Settings::default(), "and it sets nothing");
}

#[test]
fn settings_are_kept_from_one_run_to_the_next() {
    let home = Scratch::new("settings-kept");
    assert_eq!(settings::load(&home.0), Settings::default(), "before there is a file");
    settings::keep(&home.0, &chosen()).expect("the file is written");
    assert_eq!(settings::load(&home.0), chosen());
    settings::keep(&home.0, &Settings::default()).expect("the file is written");
    assert_eq!(settings::load(&home.0), Settings::default());
}
