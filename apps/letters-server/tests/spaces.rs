//! Confinement: nothing a request names, and nothing a folder holds, leads out of a space.

mod common;

use common::{Server, names};
use letters_api::{Code, Entry, Listing, NewDocument, NewFolder, Opened, Space, paths};

/// A folder outside every space, with a document and a folder in it.
fn outside(server: &Server) -> std::path::PathBuf {
    let outside = server.scratch.0.join("outside");
    std::fs::create_dir_all(outside.join("Private")).unwrap();
    let home = server.scratch.0.join("home/ben");
    std::fs::copy(home.join("Seed.let"), outside.join("Secret.let")).unwrap();
    std::fs::copy(home.join("Seed.let"), outside.join("Private/Diary.let")).unwrap();
    outside
}

#[tokio::test]
async fn a_place_is_plain_visible_names() {
    let server = Server::new("places");
    let ben = server.person("ben", &[]);
    ben.start("home", "", "Seed").await;
    outside(&server);

    let refused = [
        "..",
        "../alice",
        "../../outside",
        "Seed.let/../../../outside",
        "./.",
        ".letters-trash",
        "a/.hidden/b",
        "a\\..\\b",
        "a//b",
        "tab\there",
    ];
    for path in refused {
        let listing = ben.get::<Listing>(&paths::entries("home", path)).await;
        assert_eq!(listing.expect_err(path).code, Code::Invalid, "{path:?}");
        let made = ben.post::<Entry>(&paths::documents("home"), &NewDocument { folder: path.into(), name: None }).await;
        assert_eq!(made.expect_err(path).code, Code::Invalid, "{path:?}");
    }
    let absolute = ben.get::<Listing>(&paths::entries("home", "/")).await.expect("a leading slash is the space itself");
    assert_eq!(names(&absolute.entries), ["Seed"]);
    for name in ["..", "", "/", " . "] {
        let folder = NewFolder { parent: String::new(), name: name.into() };
        let made = ben.post::<Entry>(&paths::folders("home"), &folder).await;
        assert_eq!(made.expect_err(name).code, Code::Invalid, "{name:?}");
    }
    // A name is cleaned as a heading's is, so what is made is one visible name in the folder asked for.
    for (name, cleaned) in [(".git", "git"), ("a/b", "a b"), ("../up", "up")] {
        let folder = NewFolder { parent: String::new(), name: name.into() };
        let made = ben.post::<Entry>(&paths::folders("home"), &folder).await.expect(name);
        assert_eq!((made.name.as_str(), made.path.as_str()), (cleaned, cleaned), "{name:?}");
    }
    assert_eq!(server.scratch.names("outside"), ["Private", "Secret.let"], "nothing was made outside");
    assert_eq!(server.scratch.names("home"), ["ben"]);
    assert_eq!(server.scratch.names("home/ben"), ["Seed.let", "a b", "git", "up"]);
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_leads_nowhere() {
    use std::os::unix::fs::symlink;

    let server = Server::new("symlinks");
    let ben = server.person("ben", &[]);
    let seed = ben.start("home", "", "Seed").await.id.unwrap();
    let outside = outside(&server);
    let home = server.scratch.0.join("home/ben");
    symlink(&outside, home.join("Out")).unwrap();
    symlink(outside.join("Secret.let"), home.join("Leak.let")).unwrap();
    symlink("../../outside/Private", home.join("Relative")).unwrap();

    assert_eq!(names(&ben.get::<Listing>(&paths::entries("home", "")).await.unwrap().entries), ["Seed"]);
    assert_eq!(names(&ben.get::<Listing>(&paths::everything("home")).await.unwrap().entries), ["Seed"]);
    for folder in ["Out", "Out/Private", "Relative"] {
        let listing = ben.get::<Listing>(&paths::entries("home", folder)).await;
        assert!(listing.is_err(), "{folder} lists what is outside the space");
        let made =
            ben.post::<Entry>(&paths::documents("home"), &NewDocument { folder: folder.into(), name: None }).await;
        assert!(made.is_err(), "{folder} takes a document outside the space");
    }
    assert_eq!(server.scratch.names("outside/Private"), ["Diary.let"]);
    // The copies outside hold the seed's id, and it is still the file in the space that answers to it.
    server.app.scan();
    assert_eq!(ben.get::<Opened>(&paths::document(&seed)).await.unwrap().path, "Seed.let");
}

#[tokio::test]
async fn a_name_that_is_no_folders_name_gets_no_space() {
    let server = Server::new("usernames");
    for username in ["../ben", "a/b", ".hidden", "", "..", "ben\\x"] {
        let mallory = server.person(username, &[]);
        let spaces: Vec<Space> = mallory.get(paths::SPACES).await.unwrap();
        assert_eq!(names_of(&spaces), ["Home"], "the space is theirs in name");
        let listing = mallory.get::<Listing>(&paths::entries("home", "")).await;
        assert_eq!(listing.expect_err(username).code, Code::Forbidden, "{username:?}");
        let made = mallory.post::<Entry>(&paths::documents("home"), &NewDocument::default()).await;
        assert_eq!(made.expect_err(username).code, Code::Forbidden, "{username:?}");
    }
    assert!(server.scratch.names("home").is_empty(), "no folder was made for them");
    assert_eq!(server.scratch.names(""), ["family"], "nor one beside the spaces");
}

fn names_of(spaces: &[Space]) -> Vec<&str> {
    spaces.iter().map(|space| space.name.as_str()).collect()
}

#[tokio::test]
async fn a_space_is_listed_whole_for_whoever_reaches_it() {
    let server = Server::new("whole");
    let ben = server.person("ben", &[]);
    for (parent, name) in [("", "House"), ("House", "Roof"), ("", "Empty")] {
        let folder = NewFolder { parent: parent.into(), name: name.into() };
        ben.post::<Entry>(&paths::folders("home"), &folder).await.unwrap();
    }
    for (folder, name) in [("", "Seed"), ("House", "Garden"), ("House/Roof", "Quote")] {
        ben.start("home", folder, name).await;
    }

    // Every folder and document, each where it is, and each document as its own folder lists it.
    let whole = ben.get::<Listing>(&paths::everything("home")).await.unwrap().entries;
    let mut paths_of: Vec<&str> = whole.iter().map(|entry| entry.path.as_str()).collect();
    paths_of.sort();
    assert_eq!(paths_of, ["Empty", "House", "House/Garden.let", "House/Roof", "House/Roof/Quote.let", "Seed.let"]);
    let roof = ben.get::<Listing>(&paths::entries("home", "House/Roof")).await.unwrap().entries;
    assert!(whole.contains(&roof[0]), "{:?}", roof[0]);
    // A plain listing is the folder alone, as before.
    assert_eq!(
        names(&ben.get::<Listing>(&paths::entries("home", "")).await.unwrap().entries),
        ["Empty", "House", "Seed"]
    );

    let mallory = server.person("mallory", &[]);
    let theirs = mallory.get::<Listing>(&paths::everything("home")).await.unwrap().entries;
    assert_eq!(names(&theirs), [] as [&str; 0], "another person's own space holds none of it");
}
