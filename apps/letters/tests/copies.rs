//! Copies of a document, written from its file: another document, or the same text in another format.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Scratch, Writer};

use letters::copies::{self, Refused};
use letters::files::{self, Home};
use letters_format::Archive;
use letters_model::{Block, BlockKind, Document, Style};
use letters_ui::CopyFormat;

const EXPORTS: [CopyFormat; 4] = [CopyFormat::Word, CopyFormat::Pdf, CopyFormat::WebPage, CopyFormat::Markdown];

fn letter() -> Document {
    let blocks = vec![
        Block::text("h", BlockKind::Heading { level: 1 }, "Roof repairs"),
        Block::text("p", BlockKind::Paragraph, "Harland Roofing sent a quote."),
    ];
    Document { blocks, ..Document::default() }
}

/// A saved document in a scratch folder, and the folder.
fn saved(name: &str) -> (Scratch, PathBuf) {
    let scratch = Scratch::new(name);
    let source = scratch.0.join("Roof repairs.let");
    Writer::new(&letter(), Style::default()).save(&Home::new(&source), &letter());
    (scratch, source)
}

fn id(path: &Path) -> String {
    Archive::open(fs::File::open(path).unwrap()).unwrap().manifest().id.clone()
}

#[test]
fn a_copy_is_a_document_of_its_own_with_the_same_text() {
    let (scratch, source) = saved("copies-own");
    let output = copies::path(&scratch.0, "Roof repairs copy", CopyFormat::Letters).unwrap();
    let made = copies::write(&source, &output, CopyFormat::Letters, false).unwrap();

    assert_eq!(output, scratch.0.join("Roof repairs copy.let"));
    assert_eq!(files::open(&output, Style::default).unwrap().document, letter());
    assert_eq!(made, Some(id(&output)), "the shell is told which document it made");
    assert_ne!(id(&output), id(&source), "the copy does not pass for the document it was made from");
    assert_eq!(files::open(&source, Style::default).unwrap().document, letter(), "the original is left alone");
}

#[test]
fn an_export_is_a_file_in_its_format_that_holds_the_text() {
    // What each format's file starts with, and whether the text can be read from it as it is.
    let cases = [
        (CopyFormat::Word, "Roof repairs.docx", &b"PK"[..], false),
        (CopyFormat::Pdf, "Roof repairs.pdf", b"%PDF", false),
        (CopyFormat::WebPage, "Roof repairs.html", b"<!", true),
        (CopyFormat::Markdown, "Roof repairs.md", b"# Roof repairs", true),
    ];
    let (scratch, source) = saved("copies-export");
    for (format, name, start, readable) in cases {
        let output = copies::path(&scratch.0, "Roof repairs", format).unwrap();
        assert_eq!(output, scratch.0.join(name));
        assert_eq!(copies::write(&source, &output, format, false).unwrap(), None, "{name}: no document is made");
        let written = fs::read(&output).unwrap();
        assert!(written.starts_with(start), "{name}");
        if readable {
            assert!(String::from_utf8_lossy(&written).contains("Harland Roofing sent a quote."), "{name}");
        }
    }
}

#[test]
fn a_copy_of_an_older_version_holds_what_the_document_read_then() {
    let (scratch, source) = saved("copies-version");
    // The document was written on since the version that is asked for.
    let mut newer = letter();
    newer.blocks[1] = Block::text("p", BlockKind::Paragraph, "The roof is mended.");
    let (mut writer, home) = Writer::of(files::open(&source, Style::default).unwrap());
    writer.save(&home, &newer);
    let then = || (letter(), Style::default());

    let output = copies::path(&scratch.0, "As quoted", CopyFormat::Letters).unwrap();
    let made = copies::write_version(&source, then(), &output, CopyFormat::Letters, false).unwrap();
    assert_eq!(files::open(&output, Style::default).unwrap().document, letter());
    assert!(made.is_some_and(|made| made == id(&output) && made != id(&source)), "a document of its own");
    let archive = Archive::open(fs::File::open(&output).unwrap()).unwrap();
    assert!(!archive.names().any(|name| name.starts_with(letters_format::HISTORY)), "it starts with that text");
    assert_eq!(files::open(&source, Style::default).unwrap().document, newer, "the original is left alone");

    for format in [CopyFormat::WebPage, CopyFormat::Markdown] {
        let output = copies::path(&scratch.0, "As quoted", format).unwrap();
        copies::write_version(&source, then(), &output, format, false).unwrap();
        let written = fs::read_to_string(&output).unwrap();
        assert!(written.contains("Harland Roofing sent a quote."), "{format:?}: the version's text");
        assert!(!written.contains("The roof is mended."), "{format:?}: and not what was written since");
    }
}

#[test]
fn a_file_in_the_way_is_replaced_only_when_asked_and_a_document_never() {
    let (scratch, source) = saved("copies-taken");
    for format in CopyFormat::ALL {
        let output = copies::path(&scratch.0, "Other", format).unwrap();
        fs::write(&output, "kept").unwrap();

        let refused = copies::write(&source, &output, format, false).unwrap_err();
        assert!(matches!(refused, Refused::Taken { .. }), "{format:?}: {refused}");
        assert_eq!(refused.replaceable(), format != CopyFormat::Letters, "{format:?}");
        assert_eq!(fs::read(&output).unwrap(), b"kept", "{format:?}: nothing was written");

        let again = copies::write(&source, &output, format, true);
        let replaced = fs::read(&output).unwrap() != b"kept";
        assert_eq!((again.is_ok(), replaced), (EXPORTS.contains(&format), EXPORTS.contains(&format)), "{format:?}");
    }
}

#[test]
fn a_copy_that_cannot_be_made_says_why() {
    let (scratch, source) = saved("copies-refused");
    assert!(matches!(copies::path(&scratch.0, " /: ", CopyFormat::Pdf), Err(Refused::NoName)));
    assert_eq!(
        copies::path(&scratch.0, "Roof: repairs?", CopyFormat::Pdf).unwrap(),
        scratch.0.join("Roof repairs.pdf"),
        "characters a file name cannot hold are left out"
    );

    let cases = [
        ("a folder that is not there", source.clone(), scratch.0.join("gone").join("Copy.pdf")),
        ("a document that is not there", scratch.0.join("Missing.let"), scratch.0.join("Copy.pdf")),
    ];
    for (case, source, output) in cases {
        let refused = copies::write(&source, &output, CopyFormat::Pdf, false).unwrap_err();
        assert!(matches!(refused, Refused::Failed(_)) && !refused.replaceable(), "{case}: {refused}");
        assert!(!output.exists(), "{case}");
    }
}

#[test]
fn a_chosen_path_gets_the_ending_of_its_format() {
    let cases = [
        ("/out/Report", CopyFormat::Pdf, "/out/Report.pdf"),
        ("/out/Report.PDF", CopyFormat::Pdf, "/out/Report.PDF"),
        ("/out/Report.v2", CopyFormat::Word, "/out/Report.v2.docx"),
        ("/out/Report", CopyFormat::Letters, "/out/Report.let"),
    ];
    for (chosen, format, path) in cases {
        assert_eq!(copies::ending(chosen.into(), format), PathBuf::from(path));
    }
}
