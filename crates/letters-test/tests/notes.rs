//! Notes on a passage: the tools over selected text, a comment written on it and its mark on the page.

use std::ops::Range;

use letters_model::{Anchor, Block, BlockKind, Document, Mark, Note, NoteKind, Notes, Pos, Run, Selection};
use letters_render::{Item, Point};
use letters_test::{Found, Harness, Role};

/// A paragraph `a` about a written quote, a picture `g` and a paragraph `b`, written by Sam.
fn letter() -> Harness {
    let asset = Some("assets/roof.png".into());
    let figure =
        BlockKind::Figure { asset, src: None, alt: String::new(), width: 300.0, height: 150.0, numbered: false };
    let blocks = vec![
        Block::text("a", BlockKind::Paragraph, "Their written quote covers the slates."),
        Block::text("g", figure, "The north gable"),
        Block::text("b", BlockKind::Paragraph, "Reply by Friday."),
    ];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.ui().set_author("Sam Okafor");
    window
}

/// Where the written quote is in paragraph `a`.
const QUOTE: Range<usize> = 6..19;

/// The tools over the selection, and the names of the buttons on them.
fn tools(window: &mut Harness) -> Option<(Found, Vec<Found>)> {
    let bar = window.find(Role::Toolbar, "Selection")?;
    let on = |found: &Found| found.role == Role::Button && bar.bounds.contains(found.center());
    let buttons = window.controls().into_iter().filter(on).collect();
    Some((bar, buttons))
}

/// Writes a comment on the first place `passage` appears, and sends it.
fn comment(window: &mut Harness, passage: &str, text: &str) {
    window.select(passage).click(Role::Button, "Comment").type_text(text).key("Enter");
}

/// The comments the window keeps: the text each is about, and what it says.
fn kept(window: &mut Harness) -> Vec<(String, String)> {
    let notes = window.ui().notes().clone();
    let about = |note: &Note| {
        let text = window.editor().doc().block(&note.anchor.block).map(Block::plain_text).unwrap_or_default();
        note.anchor.range.clone().and_then(|range| text.get(range).map(str::to_owned)).unwrap_or_default()
    };
    let said = |note: &Note| note.content.iter().map(|run| run.text.as_str()).collect::<String>();
    notes.notes.iter().map(|note| (about(note), said(note))).collect()
}

/// Whether a frame marks a stretch of a block's text as commented: a line under it, from its first letter
/// to its last. The caret is left at the end of the stretch.
fn marked(window: &mut Harness, block: &str, range: Range<usize>) -> bool {
    let mut at = |offset: usize| {
        window.ui().select(Selection::caret(Pos::new(block, offset)));
        window.paint();
        window.ui().caret_rect().expect("the page shows a caret")
    };
    let (start, end) = (at(range.start), at(range.end));
    let under = |item: &Item| match item {
        Item::Fill { rect, .. } if rect.height == 1.0 => {
            let spans = (rect.x - start.x).abs() < 1.5 && (rect.x + rect.width - end.x).abs() < 1.5;
            spans && rect.y > start.y && rect.y < start.y + start.height + 4.0
        }
        _ => false,
    };
    window.paint().items.iter().any(under)
}

#[test]
fn selected_text_has_tools_over_it_to_link_it_or_comment_on_it() {
    let mut window = letter();
    window.ui().select(Selection::caret(Pos::new("a", 6)));
    assert!(tools(&mut window).is_none(), "a caret has nothing to link or comment on");

    window.select("written quote");
    let (bar, buttons) = tools(&mut window).expect("tools over the selection");
    assert_eq!(buttons.iter().map(|button| button.name.as_str()).collect::<Vec<_>>(), ["Link", "Comment"]);
    let line = window.ui().selection_ends()[0].expect("the selection starts in view");
    assert!(bar.bounds.y + bar.bounds.height <= line.y, "they are over the text, not on it: {:?}", bar.bounds);
    assert!(window.focused().is_some_and(|found| found.role == Role::Document), "and typing still goes to the page");

    // Escape puts them away, for this selection.
    window.key("Escape");
    assert!(tools(&mut window).is_none());
    assert!(window.editor().selection().is_some_and(|selection| !selection.is_caret()), "the text stays selected");
    window.select("Reply");
    let (_, buttons) = tools(&mut window).expect("another selection has them again");

    window.click_at(buttons[0].center()).assert_open(Role::Dialog, "Link");
}

#[test]
fn the_tools_wait_until_a_selection_has_been_dragged_out() {
    let mut window = letter();
    let mut at = |offset: usize| {
        window.ui().select(Selection::caret(Pos::new("a", offset)));
        window.paint();
        let caret = window.ui().caret_rect().expect("the page shows a caret");
        Point { x: caret.x, y: caret.y + caret.height / 2.0 }
    };
    let (from, to) = (at(QUOTE.start), at(QUOTE.end));
    window.press_at(from).hover_at(to);
    assert!(window.editor().selection().is_some_and(|selection| !selection.is_caret()), "text is selected");
    assert!(tools(&mut window).is_none(), "nothing comes between the pointer and the text it is selecting");
    window.release_at(to);
    assert!(tools(&mut window).is_some());
}

#[test]
fn a_selected_picture_has_no_tools_over_it() {
    let mut window = letter();
    let place = |item: &Item| match item {
        Item::Image { rect, .. } => Some(*rect),
        _ => None,
    };
    let picture = window.paint().items.iter().find_map(place).expect("a picture");
    window.click_at(Point { x: picture.x + picture.width / 2.0, y: picture.y + picture.height / 2.0 });
    assert_eq!(window.editor().picture().map(|figure| figure.as_str()), Some("g"));
    assert!(tools(&mut window).is_none());
}

#[test]
fn a_comment_is_written_under_its_passage_and_kept_as_a_note() {
    // What is typed, how the field is left, and what is kept of it.
    let cases = [
        ("Has anyone **chased** it?", "Enter", Some("Has anyone chased it?")),
        ("Thanks \u{1f64f}", "Send", Some("Thanks \u{1f64f}")),
        ("   ", "Enter", None),
        ("Never mind", "Escape", None),
    ];
    for (typed, left, said) in cases {
        let mut window = letter();
        window.select("written quote").click(Role::Button, "Comment");
        let field = window.find(Role::TextInput, "Comment").expect("a field for the comment");
        let line = window.ui().selection_ends()[1].expect("the selection ends in view");
        assert!(field.focused && field.bounds.y >= line.y + line.height, "{typed:?}: under its passage, ready");
        assert!(tools(&mut window).is_none(), "{typed:?}: the tools have done their part");

        let edits = window.ui().edits();
        window.type_text(typed);
        match left {
            "Send" => window.click(Role::Button, "Send"),
            key => window.key(key),
        };
        window.assert_closed(Role::Dialog, "New comment");
        let expected: Vec<_> = said.map(|said| ("written quote".to_owned(), said.to_owned())).into_iter().collect();
        assert_eq!(kept(&mut window), expected, "{typed:?}, {left}");
        assert_eq!(window.ui().edits() > edits, said.is_some(), "{typed:?}: a comment is something to save");
        assert!(window.focused().is_some_and(|found| found.role == Role::Document), "{typed:?}, {left}");
        assert_eq!(marked(&mut window, "a", QUOTE), said.is_some(), "{typed:?}, {left}");
    }
}

#[test]
fn a_comment_is_signed_and_reads_the_marks_markdown_writes() {
    let mut window = letter();
    comment(&mut window, "written quote", "due **Friday**, see [their email](https://a.example/m)");
    comment(&mut window, "Reply", "ok");
    let notes = window.ui().notes().clone();
    let [first, second] = &notes.notes[..] else { panic!("two notes: {notes:?}") };
    assert_eq!((first.author.as_str(), &first.kind, first.resolved), ("Sam Okafor", &NoteKind::Comment, false));
    assert_ne!(first.id, second.id);
    let bold: Vec<&str> =
        first.content.iter().filter(|run| run.marks.contains(&Mark::Bold)).map(|run| run.text.as_str()).collect();
    let links: Vec<_> =
        first.content.iter().filter_map(|run| Some((run.text.as_str(), run.link.as_deref()?))).collect();
    assert_eq!((bold, links), (vec!["Friday"], vec![("their email", "https://a.example/m")]));
    // What is kept is what the file takes.
    assert_eq!(Notes::from_json(&notes.to_json()).map(|read| read.notes.len()).ok(), Some(2));
}

#[test]
fn a_note_stays_with_its_text_as_the_document_changes() {
    let mut window = letter();
    comment(&mut window, "written quote", "Chased?");
    window.ui().select(Selection::caret(Pos::new("a", 0)));
    window.type_text("So: ");
    assert_eq!(kept(&mut window), [("written quote".to_owned(), "Chased?".to_owned())]);
    assert!(marked(&mut window, "a", QUOTE.start + 4..QUOTE.end + 4), "the mark moved with the words");

    // Handed to another window with the same document, the notes are where they were.
    let (notes, doc) = (window.ui().notes().clone(), window.editor().doc().clone());
    let mut other = Harness::open(doc);
    other.ui().set_notes(notes);
    assert!(marked(&mut other, "a", QUOTE.start + 4..QUOTE.end + 4));
}

#[test]
fn the_notes_of_a_document_are_marked_on_its_page_while_they_are_open() {
    let note = |id: &str, anchor: Anchor, resolved: bool| Note {
        id: id.into(),
        kind: NoteKind::Comment,
        anchor,
        author: "Alice Moreau".into(),
        created: "2026-11-10T08:00:00Z".into(),
        resolved,
        content: vec![Run::plain("Called on Tuesday.")],
        replies: Vec::new(),
        extra: Default::default(),
    };
    let notes = vec![
        note("open", Anchor::range("a", QUOTE), false),
        note("done", Anchor::range("b", 0..5), true),
        // A passage that is gone, and a note on a whole block, have no stretch to mark.
        note("gone", Anchor::range("nowhere", 0..5), false),
        note("whole", Anchor::block("b"), false),
    ];
    let mut window = letter();
    window.ui().set_notes(Notes { notes, ..Notes::default() });
    assert!(marked(&mut window, "a", QUOTE));
    assert!(!marked(&mut window, "b", 0..5), "a resolved comment leaves no mark");
    assert_eq!(window.ui().notes().notes.len(), 4, "every note is kept for the file");
}
