//! Notes on a passage: the tools over selected text, a comment written on it and its mark on the page, the
//! markers in the margin, a note opened under its passage and the panel that lists them all.

use std::ops::Range;

use letters_model::{
    Anchor, Block, BlockKind, Document, Mark, Note, NoteKind, Notes, Pos, Reply, Run, Selection, Version,
};
use letters_render::{Item, Point, Rect};
use letters_test::{Found, Harness, Role};
use letters_ui::{Cursor, Request};

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

/// A Sunday, 15 November 2026, at twenty past two in the morning, in seconds since 1970.
const NOW: u64 = 1_794_709_230;

/// A note that Alice wrote the Tuesday before.
fn note(id: &str, kind: NoteKind, anchor: Anchor, says: &str) -> Note {
    Note {
        id: id.into(),
        kind,
        anchor,
        author: "Alice Moreau".into(),
        created: "2026-11-10T08:00:00Z".into(),
        resolved: false,
        content: vec![Run::plain(says)],
        replies: Vec::new(),
        extra: Default::default(),
    }
}

/// What Sam answered a note with, the Friday before.
fn reply(says: &str) -> Reply {
    let (author, created) = ("Sam Okafor".into(), "2026-11-13T18:20:00Z".into());
    Reply { author, created, content: vec![Run::plain(says)], extra: Default::default() }
}

/// The two sides of a conflict, with nothing of what they changed.
fn conflict() -> NoteKind {
    let by =
        |author: &str| Version { author: author.into(), edited: String::new(), block: None, extra: Default::default() };
    NoteKind::Conflict { versions: Box::new([by("Sam Okafor"), by("Alice Moreau")]) }
}

/// Alice's comment on the written quote, which Sam answered.
fn chased() -> Note {
    let asked = note("chased", NoteKind::Comment, Anchor::range("a", QUOTE), "Has anyone chased it?");
    Note { replies: vec![reply("Called on Tuesday.")], ..asked }
}

/// The letter that Sam has open on that Sunday, with `notes` on it.
fn noted(notes: Vec<Note>) -> Harness {
    let mut window = letter();
    window.platform.wall = Some(NOW);
    window.ui().set_notes(Notes { notes, ..Notes::default() });
    window
}

/// The markers in the margin: the buttons that count the notes of a block.
fn markers(window: &mut Harness) -> Vec<Found> {
    let counts = |found: &Found| found.role == Role::Button && found.name.starts_with(|c: char| c.is_ascii_digit());
    window.controls().into_iter().filter(counts).collect()
}

/// What the note that is open says: who speaks and their words, in order.
fn said(window: &mut Harness) -> Vec<(String, String)> {
    let controls = window.controls();
    let voices = controls.iter().enumerate().filter(|(_, found)| found.role == Role::Comment);
    let spoken = voices.map(|(at, voice)| {
        let words = controls[at..].iter().find(|found| found.role == Role::Paragraph);
        (voice.name.clone(), words.map(|words| words.name.clone()).unwrap_or_default())
    });
    spoken.collect()
}

/// The notes the panel lists, by what each is called.
fn listed(window: &mut Harness) -> Vec<String> {
    let rows = window.controls().into_iter().filter(|found| found.role == Role::ListBoxOption);
    rows.map(|row| row.name).collect()
}

/// The part of the window the page is in.
fn area(window: &mut Harness) -> Rect {
    window.controls().into_iter().find(|found| found.role == Role::Document).expect("a page").bounds
}

/// Where the caret shows at a place in a block's text. The caret is left there.
fn caret_at(window: &mut Harness, block: &str, offset: usize) -> Rect {
    window.ui().select(Selection::caret(Pos::new(block, offset)));
    window.paint();
    window.ui().caret_rect().expect("the page shows a caret")
}

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
    let ends = (caret_at(window, block, range.start), caret_at(window, block, range.end));
    underlined(window, ends)
}

/// Whether a frame draws a line under the text between two carets on one line.
fn underlined(window: &mut Harness, (start, end): (Rect, Rect)) -> bool {
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
fn a_document_that_takes_no_comments_says_why_in_place_of_the_field() {
    let mut window = letter();
    let why = "Comments cannot be added: this document's notes were written by a newer version of Letters.";
    window.ui().refuse_comments(Some(why));
    window.select("written quote").click(Role::Button, "Comment");
    assert!(window.find(Role::TextInput, "Comment").is_none(), "nothing is typed that could not be kept");
    let said = |found: &Found| {
        found.role == Role::Status && [Some(found.name.as_str()), found.value.as_deref()].contains(&Some(why))
    };
    assert!(window.controls().iter().any(said), "the reason is there to read");
    window.key("Escape").assert_closed(Role::Dialog, "New comment");
    assert_eq!(kept(&mut window), []);

    window.ui().refuse_comments(None);
    comment(&mut window, "Reply", "Chased?");
    assert_eq!(kept(&mut window).len(), 1, "once the shell says they can be, comments are written again");
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

#[test]
fn a_block_with_open_notes_has_a_marker_beside_it_that_counts_them() {
    let notes = vec![
        chased(),
        note("split", NoteKind::Import, Anchor::block("b"), "Merged cells split"),
        Note { resolved: true, ..note("done", NoteKind::Comment, Anchor::range("b", 0..5), "Sent.") },
        // The text this one was about is gone, and a note of a kind from a newer build is not this one's to show.
        note("gone", NoteKind::Comment, Anchor::range("nowhere", 0..5), "Where?"),
        note("new", NoteKind::Unknown { type_name: "poll".into() }, Anchor::block("b"), "Tea?"),
    ];
    let mut window = noted(notes);
    let page = window.editor().model().style().page.clone();
    let column = (page.width - page.margin_left - page.margin_right) * 96.0 / 72.0;
    // A block, and what the marker beside it is called.
    let cases = [("a", "2 comments"), ("b", "1 import note")];
    for (block, name) in cases {
        let line = caret_at(&mut window, block, 0);
        let area = area(&mut window);
        let marker = window.find(Role::Button, name).unwrap_or_else(|| panic!("a marker called {name}"));
        let level = marker.center().y;
        assert!(level > line.y && level < line.y + line.height, "{name}: level with the first line of {block}");
        let text = area.x + (area.width + column) / 2.0;
        assert!(marker.bounds.x >= text, "{name}: in the margin, clear of the text: {:?}", marker.bounds);
        assert!(marker.bounds.x + marker.bounds.width <= area.x + area.width, "{name}: {:?}", marker.bounds);
    }
    assert_eq!(markers(&mut window).len(), cases.len(), "only notes that are open and have a place are marked");
}

#[test]
fn a_marker_keeps_within_the_narrowest_margin_a_window_leaves() {
    let mut window = noted(vec![chased()]);
    window.resize(480.0, 600.0);
    let area = area(&mut window);
    let [marker] = &markers(&mut window)[..] else { panic!("one marker") };
    let end = caret_at(&mut window, "a", QUOTE.end);
    assert!(marker.bounds.x + marker.bounds.width <= area.x + area.width, "{:?} in {area:?}", marker.bounds);
    assert!(marker.bounds.x >= area.x + area.width - 24.0, "it leaves the text alone: {:?}, {end:?}", marker.bounds);
}

#[test]
fn a_press_on_a_marker_opens_the_notes_of_its_block_one_after_another() {
    let slates = note("slates", NoteKind::Comment, Anchor::range("a", 31..37), "Welsh ones?");
    let mut window = noted(vec![chased(), slates]);
    let passage = (caret_at(&mut window, "a", QUOTE.start), caret_at(&mut window, "a", QUOTE.end));
    caret_at(&mut window, "b", 0);
    let marker = window.find(Role::Button, "3 comments").expect("a marker").center();
    assert_eq!(window.hover_at(marker).ui().cursor(), Cursor::Pointer, "the pointer says it can be pressed");

    let caret = window.editor().selection().cloned();
    window.click_at(marker);
    let open = window.find(Role::Dialog, "Comments").expect("the first note opens");
    assert_eq!(window.editor().selection().cloned(), caret, "the caret stays where it was");
    assert!(open.bounds.y >= passage.1.y + passage.1.height, "under its passage: {:?}, {passage:?}", open.bounds);
    let both = [("Alice Moreau", "Has anyone chased it?"), ("Sam Okafor", "Called on Tuesday.")];
    assert_eq!(said(&mut window), both.map(|(who, words)| (who.to_owned(), words.to_owned())));
    assert!(window.find(Role::TextInput, "Reply").is_some_and(|field| field.focused), "ready for a reply");
    assert!(underlined(&mut window, passage), "its passage stays marked");

    window.click_at(marker);
    assert_eq!(said(&mut window), [("Alice Moreau".to_owned(), "Welsh ones?".to_owned())], "then the second");
    window.click_at(marker);
    window.assert_closed(Role::Dialog, "Comments");

    // Escape puts a note away, and the page has the keyboard back.
    window.click_at(marker).key("Escape").assert_closed(Role::Dialog, "Comments");
    assert!(window.focused().is_some_and(|found| found.role == Role::Document));
    // A press beside it does too, and the next press on the marker starts with the first note again.
    window.click_at(marker).click_at(Point { x: marker.x, y: marker.y + 200.0 });
    window.assert_closed(Role::Dialog, "Comments");
    window.click_at(marker);
    assert_eq!(said(&mut window).len(), 2);
}

#[test]
fn a_note_says_when_each_part_of_it_was_written() {
    let mut window = noted(vec![chased()]);
    window.click(Role::Button, "2 comments");
    let when: Vec<_> = window.controls().into_iter().filter(|found| found.role == Role::Comment).collect();
    let when: Vec<_> = when.iter().map(|voice| voice.description.as_deref().unwrap_or_default()).collect();
    assert_eq!(when, ["Tuesday", "Friday"]);
}

#[test]
fn each_kind_of_note_opens_with_what_can_be_done_about_it() {
    // The kind, what its marker and the open note are called, and whether it can be resolved and replied to.
    let cases = [
        (NoteKind::Comment, "1 comment", "Comments", true),
        (NoteKind::Import, "1 import note", "Import note", true),
        // A conflict is settled by choosing one of its versions.
        (conflict(), "1 conflict", "Conflict", false),
    ];
    for (kind, marker, title, acts) in cases {
        let mut window = noted(vec![note("n", kind, Anchor::block("b"), "Merged cells split")]);
        window.click(Role::Button, marker).assert_open(Role::Dialog, title);
        let acted = [window.find(Role::Button, "Resolve").is_some(), window.find(Role::TextInput, "Reply").is_some()];
        assert_eq!(acted, [acts, acts], "{title}");
        if !acts {
            let both = |text: &str| text.contains("Sam Okafor") && text.contains("Alice Moreau");
            let says = |found: &Found| both(&found.name) || found.value.as_deref().is_some_and(both);
            assert!(window.controls().iter().any(says), "{title}: it says whose changes met");
        }
    }
}

#[test]
fn a_reply_is_kept_with_its_note_signed_and_dated() {
    // What is typed, how the field is left, and what is kept of it.
    let cases = [
        ("Thanks, **all**", "Enter", Some("Thanks, all")),
        ("On it \u{1f44d}", "Send", Some("On it \u{1f44d}")),
        ("   ", "Enter", None),
        ("Never mind", "Escape", None),
    ];
    for (typed, left, kept) in cases {
        let mut window = noted(vec![chased()]);
        window.click(Role::Button, "2 comments");
        let edits = window.ui().edits();
        window.type_text(typed);
        match left {
            "Send" => window.click(Role::Button, "Send"),
            key => window.key(key),
        };
        let replies = window.ui().notes().notes[0].replies.clone();
        let last = replies.last().expect("the reply it had");
        let words: String = last.content.iter().map(|run| run.text.as_str()).collect();
        assert_eq!((replies.len(), window.ui().edits() > edits), (1 + usize::from(kept.is_some()), kept.is_some()));
        if let Some(kept) = kept {
            assert_eq!((words.as_str(), last.author.as_str()), (kept, "Sam Okafor"), "{typed:?}");
            assert_eq!(last.created, "2026-11-15T02:20:30Z", "{typed:?}");
            assert_eq!(said(&mut window).last(), Some(&("Sam Okafor".to_owned(), kept.to_owned())), "{typed:?}");
            let field = window.find(Role::TextInput, "Reply").expect("the note stays open");
            assert_eq!(field.value.as_deref().unwrap_or_default(), "", "{typed:?}: ready for the next");
            assert!(window.find(Role::Button, "3 comments").is_some(), "{typed:?}: the marker counts it");
        }
        assert_eq!(window.find(Role::Dialog, "Comments").is_some(), left != "Escape", "{typed:?}, {left}");
    }
}

#[test]
fn a_reply_that_is_being_typed_survives_a_press_on_the_page() {
    let mut window = noted(vec![chased()]);
    let marker = window.find(Role::Button, "2 comments").expect("a marker").center();
    window.click_at(marker).type_text("Thanks");
    for at in [Point { x: marker.x, y: marker.y + 200.0 }, marker] {
        window.click_at(at);
        let field = window.find(Role::TextInput, "Reply").expect("the note stays open");
        assert_eq!(field.value.as_deref(), Some("Thanks"), "{at:?}");
    }
    window.key("Escape").assert_closed(Role::Dialog, "Comments");
    assert_eq!(window.ui().notes().notes[0].replies.len(), 1, "Escape drops it");
}

#[test]
fn resolving_a_note_puts_it_away_and_takes_its_marks_off_the_page() {
    let mut window = noted(vec![chased()]);
    let edits = window.ui().edits();
    window.click(Role::Button, "2 comments").click(Role::Button, "Resolve");
    window.assert_closed(Role::Dialog, "Comments");
    assert_eq!(markers(&mut window), []);
    assert!(!marked(&mut window, "a", QUOTE));
    let notes = window.ui().notes().clone();
    assert!(notes.notes.len() == 1 && notes.notes[0].resolved, "it is kept for the file, resolved: {notes:?}");
    assert!(window.ui().edits() > edits, "which is something to save");
}

#[test]
fn a_document_that_takes_no_comments_takes_no_replies_either() {
    let mut window = noted(vec![chased()]);
    let why = "Comments cannot be added: this document's notes were written by a newer version of Letters.";
    window.ui().refuse_comments(Some(why));
    let edits = window.ui().edits();
    window.click(Role::Button, "2 comments").assert_open(Role::Dialog, "Comments");
    assert_eq!(said(&mut window).len(), 2, "what was said can still be read");
    assert!(window.find(Role::TextInput, "Reply").is_none());
    let told = |found: &Found| {
        found.role == Role::Status && [Some(found.name.as_str()), found.value.as_deref()].contains(&Some(why))
    };
    assert!(window.controls().iter().any(told), "the reason is there to read");
    assert!(window.find(Role::Button, "Resolve").is_some_and(|resolve| resolve.disabled));
    window.click(Role::Button, "Resolve");
    assert_eq!((window.ui().notes().notes[0].resolved, window.ui().edits()), (false, edits));
}

#[test]
fn a_link_in_a_note_is_the_shells_to_open() {
    let href = "https://harland.example/quotes/north-slope";
    // What the note says, and what a press on its first word asks the shell for.
    let cases =
        [(Run { link: Some(href.into()), ..Run::plain("Their email") }, Some(href)), (Run::plain("Their email"), None)];
    for (says, asked) in cases {
        let linked = Note { content: vec![says], ..note("n", NoteKind::Comment, Anchor::range("a", QUOTE), "") };
        let mut window = noted(vec![linked]);
        window.click(Role::Button, "1 comment");
        window.ui().take_requests();
        let words = window.find(Role::Paragraph, "Their email").expect("what the note says").bounds;
        window.click_at(Point { x: words.x + 12.0, y: words.y + words.height / 2.0 });
        let links = |request: &Request| matches!(request, Request::OpenLink { .. });
        let opened: Vec<Request> = window.ui().take_requests().into_iter().filter(links).collect();
        let expected: Vec<Request> =
            asked.map(|href| Request::OpenLink { href: href.to_owned(), confirmed: false }).into_iter().collect();
        assert_eq!(opened, expected);
        window.assert_open(Role::Dialog, "Comments");
    }
}

#[test]
fn the_notes_panel_lists_open_notes_and_opens_one_at_its_passage() {
    // A long letter, with a comment on its last line and a note of an import on its first.
    let mut window = Harness::doc(&format!("{}Their written quote covers the slates.", "A line.\n\n".repeat(80)));
    window.platform.wall = Some(NOW);
    let (first, last) = {
        let mut blocks = window.editor().doc().walk().map(|block| block.id.clone());
        (blocks.next().expect("a first line"), blocks.last().expect("a last line"))
    };
    let notes = vec![
        Note { replies: vec![reply("Called on Tuesday.")], ..chased() },
        note("split", NoteKind::Import, Anchor::block(first.clone()), "Merged cells split"),
        Note { resolved: true, ..note("done", NoteKind::Comment, Anchor::block(first), "Sent.") },
    ];
    let notes = notes.into_iter().map(|note| match note.id.as_str() {
        "chased" => Note { anchor: Anchor::range(last.clone(), QUOTE), ..note },
        _ => note,
    });
    window.ui().set_notes(Notes { notes: notes.collect(), ..Notes::default() });

    window.click(Role::Button, "Notes");
    let (comment, import) = ("Alice Moreau: Has anyone chased it?", "Alice Moreau: Merged cells split");
    assert_eq!(listed(&mut window), [comment, import], "the open ones, in the order they were written");
    let row = window.find(Role::ListBoxOption, comment).expect("the comment");
    let told = row.description.unwrap_or_default();
    assert!(told.contains("written quote") && told.contains("Tuesday"), "what it is about, and when: {told}");

    // Each kind can be listed alone.
    for (only, shown) in [("Import", vec![import]), ("Comments", vec![comment]), ("All", vec![comment, import])] {
        window.click(Role::Button, only);
        assert_eq!(listed(&mut window), shown, "{only}");
    }

    // The comment is far below what shows. Opening it brings its passage into view, with the note by it.
    assert_eq!(markers(&mut window).len(), 1, "only the first line is in view");
    window.click(Role::ListBoxOption, comment);
    let area = area(&mut window);
    let open = window.find(Role::Dialog, "Comments").expect("the note opens").bounds;
    let marker = window.find(Role::Button, "2 comments").expect("its marker shows").bounds;
    let within = |rect: Rect| rect.y >= area.y && rect.y + rect.height <= area.y + area.height;
    assert!(within(marker) && within(open), "the passage and its note are in view: {marker:?}, {open:?}");
    let apart = open.y >= marker.y + marker.height || open.y + open.height <= marker.y;
    assert!(apart, "and the note does not cover the line it is about: {marker:?}, {open:?}");
    assert_eq!(window.find(Role::ListBoxOption, comment).and_then(|row| row.selected), Some(true));
}

#[test]
fn the_notes_an_import_left_are_resolved_all_at_once() {
    let import = |id: &str, block: &str| note(id, NoteKind::Import, Anchor::block(block), "Merged cells split");
    let mut window = noted(vec![import("one", "a"), chased(), import("two", "b")]);
    window.click(Role::Button, "Notes");
    assert!(window.find(Role::Button, "Resolve all").is_none(), "not while the comments are listed too");
    let edits = window.ui().edits();
    window.click(Role::Button, "Import").click(Role::Button, "Resolve all");
    let open: Vec<String> =
        window.ui().notes().notes.iter().filter(|note| !note.resolved).map(|note| note.id.clone()).collect();
    assert_eq!(open, ["chased"]);
    assert!(window.ui().edits() > edits);
    assert_eq!(listed(&mut window), ["Alice Moreau: Has anyone chased it?"], "the panel lists what is left");
}

#[test]
fn a_panel_with_no_open_notes_says_so() {
    let mut window = noted(vec![Note { resolved: true, ..chased() }]);
    window.click(Role::Button, "Notes");
    assert_eq!(listed(&mut window), Vec::<String>::new());
    assert!(window.controls().iter().any(|found| found.name == "No open notes."));
}
