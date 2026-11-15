//! Conflict copies: found beside a document, and merged into it with a save.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Scratch, Writer};
use letters::conflicts;
use letters::files::{self, Exchange, Home};
use letters_model::{Block, BlockKind, Document, NoteKind, Notes, Style};

/// A document of paragraphs with ids `p0`, `p1` and so on.
fn paragraphs(texts: &[&str]) -> Document {
    let block = |(n, text): (usize, &&str)| Block::text(format!("p{n}").as_str(), BlockKind::Paragraph, *text);
    Document { blocks: texts.iter().enumerate().map(block).collect(), ..Document::default() }
}

/// Saves a new document of `texts` in `folder` under `name`.
fn create(folder: &Scratch, name: &str, texts: &[&str]) -> PathBuf {
    let path = folder.0.join(name);
    Writer::new(&paragraphs(texts), Style::default()).save(&Home::new(&path), &paragraphs(texts));
    path
}

fn writer(path: &Path) -> (Writer, Home) {
    Writer::of(files::open(path, Style::default).unwrap())
}

/// Leaves a copy of the document at `path` under `name`, as a sync tool does when another device saved
/// it apart: the same document, where it reads as `texts`.
fn conflict(path: &Path, name: &str, texts: &[&str]) -> PathBuf {
    let copy = path.with_file_name(name);
    fs::copy(path, &copy).unwrap();
    let (mut other, home) = writer(&copy);
    other.save(&home, &paragraphs(texts));
    copy
}

/// Saves `writer` to `home` as an autosave does, with `others` to merge on the way, and gives the editor
/// what came of it.
fn exchange(writer: &mut Writer, home: &Home, others: &[PathBuf]) -> Exchange {
    let seen = writer.model.revision();
    let exchange = home.exchange(&mut writer.copy, &writer.model.save(), Notes::default(), &seen, others).unwrap();
    if let Some(news) = &exchange.news {
        writer.model.merge(news).unwrap();
    }
    exchange
}

fn found(path: &Path, id: &str) -> Vec<String> {
    let name = |copy: conflicts::Copy| copy.path.file_name().unwrap().to_string_lossy().into_owned();
    conflicts::beside(path, id).into_iter().map(name).collect()
}

#[test]
fn copies_are_found_by_their_names_and_by_the_document_they_say_they_are() {
    let folder = Scratch::new("conflicts-found");
    let path = create(&folder, "Report.let", &["One", "Two"]);
    let id = writer(&path).0.copy.id.clone();

    conflict(&path, "Report (conflicted copy 2026-11-15 desk).let", &["One", "Two, at the desk"]);
    conflict(&path, "Report.sync-conflict-20261115-101500-ABCDEFG.let", &["One, on the phone", "Two"]);
    // A copy made by hand is a document of its own, whatever it holds.
    conflict(&path, "Report copy.let", &["One", "Two, by hand"]);
    // A name proves nothing: another document, a file that is still arriving, a copy of another file.
    create(&folder, "Report (conflicted copy 2026-11-15 other).let", &["Another document"]);
    fs::write(folder.0.join("Report (conflicted copy 2026-11-15 torn).let"), b"PK\x03\x04 and no more").unwrap();
    conflict(&path, "Minutes (conflicted copy 2026-11-15 desk).let", &["One", "Two"]);

    let expected = ["Report (conflicted copy 2026-11-15 desk).let", "Report.sync-conflict-20261115-101500-ABCDEFG.let"];
    assert_eq!(found(&path, &id), expected);
    assert!(found(&folder.0.join("Gone.let"), &id).is_empty(), "nothing lies beside a file that is not there");
}

#[test]
fn a_copy_is_a_conflict_while_it_holds_what_the_document_lacks() {
    let folder = Scratch::new("conflicts-adds");
    let path = create(&folder, "Report.let", &["One", "Two"]);
    let (mut ada, home) = writer(&path);
    conflict(&path, "Report (conflicted copy 2026-11-15 same).let", &["One", "Two"]);
    let edited = conflict(&path, "Report (conflicted copy 2026-11-15 desk).let", &["One", "Two, at the desk"]);
    ada.save(&home, &paragraphs(&["One, says Ada", "Two"]));

    let adds = |ada: &Writer| -> Vec<bool> {
        conflicts::beside(&path, &ada.copy.id).iter().map(|copy| copy.adds_to(&ada.model)).collect()
    };
    assert_eq!(adds(&ada), [true, false], "only the copy that was written in is a conflict");
    exchange(&mut ada, &home, &[edited]);
    assert_eq!(adds(&ada), [false, false], "a copy that was merged is none");
}

#[test]
fn a_copy_merged_with_a_save_brings_its_words_and_is_only_read() {
    let folder = Scratch::new("conflicts-merge");
    let path = create(&folder, "Report.let", &["One", "Two"]);
    let (mut ada, home) = writer(&path);
    let copy = conflict(&path, "Report (conflicted copy 2026-11-15 desk).let", &["One", "Two, at the desk"]);
    let held = fs::read(&copy).unwrap();
    ada.write(&paragraphs(&["One, says Ada", "Two"]));

    let saved = exchange(&mut ada, &home, std::slice::from_ref(&copy));

    let both = paragraphs(&["One, says Ada", "Two, at the desk"]);
    assert!(saved.merged.iter().all(|merged| merged.result.is_ok()), "{:?}", saved.merged);
    assert_eq!(ada.model.doc(), &both, "the editor is handed what the copy held");
    assert_eq!(files::open(&path, Style::default).unwrap().document, both, "and the file holds it");
    assert_eq!(fs::read(&copy).unwrap(), held, "what becomes of the copy is the person's to say");
    // Merging it once more changes nothing.
    let again = exchange(&mut ada, &home, &[copy]);
    assert!(again.news.is_none() && again.merged[0].result.is_ok());
}

#[test]
fn words_changed_on_both_sides_are_left_as_a_conflict_to_review() {
    let folder = Scratch::new("conflicts-clash");
    let path = create(&folder, "Report.let", &["One", "Two"]);
    let (mut ada, home) = writer(&path);
    let copy = conflict(&path, "Report (conflicted copy 2026-11-15 desk).let", &["One", "Three"]);
    ada.write(&paragraphs(&["One", "Four"]));

    let saved = exchange(&mut ada, &home, &[copy]);

    let notes = saved.notes.expect("the editor is handed the note");
    let conflicts = notes.notes.iter().filter(|note| matches!(note.kind, NoteKind::Conflict { .. }));
    assert_eq!(conflicts.map(|note| note.anchor.block.as_str()).collect::<Vec<_>>(), ["p1"]);
}

#[test]
fn a_file_that_cannot_be_merged_is_said_and_does_not_hold_the_save_back() {
    let folder = Scratch::new("conflicts-refused");
    let path = create(&folder, "Report.let", &["One"]);
    let (mut ada, home) = writer(&path);
    // (the file asked for, why it cannot be merged)
    let other = create(&folder, "Minutes.let", &["Another document"]);
    let torn = folder.0.join("Torn.let");
    fs::write(&torn, b"PK\x03\x04 and no more").unwrap();
    let files = [other, torn, folder.0.join("Gone.let")];
    ada.write(&paragraphs(&["One, says Ada"]));

    let saved = exchange(&mut ada, &home, &files);

    for merged in &saved.merged {
        assert!(merged.result.is_err(), "{}", merged.path.display());
    }
    assert_eq!(saved.merged.len(), files.len());
    let written = paragraphs(&["One, says Ada"]);
    assert_eq!((ada.model.doc(), &files::open(&path, Style::default).unwrap().document), (&written, &written));
}
