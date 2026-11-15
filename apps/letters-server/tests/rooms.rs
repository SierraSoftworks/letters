//! Documents open on several devices at once: what each ends up with, what the file holds, and what
//! becomes of a session when its document or its sign-in goes.

mod common;

use std::io::Cursor;
use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::desk::{Desk, settle};
use common::{Client, Server, texts};
use letters_api::{Code, Entry, Listing, Move, NewFolder, Rename, Tag, paths};
use letters_format::{Archive, MIMETYPE, WorkingCopy, hash};
use letters_model::{Block, BlockKind, Op, Pos, ReplicaId};
use letters_sync::{Link, Refusal, Role};

/// A smallest PNG, by its first bytes.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n and the rest";

/// A server with the document "Plan" in the family's space, the person who made it, and its id.
async fn shared(name: &str) -> (Server, Client, String) {
    let server = Server::new(name);
    let ada = server.person("ada", &["family"]);
    let id = ada.start("family", "", "Plan").await.id.expect("a document has an id");
    (server, ada, id)
}

/// The text of the document `id` as the server sends its file.
async fn on_file(who: &Client, id: &str) -> String {
    let sent = who.send(Method::GET, &paths::file(id), "", Vec::new()).await;
    assert_eq!(sent.status, StatusCode::OK);
    texts(sent.body).join("\n")
}

/// The row of the family's only document.
async fn row(who: &Client) -> Entry {
    let listing: Listing = who.get(&paths::entries("family", "")).await.expect("the folder lists");
    listing.entries.into_iter().next().expect("the document is listed")
}

/// The name a picture's bytes go by.
fn named(bytes: &[u8]) -> String {
    format!("{}.png", hash(bytes).strip_prefix("sha256:").unwrap())
}

#[tokio::test]
async fn two_people_write_one_document_and_its_file_holds_both() {
    let (server, ada, id) = shared("together").await;
    let ben = server.person("ben", &["family"]);
    let mut a = Desk::open(&server, &ada, &id).await.unwrap();
    let mut b = Desk::open(&server, &ben, &id).await.unwrap();
    settle([&mut a, &mut b]).await;
    assert_eq!(a.line.link(), Link::Live(Role::Editor));

    let (first, second) = (a.write("Ada was here. "), b.write("So was Ben. "));
    settle([&mut a, &mut b]).await;

    let text = a.texts().join("\n");
    assert!(text.contains("Ada was here.") && text.contains("So was Ben."), "both are kept: {text}");
    assert_eq!(a.texts(), b.texts());
    assert!(a.holds(first) && b.holds(second), "each was told that the server keeps what they wrote");
    let editing = Tag::Editing { people: vec!["Ada".into(), "Ben".into()] };
    assert!(row(&ada).await.tags.contains(&editing), "the file table says who has it open");
    // The file that is sent holds what was written a moment ago.
    assert_eq!(on_file(&ada, &id).await, text);

    a.leave().await;
    b.leave().await;
    assert!(row(&ada).await.tags.is_empty(), "nobody has it open now");
    assert_eq!(on_file(&ben, &id).await, text);
}

#[tokio::test]
async fn a_role_in_the_space_decides_what_a_session_may_write() {
    let (server, ada, id) = shared("roles").await;
    let cases =
        [("friends", Role::Reader, false), ("reviewers", Role::Commenter, false), ("family", Role::Editor, true)];
    for (group, role, writes) in cases {
        let guest = server.person(&format!("guest-of-{group}"), &[group]);
        let mut desk = Desk::open(&server, &guest, &id).await.unwrap();
        settle([&mut desk]).await;
        assert_eq!(desk.line.link(), Link::Live(role), "{group}");

        let wrote = format!("From the {group}. ");
        desk.write(&wrote);
        settle([&mut desk]).await;
        assert_eq!(on_file(&ada, &id).await.contains(&wrote), writes, "{group}");
        desk.leave().await;
    }
    let stranger = server.person("stranger", &[]);
    let refused = Desk::open(&server, &stranger, &id).await.err().expect("a document out of reach");
    assert_eq!(refused.code, letters_api::Code::NotFound);
}

#[tokio::test]
async fn a_session_follows_its_document_when_it_is_renamed_and_moved() {
    let (server, ada, id) = shared("follows").await;
    let mut desk = Desk::open(&server, &ada, &id).await.unwrap();
    settle([&mut desk]).await;
    desk.write("Before. ");
    settle([&mut desk]).await;

    let _: Entry = ada.post(&paths::rename(&id), &Rename { name: "Scheme".into() }).await.unwrap();
    let plans = NewFolder { parent: String::new(), name: "Plans".into() };
    let _: Entry = ada.post(&paths::folders("family"), &plans).await.unwrap();
    let there = Move { space: "family".into(), folder: "Plans".into() };
    let _: Entry = ada.post(&paths::relocate(&id), &there).await.unwrap();
    desk.write("After. ");
    settle([&mut desk]).await;
    assert_eq!(desk.line.link(), Link::Live(Role::Editor), "the session went on");
    desk.leave().await;

    assert_eq!(server.scratch.names("family"), ["Plans"], "nothing is written where the document was");
    assert_eq!(server.scratch.names("family/Plans"), ["Scheme.let"]);
    let text = on_file(&ada, &id).await;
    assert!(text.contains("Before.") && text.contains("After."), "{text}");
}

#[tokio::test]
async fn a_document_that_is_thrown_away_ends_its_sessions() {
    let (server, ada, id) = shared("thrown").await;
    let mut desk = Desk::open(&server, &ada, &id).await.unwrap();
    settle([&mut desk]).await;
    desk.write("Kept all the same. ");
    settle([&mut desk]).await;

    ada.post::<()>(&paths::trash(&id), &()).await.unwrap();
    settle([&mut desk]).await;

    assert_eq!(desk.line.link(), Link::Refused(Refusal::Gone));
    let thrown = std::fs::read(server.scratch.0.join("family/.letters-trash/Plan.let")).expect("it is in the trash");
    assert!(texts(thrown).join("\n").contains("Kept all the same."), "with what was written until then");
}

#[tokio::test]
async fn signing_out_ends_the_sessions_of_that_sign_in_alone() {
    let (server, ada, id) = shared("signed-out").await;
    let ben = server.person("ben", &["family"]);
    let mut a = Desk::open(&server, &ada, &id).await.unwrap();
    let mut b = Desk::open(&server, &ben, &id).await.unwrap();
    settle([&mut a, &mut b]).await;

    ada.post::<()>(paths::SIGN_OUT, &()).await.unwrap();
    settle([&mut a, &mut b]).await;

    assert_eq!(a.line.link(), Link::Refused(Refusal::SignedOut));
    assert_eq!(b.line.link(), Link::Live(Role::Editor));
}

#[tokio::test]
async fn a_picture_is_handed_over_ahead_of_the_edit_that_names_it() {
    let (server, ada, id) = shared("picture").await;
    let guest = server.person("guest", &["friends"]);
    let name = named(PNG);
    let put = async |who: &Client, name: &str, bytes: &[u8]| {
        who.send(Method::PUT, &paths::asset(&id, name), "image/png", bytes.to_vec()).await.status
    };
    let huge = vec![0; letters_api::Limits::default().asset as usize + 1];
    let cases: [(&Client, &str, &[u8], StatusCode, &str); 4] = [
        (&guest, &name, PNG, StatusCode::FORBIDDEN, "a reader"),
        (&ada, &named(b"another"), PNG, StatusCode::BAD_REQUEST, "under another picture's name"),
        (&ada, &named(b"plain text"), b"plain text", StatusCode::BAD_REQUEST, "no picture"),
        (&ada, &named(&huge), &huge, StatusCode::PAYLOAD_TOO_LARGE, "more than the server takes"),
    ];
    for (who, name, bytes, expected, what) in cases {
        assert_eq!(put(who, name, bytes).await, expected, "{what}");
    }

    assert_eq!(put(&ada, &name, PNG).await, StatusCode::NO_CONTENT);
    let fetched = guest.send(Method::GET, &paths::asset(&id, &name), "", Vec::new()).await;
    assert_eq!((fetched.status, fetched.body.as_slice()), (StatusCode::OK, PNG), "the others can fetch it at once");

    let mut desk = Desk::open(&server, &ada, &id).await.unwrap();
    settle([&mut desk]).await;
    let figure = BlockKind::Figure {
        asset: Some(format!("assets/{name}")),
        src: None,
        alt: "A chart".into(),
        width: 320.0,
        height: 200.0,
        numbered: false,
    };
    desk.edit([Op::InsertBlock { parent: None, index: 1, block: Block::new("f", figure) }]);
    settle([&mut desk]).await;
    desk.leave().await;

    assert_eq!(ada.get::<Vec<String>>(&paths::assets(&id)).await.unwrap(), std::slice::from_ref(&name));
    let file = ada.send(Method::GET, &paths::file(&id), "", Vec::new()).await.body;
    let kept = Archive::open(Cursor::new(file)).unwrap().read(&format!("assets/{name}")).unwrap();
    assert_eq!(kept.as_deref(), Some(PNG), "the file holds it once the document names it");
    assert_eq!(put(&ada, &name, PNG).await, StatusCode::NO_CONTENT, "handing it over again is no fault");
}

#[tokio::test]
async fn a_device_hands_its_whole_file_over_and_nothing_is_replaced() {
    let (server, ada, id) = shared("handed-over").await;
    let mut desk = Desk::open(&server, &ada, &id).await.unwrap();
    settle([&mut desk]).await;
    // A device wrote on its own copy of the file while it was away, and someone else wrote here.
    let was = ada.send(Method::GET, &paths::file(&id), "", Vec::new()).await.body;
    let mut away = WorkingCopy::open(&mut Archive::open(Cursor::new(was)).unwrap(), ReplicaId::from(9)).unwrap();
    let first = away.model().doc().blocks[0].id.clone();
    away.apply([Op::InsertText { at: Pos::new(first, 0), text: "Written away. ".into() }]).unwrap();
    let theirs = letters_server::library::packed(&away).expect("the copy is a file");
    desk.write("Written here. ");
    settle([&mut desk]).await;

    let put = async |who: &Client, file: &[u8]| {
        who.send(Method::PUT, &paths::file(&id), MIMETYPE, file.to_vec()).await.status
    };
    let guest = server.person("guest", &["friends"]);
    let other = ada.start("family", "", "Other").await.id.unwrap();
    let other = ada.send(Method::GET, &paths::file(&other), "", Vec::new()).await.body;
    let cases: [(&Client, &[u8], StatusCode, &str); 3] = [
        (&guest, &theirs, StatusCode::FORBIDDEN, "a reader"),
        (&ada, &other, StatusCode::BAD_REQUEST, "another document"),
        (&ada, b"no document at all", StatusCode::BAD_REQUEST, "no document"),
    ];
    for (who, file, expected, what) in cases {
        assert_eq!(put(who, file).await, expected, "{what}");
    }
    for _ in 0..2 {
        assert_eq!(put(&ada, &theirs).await, StatusCode::NO_CONTENT, "handing it over twice is no fault");
    }
    settle([&mut desk]).await;

    let text = desk.texts().join("\n");
    assert!(text.contains("Written away.") && text.contains("Written here."), "whoever has it open is told: {text}");
    assert_eq!(on_file(&ada, &id).await, text);
}

#[tokio::test]
async fn a_conflict_copy_is_merged_when_an_editor_asks_and_then_thrown_away() {
    let (server, ada, id) = shared("merge-conflicts").await;
    let mut desk = Desk::open(&server, &ada, &id).await.unwrap();
    settle([&mut desk]).await;
    // A sync tool leaves what another machine wrote beside the document, and someone writes here.
    let was = ada.send(Method::GET, &paths::file(&id), "", Vec::new()).await.body;
    let mut away = WorkingCopy::open(&mut Archive::open(Cursor::new(was)).unwrap(), ReplicaId::from(9)).unwrap();
    let first = away.model().doc().blocks[0].id.clone();
    away.apply([Op::InsertText { at: Pos::new(first, 0), text: "Written away. ".into() }]).unwrap();
    let copy = "Plan (conflicted copy 2026-11-15)";
    let beside = server.scratch.0.join("family").join(format!("{copy}.let"));
    std::fs::write(beside, letters_server::library::packed(&away).unwrap()).unwrap();
    desk.write("Written here. ");
    settle([&mut desk]).await;

    let merge = async |who: &Client| who.post::<Vec<String>>(&paths::merge_conflicts(&id), &()).await;
    let guest = server.person("guest", &["friends"]);
    assert_eq!(merge(&guest).await.expect_err("a reader merges nothing").code, Code::Forbidden);
    assert!(!on_file(&ada, &id).await.contains("Written away."), "nothing is merged before an editor asks");

    assert_eq!(merge(&ada).await.unwrap(), [copy]);
    settle([&mut desk]).await;
    let text = desk.texts().join("\n");
    assert!(text.contains("Written away.") && text.contains("Written here."), "whoever has it open is told: {text}");
    assert_eq!(on_file(&ada, &id).await, text);
    assert_eq!(server.scratch.names("family/.letters-trash"), [format!("{copy}.let")], "the copy can be put back");
    assert_eq!(merge(&ada).await.unwrap(), [""; 0], "with no copy left there is nothing to merge");
}

#[tokio::test(start_paused = true)]
async fn a_file_moved_behind_the_servers_back_is_not_written_again_where_it_was() {
    let (server, ada, id) = shared("moved-away").await;
    let mut desk = Desk::open(&server, &ada, &id).await.unwrap();
    settle([&mut desk]).await;
    desk.write("Written on the way. ");
    settle([&mut desk]).await;

    // A sync tool brings a rename that was made on another machine.
    let family = server.scratch.0.join("family");
    std::fs::rename(family.join("Plan.let"), family.join("Elsewhere.let")).unwrap();
    tokio::time::sleep(Duration::from_secs(60)).await;
    settle([&mut desk]).await;

    assert_eq!(desk.line.link(), Link::Refused(Refusal::Gone));
    assert_eq!(server.scratch.names("family"), ["Elsewhere.let"]);
    // What the server was told to keep is in the document for whoever opens it where it is now.
    let mut again = Desk::open(&server, &ada, &id).await.unwrap();
    settle([&mut again]).await;
    assert!(again.texts().join("\n").contains("Written on the way."));
}

#[test]
fn what_a_run_was_told_to_keep_reaches_the_file_at_the_next_start() {
    let run = || tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let first = run();
    let (server, id, desk) = first.block_on(async {
        let (server, ada, id) = shared("recovered").await;
        let mut desk = Desk::open(&server, &ada, &id).await.unwrap();
        settle([&mut desk]).await;
        let written = desk.write("Told and kept. ");
        settle([&mut desk]).await;
        assert!(desk.holds(written), "the server said it keeps this");
        (server, id, desk)
    });
    // The server stops with no time to save: its tasks end where they are, and nobody leaves.
    drop(first);
    drop(desk);

    run().block_on(async {
        let server = server.restarted();
        let app = server.app.clone();
        tokio::task::spawn_blocking(move || app.rooms.recover(&app)).await.unwrap();
        let ada = server.person("ada", &["family"]);
        assert!(on_file(&ada, &id).await.contains("Told and kept."));
    });
}
