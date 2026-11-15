mod common;

use common::Scratch;
use letters::writer;

#[test]
fn settings_name_their_writer() {
    let cases = [
        ("[writer]\nname = \"Ada Lovelace\"\n", Some("Ada Lovelace")),
        ("[trust]\nfolders = []\n\n[writer]\nname = \"  Ada  \"\n", Some("Ada")),
        ("[writer]\nname = \"\"\n", None),
        ("[writer]\nname = 7\n", None),
        ("name = \"Ada\"\n", None),
        ("[trust]\nfolders = []\n", None),
        ("not toml [", None),
        ("", None),
    ];
    for (settings, name) in cases {
        assert_eq!(writer::named(settings).as_deref(), name, "{settings:?}");
    }
}

#[test]
fn an_account_gives_its_full_name() {
    let cases = [
        ("ada:x:1000:1000:Ada Lovelace,,,:/home/ada:/bin/bash\n", Some("Ada Lovelace")),
        ("ada:x:1000:1000:Ada Lovelace:/home/ada:/bin/bash", Some("Ada Lovelace")),
        ("ada:x:1000:1000:,,,:/home/ada:/bin/bash", None),
        ("ada:x:1000:1000::/home/ada:/bin/bash", None),
        ("ada", None),
        ("", None),
    ];
    for (account, name) in cases {
        assert_eq!(writer::full_name(account).as_deref(), name, "{account:?}");
    }
}

#[test]
fn the_name_in_the_settings_comes_before_the_accounts() {
    let home = Scratch::new("writer");
    std::fs::create_dir_all(home.0.join(".letters")).unwrap();
    std::fs::write(home.0.join(".letters/settings.toml"), "[writer]\nname = \"Ada Lovelace\"\n").unwrap();
    assert_eq!(writer::find(&home.0).as_deref(), Some("Ada Lovelace"));
}
