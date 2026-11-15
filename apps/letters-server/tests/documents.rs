//! The documents of a space: making, listing, opening and moving them, by the roles a space gives.

mod common;

use std::collections::BTreeMap;
use std::io::Cursor;

use axum::http::{Method, StatusCode};
use common::{Server, names};
use letters_api::{
    Code, Entry, EntryKind, Listing, Move, NewDocument, NewFolder, Opened, Rename, Role, Settings, Space, SpaceKind,
    Tag, Trust, paths,
};
use letters_format::{Archive, MIMETYPE, WorkingCopy};
use letters_model::{Document, ReplicaId, Style};

/// What the folder at `path` of a space lists for `who`.
async fn listed(who: &common::Client, space: &str, path: &str) -> Vec<Entry> {
    who.get::<Listing>(&paths::entries(space, path)).await.expect("the folder lists").entries
}

/// A smallest PNG, by its first bytes.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n and the rest";

#[tokio::test]
async fn a_person_reaches_their_own_space_and_those_of_their_groups() {
    let server = Server::new("reach");
    let own = ("home", Role::Owner);
    let cases = [
        ("ben", "family", vec![own, ("family", Role::Editor)]),
        ("critic", "reviewers", vec![own, ("family", Role::Commenter)]),
        ("guest", "friends", vec![own, ("family", Role::Reader)]),
        ("visitor", "strangers", vec![own]),
    ];
    for (username, group, expected) in cases {
        let spaces: Vec<Space> = server.person(username, &[group]).get(paths::SPACES).await.unwrap();
        let reached: Vec<(&str, Role)> = spaces.iter().map(|space| (space.id.as_str(), space.role)).collect();
        assert_eq!(reached, expected, "{username}");
        assert_eq!(spaces[0].kind, SpaceKind::Personal);
    }
    // A space out of reach is not there, for a listing as for anything else.
    let visitor = server.person("visitor", &[]);
    let refused = visitor.get::<Listing>(&paths::entries("family", "")).await.expect_err("out of reach");
    assert_eq!(refused.code, Code::NotFound);
}

#[tokio::test]
async fn each_person_has_a_space_of_their_own() {
    let server = Server::new("own");
    let (ben, alice) = (server.person("ben", &[]), server.person("alice", &[]));
    ben.start("home", "", "Diary").await;
    assert_eq!(names(&listed(&ben, "home", "").await), ["Diary"]);
    assert!(listed(&alice, "home", "").await.is_empty(), "one person's space is not another's");
    assert_eq!(server.scratch.names("home"), ["alice", "ben"], "each was made when its person first came");
}

#[tokio::test]
async fn a_document_is_made_listed_and_opened() {
    let server = Server::new("made");
    let ben = server.person("ben", &["family"]);
    let made = ben.start("family", "", "Roof quote").await;
    assert_eq!(
        (made.kind, made.name.as_str(), made.path.as_str()),
        (EntryKind::Document, "Roof quote", "Roof quote.let")
    );
    assert_eq!(made.contributors, ["Ben"], "its maker signed it");
    assert_eq!(made.created.as_deref(), Some("2027-01-15T08:00:00Z"));
    let id = made.id.clone().expect("a document has an id");

    assert_eq!(listed(&ben, "family", "").await, [made]);
    let opened: Opened = ben.get(&paths::document(&id)).await.unwrap();
    assert_eq!((opened.id.as_str(), opened.name.as_str(), opened.role), (id.as_str(), "Roof quote", Role::Editor));
    assert_eq!((opened.author.as_str(), opened.trust), ("Ben", Trust::Trusted));
    assert_eq!(opened.socket, paths::socket(&id));
    assert!(opened.style.is_none() && opened.notes.is_none(), "the app reads those from the file");

    let file = ben.send(Method::GET, &paths::file(&id), "", Vec::new()).await;
    assert_eq!((file.status, file.header("content-type")), (StatusCode::OK, MIMETYPE));
    assert!(file.header("content-disposition").contains("Roof%20quote.let"));
    let mut archive = Archive::open(Cursor::new(file.body)).expect("the file is a document");
    assert_eq!(archive.manifest().id, id);
    assert_eq!(archive.document().unwrap().blocks.len(), 1, "a new document is one empty line");
}

#[tokio::test]
async fn a_document_without_a_name_takes_the_first_free_one() {
    let server = Server::new("untitled");
    let ben = server.person("ben", &[]);
    for expected in ["Untitled", "Untitled 2", "Untitled 3"] {
        let made: Entry = ben.post(&paths::documents("home"), &NewDocument::default()).await.unwrap();
        assert_eq!(made.name, expected);
    }
    let taken = NewDocument { folder: String::new(), name: Some("Untitled 2".into()) };
    let refused = ben.post::<Entry>(&paths::documents("home"), &taken).await.expect_err("the name is taken");
    assert_eq!(refused.code, Code::NameTaken);
    let unusable = NewDocument { folder: String::new(), name: Some(" ... ".into()) };
    assert_eq!(ben.post::<Entry>(&paths::documents("home"), &unusable).await.expect_err("no name").code, Code::Invalid);
    assert_eq!(server.scratch.names("home/ben").len(), 3, "nothing half-made is left beside them");
}

#[tokio::test]
async fn a_folder_lists_its_folders_first_and_only_what_is_a_document() {
    let server = Server::new("listing");
    let ben = server.person("ben", &[]);
    ben.start("home", "", "beta").await;
    ben.start("home", "", "Alpha").await;
    let folder: Entry =
        ben.post(&paths::folders("home"), &NewFolder { parent: String::new(), name: "Zebra".into() }).await.unwrap();
    assert_eq!((folder.kind, folder.id.as_ref()), (EntryKind::Folder, None));
    ben.start("home", "Zebra", "Inside").await;
    let home = server.scratch.0.join("home/ben");
    std::fs::write(home.join(".hidden.let"), b"").unwrap();
    std::fs::write(home.join("notes.txt"), b"not a document").unwrap();
    std::fs::write(home.join("Broken.let"), b"not a zip").unwrap();

    let rows = listed(&ben, "home", "").await;
    assert_eq!(names(&rows), ["Zebra", "Alpha", "beta", "Broken"]);
    let broken = rows.last().unwrap();
    assert_eq!((broken.id.as_ref(), broken.tags.as_slice()), (None, [Tag::Damaged].as_slice()));
    assert_eq!(names(&listed(&ben, "home", "Zebra").await), ["Inside"]);
    assert_eq!(
        ben.get::<Listing>(&paths::entries("home", "Nowhere")).await.expect_err("no such folder").code,
        Code::NotFound
    );
}

#[tokio::test]
async fn a_document_is_found_by_its_id_wherever_its_file_goes() {
    let server = Server::new("moved");
    let ben = server.person("ben", &["family"]);
    let id = ben.start("family", "", "Plan").await.id.unwrap();
    let family = server.scratch.0.join("family");

    // Someone renames and moves the file over the share, behind the server's back.
    std::fs::create_dir(family.join("Archive")).unwrap();
    std::fs::rename(family.join("Plan.let"), family.join("Archive/Old plan.let")).unwrap();
    let opened: Opened = ben.get(&paths::document(&id)).await.expect("it is found again by its id");
    assert_eq!((opened.name.as_str(), opened.path.as_str()), ("Old plan", "Archive/Old plan.let"));

    // Moved again at once, it is lost until the folders are next looked through.
    std::fs::rename(family.join("Archive/Old plan.let"), family.join("Plan again.let")).unwrap();
    assert_eq!(ben.get::<Opened>(&paths::document(&id)).await.expect_err("not yet").code, Code::NotFound);
    server.wait(60);
    assert_eq!(ben.get::<Opened>(&paths::document(&id)).await.unwrap().name, "Plan again");
}

#[tokio::test]
async fn a_document_is_renamed_moved_and_thrown_away() {
    let server = Server::new("relocate");
    let ben = server.person("ben", &["family"]);
    let id = ben.start("family", "", "Draft").await.id.unwrap();
    ben.start("family", "", "Final").await;

    let renamed: Entry = ben.post(&paths::rename(&id), &Rename { name: "Quote: roof?".into() }).await.unwrap();
    assert_eq!(
        (renamed.name.as_str(), renamed.id.as_deref()),
        ("Quote roof", Some(id.as_str())),
        "a name is made safe"
    );
    let taken = ben.post::<Entry>(&paths::rename(&id), &Rename { name: "Final".into() }).await.expect_err("taken");
    assert_eq!(taken.code, Code::NameTaken);
    assert_eq!(names(&listed(&ben, "family", "").await), ["Final", "Quote roof"], "nothing was replaced");

    ben.post::<Entry>(&paths::folders("home"), &NewFolder { parent: String::new(), name: "House".into() })
        .await
        .unwrap();
    let moved: Entry =
        ben.post(&paths::relocate(&id), &Move { space: "home".into(), folder: "House".into() }).await.unwrap();
    assert_eq!(
        (moved.space.as_deref(), moved.path.as_str(), moved.role),
        (Some("home"), "House/Quote roof.let", Role::Owner)
    );
    assert_eq!(ben.get::<Opened>(&paths::document(&id)).await.unwrap().space.as_deref(), Some("home"));
    let nowhere = Move { space: "home".into(), folder: "Nowhere".into() };
    assert_eq!(ben.post::<Entry>(&paths::relocate(&id), &nowhere).await.expect_err("no folder").code, Code::NotFound);

    ben.post::<()>(&paths::trash(&id), &()).await.expect("thrown away");
    assert!(listed(&ben, "home", "House").await.is_empty());
    assert_eq!(ben.get::<Opened>(&paths::document(&id)).await.expect_err("in the trash").code, Code::NotFound);
    assert_eq!(server.scratch.names("home/ben/.letters-trash"), ["Quote roof.let"], "the file is kept, out of sight");
}

#[tokio::test]
async fn a_reader_reads_and_changes_nothing() {
    let server = Server::new("reader");
    let (ben, guest, visitor) =
        (server.person("ben", &["family"]), server.person("guest", &["friends"]), server.person("visitor", &[]));
    let id = ben.start("family", "", "Plan").await.id.unwrap();

    assert_eq!(names(&listed(&guest, "family", "").await), ["Plan"]);
    assert_eq!(guest.get::<Opened>(&paths::document(&id)).await.unwrap().role, Role::Reader);
    assert_eq!(guest.send(Method::GET, &paths::file(&id), "", Vec::new()).await.status, StatusCode::OK);
    let refusals = [
        guest.post::<Entry>(&paths::documents("family"), &NewDocument::default()).await.map(drop),
        guest
            .post::<Entry>(&paths::folders("family"), &NewFolder { parent: String::new(), name: "Mine".into() })
            .await
            .map(drop),
        guest.post::<Entry>(&paths::rename(&id), &Rename { name: "Mine".into() }).await.map(drop),
        guest
            .post::<Entry>(&paths::relocate(&id), &Move { space: "home".into(), folder: String::new() })
            .await
            .map(drop),
        guest.post::<()>(&paths::trash(&id), &()).await,
    ];
    for (n, refusal) in refusals.into_iter().enumerate() {
        assert_eq!(refusal.expect_err("a reader changes nothing").code, Code::Forbidden, "change {n}");
    }
    assert_eq!(server.scratch.names("family"), ["Plan.let"]);

    // Someone the space does not let in learns nothing of the document, not even that it is there.
    for path in [paths::document(&id), paths::file(&id)] {
        assert_eq!(visitor.get::<serde_json::Value>(&path).await.expect_err("out of reach").code, Code::NotFound);
    }
    let pushed = Move { space: "family".into(), folder: String::new() };
    let own = visitor.start("home", "", "Mine").await.id.unwrap();
    assert_eq!(
        visitor.post::<Entry>(&paths::relocate(&own), &pushed).await.expect_err("out of reach").code,
        Code::NotFound
    );
}

#[tokio::test]
async fn a_conflict_copy_is_marked_and_its_original_is_the_document() {
    let server = Server::new("conflict");
    let ben = server.person("ben", &[]);
    let id = ben.start("home", "", "Plan").await.id.unwrap();
    let home = server.scratch.0.join("home/ben");
    std::fs::copy(home.join("Plan.let"), home.join("Plan (conflicted copy 2026-11-15).let")).unwrap();
    // A file that only looks like a copy by its name is another document.
    ben.start("home", "", "Other").await;
    std::fs::rename(home.join("Other.let"), home.join("Plan (Ben's conflicted copy).let")).unwrap();

    let rows = listed(&ben, "home", "").await;
    let tags: Vec<(&str, &[Tag])> = rows.iter().map(|row| (row.name.as_str(), row.tags.as_slice())).collect();
    let copy = [Tag::ConflictCopy { of: "Plan.let".into() }];
    let expected: [(&str, &[Tag]); 3] =
        [("Plan", &[]), ("Plan (Ben's conflicted copy)", &[]), ("Plan (conflicted copy 2026-11-15)", &copy)];
    assert_eq!(tags, expected);
    assert_eq!(ben.get::<Opened>(&paths::document(&id)).await.unwrap().path, "Plan.let");
}

#[tokio::test]
async fn what_a_person_opened_lately_is_listed_newest_first() {
    let server = Server::new("recent");
    let ben = server.person("ben", &["family"]);
    let mut ids = Vec::new();
    for (space, name) in [("home", "First"), ("family", "Second"), ("home", "Third")] {
        ids.push(ben.start(space, "", name).await.id.unwrap());
    }
    for id in [&ids[0], &ids[1], &ids[2], &ids[0]] {
        ben.get::<Opened>(&paths::document(id)).await.unwrap();
    }
    assert_eq!(names(&ben.get::<Vec<Entry>>(paths::RECENT).await.unwrap()), ["First", "Third", "Second"]);
    ben.post::<()>(&paths::trash(&ids[2]), &()).await.unwrap();
    assert_eq!(names(&ben.get::<Vec<Entry>>(paths::RECENT).await.unwrap()), ["First", "Second"]);
    assert!(server.person("alice", &["family"]).get::<Vec<Entry>>(paths::RECENT).await.unwrap().is_empty());
}

#[tokio::test]
async fn what_a_person_set_is_kept_and_shapes_their_new_documents() {
    let server = Server::new("settings");
    let ben = server.person("ben", &[]);
    assert_eq!(ben.get::<Settings>(paths::SETTINGS).await.unwrap(), Settings::default());

    let mut style = Style::default();
    style.types.get_mut("body").expect("the built-in style sets body text").size = 15.0;
    let settings = Settings {
        name: Some("Benjamin".into()),
        accent: Some([10, 120, 60]),
        dictionary: vec!["yewi".into()],
        default_style: Some(serde_json::from_str(&style.to_json()).unwrap()),
        ..Settings::default()
    };
    assert_eq!(ben.put::<Settings>(paths::SETTINGS, &settings).await.unwrap(), settings);
    assert_eq!(ben.get::<Settings>(paths::SETTINGS).await.unwrap(), settings);
    assert_eq!(server.person("alice", &[]).get::<Settings>(paths::SETTINGS).await.unwrap(), Settings::default());

    let made = ben.start("home", "", "Styled").await;
    assert_eq!(made.contributors, ["Benjamin"], "edits are signed with the name the person set");
    let file = ben.send(Method::GET, &paths::file(&made.id.unwrap()), "", Vec::new()).await;
    assert_eq!(Archive::open(Cursor::new(file.body)).unwrap().style().unwrap(), style);

    let broken = Settings { default_style: Some(serde_json::json!({"types": 7})), ..Settings::default() };
    assert_eq!(ben.put::<Settings>(paths::SETTINGS, &broken).await.expect_err("not a style").code, Code::Invalid);
}

#[tokio::test]
async fn a_documents_picture_is_served_as_what_it_is() {
    let server = Server::new("asset");
    let ben = server.person("ben", &[]);
    ben.get::<Vec<Space>>(paths::SPACES).await.unwrap();
    ben.start("home", "", "Seed").await;
    // A document with a picture, as an editor would have saved it.
    let mut copy = WorkingCopy::new(
        "with-picture",
        "2026-11-15T10:00:00Z",
        Document::default(),
        Style::default(),
        ReplicaId::from(1),
    );
    copy.entries = BTreeMap::from([
        ("assets/ab12.png".to_owned(), PNG.to_vec()),
        ("assets/cd34.bin".to_owned(), b"<script>".to_vec()),
    ]);
    let mut bytes = Cursor::new(Vec::new());
    letters_format::write(&mut bytes, &letters_format::Replica::package(&copy)).unwrap();
    std::fs::write(server.scratch.0.join("home/ben/Pictured.let"), bytes.into_inner()).unwrap();
    names(&listed(&ben, "home", "").await);

    let held: Vec<String> = ben.get(&paths::assets("with-picture")).await.unwrap();
    assert_eq!(held, ["ab12.png", "cd34.bin"], "a device learns which pictures the server has");
    let picture = ben.send(Method::GET, &paths::asset("with-picture", "ab12.png"), "", Vec::new()).await;
    assert_eq!(
        (picture.status, picture.header("content-type"), picture.body.as_slice()),
        (StatusCode::OK, "image/png", PNG)
    );
    let other = ben.send(Method::GET, &paths::asset("with-picture", "cd34.bin"), "", Vec::new()).await;
    assert_eq!(other.header("content-type"), "application/octet-stream", "what is no picture is never rendered");
    assert_eq!(other.header("x-content-type-options"), "nosniff");
    for name in ["zz99.png", "../document.json", "..%2Fdocument.json", "ab12.png/.."] {
        let missing = ben.send(Method::GET, &paths::asset("with-picture", name), "", Vec::new()).await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND, "{name}");
    }
}
