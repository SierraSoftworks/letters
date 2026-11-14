use std::io::Cursor;

use letters_format::{Archive, DOCUMENT, Error, MIMETYPE, Package, write};
use letters_model::{Block, BlockKind, Document};

fn package() -> Package {
    let document = Document { blocks: vec![Block::text("p", BlockKind::Paragraph, "Hello")], ..Document::default() };
    let entries = [
        ("style.json", b"{}".to_vec()),
        ("assets/0a1b.png", vec![0x89, b'P', b'N', b'G']),
        ("history/oplog", vec![1, 2, 3]),
    ];
    Package {
        id: "8f14e45f-ceea-467f-a0e6-5b1d3c2a9f00".into(),
        created: "2026-11-14T20:00:00Z".into(),
        document,
        entries: entries.into_iter().map(|(name, bytes)| (name.to_string(), bytes)).collect(),
    }
}

fn bytes(package: &Package) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    write(&mut out, package).unwrap();
    out.into_inner()
}

/// Rebuilds a file, passing each entry through `edit`, which may change or drop it.
fn rezip(file: &[u8], edit: impl Fn(&str, Vec<u8>) -> Option<Vec<u8>>) -> Vec<u8> {
    use std::io::{Read, Write};
    let mut zip = zip::ZipArchive::new(Cursor::new(file)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let name = entry.name().unwrap().to_string();
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        if let Some(content) = edit(&name, content) {
            out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            out.write_all(&content).unwrap();
        }
    }
    out.finish().unwrap().into_inner()
}

#[test]
fn a_package_survives_the_round_trip() {
    let package = package();
    let mut archive = Archive::open(Cursor::new(bytes(&package))).unwrap();
    assert_eq!(archive.manifest().id, package.id);
    assert_eq!(archive.package().unwrap(), package);
}

#[test]
fn the_same_package_always_gives_the_same_bytes() {
    assert_eq!(bytes(&package()), bytes(&package()));
}

#[test]
fn the_mimetype_is_readable_from_the_first_bytes() {
    let file = bytes(&package());
    let head = String::from_utf8_lossy(&file[..100]);
    assert!(head.contains("mimetype") && head.contains(MIMETYPE), "{head}");
}

#[test]
fn a_file_without_history_is_still_a_document() {
    let file = rezip(&bytes(&package()), |name, content| (!name.starts_with("history/")).then_some(content));
    // The manifest still lists the history, so only reading that entry fails.
    let mut archive = Archive::open(Cursor::new(file)).unwrap();
    assert_eq!(archive.document().unwrap(), package().document);
}

#[test]
fn damaged_files_are_refused() {
    let good = bytes(&package());
    let tampered = rezip(&good, |name, content| Some(if name == DOCUMENT { b"{}".to_vec() } else { content }));
    let not_letters = rezip(&good, |name, content| (name != "mimetype").then_some(content));
    let no_manifest = rezip(&good, |name, content| (name != "manifest.json").then_some(content));
    let newer = rezip(&good, |name, content| {
        Some(if name == "manifest.json" { br#"{"format": 2}"#.to_vec() } else { content })
    });

    let open = |file: &[u8]| Archive::open(Cursor::new(file.to_vec())).and_then(|mut a| a.document());
    assert!(matches!(open(&good[..good.len() / 2]), Err(Error::Zip(_))));
    assert!(matches!(open(b"plain text"), Err(Error::Zip(_))));
    assert!(matches!(open(&tampered), Err(Error::HashMismatch(_))));
    assert!(matches!(open(&not_letters), Err(Error::NotLetters)));
    assert!(matches!(open(&no_manifest), Err(Error::Missing(_))));
    assert!(matches!(open(&newer), Err(Error::NewerFormat(2))));
}
