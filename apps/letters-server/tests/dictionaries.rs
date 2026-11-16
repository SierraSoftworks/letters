//! The spelling dictionaries of other languages that a server's owner put into its state folder: which
//! languages it says it has, which dictionary it sends for a language, and how a browser keeps one.

mod common;

use std::io::Read;

use axum::http::{Method, StatusCode};
use common::{Browser, Raw, Server};
use letters_api::paths;

/// A server whose owner keeps `files` in the dictionaries folder: each a name and what it holds.
fn holding(name: &str, files: &[(&str, &[u8])]) -> Server {
    let server = Server::new(name);
    let folder = server.scratch.0.join("state/dictionaries");
    std::fs::create_dir_all(&folder).unwrap();
    for (name, bytes) in files {
        std::fs::write(folder.join(name), bytes).unwrap();
    }
    server
}

/// What an answer holds as text, out of brotli where it says it is in it.
fn text(raw: &Raw) -> String {
    if raw.header("content-encoding") != "br" {
        return raw.text();
    }
    let mut whole = String::new();
    brotli::Decompressor::new(&raw.body[..], 4096).read_to_string(&mut whole).expect("brotli of UTF-8");
    whole
}

async fn asked(browser: &mut Browser, path: &str, said: &[(&str, &str)]) -> Raw {
    browser.send(Method::GET, path, "", Vec::new(), said).await
}

/// German as an older dictionary is kept, in Latin-1, and French in UTF-8.
const GERMAN: [(&str, &[u8]); 2] =
    [("de_DE.aff", b"SET ISO8859-1\nTRY esn\n"), ("de_DE.dic", b"2\nStra\xdfe\n\xdcbung\n")];
const FRENCH: [(&str, &[u8]); 2] = [("fr.aff", "SET UTF-8\n".as_bytes()), ("fr.dic", "1\nchâteau\n".as_bytes())];

#[tokio::test]
async fn a_language_is_answered_with_the_dictionary_that_best_serves_it() {
    let austrian: [(&str, &[u8]); 2] = [("de_AT.aff", b"SET UTF-8\n"), ("de_AT.dic", "1\nJänner\n".as_bytes())];
    // English is the editor's own, and half a dictionary is none.
    let others: [(&str, &[u8]); 3] = [("en_GB.aff", b"SET UTF-8\n"), ("en_GB.dic", b"1\ncolour\n"), ("nl.aff", b"")];
    let files = [GERMAN.as_slice(), &FRENCH, &austrian, &others].concat();
    let server = holding("dictionaries-best", &files);
    // Nobody is signed in: the lists are anyone's to have.
    let mut anyone = server.visitor();
    let told = asked(&mut anyone, paths::DICTIONARIES, &[]).await;
    assert_eq!(told.json::<Vec<String>>(), ["de-AT", "de-DE", "fr"]);

    // The language asked for, and a word of the dictionary that serves it.
    let cases = [
        ("de-DE", Some("Straße")),
        ("de-AT", Some("Jänner")),
        ("de", Some("Straße")),
        ("de-CH", Some("Straße")),
        ("fr-CA", Some("château")),
        ("fr", Some("château")),
        ("es", None),
        ("en-GB", None),
        ("nl", None),
        ("..", None),
    ];
    for (tag, word) in cases {
        let words = asked(&mut anyone, &paths::dictionary_dic(tag), &[]).await;
        let rules = asked(&mut anyone, &paths::dictionary_aff(tag), &[]).await;
        match word {
            Some(word) => {
                assert_eq!((words.status, rules.status), (StatusCode::OK, StatusCode::OK), "{tag}");
                assert!(text(&words).contains(word), "{tag}: {:?}", text(&words));
                assert!(text(&rules).contains("SET"), "{tag}: its rules come with it");
            }
            None => assert_eq!((words.status, rules.status), (StatusCode::NOT_FOUND, StatusCode::NOT_FOUND), "{tag}"),
        }
    }
    assert_eq!(asked(&mut anyone, "/api/v1/dictionaries/de/txt", &[]).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_browser_keeps_a_dictionary_until_its_file_changes() {
    let server = holding("dictionaries-kept", &GERMAN);
    let (mut browser, words) = (server.visitor(), paths::dictionary_dic("de"));
    // It is sent small to a browser that reads brotli, and is the same text to one that does not.
    let plain = asked(&mut browser, &words, &[]).await;
    let small = asked(&mut browser, &words, &[("accept-encoding", "gzip, br")]).await;
    assert_eq!((plain.header("content-encoding"), small.header("content-encoding")), ("", "br"));
    assert_eq!(text(&plain), text(&small));
    assert!(plain.header("content-type").contains("utf-8"));

    // The browser asks again each time, and is sent nothing while the file is what it has.
    let had = plain.header("etag").to_owned();
    assert!(!had.is_empty() && plain.header("cache-control").contains("no-cache"));
    let again = asked(&mut browser, &words, &[("if-none-match", &had)]).await;
    assert_eq!((again.status, again.body.len()), (StatusCode::NOT_MODIFIED, 0));

    // The owner puts another list there.
    std::fs::write(server.scratch.0.join("state/dictionaries/de_DE.dic"), b"3\nStra\xdfe\n\xdcbung\nZug\n").unwrap();
    let changed = asked(&mut browser, &words, &[("if-none-match", &had)]).await;
    assert_eq!(changed.status, StatusCode::OK);
    assert!(changed.header("etag") != had && text(&changed).contains("Zug"));
}
