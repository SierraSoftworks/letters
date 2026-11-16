//! The documents of a space: making, listing, opening and moving them, by the roles a space gives.

mod common;

use std::collections::BTreeMap;
use std::io::Cursor;

use axum::http::{Method, StatusCode};
use common::{Server, names};
use letters_api::{
    Code, Entry, EntryKind, Error, Listing, Move, NewDocument, NewFolder, Opened, Rename, Role, Settings, Space,
    SpaceKind, Tag, Trashed, Trust, paths,
};
use letters_format::{Archive, MIMETYPE, OPLOG, WorkingCopy};
use letters_model::{Block, BlockId, BlockKind, Document, Model, Notes, ReplicaId, Style, TagKind};
use letters_server::store::Table;

/// What the folder at `path` of a space lists for `who`.
async fn listed(who: &common::Client, space: &str, path: &str) -> Vec<Entry> {
    who.get::<Listing>(&paths::entries(space, path)).await.expect("the folder lists").entries
}

/// The words as a dictionary answers them.
fn owned(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

/// Adds a word to a person's dictionary or takes it out, and answers the words it holds then.
async fn word(who: &common::Client, method: Method, word: &str) -> Vec<String> {
    who.send(method, &paths::word(word), "", Vec::new()).await.json()
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
    let mut last = String::new();
    for expected in ["Untitled", "Untitled 2", "Untitled 3"] {
        let made: Entry = ben.post(&paths::documents("home"), &NewDocument::default()).await.unwrap();
        assert_eq!(made.name, expected);
        last = made.id.expect("a document has an id");
        let opened: Opened = ben.get(&paths::document(&last)).await.unwrap();
        assert!(!opened.named, "{expected} is not a name of the document's own");
    }
    // Its first heading names it, which the editor asks for as a rename.
    let _: Entry = ben.post(&paths::rename(&last), &Rename { name: "Roof repairs".into() }).await.unwrap();
    let opened: Opened = ben.get(&paths::document(&last)).await.unwrap();
    assert!(opened.named && opened.name == "Roof repairs", "{opened:?}");
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

/// Makes the folders at `paths` in the family's space, each after the one it is in.
async fn folders(who: &common::Client, paths: &[&str]) {
    for path in paths {
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        let new = NewFolder { parent: parent.into(), name: name.into() };
        who.post::<Entry>(&paths::folders("family"), &new).await.expect("the folder is made");
    }
}

#[tokio::test]
async fn a_folder_is_renamed_moved_and_thrown_away_with_what_it_holds() {
    let server = Server::new("folders");
    let ben = server.person("ben", &["family"]);
    folders(&ben, &["House", "House/Roof"]).await;
    let id = ben.start("family", "House/Roof", "Quote").await.id.unwrap();
    // The folders were just looked through for a document that is not there, so none is looked for again
    // for a while: where this one is must be known as its folder moves.
    assert_eq!(ben.get::<Opened>(&paths::document("no-such")).await.expect_err("nothing").code, Code::NotFound);
    let at = async || {
        let opened = ben.get::<Opened>(&paths::document(&id)).await.expect("the document is found");
        (opened.space.unwrap_or_default(), opened.path)
    };

    let renamed: Entry =
        ben.post(&paths::rename_folder("family", "House"), &Rename { name: "Home: ours".into() }).await.unwrap();
    assert_eq!(
        (renamed.kind, renamed.name.as_str(), renamed.path.as_str()),
        (EntryKind::Folder, "Home ours", "Home ours")
    );
    assert_eq!(at().await, ("family".to_owned(), "Home ours/Roof/Quote.let".to_owned()));

    let home = Move { space: "home".into(), folder: String::new() };
    let moved: Entry = ben.post(&paths::move_folder("family", "Home ours/Roof"), &home).await.unwrap();
    assert_eq!((moved.space.as_deref(), moved.path.as_str(), moved.role), (Some("home"), "Roof", Role::Owner));
    assert_eq!(at().await, ("home".to_owned(), "Roof/Quote.let".to_owned()));
    assert_eq!(names(&listed(&ben, "family", "Home ours").await), [] as [&str; 0], "nothing stays behind");

    ben.post::<()>(&paths::trash_folder("home", "Roof"), &()).await.expect("thrown away");
    assert!(listed(&ben, "home", "").await.is_empty());
    assert_eq!(ben.get::<Opened>(&paths::document(&id)).await.expect_err("in the trash").code, Code::NotFound);
    assert_eq!(server.scratch.names("home/ben/.letters-trash/Roof"), ["Quote.let"], "kept whole, out of sight");
}

#[tokio::test]
async fn what_is_thrown_away_is_listed_and_put_back_by_whoever_edits_its_space() {
    let server = Server::new("trash-listed");
    let (ben, guest) = (server.person("ben", &["family"]), server.person("guest", &["friends"]));
    folders(&ben, &["House", "House/Roof"]).await;
    let quote = ben.start("family", "House", "Quote").await.id.unwrap();
    ben.start("family", "House/Roof", "Tiles").await;
    let diary = ben.start("home", "", "Diary").await.id.unwrap();
    // A document goes, then the folder beside it with what that holds, then a document of Ben's own.
    for thrown in [paths::trash(&quote), paths::trash_folder("family", "House/Roof"), paths::trash(&diary)] {
        ben.post::<()>(&thrown, &()).await.expect("thrown away");
        server.wait(10);
    }

    // The trash tells what each was and where, the last thrown away first.
    let told = |trash: &[Trashed]| -> Vec<(String, EntryKind, String, String, Role)> {
        let row = |thrown: &Trashed| {
            (thrown.space.clone(), thrown.kind, thrown.name.clone(), thrown.folder.clone(), thrown.role)
        };
        trash.iter().map(row).collect()
    };
    let row = |space: &str, kind, name: &str, folder: &str, role| {
        (space.to_owned(), kind, name.to_owned(), folder.to_owned(), role)
    };
    let trash: Vec<Trashed> = ben.get(paths::TRASH).await.unwrap();
    let family = |role| {
        [
            row("family", EntryKind::Folder, "Roof", "House", role),
            row("family", EntryKind::Document, "Quote", "House", role),
        ]
    };
    let own = row("home", EntryKind::Document, "Diary", "", Role::Owner);
    assert_eq!(told(&trash), [vec![own], family(Role::Editor).to_vec()].concat());

    // A reader of a space sees its trash and takes nothing out of it. Nobody sees another's own.
    let seen: Vec<Trashed> = guest.get(paths::TRASH).await.unwrap();
    assert_eq!(told(&seen), family(Role::Reader));
    let back = |thrown: &Trashed| paths::put_back(&thrown.space, &thrown.file);
    assert_eq!(guest.post::<Entry>(&back(&seen[0]), &()).await.expect_err("by a reader").code, Code::Forbidden);

    // Each comes back where it was, the folder with what it held, and is in the trash no more.
    let roof: Entry = ben.post(&back(&trash[1]), &()).await.expect("the folder is back");
    let quote: Entry = ben.post(&back(&trash[2]), &()).await.expect("the document is back");
    assert_eq!((roof.kind, roof.path.as_str()), (EntryKind::Folder, "House/Roof"));
    assert_eq!((quote.kind, quote.path.as_str()), (EntryKind::Document, "House/Quote.let"));
    assert_eq!(names(&listed(&ben, "family", "House/Roof").await), ["Tiles"]);
    assert_eq!(told(&ben.get::<Vec<Trashed>>(paths::TRASH).await.unwrap()).len(), 1);
    let twice = ben.post::<Entry>(&back(&trash[2]), &()).await;
    assert_eq!(twice.expect_err("it is in the trash no more").code, Code::NotFound);
}

#[tokio::test]
async fn a_folder_goes_nowhere_it_cannot() {
    let server = Server::new("folders-refused");
    let ben = server.person("ben", &["family"]);
    let guest = server.person("guest", &["friends"]);
    folders(&ben, &["House", "House/Roof", "Garden", "Garden/Roof"]).await;
    ben.start("family", "", "Plan").await;
    let before = server.scratch.names("family");

    let to = |folder: &str| Move { space: "family".into(), folder: folder.into() };
    let moves = [
        (&ben, "House", to("House"), Code::Invalid, "into itself"),
        (&ben, "House", to("House/Roof"), Code::Invalid, "into a folder of its own"),
        (&ben, "House/Roof", to("Garden"), Code::NameTaken, "where its name is taken"),
        (&ben, "House", to("Nowhere"), Code::NotFound, "into no folder"),
        (&ben, "Nowhere", to("Garden"), Code::NotFound, "no folder"),
        (&ben, "Plan.let", to("Garden"), Code::NotFound, "a document, which goes by its id"),
        (&ben, "", to("Garden"), Code::Invalid, "the space itself"),
        (&ben, "House/../Garden", to(""), Code::Invalid, "no place in the space"),
        (&guest, "House", to("Garden"), Code::Forbidden, "by a reader"),
    ];
    for (who, path, to, code, what) in moves {
        let refused = who.post::<Entry>(&paths::move_folder("family", path), &to).await.expect_err(what);
        assert_eq!(refused.code, code, "{what}");
    }
    let renames = [
        (&ben, "House", "Garden", Code::NameTaken, "to a name that is taken"),
        (&ben, "House", "Plan.let", Code::NameTaken, "to a document's name"),
        (&ben, "House", "/", Code::Invalid, "to no name"),
        (&guest, "House", "Home", Code::Forbidden, "by a reader"),
    ];
    for (who, path, name, code, what) in renames {
        let rename = Rename { name: name.into() };
        let refused = who.post::<Entry>(&paths::rename_folder("family", path), &rename).await.expect_err(what);
        assert_eq!(refused.code, code, "{what}");
    }
    let thrown = guest.post::<()>(&paths::trash_folder("family", "House"), &()).await.expect_err("by a reader");
    assert_eq!(thrown.code, Code::Forbidden);

    assert_eq!(server.scratch.names("family"), before, "nothing moved");
    assert_eq!(names(&listed(&ben, "family", "House").await), ["Roof"]);
    // Where it already is, it stays, and that is no fault.
    let stays: Entry = ben.post(&paths::move_folder("family", "House/Roof"), &to("House")).await.unwrap();
    assert_eq!(stays.path, "House/Roof");
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

/// A server where everyone has a space of their own, and one where nobody has: what a person keeps for
/// themselves is in files of their space on the first and in the server's store on the second, and is
/// asked for and changed alike. Each with a space its people write in.
fn homes(name: &str) -> [(Server, &'static str); 2] {
    [(Server::new(name), "home"), (Server::shared(&format!("{name}-shared")), "family")]
}

#[tokio::test]
async fn a_dictionary_changes_a_word_at_a_time_and_only_so() {
    for (server, _) in homes("words") {
        a_dictionary_changes(&server).await;
    }
}

async fn a_dictionary_changes(server: &Server) {
    let ben = server.person("ben", &[]);
    assert!(ben.get::<Vec<String>>(paths::DICTIONARY).await.unwrap().is_empty());

    // A word already there and a word already gone are no trouble, and a word is told from another
    // exactly as it is written.
    let steps: [(Method, &str, &[&str]); 6] = [
        (Method::PUT, "yewi", &["yewi"]),
        (Method::PUT, "naïve/2", &["yewi", "naïve/2"]),
        (Method::PUT, "yewi", &["yewi", "naïve/2"]),
        (Method::PUT, "Yewi", &["yewi", "naïve/2", "Yewi"]),
        (Method::DELETE, "yewi", &["naïve/2", "Yewi"]),
        (Method::DELETE, "yewi", &["naïve/2", "Yewi"]),
    ];
    for (method, word, then) in steps {
        let answer = ben.send(method.clone(), &paths::word(word), "", Vec::new()).await;
        assert_eq!(answer.json::<Vec<String>>(), owned(then), "{method} {word}");
    }
    let kept = owned(&["naïve/2", "Yewi"]);
    assert_eq!(ben.get::<Vec<String>>(paths::DICTIONARY).await.unwrap(), kept);

    // A device with an old list sends its settings whole: the words stay as they are, and the rest is kept.
    let old = Settings { language: Some("en-GB".into()), dictionary: owned(&["yewi"]), ..Settings::default() };
    let told = ben.put::<Settings>(paths::SETTINGS, &old).await.unwrap();
    assert_eq!((told.language.as_deref(), &told.dictionary), (Some("en-GB"), &kept));
    assert_eq!(ben.get::<Settings>(paths::SETTINGS).await.unwrap().dictionary, kept);

    // What is not one word is refused, and the words are the person's own.
    let long = "a".repeat(65);
    for word in ["two words", "tab\tbed", long.as_str()] {
        let refused = ben.send(Method::PUT, &paths::word(word), "", Vec::new()).await;
        assert_eq!(refused.json::<Error>().code, Code::Invalid, "{word}");
    }
    assert_eq!(ben.get::<Vec<String>>(paths::DICTIONARY).await.unwrap(), kept);
    assert!(server.person("alice", &[]).get::<Vec<String>>(paths::DICTIONARY).await.unwrap().is_empty());
}

/// A Word file that says `text`.
fn docx(text: &str) -> Vec<u8> {
    let line = Block::text(BlockId::new("line"), BlockKind::Paragraph, text);
    let document = Document { blocks: vec![line], ..Document::default() };
    letters_convert::to_docx(&document, &Style::default(), &Notes::default(), "Quote", &mut |_| None)
}

#[tokio::test]
async fn a_markdown_or_word_file_is_brought_in_as_a_document_that_replaces_nothing() {
    let server = Server::new("import");
    let ben = server.person("ben", &[]);
    // What the file is called and holds, then what the document is called, a line of it, and whether the
    // import had to simplify something.
    let markdown = "# Roof\n\nThe quote is *due*.\n\n<video src=\"roof.mp4\"></video>\n";
    let cases: [(&str, Vec<u8>, &str, &str, bool); 4] = [
        ("Roof.md", markdown.into(), "Roof", "The quote is due.", true),
        ("C:\\notes\\Roof.md", "Plain words.".into(), "Roof 2", "Plain words.", false),
        ("Quote.docx", docx("From the builder."), "Quote", "From the builder.", false),
        // What a file is, is told from what it holds.
        ("Quote.md", docx("Named as Markdown."), "Quote 2", "Named as Markdown.", false),
    ];
    for (name, file, called, line, simplified) in cases {
        let made = ben.send(Method::POST, &paths::import("home", "", name), "", file).await;
        assert_eq!(made.status, StatusCode::OK, "{name}: {}", made.text());
        let entry: Entry = made.json();
        let noted = entry.tags.iter().any(|tag| matches!(tag, Tag::ImportNotes { .. }));
        assert_eq!((entry.name.as_str(), noted), (called, simplified), "{name}");

        let file = ben.send(Method::GET, &paths::file(&entry.id.unwrap()), "", Vec::new()).await.body;
        let mut archive = Archive::open(Cursor::new(file)).unwrap();
        let text: Vec<String> = archive.document().unwrap().walk().map(Block::plain_text).collect();
        assert!(text.iter().any(|block| block == line), "{name}: {text:?}");
        // Its history says where it came from.
        let history = archive.read(OPLOG).unwrap().expect("a history");
        let marks = Model::load(&history, ReplicaId::new("reader")).unwrap().tags();
        let from = name.rsplit('\\').next();
        assert!(marks.iter().any(|mark| mark.kind == TagKind::Import && mark.name.as_deref() == from), "{name}");
    }
    assert_eq!(names(&listed(&ben, "home", "").await), ["Quote", "Quote 2", "Roof", "Roof 2"]);
}

#[tokio::test]
async fn a_file_that_makes_no_document_is_refused_and_nothing_is_kept() {
    let server = Server::new("import-refused");
    let (ben, guest) = (server.person("ben", &["family"]), server.person("guest", &["friends"]));
    // Who hands it over, where, what it holds, and what comes of it.
    let cases: [(&common::Client, &str, &str, &[u8], Code); 5] = [
        (&ben, "home", "", b"\xff\xfe neither words nor Word", Code::Invalid),
        (&ben, "home", "", b"\xd0\xcf\x11\xe0 an older .doc", Code::Invalid),
        (&ben, "home", "", b"PK\x03\x04 and no more of a zip", Code::Invalid),
        (&guest, "family", "", b"A reader's words.", Code::Forbidden),
        (&ben, "home", "Nowhere", b"Words for no folder.", Code::NotFound),
    ];
    for (who, space, folder, file, code) in cases {
        let refused = who.send(Method::POST, &paths::import(space, folder, "File.md"), "", file.to_vec()).await;
        assert_eq!(refused.json::<Error>().code, code, "{}", String::from_utf8_lossy(file));
    }
    for space in ["home", "family"] {
        assert!(listed(&ben, space, "").await.is_empty(), "{space}");
    }
}

/// A style that is not the built-in one, and as a person's settings carry it.
fn styled() -> (Style, serde_json::Value) {
    let mut style = Style::default();
    style.types.get_mut("body").expect("the built-in style sets body text").size = 15.0;
    let json = serde_json::from_str(&style.to_json()).unwrap();
    (style, json)
}

#[tokio::test]
async fn what_a_person_keeps_for_themselves_is_in_files_of_their_own_space() {
    let server = Server::new("kept");
    let ben = server.person("ben", &[]);
    let kept = server.scratch.0.join("home/ben/.letters");
    let read = |name: &str| std::fs::read_to_string(kept.join(name));

    // The files are the ones the desktop app keeps in a home folder: a word a line, and a style.
    let (style, json) = styled();
    for new in ["yewi", "Pannell"] {
        word(&ben, Method::PUT, new).await;
    }
    ben.put::<Settings>(paths::SETTINGS, &Settings { default_style: Some(json), ..Settings::default() }).await.unwrap();
    assert_eq!(read("dictionary.txt").unwrap().lines().collect::<Vec<_>>(), ["yewi", "Pannell"]);
    assert_eq!(Style::from_json(&read("style.json").unwrap()).unwrap(), style);

    // What another writer of the folder left there is the person's as well, and stays as it was written
    // when the server changes a word.
    std::fs::write(kept.join("dictionary.txt"), "yewi\nPannell\n  tsidp  \nsierra").unwrap();
    assert_eq!(ben.get::<Vec<String>>(paths::DICTIONARY).await.unwrap(), ["yewi", "Pannell", "tsidp", "sierra"]);
    word(&ben, Method::PUT, "letters").await;
    assert_eq!(word(&ben, Method::DELETE, "Pannell").await, ["yewi", "tsidp", "sierra", "letters"]);
    assert_eq!(read("dictionary.txt").unwrap(), "yewi\n  tsidp  \nsierra\nletters\n");

    // A person with no style of their own any more has no file of one.
    let told = ben.put::<Settings>(paths::SETTINGS, &Settings::default()).await.unwrap();
    assert_eq!((told.default_style, read("style.json").is_ok()), (None, false));

    // The folder is no part of the space: not listed, not to be asked for, and a document's file in it,
    // such as the desktop app's own copies, is no document here.
    let id = ben.start("home", "", "Plan").await.id.unwrap();
    std::fs::create_dir_all(kept.join("servers")).unwrap();
    for copy in ["Plan.let", "servers/Plan.let"] {
        std::fs::copy(server.scratch.0.join("home/ben/Plan.let"), kept.join(copy)).unwrap();
    }
    server.wait(60);
    let rows = listed(&ben, "home", "").await;
    assert_eq!((names(&rows), &rows[0].tags), (vec!["Plan"], &Vec::new()));
    assert_ne!(ben.get::<Listing>(&paths::entries("home", ".letters")).await.map(|_| ()), Ok(()));
    assert_eq!(ben.get::<Opened>(&paths::document(&id)).await.unwrap().path, "Plan.let");
}

#[tokio::test]
async fn what_an_earlier_build_kept_in_its_store_is_written_to_the_files_once() {
    let server = Server::new("kept-moved");
    let ben = server.person("ben", &[]);
    let rows: Table<Settings> = Table::new("settings");
    let (_, style) = styled();
    let before = Settings {
        name: Some("Benjamin".into()),
        dictionary: owned(&["yewi", "tsidp"]),
        default_style: Some(style.clone()),
        ..Settings::default()
    };
    server.app.store.put(&rows, "sub-ben", &before).unwrap();
    // The folder has words of its own already, from a device that has it as its home folder.
    let kept = server.scratch.0.join("home/ben/.letters");
    std::fs::create_dir_all(&kept).unwrap();
    std::fs::write(kept.join("dictionary.txt"), "tsidp\nsierra").unwrap();

    let told = ben.get::<Settings>(paths::SETTINGS).await.unwrap();
    let all = owned(&["tsidp", "sierra", "yewi"]);
    assert_eq!((told.name.as_deref(), told.dictionary, told.default_style), (Some("Benjamin"), all, Some(style)));

    // The store keeps neither afterwards, so a word that is taken out stays out.
    let row = server.app.store.get(&rows, "sub-ben").unwrap().unwrap();
    assert_eq!((row.name.as_deref(), row.dictionary.len(), row.default_style), (Some("Benjamin"), 0, None));
    assert_eq!(word(&ben, Method::DELETE, "yewi").await, ["tsidp", "sierra"]);
    assert_eq!(ben.get::<Vec<String>>(paths::DICTIONARY).await.unwrap(), ["tsidp", "sierra"]);
}

#[tokio::test]
async fn what_a_person_set_is_kept_and_shapes_their_new_documents() {
    for (server, space) in homes("settings") {
        what_a_person_set_is_kept(&server, space).await;
    }
}

async fn what_a_person_set_is_kept(server: &Server, space: &str) {
    let ben = server.person("ben", &["family"]);
    assert_eq!(ben.get::<Settings>(paths::SETTINGS).await.unwrap(), Settings::default());

    let (style, json) = styled();
    let settings = Settings {
        name: Some("Benjamin".into()),
        accent: Some([10, 120, 60]),
        default_style: Some(json),
        ..Settings::default()
    };
    assert_eq!(ben.put::<Settings>(paths::SETTINGS, &settings).await.unwrap(), settings);
    assert_eq!(ben.get::<Settings>(paths::SETTINGS).await.unwrap(), settings);
    assert_eq!(server.person("alice", &[]).get::<Settings>(paths::SETTINGS).await.unwrap(), Settings::default());

    let made = ben.start(space, "", "Styled").await;
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
