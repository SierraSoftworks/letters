use letters::writer;

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
