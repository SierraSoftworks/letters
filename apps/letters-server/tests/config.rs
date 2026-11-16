//! The settings file: what it takes, and what it says of a file it cannot run with.

mod common;

use letters_api::Role;
use letters_server::config::Config;

fn with(change: impl Fn(String) -> String) -> Result<Config, String> {
    let sound = common::settings(std::path::Path::new("/srv/letters"), "https://id.example.test", "");
    Config::parse(&change(sound)).map_err(|error| error.to_string())
}

#[test]
fn sound_settings_are_read_with_their_defaults() {
    let config = with(|sound| sound).expect("the settings are sound");
    assert_eq!(config.server.url, common::URL);
    assert_eq!(config.auth.oidc.groups_claim, "groups");
    assert!(config.auth.oidc.scopes.iter().any(|scope| scope == "openid"));
    assert!(!config.links.enabled, "links are off until the owner turns them on");
    assert!(config.telemetry.enabled, "usage is reported until the owner says no");
    let quiet = with(|sound| format!("{sound}\n[telemetry]\nenabled = false\n")).expect("the settings are sound");
    assert!(!quiet.telemetry.enabled);
    assert!(config.server.proxies.iter().all(|proxy| proxy.is_loopback()), "only this machine speaks for others");
    let ids: Vec<String> = config.spaces.iter().map(|space| space.id()).collect();
    assert_eq!(ids, ["home", "family"]);
    assert!(config.spaces[0].personal() && !config.spaces[1].personal());
}

#[test]
fn the_clients_secret_is_read_from_the_settings_or_from_a_file() {
    let scratch = common::Scratch::new("secret");
    let file = scratch.0.join("secret");
    std::fs::write(&file, "from a file\n").unwrap();
    let cases = [
        (String::new(), Ok(None)),
        ("client_secret = \"inline\"".to_owned(), Ok(Some("inline"))),
        (format!("client_secret_file = '{}'", file.display()), Ok(Some("from a file"))),
        ("client_secret_file = \"/nowhere/secret\"".to_owned(), Err("client_secret_file")),
        ("client_secret = \"\"".to_owned(), Err("nothing in it")),
        (format!("client_secret = \"a\"\nclient_secret_file = '{}'", file.display()), Err("both")),
    ];
    for (setting, expected) in cases {
        let line = "client_id = \"letters\"";
        let read = with(|sound| sound.replace(line, &format!("{line}\n{setting}"))).and_then(|mut config| {
            config.read_secrets().map_err(|error| error.to_string())?;
            Ok(config.auth.oidc.client_secret)
        });
        match (read, expected) {
            (Ok(secret), Ok(expected)) => assert_eq!(secret.as_deref(), expected, "{setting}"),
            (Err(refusal), Err(word)) => assert!(refusal.contains(word), "{setting}: {refusal}"),
            (read, _) => panic!("{setting}: {read:?}"),
        }
    }
}

#[test]
fn settings_that_cannot_be_run_with_say_why() {
    // What is changed in sound settings, and a word the refusal must hold.
    let cases: [(&str, &str, &str); 14] = [
        ("name = \"Test\"", "name = \"Test\"\n[telemetry]\nserver = \"https://elsewhere.example\"", "server"),
        ("editors = [\"family\"]", "editor = [\"family\"]", "editor"),
        ("url = \"https://letters.example.test\"", "url = \"http://letters.example.test\"", "TLS"),
        ("url = \"https://letters.example.test\"", "url = \"https://letters.example.test/app\"", "host"),
        ("issuer = \"https://id.example.test\"", "issuer = \"ftp://id.example.test\"", "https://"),
        ("client_id = \"letters\"", "client_id = \"\"", "client_id"),
        ("client_id = \"letters\"", "client_id = \"letters\"\nscopes = [\"profile\"]", "openid"),
        ("name = \"Test\"", "name = \"Test\"\nproxies = [\"the proxy\"]", "proxies"),
        ("name = \"Test\"", "name = \"Test\"\nlog = \"everything\"", "debug"),
        ("name = \"Family\"", "name = \"home\"", "two spaces"),
        ("name = \"Family\"", "name = \"!!\"", "letter"),
        ("path = '/srv/letters/family'", "path = \"family\"", "root"),
        ("path = '/srv/letters/family'", "path = \"/srv/{group}/family\"", "braces"),
        ("editors = [\"family\"]\ncommenters = [\"reviewers\"]\nreaders = [\"friends\"]", "", "nobody"),
    ];
    for (from, to, word) in cases {
        let refusal = with(|sound| {
            assert!(sound.contains(from), "the sound settings hold {from:?}");
            sound.replace(from, to)
        })
        .expect_err(to);
        assert!(refusal.contains(word), "{to:?} was refused with {refusal:?}, which does not say {word:?}");
    }
    let personal = with(|sound| {
        sound.replace(
            "path = '/srv/letters/home/{username}'",
            "path = \"/srv/letters/home/{username}\"\nreaders = [\"x\"]",
        )
    });
    assert!(personal.expect_err("a person's own space takes no groups").contains("own"));
    let none = with(|sound| sound[..sound.find("[[space]]").unwrap()].to_owned());
    assert!(none.expect_err("no space").contains("[[space]]"));
}

#[test]
fn a_server_on_this_machine_may_go_without_tls() {
    let config = with(|sound| {
        sound
            .replace("https://letters.example.test", "http://localhost:8080/")
            .replace("https://id.example.test", "http://127.0.0.1:9000/realms/home/")
    });
    let config = config.expect("loopback addresses are taken without TLS");
    assert_eq!(config.server.url, "http://localhost:8080");
    assert_eq!(config.auth.oidc.issuer, "http://127.0.0.1:9000/realms/home");
}

#[test]
fn a_space_gives_each_group_its_role() {
    let config = with(|sound| sound).unwrap();
    let (home, family) = (&config.spaces[0], &config.spaces[1]);
    let groups = |names: &[&str]| names.iter().map(|name| name.to_string()).collect::<Vec<_>>();
    let cases: [(&[&str], Option<Role>); 5] = [
        (&[], None),
        (&["strangers"], None),
        (&["friends"], Some(Role::Reader)),
        (&["friends", "reviewers"], Some(Role::Commenter)),
        (&["reviewers", "family", "friends"], Some(Role::Editor)),
    ];
    for (names, role) in cases {
        assert_eq!(family.role(&groups(names)), role, "{names:?}");
    }
    assert_eq!(home.role(&[]), Some(Role::Owner), "a person's own space is theirs whatever their groups");
    assert_eq!(home.folder("ben"), "/srv/letters/home/ben");
}
