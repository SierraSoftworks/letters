mod common;

use std::io::Cursor;

use common::{bytes, open, package, rezip};
use letters_format::{Archive, DOCUMENT, Error, MIMETYPE, NOTES, STYLE, write_over};
use letters_model::{Anchor, Note, NoteKind, Notes, Run, Style};

fn note() -> Note {
    Note {
        id: "n1".into(),
        kind: NoteKind::Comment,
        anchor: Anchor::block("p"),
        author: "Ana".into(),
        created: "2026-11-14T20:00:00Z".into(),
        resolved: false,
        content: vec![Run::plain("Warmer?")],
        replies: Vec::new(),
        extra: Default::default(),
    }
}

/// How each entry of a file is compressed, by name.
fn compression(file: &[u8]) -> Vec<(String, zip::CompressionMethod)> {
    let mut zip = zip::ZipArchive::new(Cursor::new(file)).unwrap();
    (0..zip.len())
        .map(|i| {
            let entry = zip.by_index(i).unwrap();
            (entry.name().unwrap().to_string(), entry.compression())
        })
        .collect()
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

#[test]
fn style_and_notes_travel_as_their_entries() {
    let mut package = package();
    assert_eq!(package.style().unwrap(), Style::default(), "a file without a style takes the built-in one");
    assert_eq!(package.notes().unwrap(), Notes::default());

    let mut style = Style::default();
    style.types.get_mut("body").unwrap().size = 12.5;
    let notes = Notes { notes: vec![note()], ..Notes::default() };
    package.set_style(&style);
    package.set_notes(&notes);

    let mut archive = open(&bytes(&package));
    assert_eq!(archive.style().unwrap(), style);
    assert_eq!(archive.notes().unwrap(), notes);
    assert_eq!(archive.package().unwrap(), package);
}

#[test]
fn a_broken_style_or_notes_entry_is_refused_by_name() {
    let mut package = package();
    package.entries.insert(STYLE.into(), br#"{"format":1,"page":{"width":0}}"#.to_vec());
    package.entries.insert(NOTES.into(), br#"{"format":1,"notes":[{}]}"#.to_vec());
    let mut archive = open(&bytes(&package));
    assert!(matches!(archive.style(), Err(Error::Style(error)) if error.path == "page.width"));
    assert!(matches!(archive.notes(), Err(Error::Notes(error)) if error.path == "notes[0].id"));
    assert!(matches!(package.style(), Err(Error::Style(_))));
    assert!(matches!(package.notes(), Err(Error::Notes(_))));
}

#[test]
fn a_save_over_an_older_copy_recompresses_only_what_changed() {
    let mut package = package();
    package.entries.insert("assets/big.bin".into(), vec![7; 4096]);
    // The older copy is stored uncompressed, so an entry that is still uncompressed afterwards was copied.
    let older = rezip(&bytes(&package), |_, content| Some(content));
    package.entries.insert("history/oplog".into(), vec![1, 2, 3, 4]);
    package.entries.insert("assets/new.png".into(), vec![1; 64]);
    // The big asset is no longer in memory, and is carried over from the older copy all the same.
    package.entries.remove("assets/big.bin");

    let mut out = Cursor::new(Vec::new());
    write_over(&mut out, &package, &mut open(&older), |_| true).unwrap();
    let saved = out.into_inner();

    let copied = |name: &str| compression(&saved).contains(&(name.to_string(), zip::CompressionMethod::Stored));
    assert!(copied("assets/big.bin") && copied("assets/0a1b.png"));
    assert!(!copied("history/oplog") && !copied("assets/new.png"));

    package.entries.insert("assets/big.bin".into(), vec![7; 4096]);
    assert_eq!(open(&saved).package().unwrap(), package);
}

#[test]
fn a_save_over_an_older_copy_drops_what_it_is_told_to() {
    let older = bytes(&package());
    let mut package = package();
    package.entries.remove("assets/0a1b.png");

    let mut out = Cursor::new(Vec::new());
    write_over(&mut out, &package, &mut open(&older), |name| !name.starts_with("assets/")).unwrap();
    assert_eq!(open(out.get_ref()).package().unwrap(), package);
}
