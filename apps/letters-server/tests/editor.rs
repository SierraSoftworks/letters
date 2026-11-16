//! A document in the browser's editor: the page it is sent in, and the files that page runs.

mod common;

use std::io::Write;

use axum::http::StatusCode;
use common::{Server, URL, let_in, link};
use letters_api::{Access, Action, Opened, Role, paths};
use letters_server::pages::editor::{Editor, FALLBACK, File};

/// The script a page starts with, which brotli makes no smaller.
const SCRIPT: &[u8] = b"import init from './letters.js';";
/// A backup face, as the server's owner put it in the state directory.
const FACE: &[u8] = b"OTTO, and the rest of a face";

/// The editor's module: large, and all alike.
fn module() -> Vec<u8> {
    [b"\0asm".as_slice(), &[7; 4096]].concat()
}

fn brotli(bytes: &[u8]) -> Vec<u8> {
    let mut small = Vec::new();
    brotli::CompressorWriter::new(&mut small, 4096, 5, 22).write_all(bytes).unwrap();
    small
}

/// An editor as a build compiles it in: what brotli makes smaller is in brotli.
fn editor() -> Editor {
    Editor::of(vec![
        File { name: "start.js".into(), bytes: SCRIPT.into(), brotli: false },
        File { name: "letters_bg.wasm".into(), bytes: brotli(&module()).into(), brotli: true },
    ])
}

/// The JSON a page carries in the script `id`.
fn carried(page: &str, id: &str) -> String {
    let from = format!("<script type=\"application/json\" id=\"{id}\">");
    let after = page.split_once(&from).unwrap_or_else(|| panic!("the page carries {id}")).1;
    after.split_once("</script>").expect("the script ends").0.to_owned()
}

#[tokio::test]
async fn a_document_opens_in_the_editor_with_what_the_editor_starts_from() {
    let server = Server::editing("editor-page", editor());
    let base = server.app.editor.as_ref().unwrap().base.clone();
    let mut ben = server.reader("ben", &["family"]);
    let id = ben.pages().start("family", "", "Plan").await.id.unwrap();

    let raw = ben.get(&paths::editor_page(&id)).await;
    assert_eq!(raw.status, StatusCode::OK, "{}", raw.text());
    let page = raw.text();
    assert!(page.contains(&format!("<base href=\"{base}\"")), "the page says where the editor's files are");
    assert!(page.contains("<title>Plan</title>"));
    let opened: Opened = serde_json::from_str(&carried(&page, "letters-opened")).expect("how the document is opened");
    assert_eq!((opened.id.as_str(), opened.name.as_str()), (id.as_str(), "Plan"));
    assert_eq!(opened.socket, paths::socket(&id), "the editor is told where the document's socket is");
    assert!(carried(&page, "letters-document").contains("\"blocks\""), "the document is in the page to open");

    // The page may run the editor and reach its own server, and nothing else.
    let policy = raw.header("content-security-policy");
    assert!(policy.contains("default-src 'none'") && policy.contains("'wasm-unsafe-eval'"), "{policy}");
    assert!(policy.contains(&URL.replace("https", "wss")), "the socket is let through: {policy}");
    assert_eq!(raw.header("cache-control"), "no-store", "the page is made anew for whoever asks");

    let mut stranger = server.reader("stranger", &[]);
    let hidden = stranger.get(&paths::editor_page(&id)).await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    assert!(!hidden.text().contains("Plan"));
}

#[tokio::test]
async fn whoever_a_link_for_a_reader_leads_in_is_sent_on_to_the_page_to_read() {
    let server = Server::linking_to("editor-links", editor());
    let ada = server.person("ada", &[]);
    let id = ada.start("home", "", "Plan").await.id.unwrap();
    // What a link lets its reader do, and whether the document is then shown to read and not in the editor.
    for (role, reads) in [(Role::Reader, true), (Role::Commenter, false), (Role::Editor, false)] {
        let (mut guest, _) = let_in(&server, &ada, &id, &link("Guests", role)).await;
        let page = guest.get(&paths::editor_page(&id)).await;
        let sent = (page.status, page.location().to_owned(), page.text().contains("letters-opened"));
        let expected = match reads {
            true => (StatusCode::SEE_OTHER, paths::read_page(&id), false),
            false => (StatusCode::OK, String::new(), true),
        };
        assert_eq!(sent, expected, "a link for a {role:?}");
        // Being sent on is no view of the document. The page to read is one, as the editor's is.
        let views = async || {
            let rows: Vec<Access> = ada.get(&paths::access(&id)).await.unwrap();
            rows.iter().filter(|row| (row.role, row.action, row.refused) == (role, Action::Open, false)).count()
        };
        assert_eq!(views().await, usize::from(!reads), "a link for a {role:?}, sent its page");
        assert_eq!(guest.get(&paths::read_page(&id)).await.status, StatusCode::OK);
        assert_eq!(views().await, 1, "a link for a {role:?}, having read");
    }
}

#[tokio::test]
async fn the_editors_files_are_sent_as_a_browser_reads_them_and_kept_by_it() {
    let server = Server::editing("editor-files", editor());
    let base = server.app.editor.as_ref().unwrap().base.clone();
    let faces = server.scratch.0.join("state").join(FALLBACK);
    std::fs::create_dir_all(&faces).unwrap();
    std::fs::write(faces.join("NotoSansThai.ttf"), FACE).unwrap();
    std::fs::write(server.scratch.0.join("state/secret"), "not a face").unwrap();
    // Nobody is signed in: the files are the same for everyone.
    let mut browser = server.visitor();

    // The file asked for, what the browser says it reads, and the encoding, type and bytes it is sent.
    let packed = brotli(&module());
    let found: [(&str, &str, &str, &str, &[u8]); 6] = [
        ("start.js", "gzip, br", "", "text/javascript", SCRIPT),
        ("letters_bg.wasm", "gzip, deflate, br, zstd", "br", "application/wasm", &packed),
        ("letters_bg.wasm", "gzip, deflate", "", "application/wasm", &module()),
        ("letters_bg.wasm", "br;q=0, gzip", "", "application/wasm", &module()),
        ("letters_bg.wasm", "", "", "application/wasm", &module()),
        ("fallback/NotoSansThai.ttf", "br", "", "font/ttf", FACE),
    ];
    for (name, accepts, encoding, kind, bytes) in found {
        let raw = browser.fetch(&format!("{base}{name}"), accepts).await;
        assert_eq!(raw.status, StatusCode::OK, "{name} for {accepts:?}");
        assert_eq!(raw.header("content-encoding"), encoding, "{name} for {accepts:?}");
        assert!(raw.header("content-type").starts_with(kind), "{name}: {}", raw.header("content-type"));
        assert!(raw.header("cache-control").contains("immutable"), "{name} is fetched once");
        assert!(raw.body == bytes, "{name} for {accepts:?}");
    }

    let missing = [
        format!("{base}nothing.js"),
        // Another build's editor.
        "/assets/not-this-one/start.js".to_owned(),
        format!("{base}fallback/NotoSansLao.ttf"),
        format!("{base}fallback/../secret"),
        format!("{base}fallback/..%2Fsecret"),
        format!("{base}fallback/"),
    ];
    for path in missing {
        let raw = browser.fetch(&path, "br").await;
        assert_eq!(raw.status, StatusCode::NOT_FOUND, "{path}");
    }
}
