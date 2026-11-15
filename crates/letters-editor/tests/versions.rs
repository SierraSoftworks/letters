//! The document's history through the editor: a version read back, restored and named.

mod common;

use common::Harness;
use letters_editor::Command;
use letters_model::{Revision, TagKind};

/// A line that was written on twice, with the revision between the two and the caret at its end.
fn written() -> (Harness, Revision) {
    let mut editor = Harness::new("p Slates|\np Lead");
    editor.type_text(" first");
    let then = editor.editor.model().revision();
    editor.platform.now += 5_000;
    editor.type_text(" now");
    (editor, then)
}

#[test]
fn a_version_is_read_back_while_the_document_stays_as_it_is() {
    let (mut editor, then) = written();
    let shown = editor.state();
    let (doc, style) = editor.editor.document_at(&then).expect("a revision of this history");
    assert_eq!(doc.blocks[0].plain_text(), "Slates first");
    assert_eq!(&style, editor.editor.model().style());
    assert_eq!(editor.state(), shown);
    assert!(!editor.editor.timeline().is_empty(), "the steps it was written in are listed");
}

#[test]
fn restoring_a_version_is_one_step_that_undo_takes_back() {
    let (mut editor, then) = written();
    let shown = editor.state();
    assert!(editor.editor.applies(&Command::Restore(then.clone())));
    editor.run(Command::Restore(then.clone()));
    assert_eq!(editor.state(), "p Slates first|\np Lead", "the caret stays with the text that is still there");

    editor.key("Mod+z");
    assert_eq!(editor.state(), shown, "one step back, with the caret where it was");
    editor.key("Mod+z");
    assert_eq!(editor.state(), "p Slates first|\np Lead", "and then what was typed before");
    editor.key("Mod+Shift+z");
    editor.key("Mod+Shift+z");
    assert_eq!(editor.state(), "p Slates first|\np Lead", "redo restores again");
}

#[test]
fn restoring_what_the_document_reads_already_is_no_step() {
    let (mut editor, _) = written();
    let (now, shown) = (editor.editor.model().revision(), editor.state());
    assert!(!editor.editor.applies(&Command::Restore(now.clone())));
    editor.run(Command::Restore(now));
    editor.key("Mod+z");
    assert_eq!(editor.state(), "p Slates first|\np Lead", "undo takes back the typing: {shown}");
}

#[test]
fn a_version_is_named_and_its_name_taken_off_without_an_undo_step() {
    let (mut editor, then) = written();
    let names = |editor: &mut Harness| -> Vec<(TagKind, Option<String>)> {
        let points = editor.editor.timeline();
        points.iter().flat_map(|point| &point.tags).map(|tag| (tag.kind, tag.name.clone())).collect()
    };
    editor.editor.name_version(&then, Some("Sent to builder")).expect("a revision of this history");
    assert_eq!(names(&mut editor), [(TagKind::Named, Some("Sent to builder".to_owned()))]);
    editor.key("Mod+z");
    assert_eq!(editor.state(), "p Slates first|\np Lead", "undo takes back typing, not the name");
    assert_eq!(names(&mut editor).len(), 1);

    editor.editor.name_version(&then, None).expect("a revision of this history");
    assert_eq!(names(&mut editor), []);
}
