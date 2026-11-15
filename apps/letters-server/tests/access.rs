//! A document's access log: what is noted of whoever a share or a link led to it, and who reads it.

mod common;

use axum::http::{Method, StatusCode};
use common::desk::{Desk, settle};
use common::{Client, START, URL, follow, let_in, link, planned};
use letters_api::{Access, Action, Code, CreatedLink, NewLink, Opened, Role, Server as Told, Who, paths};

/// The log of the document as Ada reads it, newest first.
async fn log(ada: &Client, id: &str) -> Vec<Access> {
    ada.get(&paths::access(id)).await.expect("whoever shares a document reads its log")
}

/// The name of whoever a row is of, when they signed in, and the label of the link they came by.
fn named(row: &Access) -> (&str, Option<&str>) {
    match &row.who {
        Who::Person { name, .. } => (name, row.link.as_ref().map(|link| link.label.as_str())),
        Who::Link { label, .. } => ("", Some(label)),
    }
}

#[tokio::test]
async fn what_a_share_or_a_link_let_someone_do_is_noted_and_what_their_space_lets_them_is_not() {
    let (server, ada, ben, id) = planned("access-noted").await;
    let reached = paths::document(&id);
    // Ada's space lets her open her own document, and nobody is told. Ben's share lets him.
    let _: Opened = ada.get(&reached).await.unwrap();
    let _: Opened = ben.get(&reached).await.unwrap();
    server.wait(10);
    // A link's reader opens it, and asks for the file, which this link withholds.
    let (mut guest, made) = let_in(&server, &ada, &id, &link("Accountant", Role::Reader)).await;
    guest.send(Method::GET, &reached, "", Vec::new(), &[("user-agent", "Firefox")]).await;
    server.wait(10);
    assert_eq!(guest.get(&paths::file(&id)).await.status, StatusCode::FORBIDDEN);
    server.wait(10);
    // Carol is signed in, and comes by the same link.
    let mut carol = server.reader("carol", &[]);
    follow(&mut carol, &made).await;
    assert_eq!(carol.get(&reached).await.status, StatusCode::OK);

    let rows = log(&ada, &id).await;
    let told: Vec<_> = rows.iter().map(|row| (named(row), row.action, row.refused, row.role)).collect();
    let expected = [
        (("Carol", Some("Accountant")), Action::Open, false, Role::Reader),
        (("", Some("Accountant")), Action::Download, true, Role::Reader),
        (("", Some("Accountant")), Action::Open, false, Role::Reader),
        (("Ben", None), Action::Open, false, Role::Editor),
    ];
    assert_eq!(told, expected);
    assert_eq!(rows[2].browser, "Firefox", "a row says what asked");

    // Whoever may share the document reads its log, and nobody that a share or a link led to it.
    assert_eq!(ben.get::<Vec<Access>>(&paths::access(&id)).await.unwrap_err().code, Code::Forbidden);
    assert_eq!(guest.get(&paths::access(&id)).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_same_thing_again_is_one_row_and_a_session_of_edits_says_how_long_it_lasted() {
    let (server, ada, _, id) = planned("access-sessions").await;
    let (mut guest, _) = let_in(&server, &ada, &id, &link("Accountant", Role::Editor)).await;
    // The page is loaded three times within a minute, and then written on for some minutes.
    for _ in 0..3 {
        assert_eq!(guest.get(&paths::document(&id)).await.status, StatusCode::OK);
        server.wait(20);
    }
    let mut desk = Desk::paged(&server, &guest, &id).await.unwrap();
    settle([&mut desk]).await;
    for text in ["One. ", "Two. ", "Three. "] {
        desk.write(text);
        settle([&mut desk]).await;
        server.wait(120);
    }
    desk.comment("Is this right?");
    settle([&mut desk]).await;

    let rows = log(&ada, &id).await;
    let told: Vec<_> = rows.iter().map(|row| row.action).collect();
    assert_eq!(told, [Action::Comment, Action::Edit, Action::Open]);
    let lasted = rows[1].lasted.expect("a session of edits says how long it lasted");
    assert!((200..=400).contains(&lasted), "three edits two minutes apart lasted {lasted} seconds");

    // Writing again much later is another session.
    server.wait(3600);
    desk.write("Four. ");
    settle([&mut desk]).await;
    let ada = server.person("ada", &[]);
    let edits = log(&ada, &id).await.into_iter().filter(|row| row.action == Action::Edit).count();
    assert_eq!(edits, 2);
}

#[tokio::test]
async fn a_link_that_opened_nothing_for_whoever_tried_it_is_noted_as_refused() {
    let (server, ada, _, id) = planned("access-refused").await;
    let asks = Some("orchard".to_owned());
    let new = NewLink { passphrase: asks, expires: Some(START + 3600), ..link("Builder", Role::Commenter) };
    let made: CreatedLink = ada.post(&paths::links(&id), &new).await.unwrap();
    let mut guest = server.visitor();
    // A look at the page that asks is no try. A wrong passphrase is one, and so is the link once it ran out.
    assert_eq!(follow(&mut guest, &made).await.status, StatusCode::OK);
    let wrong = guest.submit(made.url.strip_prefix(URL).unwrap(), &[("passphrase", "apple")], None).await;
    assert_eq!(wrong.status, StatusCode::FORBIDDEN);
    server.wait(3600);
    assert_eq!(follow(&mut guest, &made).await.status, StatusCode::NOT_FOUND);

    let rows = log(&server.person("ada", &[]), &id).await;
    let told: Vec<_> = rows.iter().map(|row| (named(row), row.action, row.refused, row.role)).collect();
    assert_eq!(told, [(("", Some("Builder")), Action::Open, true, Role::Commenter); 2]);
}

#[tokio::test]
async fn the_log_is_read_a_page_at_a_time_and_kept_for_as_long_as_the_server_says() {
    let (server, _, _, id) = planned("access-paged").await;
    // Ben opens the document and takes its file away, on each of three days.
    for _ in 0..3 {
        let mut ben = server.reader("ben", &[]);
        for path in [paths::document(&id), paths::file(&id)] {
            assert_eq!(ben.get(&path).await.status, StatusCode::OK, "{path}");
        }
        server.wait(24 * 3600);
    }
    let ada = server.person("ada", &[]);
    let all = log(&ada, &id).await;
    assert_eq!(all.len(), 6);
    // Each page is asked for from before the time of the last row had, and none of a second's rows is missed.
    let (mut pages, mut before) = (Vec::new(), String::new());
    loop {
        let page: Vec<Access> = ada.get(&format!("{}?limit=3{before}", paths::access(&id))).await.unwrap();
        let Some(last) = page.last() else { break };
        assert!(page.len() < all.len(), "a page is a part of the log");
        before = format!("&before={}", last.time);
        pages.extend(page);
    }
    assert_eq!(pages, all, "the pages are the log, in its order");

    let told: Told = ada.get(paths::SERVER).await.unwrap();
    assert!(told.log_days > 0, "the server says how long it keeps a row");
    server.wait(u64::from(told.log_days) * 24 * 3600);
    assert_eq!(log(&server.person("ada", &[]), &id).await, []);
}
