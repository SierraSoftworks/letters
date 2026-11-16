//! A document in another format: who is written one, what each holds, what a link withholds, and where
//! the pages offer them.

mod common;

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use axum::http::{Method, StatusCode};
use common::desk::{Desk, settle};
use common::{Client, Raw, Server, let_in, link, planned};
use letters_api::{Access, Action, Allows, Export, Listing, NewLink, Role, Who, paths};
use letters_convert::{Loss, LossKind, Target};
use letters_format::WorkingCopy;
use letters_model::{Block, BlockKind, Document, ReplicaId, Style};
use letters_server::pages::asking;
use zip::ZipArchive;

/// A smallest PNG, by its first bytes.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n and the rest";
/// What is written into a document here, and what is said of it in the margin.
const LINE: &str = "We would like to make an offer.";
const COMMENT: &str = "Is 400,000 still the ceiling?";

/// Someone writes a line into the document and leaves a comment on it.
async fn written(server: &Server, who: &Client, id: &str) {
    let mut desk = Desk::open(server, who, id).await.unwrap();
    settle([&mut desk]).await;
    desk.write(LINE);
    desk.comment(COMMENT);
    settle([&mut desk]).await;
    desk.leave().await;
}

/// The files of a zip, each by its name.
fn unzipped(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).expect("a zip");
    let files = zip.len();
    let read = |at: usize| {
        let (mut file, mut held) = (zip.by_index(at).unwrap(), Vec::new());
        file.read_to_end(&mut held).unwrap();
        (file.name().unwrap().to_string(), held)
    };
    (0..files).map(read).collect()
}

/// Everything an export says in words, whatever it is packed as. A PDF presses its text, so none of it
/// is read here.
fn said(to: Export, file: &Raw) -> String {
    match to {
        Export::Docx => unzipped(&file.body).values().map(|part| String::from_utf8_lossy(part).into_owned()).collect(),
        _ => file.text(),
    }
}

/// What a file is called when a browser keeps it.
fn kept_as(file: &Raw) -> String {
    let told = file.header("content-disposition");
    let name = told.split("filename=\"").nth(1).and_then(|rest| rest.split('"').next());
    name.unwrap_or_default().to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_document_is_written_in_each_format_for_whoever_reads_it() {
    let (server, ada, ben, id) = planned("export-formats").await;
    written(&server, &ada, &id).await;
    for (to, name) in [
        (Export::Pdf, "Plan.pdf"),
        (Export::Docx, "Plan.docx"),
        (Export::Markdown, "Plan.md"),
        (Export::Html, "Plan.html"),
    ] {
        // Ada owns the document and Ben was handed it. Each is given a file to keep, under its name.
        for who in [&ada, &ben] {
            let file = who.send(Method::GET, &paths::export(&id, to), "", Vec::new()).await;
            assert_eq!((file.status, kept_as(&file).as_str()), (StatusCode::OK, name), "{to:?}");
            match to {
                Export::Pdf => assert!(file.body.starts_with(b"%PDF-"), "a PDF"),
                _ => assert!(said(to, &file).contains(LINE), "{to:?} holds what was written"),
            }
            // Only a Word file has a place for a comment.
            assert_eq!(said(to, &file).contains(COMMENT), to == Export::Docx, "{to:?}");
        }
        // Whoever does not reach the document is written nothing.
        let carol = server.person("carol", &[]);
        let hidden = carol.send(Method::GET, &paths::export(&id, to), "", Vec::new()).await;
        assert_eq!(hidden.status, StatusCode::NOT_FOUND, "{to:?}");
    }
    // And nothing is written in a format the server does not know.
    for unknown in ["let", "exe", ""] {
        let path = format!("{}/export/{unknown}", paths::document(&id));
        assert_eq!(ada.send(Method::GET, &path, "", Vec::new()).await.status, StatusCode::NOT_FOUND, "{unknown:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn markdown_goes_out_with_the_pictures_it_names() {
    let server = Server::new("export-pictures");
    let ben = server.person("ben", &[]);
    ben.start("home", "", "Seed").await;
    // A document with a picture, as an editor would have saved it.
    let asset = "assets/ab12.png";
    let kind = BlockKind::Figure {
        asset: Some(asset.into()),
        src: None,
        alt: "The sky".into(),
        width: 300.0,
        height: 200.0,
        numbered: false,
    };
    let mut document = Document::default();
    document.blocks.insert(0, Block::new("sky", kind));
    let mut copy =
        WorkingCopy::new("with-picture", "2026-11-15T10:00:00Z", document, Style::default(), ReplicaId::from(1));
    copy.entries = BTreeMap::from([(asset.to_owned(), PNG.to_vec())]);
    let mut bytes = Cursor::new(Vec::new());
    letters_format::write(&mut bytes, &letters_format::Replica::package(&copy)).unwrap();
    std::fs::write(server.scratch.0.join("home/ben/Pictured.let"), bytes.into_inner()).unwrap();
    ben.get::<Listing>(&paths::entries("home", "")).await.unwrap();

    // One file to keep: the text, and the picture where the text looks for it.
    let file = ben.send(Method::GET, &paths::export("with-picture", Export::Markdown), "", Vec::new()).await;
    assert_eq!((file.status, kept_as(&file).as_str()), (StatusCode::OK, "Pictured.zip"));
    let held = unzipped(&file.body);
    assert_eq!(held.keys().collect::<Vec<_>>(), ["Pictured.md", asset]);
    assert!(String::from_utf8_lossy(&held["Pictured.md"]).contains(asset), "the text names its picture");
    assert_eq!(held[asset], PNG);
    // A web page holds its picture within itself.
    let page = ben.send(Method::GET, &paths::export("with-picture", Export::Html), "", Vec::new()).await;
    assert!(page.text().contains("src=\"data:image/png;base64,"), "{}", page.text());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pdf_sets_what_the_fonts_lack_in_a_face_from_the_state_directory() {
    let (server, ada, _, id) = planned("export-faces").await;
    // A letter the fonts lack, which a small face of Hebrew has.
    let face = font_test_data::NOTOSERIFHEBREW_AUTOHINT_METRICS;
    let letter = ('\u{5d0}'..='\u{5ea}').find(|ch| letters_web::faces::has(face, *ch)).expect("the face has a letter");
    let mut desk = Desk::open(&server, &ada, &id).await.unwrap();
    settle([&mut desk]).await;
    desk.write(&letter.to_string());
    settle([&mut desk]).await;
    desk.leave().await;

    // Whether a PDF was written, and whether it carries a face of Hebrew.
    let pdf = paths::export(&id, Export::Pdf);
    let told = |file: Raw| (file.status, String::from_utf8_lossy(&file.body).contains("Hebrew"));
    // With no face for it the letter is a box, and the PDF is written all the same.
    assert_eq!(told(ada.send(Method::GET, &pdf, "", Vec::new()).await), (StatusCode::OK, false));
    // The owner puts the face where the editor's faces are, and the next PDF is set in it.
    let folder = server.scratch.0.join("state/fallback");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("NotoSansHebrew.ttf"), face).unwrap();
    assert_eq!(told(ada.send(Method::GET, &pdf, "", Vec::new()).await), (StatusCode::OK, true));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_hands_over_a_format_where_it_lets_the_document_be_taken_and_nothing_of_its_past() {
    let (server, ada, ben, id) = planned("export-link").await;
    written(&server, &ada, &id).await;
    let word = paths::export(&id, Export::Docx);
    // Ben was handed the document, and takes its comments along. A link's visitor takes the text alone.
    let whole = ben.send(Method::GET, &word, "", Vec::new()).await;
    assert!(said(Export::Docx, &whole).contains(COMMENT));
    server.wait(10);
    let taking = NewLink { allows: Allows { download: true, ..Allows::default() }, ..link("Seller", Role::Commenter) };
    let (mut guest, _) = let_in(&server, &ada, &id, &taking).await;
    let clean = guest.get(&word).await;
    let told = (clean.status, said(Export::Docx, &clean).contains(LINE), said(Export::Docx, &clean).contains(COMMENT));
    assert_eq!(told, (StatusCode::OK, true, false));
    // A PDF names who wrote the document to whoever was handed it, and to no link's visitor.
    let pdf = paths::export(&id, Export::Pdf);
    let names = |file: Raw| String::from_utf8_lossy(&file.body).contains("Ada");
    let named = (names(ben.send(Method::GET, &pdf, "", Vec::new()).await), names(guest.get(&pdf).await));
    assert_eq!(named, (true, false));
    server.wait(10);

    // A link that lets nothing be taken away hands over no format.
    let (mut held, _) = let_in(&server, &ada, &id, &link("Builder", Role::Reader)).await;
    for to in Export::ALL {
        assert_eq!(held.get(&paths::export(&id, to)).await.status, StatusCode::FORBIDDEN, "{to:?}");
    }
    // The log tells who took the document away and who was kept from it, newest first.
    let rows: Vec<Access> = ada.get(&paths::access(&id)).await.unwrap();
    let named = |row: &Access| match &row.who {
        Who::Person { name, .. } => name.clone(),
        Who::Link { label, .. } => label.clone(),
    };
    let taken = rows.iter().filter(|row| row.action == Action::Download).map(|row| (named(row), row.refused));
    let taken: Vec<_> = taken.collect();
    assert_eq!(taken, [("Builder".to_owned(), true), ("Seller".to_owned(), false), ("Ben".to_owned(), false)]);

    // A visit is written only so many a minute, and someone signed in is not counted with it.
    let mut asked = 0;
    let slow = loop {
        let answer = guest.get(&word).await;
        if answer.status != StatusCode::OK || asked == 50 {
            break answer;
        }
        asked += 1;
    };
    assert_eq!(slow.status, StatusCode::TOO_MANY_REQUESTS, "after {asked}");
    assert!(asked >= 2, "a visitor takes a format or two: {asked}");
    assert_eq!(ben.send(Method::GET, &word, "", Vec::new()).await.status, StatusCode::OK);
    server.wait(60);
    assert_eq!(guest.get(&word).await.status, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pages_offer_each_format_and_say_what_it_will_simplify() {
    let server = Server::new("export-pages");
    let mut ben = server.reader("ben", &["family"]);
    let id = ben.pages().start("family", "", "Plan").await.id.unwrap();
    written(&server, &server.person("ben", &["family"]), &id).await;

    // A row of the file table asks, and the question opens over its folder.
    let here = paths::folder_page("family", "");
    let ask = asking(&here, "export", &id);
    assert!(ben.get(&here).await.text().contains(&ask.replace('&', "&amp;")), "the row offers to export");
    // Whoever only reads is offered as much: on the folder's page, and on the page of the text.
    let mut guest = server.reader("guest", &["friends"]);
    let lost = Loss { kind: LossKind::Comments, count: 1, to: Target::Pdf }.to_string();
    for (who, at) in
        [("ben", &ask), ("ben", &paths::read_page(&id)), ("guest", &ask), ("guest", &paths::read_page(&id))]
    {
        let browser = if who == "ben" { &mut ben } else { &mut guest };
        let page = browser.get(at).await;
        assert_eq!(page.status, StatusCode::OK, "{who} at {at}");
        let page = page.text();
        for to in Export::ALL {
            assert!(page.contains(&format!("href=\"{}\"", paths::export(&id, to))), "{who} at {at}: {to:?}");
        }
        // Only a Word file carries the comment, which is said before a format is asked for.
        assert_eq!(page.matches(&lost).count(), Export::ALL.len() - 1, "{who} at {at}: {page}");
    }
    // Nothing is offered of a document that is out of reach.
    let mut stranger = server.reader("stranger", &[]);
    let hidden = stranger.get(&asking(&paths::folder_page("home", ""), "export", &id)).await.text();
    assert!(!hidden.contains("/export/"), "{hidden}");
}
