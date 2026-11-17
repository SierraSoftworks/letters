//! The Google Fonts library for a browser: the families a writer can choose, and a face of one, which
//! the server fetches so that the library hears of nobody else.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::http::{Method, StatusCode};
use common::Server;
use letters_api::paths;
use letters_server::fonts::Fetch;

/// A font file that stands for whatever the library serves.
const FILE: &[u8] = include_bytes!("../../../crates/letters-math/fonts/KaTeX_AMS-Regular.ttf");

/// A library that answers as Google's does, and counts what it is asked. One that is `down` answers
/// nothing.
fn library(down: bool) -> (Fetch, Arc<AtomicUsize>) {
    let asked = Arc::new(AtomicUsize::new(0));
    let count = asked.clone();
    let fetch = move |address: &str| {
        count.fetch_add(1, Ordering::Relaxed);
        match address {
            _ if down => None,
            sheet if sheet.starts_with("https://fonts.googleapis.com/css2?") => {
                Some(b"@font-face { src: url(https://fonts.gstatic.com/s/face.ttf) format('truetype'); }".to_vec())
            }
            "https://fonts.gstatic.com/s/face.ttf" => Some(FILE.to_vec()),
            _ => None,
        }
    };
    (Arc::new(fetch), asked)
}

#[tokio::test]
async fn the_families_a_writer_can_choose_are_listed_as_the_library_reads_them() {
    let server = Server::new("fonts-listed");
    let listed = server.person("ada", &["family"]).send(Method::GET, paths::FONTS, "", Vec::new()).await;
    assert_eq!(listed.status, StatusCode::OK);
    let listing = String::from_utf8(listed.body).unwrap();
    let lora = letters_fonts::library::find_in(&listing, "lora").expect("a family of the library is listed");
    assert_eq!((lora.name, lora.italic), ("Lora", true));
}

#[tokio::test]
async fn a_face_is_fetched_for_whoever_writes_and_only_one_the_family_has() {
    let (fonts, asked) = library(false);
    let server = Server::with_fonts("fonts-face", fonts);
    let (ada, guest) = (server.person("ada", &["family"]), server.person("guest", &["friends"]));
    let id = ada.start("family", "", "Plan").await.id.expect("a document has an id");

    let face = ada.send(Method::GET, &paths::font_face(&id, "Lora", 700, true), "", Vec::new()).await;
    assert_eq!((face.status, face.body.as_slice()), (StatusCode::OK, FILE));
    let before = asked.load(Ordering::Relaxed);

    // Who asks, for which face, and what comes of it without the library hearing.
    let cases = [
        (&guest, "Lora", 400, false, StatusCode::FORBIDDEN, "a reader"),
        (&ada, "Lora", 450, false, StatusCode::NOT_FOUND, "a weight the library does not serve"),
        (&ada, "Lobster", 400, true, StatusCode::NOT_FOUND, "an italic the family lacks"),
        (&ada, "Calibri", 400, false, StatusCode::NOT_FOUND, "a family of a device"),
    ];
    for (who, family, weight, italic, expected, what) in cases {
        let answer = who.send(Method::GET, &paths::font_face(&id, family, weight, italic), "", Vec::new()).await;
        assert_eq!(answer.status, expected, "{what}");
    }
    assert_eq!(asked.load(Ordering::Relaxed), before, "the library was asked for none of them");
}

#[tokio::test]
async fn a_library_that_cannot_be_reached_is_said_to_be_so() {
    let server = Server::with_fonts("fonts-down", library(true).0);
    let ada = server.person("ada", &["family"]);
    let id = ada.start("family", "", "Plan").await.id.expect("a document has an id");
    let face = ada.send(Method::GET, &paths::font_face(&id, "Lora", 400, false), "", Vec::new()).await;
    assert_eq!(face.status, StatusCode::BAD_GATEWAY);
}
