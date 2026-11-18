//! Notes on a passage: the tools over selected text, a comment written on it and its mark on the page, the
//! cards of the notes beside the page, and in a window with no room for those the markers in the margin
//! and a note opened under its passage, and the panel that lists them all.

use std::ops::Range;

use letters_editor::Clip;
use letters_model::{
    Anchor, Block, BlockKind, Body, Document, Mark, Note, NoteKind, Notes, Pos, Reply, Run, Selection,
};
use letters_render::{Item, Point, Rect};
use letters_test::{Found, Harness, Role};
use letters_ui::{Cursor, Landing, Request};

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
    window.resize(NARROW, 800.0);
    window
}

/// A window with no room for cards beside the page: its notes open under their passages.
const NARROW: f32 = 780.0;

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

/// A point on the text of a block, just after a place in it. The caret is left at the start of paragraph
/// `b`, away from every note.
fn spot(window: &mut Harness, block: &str, offset: usize) -> Point {
    let after = caret_at(window, block, offset);
    caret_at(window, "b", 0);
    Point { x: after.x + 1.0, y: after.y + after.height / 2.0 }
}

/// Alice's comment on the slates, at the end of paragraph `a`.
fn slates() -> Note {
    note("slates", NoteKind::Comment, Anchor::range("a", 31..37), "Welsh ones?")
}

/// What the note that is open starts with, when one of any kind is open.
fn reading(window: &mut Harness) -> Option<String> {
    said(window).into_iter().next().map(|(_, words)| words)
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
        ("  Thanks  ", "Enter", Some("Thanks")),
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

/// The runs of the one comment the window keeps: what each says, whether it is bold and where it links.
fn written(window: &mut Harness) -> Vec<(String, bool, Option<String>)> {
    let notes = window.ui().notes().clone();
    let [note] = &notes.notes[..] else { panic!("one note: {notes:?}") };
    note.content.iter().map(|run| (run.text.clone(), run.marks.contains(&Mark::Bold), run.link.clone())).collect()
}

/// How many glyphs a frame draws in a part of the window.
fn glyphs_in(window: &mut Harness, area: Rect) -> usize {
    let drawn = |item: &Item| match item {
        Item::Glyphs { origin, glyphs, .. } if area.contains(*origin) => glyphs.len(),
        _ => 0,
    };
    window.paint().items.iter().map(drawn).sum()
}

#[test]
fn a_comment_is_typed_with_the_shorthand_of_the_page_and_shows_its_marks_at_the_caret() {
    let mut window = letter();
    window.select("written quote").click(Role::Button, "Comment").type_text("Has anyone **chased** it");
    // The marks are read as they are typed: the stars are gone, and the word is bold.
    let field = window.find(Role::TextInput, "Comment").expect("the field");
    assert_eq!(field.value.as_deref(), Some("Has anyone chased it"));
    let caret = window.ui().caret_rect().expect("the caret of the field");
    assert!(field.bounds.contains(Point { x: caret.x, y: caret.y }), "{caret:?} in {:?}", field.bounds);
    // They show again while the caret is in the word they mark, as on the page.
    let away = glyphs_in(&mut window, field.bounds);
    for _ in 0..5 {
        window.key("Left");
    }
    assert!(glyphs_in(&mut window, field.bounds) > away, "the stars show around the word the caret is in");
    window.key("Enter");
    let said = [("Has anyone ", false), ("chased", true), (" it", false)];
    assert_eq!(written(&mut window), said.map(|(text, bold)| (text.to_owned(), bold, None)));
}

#[test]
fn the_shorthand_that_makes_a_block_of_the_page_means_nothing_in_a_comment() {
    // What is typed is what is kept: a comment is one stretch of text.
    for typed in ["- slates first", "# 1", "> said who?", "costs $40 or $50"] {
        let mut window = letter();
        comment(&mut window, "written quote", typed);
        assert_eq!(kept(&mut window), [("written quote".to_owned(), typed.to_owned())]);
    }
}

#[test]
fn an_address_in_a_comment_is_a_link_wherever_it_is_written() {
    const MAIL: &str = "https://a.example/m";
    // What is typed, and the runs that link somewhere.
    let cases = [
        (format!("see {MAIL}"), vec![(MAIL, MAIL)]),
        (format!("{MAIL} has it, and so has {MAIL}."), vec![(MAIL, MAIL), (MAIL, MAIL)]),
        (format!("see [their email]({MAIL}) again"), vec![("their email", MAIL)]),
        ("no address here".to_owned(), vec![]),
    ];
    for (typed, links) in cases {
        let mut window = letter();
        comment(&mut window, "written quote", &typed);
        let linked: Vec<(String, String)> =
            written(&mut window).into_iter().filter_map(|(text, _, link)| Some((text, link?))).collect();
        let links: Vec<(String, String)> =
            links.iter().map(|(text, to)| ((*text).to_owned(), (*to).to_owned())).collect();
        assert_eq!(linked, links, "{typed:?}");
    }
}

#[test]
fn the_keys_and_the_pointer_edit_a_comment_as_they_edit_the_page() {
    let mut window = letter();
    let (page, edits) = (window.markdown(), window.ui().edits());
    window.select("written quote").click(Role::Button, "Comment");
    let value = |window: &mut Harness| window.find(Role::TextInput, "Comment").and_then(|field| field.value);

    // The key that makes text bold on the page makes it bold here.
    window.type_text("due ").key("Mod+B").type_text("Friday").key("Mod+B").type_text(" noon");
    // A press puts the caret where it lands, and what is typed goes in there.
    let field = window.find(Role::TextInput, "Comment").expect("the field");
    window.click_at(Point { x: field.bounds.x + 3.0, y: field.center().y }).type_text("Now ");
    assert_eq!(value(&mut window).as_deref(), Some("Now due Friday noon"));
    // What is pasted comes as plain text, and stays in the comment.
    window.platform.clipboard =
        Some(Clip { text: "really ".into(), html: Some("<h1>really </h1>".into()), pictures: Vec::new() });
    window.key("Mod+V");
    assert_eq!(value(&mut window).as_deref(), Some("Now really due Friday noon"));
    // Undo takes back what was done in the field, and nothing on the page.
    window.key("Mod+Z");
    assert_eq!(value(&mut window).as_deref(), Some("Now due Friday noon"));
    assert_eq!((window.markdown(), window.ui().edits()), (page, edits), "the page is as it was");

    window.key("Enter");
    let said = [("Now due ", false), ("Friday", true), (" noon", false)];
    assert_eq!(written(&mut window), said.map(|(text, bold)| (text.to_owned(), bold, None)));
}

#[test]
fn a_drag_in_a_comment_selects_what_it_passes_and_typing_replaces_it() {
    let mut window = letter();
    window.select("written quote").click(Role::Button, "Comment").type_text("Chased it?");
    let field = window.find(Role::TextInput, "Comment").expect("the field");
    let fills = |window: &mut Harness| {
        let within = |item: &Item| matches!(item, Item::Fill { rect, .. } if field.bounds.contains(Point { x: rect.x, y: rect.y }) && rect.height < field.bounds.height - 4.0);
        window.paint().items.iter().filter(|item| within(item)).count()
    };
    let plain = fills(&mut window);
    let (y, right) = (field.center().y, field.bounds.x + field.bounds.width - 3.0);
    window.press_at(Point { x: field.bounds.x + 3.0, y }).hover_at(Point { x: right, y });
    assert!(fills(&mut window) > plain, "what is selected shows as selected");
    window.release_at(Point { x: right, y }).type_text("Paid?");
    assert_eq!(window.find(Role::TextInput, "Comment").and_then(|field| field.value).as_deref(), Some("Paid?"));
}

#[test]
fn a_shortcut_a_comment_has_no_use_for_is_the_windows() {
    let mut window = letter();
    window.select("written quote").click(Role::Button, "Comment").type_text("Chased?");
    // Tab moves on to the button that sends it, where on the page it would indent.
    window.key("Tab");
    assert!(window.focused().is_some_and(|found| found.name == "Send"), "{:?}", window.focused());
    window.key("Mod+Shift+P").assert_open(Role::Dialog, "Commands");
}

#[test]
fn a_reply_is_written_as_a_comment_is() {
    let mut window = noted(vec![chased()]);
    let on = spot(&mut window, "a", QUOTE.start + 4);
    window.click_at(on).click(Role::TextInput, "Reply").type_text("on it, **today** see https://a.example/m");
    let field = window.find(Role::TextInput, "Reply").expect("the field");
    assert_eq!(field.value.as_deref(), Some("on it, today see https://a.example/m"));
    window.key("Enter");
    let replies = &window.ui().notes().notes[0].replies;
    let said: Vec<_> = replies.last().into_iter().flat_map(|reply| &reply.content).collect();
    let bold: Vec<&str> =
        said.iter().filter(|run| run.marks.contains(&Mark::Bold)).map(|run| run.text.as_str()).collect();
    let links: Vec<Option<&str>> =
        said.iter().filter_map(|run| run.link.as_ref().map(|link| Some(link.as_str()))).collect();
    assert_eq!((bold, links), (vec!["today"], vec![Some("https://a.example/m")]), "{said:?}");
    // The field is empty again for the next reply.
    assert_eq!(window.find(Role::TextInput, "Reply").and_then(|field| field.value).as_deref(), Some(""));
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
    // The kind, and what its marker and the open note are called. A conflict opens in the page instead,
    // with both of its versions: see the tests of conflict review.
    // What an import left is no one's to answer.
    let cases =
        [(NoteKind::Comment, "1 comment", "Comments", true), (NoteKind::Import, "1 import note", "Import note", false)];
    for (kind, marker, title, answered) in cases {
        let mut window = noted(vec![note("n", kind, Anchor::block("b"), "Merged cells split")]);
        window.click(Role::Button, marker).assert_open(Role::Dialog, title);
        assert!(window.find(Role::Button, "Resolve").is_some(), "{title} can be resolved");
        assert_eq!(window.find(Role::TextInput, "Reply").is_some(), answered, "{title}: a reply");
    }
}

#[test]
fn a_comment_opens_when_the_caret_is_put_in_its_passage() {
    // How the caret comes into the written quote: by a press on it, or by keys from the word before it.
    for by in ["a press", "keys"] {
        let mut window = noted(vec![chased()]);
        let passage = (caret_at(&mut window, "a", QUOTE.start), caret_at(&mut window, "a", QUOTE.end));
        if by == "keys" {
            caret_at(&mut window, "a", QUOTE.start - 2);
            window.key("Right").assert_closed(Role::Dialog, "Comments").key("Right");
        } else {
            let on = spot(&mut window, "a", QUOTE.start + 4);
            window.click_at(on);
        }
        let open = window.find(Role::Dialog, "Comments").unwrap_or_else(|| panic!("{by}: the comment opens"));
        assert!(open.bounds.y >= passage.1.y + passage.1.height, "{by}: under its passage, {:?}", open.bounds);
        let both = [("Alice Moreau", "Has anyone chased it?"), ("Sam Okafor", "Called on Tuesday.")];
        assert_eq!(said(&mut window), both.map(|(who, words)| (who.to_owned(), words.to_owned())), "{by}");
        let when = window.find(Role::Comment, "Alice Moreau").and_then(|voice| voice.description);
        assert_eq!(when.as_deref(), Some("Tuesday"), "{by}: it says when it was written");

        // The page keeps the keyboard: what is typed is written in the passage, and the note stays by it.
        assert!(window.focused().is_some_and(|found| found.role == Role::Document), "{by}");
        window.type_text("x");
        let text = window.editor().doc().block(&"a".into()).map(Block::plain_text).unwrap_or_default();
        assert_eq!(text.len(), "Their written quote covers the slates.".len() + 1, "{by}: {text}");
        assert_eq!(reading(&mut window).as_deref(), Some("Has anyone chased it?"), "{by}");
    }
}

#[test]
fn a_note_the_caret_opened_goes_when_the_caret_leaves_its_passage() {
    // What is done from the last letter of the written quote, and whether its note is still open after.
    let cases: [(&[&str], bool); 4] =
        [(&["Right"], true), (&["Right", "Right"], false), (&["Shift+Left"], false), (&["a press elsewhere"], false)];
    for (done, stays) in cases {
        let mut window = noted(vec![chased()]);
        let elsewhere = spot(&mut window, "b", 5);
        let on = spot(&mut window, "a", QUOTE.end - 1);
        window.click_at(on).assert_open(Role::Dialog, "Comments");
        for step in done {
            match *step {
                "a press elsewhere" => window.click_at(elsewhere),
                key => window.key(key),
            };
        }
        assert_eq!(reading(&mut window).is_some(), stays, "{done:?}");
        // A stretch selected in the passage has the tools over it, to link it or comment on it again.
        assert_eq!(tools(&mut window).is_some(), done == ["Shift+Left"], "{done:?}");
    }
}

#[test]
fn a_note_the_caret_opened_stays_away_once_it_is_put_away() {
    let mut window = noted(vec![chased()]);
    let on = spot(&mut window, "a", QUOTE.start + 4);
    window.click_at(on).key("Escape").assert_closed(Role::Dialog, "Comments");
    assert!(window.focused().is_some_and(|found| found.role == Role::Document), "the page keeps the keyboard");
    window.key("Right").key("Left").key("Left").assert_closed(Role::Dialog, "Comments");

    // It opens again once the caret has left its passage and come back, or the passage is pressed.
    window.key("Home").key("Right").assert_closed(Role::Dialog, "Comments");
    for _ in 0..QUOTE.start {
        window.key("Right");
    }
    window.assert_open(Role::Dialog, "Comments").key("Escape").assert_closed(Role::Dialog, "Comments");
    // Long enough after the first press that this is not the second of a double click.
    window.advance(2_000).click_at(on).assert_open(Role::Dialog, "Comments");
}

#[test]
fn a_reply_is_written_in_a_note_the_caret_opened() {
    let mut window = noted(vec![chased()]);
    let elsewhere = spot(&mut window, "b", 5);
    let on = spot(&mut window, "a", QUOTE.start + 4);
    window.click_at(on).click(Role::TextInput, "Reply").type_text("On it");
    assert_eq!(window.markdown().matches("On it").count(), 0, "the reply is not written on the page");

    // What is typed keeps the note, wherever the caret is put, until it is sent.
    window.click_at(elsewhere);
    let field = window.find(Role::TextInput, "Reply").expect("the note stays open");
    assert_eq!(field.value.as_deref(), Some("On it"));
    window.click(Role::Button, "Send");
    let replies = &window.ui().notes().notes[0].replies;
    assert_eq!(replies.len(), 2, "the reply is kept: {replies:?}");
    window.assert_closed(Role::Dialog, "Comments");
}

#[test]
fn the_caret_opens_the_comments_that_are_marked_on_the_page_and_the_nearest_of_them() {
    // The notes on the written quote, where in paragraph `a` the caret is put, and what opens there.
    let on_quote = |id: &str, kind, says: &str| note(id, kind, Anchor::range("a", QUOTE), says);
    let word = note("word", NoteKind::Comment, Anchor::range("a", 14..19), "Which quote?");
    let cases = [
        (vec![chased()], QUOTE.start, Some("Has anyone chased it?")),
        (vec![chased()], QUOTE.end, Some("Has anyone chased it?")),
        (vec![chased()], QUOTE.end + 1, None),
        (vec![chased(), word.clone()], 16, Some("Which quote?")),
        (vec![chased(), word], 8, Some("Has anyone chased it?")),
        (vec![on_quote("split", NoteKind::Import, "Merged cells split")], 10, None),
        (vec![Note { resolved: true, ..chased() }], 10, None),
        (vec![note("whole", NoteKind::Comment, Anchor::block("a"), "Too long")], 10, None),
    ];
    for (notes, offset, opens) in cases {
        let mut window = noted(notes);
        let on = spot(&mut window, "a", offset);
        window.click_at(on);
        assert_eq!(reading(&mut window).as_deref(), opens, "at {offset}");
    }
}

#[test]
fn nothing_that_is_open_or_being_written_gives_way_to_the_caret() {
    // A comment that is being written on the slates stays when the written quote is pressed.
    let mut window = noted(vec![chased()]);
    let on = spot(&mut window, "a", QUOTE.start + 4);
    window.select("slates").click(Role::Button, "Comment").type_text("Welsh ones?").click_at(on);
    let field = window.find(Role::TextInput, "Comment").expect("the comment is still being written");
    assert_eq!(field.value.as_deref(), Some("Welsh ones?"));
    window.assert_closed(Role::Dialog, "Comments");

    // So does a note opened from its marker, once a reply is typed in it.
    let mut window = noted(vec![chased(), slates()]);
    let on = spot(&mut window, "a", 33);
    window.click(Role::Button, "3 comments").type_text("Thanks").click_at(on);
    assert_eq!(reading(&mut window).as_deref(), Some("Has anyone chased it?"));

    // A comment that was just written stays away while the caret only lets go of what it was written on,
    // and opens when its passage is pressed.
    let mut window = noted(Vec::new());
    let on = spot(&mut window, "a", 33);
    comment(&mut window, "slates", "Welsh ones?");
    window.key("Right").assert_closed(Role::Dialog, "Comments");
    window.click_at(on);
    assert_eq!(said(&mut window), [("Sam Okafor".to_owned(), "Welsh ones?".to_owned())]);
}

#[test]
fn the_page_and_the_margin_work_as_before_under_a_note_the_caret_opened() {
    // A link on the page is still followed with Ctrl or Cmd held, and the note stays by its passage.
    let href = "https://harland.example/quotes/north-slope";
    let mut window = Harness::doc(&format!("See [their email]({href}) for the written quote."));
    window.resize(NARROW, 800.0);
    let line = window.editor().doc().blocks[0].id.clone();
    let text = window.editor().doc().blocks[0].plain_text();
    let (link, quote) = (text.find("email").expect("a link"), text.find("written").expect("a passage"));
    let asked = note("chased", NoteKind::Comment, Anchor::range(line.as_str(), quote..quote + 13), "Chased?");
    window.ui().set_notes(Notes { notes: vec![asked], ..Notes::default() });
    let (on, link) = (caret_at(&mut window, line.as_str(), quote + 4), caret_at(&mut window, line.as_str(), link));
    window.ui().select(Selection::caret(Pos::new(line.as_str(), 0)));
    window.click_at(Point { x: on.x + 1.0, y: on.y + on.height / 2.0 }).assert_open(Role::Dialog, "Comments");
    window.ui().take_requests();
    window.mod_click_at(Point { x: link.x + 1.0, y: link.y + link.height / 2.0 });
    assert_eq!(window.ui().take_requests(), [Request::OpenLink { href: href.into(), confirmed: false }]);
    window.assert_open(Role::Dialog, "Comments");

    // A press on the marker of its block goes on to the next note, which takes the keyboard as any note
    // opened from a marker does.
    let mut window = noted(vec![chased(), slates()]);
    let on = spot(&mut window, "a", QUOTE.start + 4);
    window.click_at(on).click(Role::Button, "3 comments");
    assert_eq!(reading(&mut window).as_deref(), Some("Welsh ones?"));
    assert!(window.find(Role::TextInput, "Reply").is_some_and(|field| field.focused), "ready for a reply");
}

#[test]
fn the_sheet_of_a_formula_shows_in_place_of_a_note_the_caret_opened() {
    // A comment on words that end in a formula: `rafter has length c = a + b`.
    let blocks = vec![Block::text("p", BlockKind::Paragraph, "A rafter has length ")];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.resize(NARROW, 800.0);
    window.ui().set_reduced_motion(true);
    window.ui().select(Selection::caret(Pos::new("p", 20)));
    window.type_text("$c = a + b$");
    let long = window.editor().doc().blocks[0].plain_text().len();
    let asked = note("which", NoteKind::Comment, Anchor::range("p", 2..long), "Of which rafter?");
    window.ui().set_notes(Notes { notes: vec![asked], ..Notes::default() });
    window.ui().select(Selection::caret(Pos::new("p", 0)));
    window.key("Right").key("Right").assert_open(Role::Dialog, "Comments");

    // At the end of the line the caret is in the formula, and its source is what shows under it. The note
    // is back once the caret has stepped out of the formula.
    window.key("End").assert_closed(Role::Dialog, "Comments");
    assert!(window.find(Role::Code, "Formula source").is_some(), "the sheet is out");
    window.key("Right").assert_open(Role::Dialog, "Comments");
}

#[test]
fn a_press_opens_a_comment_when_the_button_comes_up_and_a_drag_selects_instead() {
    let mut window = noted(vec![chased()]);
    let (from, to) = (spot(&mut window, "a", QUOTE.start + 2), spot(&mut window, "a", QUOTE.start + 8));
    window.press_at(from).assert_closed(Role::Dialog, "Comments");
    window.release_at(from).assert_open(Role::Dialog, "Comments");

    // The note stays while its passage is pressed again. A stretch dragged out inside the passage is
    // selected, with the tools over it and no note under it.
    window.advance(2_000).press_at(from).assert_open(Role::Dialog, "Comments");
    window.release_at(to).assert_closed(Role::Dialog, "Comments");
    assert!(window.editor().selection().is_some_and(|selection| !selection.is_caret()), "a stretch is selected");
    assert!(tools(&mut window).is_some(), "the tools show over it");
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
    // Beside the panel the page has no room for cards.
    window.resize(1100.0, 800.0);
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

#[test]
fn a_comment_is_started_from_the_keyboard_on_what_is_selected() {
    let mut window = letter();
    window.select("written quote").key("Mod+Alt+M");
    assert!(window.find(Role::TextInput, "Comment").is_some_and(|field| field.focused), "ready to be typed");
    window.type_text("Chased?").key("Enter");
    assert_eq!(kept(&mut window), [("written quote".to_owned(), "Chased?".to_owned())]);

    // A caret has nothing to comment on.
    caret_at(&mut window, "b", 0);
    window.key("Mod+Alt+M").assert_closed(Role::Dialog, "New comment");
}

/// The keys that step to the next note and to the one before.
const NEXT: &str = "Mod+Alt+PageDown";
const BACK: &str = "Mod+Alt+PageUp";

/// The letter with two comments beside paragraph `a` and an import note beside paragraph `b`.
fn thrice() -> Harness {
    noted(vec![chased(), slates(), note("split", NoteKind::Import, Anchor::block("b"), "Merged cells split")])
}

#[test]
fn the_keyboard_steps_through_the_notes_in_the_order_of_the_page() {
    let (chased, slates, split) = ("Has anyone chased it?", "Welsh ones?", "Merged cells split");
    // The block the caret is in, the keys pressed, and what the note that is open then starts with.
    let cases: [(&str, &[&str], &str); 9] = [
        ("a", &[NEXT], chased),
        ("a", &[NEXT, NEXT], slates),
        ("a", &[NEXT, NEXT, NEXT], split),
        ("a", &[NEXT, NEXT, NEXT, NEXT], chased),
        ("a", &[BACK], slates),
        ("a", &[BACK, BACK, BACK], split),
        ("g", &[NEXT], split),
        ("g", &[BACK], slates),
        ("b", &[BACK, BACK], slates),
    ];
    for (block, keys, starts) in cases {
        let mut window = thrice();
        caret_at(&mut window, block, 0);
        let caret = window.editor().selection().cloned();
        for key in keys {
            window.key(key);
        }
        assert_eq!(reading(&mut window).as_deref(), Some(starts), "from {block}: {keys:?}");
        assert_eq!(window.editor().selection().cloned(), caret, "from {block}: the caret stays where it was");
    }
}

#[test]
fn the_keyboard_steps_only_where_there_is_another_note_to_go_to() {
    // No notes: nothing opens, and the palette does not offer the step.
    let mut window = noted(Vec::new());
    window.key(NEXT);
    assert_eq!(reading(&mut window), None);
    window.key("Mod+Shift+P").type_text("next note");
    assert_eq!(listed(&mut window), [""; 0]);

    // The only note opens, and then there is no other to step to.
    let mut window = noted(vec![chased()]);
    window.key(NEXT);
    assert_eq!(reading(&mut window).as_deref(), Some("Has anyone chased it?"));
    window.key("Mod+Shift+P").type_text("next note");
    assert_eq!(listed(&mut window), [""; 0]);

    // A reply that is being typed keeps its note open.
    let mut window = thrice();
    window.key(NEXT).type_text("On Tuesday").key(NEXT);
    assert_eq!(reading(&mut window).as_deref(), Some("Has anyone chased it?"));

    // A card over the window is answered first: there is no page to open a note on.
    let mut window = thrice();
    window.ui().set_landing(Some(Landing::default()));
    window.key(NEXT);
    window.ui().set_landing(None);
    assert_eq!(reading(&mut window), None);

    // The palette names the step and its key, and takes it.
    let mut window = thrice();
    caret_at(&mut window, "b", 0);
    window.key("Mod+Shift+P").type_text("previous note");
    let row = window.find(Role::ListBoxOption, "Previous note").expect("the step is a command");
    assert_eq!(row.description.as_deref(), Some("Ctrl+Alt+Page Up"));
    window.key("Enter");
    assert_eq!(reading(&mut window).as_deref(), Some("Merged cells split"));
}

#[test]
fn the_first_step_counts_from_the_block_the_caret_is_in_however_deep_it_stands() {
    // A quote between two lines that each have a comment, and the caret in the line the quote holds.
    let line = |id: &str, text: &str| Block::text(id, BlockKind::Paragraph, text);
    let quote = Block { body: Body::Blocks(vec![line("in", "As they wrote")]), ..Block::new("q", BlockKind::Quote) };
    let blocks = vec![line("a", "Above the quote"), quote, line("b", "Below the quote")];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.resize(NARROW, 800.0);
    let about = |id: &str, says: &str| note(id, NoteKind::Comment, Anchor::block(id), says);
    window.ui().set_notes(Notes { notes: vec![about("a", "Above it"), about("b", "Below it")], ..Notes::default() });
    caret_at(&mut window, "in", 0);
    window.key(NEXT);
    assert_eq!(reading(&mut window).as_deref(), Some("Below it"));
}

#[test]
fn a_step_to_a_note_that_shows_leaves_the_page_where_it_is_read() {
    // A long letter with the caret at its top, read further down where a comment is.
    let line = |n: usize| Block::text(format!("p{n}").as_str(), BlockKind::Paragraph, format!("Item {n} of the works"));
    let mut window = Harness::open(Document { blocks: (0..60).map(line).collect(), ..Document::default() });
    window.resize(NARROW, 800.0);
    let far = note("far", NoteKind::Comment, Anchor::block("p40"), "Still to do?");
    window.ui().set_notes(Notes { notes: vec![far], ..Notes::default() });
    caret_at(&mut window, "p0", 0);
    let page = area(&mut window);
    window.scroll_at(Point { x: page.x + page.width / 2.0, y: page.y + page.height / 2.0 }, 1000.0);
    let marker = window.find(Role::Button, "1 comment").expect("the block of the comment is in view").bounds;

    window.key(NEXT);
    assert_eq!(reading(&mut window).as_deref(), Some("Still to do?"));
    let after = window.find(Role::Button, "1 comment").map(|marker| marker.bounds);
    assert_eq!(after, Some(marker), "the page is not brought to the caret on the way");
}

/// The letter in a window with room for cards beside the page, with `notes` on it.
fn beside(notes: Vec<Note>) -> Harness {
    let mut window = noted(notes);
    window.resize(WIDE.0, WIDE.1);
    window
}

/// A window with room for the page and the cards of its notes.
const WIDE: (f32, f32) = (1280.0, 800.0);

/// What an import left on paragraph `b`.
fn split() -> Note {
    note("split", NoteKind::Import, Anchor::block("b"), "Merged cells split")
}

/// The cards beside the page, from the top, once they have come to rest.
fn cards(window: &mut Harness) -> Vec<Found> {
    window.advance(1).advance(1000);
    let card = |found: &Found| found.role == Role::Group && ["Comment", "Import note"].contains(&found.name.as_str());
    window.controls().into_iter().filter(card).collect()
}

/// What the card that is open starts with, when one is.
fn open(window: &mut Harness) -> Option<String> {
    let controls = window.controls();
    let open = controls.iter().position(|found| found.role == Role::Group && found.selected == Some(true))?;
    controls[open..].iter().find(|found| found.role == Role::Paragraph).map(|words| words.name.clone())
}

#[test]
fn every_open_note_shows_on_a_card_beside_the_page_in_the_order_of_the_page() {
    let done = Note { resolved: true, ..note("done", NoteKind::Comment, Anchor::block("b"), "Sent.") };
    let mut window = beside(vec![split(), slates(), done, chased()]);
    let said: Vec<String> = said(&mut window).into_iter().map(|(_, words)| words).collect();
    assert_eq!(said, ["Has anyone chased it?", "Called on Tuesday.", "Welsh ones?", "Merged cells split"]);
    window.assert_closed(Role::Dialog, "Comments");
    assert!(markers(&mut window).is_empty(), "a note with a card needs no marker");

    // They stand right of the text, level with their passages or under one another.
    let text = caret_at(&mut window, "a", 38);
    let (page, all) = (area(&mut window), cards(&mut window));
    assert_eq!(all.len(), 3);
    for (at, card) in all.iter().enumerate() {
        let (x, right) = (card.bounds.x, card.bounds.x + card.bounds.width);
        assert!(x > text.x && right <= page.x + page.width, "{at}: beside the text, in the page area");
        let above = at.checked_sub(1).map_or(page.y, |above| all[above].bounds.y + all[above].bounds.height);
        assert!(card.bounds.y >= above, "{at}: clear of the card above");
    }
    assert!((all[0].bounds.y - text.y).abs() <= text.height, "the first is level with its line: {:?}", all[0].bounds);
    // The page is written on as before, with the cards beside it.
    assert!(window.focused().is_some_and(|found| found.role == Role::Document));
}

#[test]
fn an_import_note_names_no_one_and_takes_no_reply() {
    let mut window = beside(vec![split(), slates()]);
    let who: Vec<String> = said(&mut window).into_iter().map(|(who, _)| who).collect();
    assert_eq!(who, ["Alice Moreau", "Import note"]);
    // Opened, a comment has a reply to write and what an import left has none.
    window.click(Role::Group, "Comment");
    assert!(window.find(Role::TextInput, "Reply").is_some());
    window.click(Role::Group, "Import note");
    assert_eq!(open(&mut window).as_deref(), Some("Merged cells split"));
    assert!(window.find(Role::TextInput, "Reply").is_none());
}

#[test]
fn a_card_comes_to_its_passage_when_the_caret_is_put_there_or_the_card_is_pressed() {
    // Two comments on one line: the second card stands under the first.
    let mut window = beside(vec![chased(), slates()]);
    let line = caret_at(&mut window, "a", 33);
    let on = spot(&mut window, "a", 33);
    let rest = cards(&mut window);
    assert!(rest[1].bounds.y > line.y + line.height && open(&mut window).is_none(), "{:?}", rest[1].bounds);

    // The caret in its passage brings it level with the passage, and the page keeps the keyboard.
    window.click_at(on);
    assert_eq!(open(&mut window).as_deref(), Some("Welsh ones?"));
    let moved = cards(&mut window);
    assert!((moved[1].bounds.y - line.y).abs() <= line.height, "level with its passage: {:?}", moved[1].bounds);
    assert!(moved[0].bounds.y + moved[0].bounds.height <= moved[1].bounds.y, "and the card above made room");
    assert!(window.focused().is_some_and(|found| found.role == Role::Document));
    // It lets go when the caret leaves the passage.
    let away = spot(&mut window, "b", 3);
    window.click_at(away);
    assert_eq!(open(&mut window), None);

    // A press on a card opens it for a reply, and a press on the page leaves it.
    let first = cards(&mut window)[0].center();
    window.click_at(first);
    assert_eq!(open(&mut window).as_deref(), Some("Has anyone chased it?"));
    assert!(window.find(Role::TextInput, "Reply").is_some_and(|field| field.focused), "ready for a reply");
    window.type_text("Twice now.").key("Enter");
    assert_eq!(window.ui().notes().notes[0].replies.len(), 2);
    window.click_at(away);
    assert_eq!(open(&mut window), None);
    // Escape leaves it too, and the page has the keyboard back.
    window.click(Role::Group, "Comment").key("Escape");
    assert_eq!(open(&mut window), None);
    assert!(window.focused().is_some_and(|found| found.role == Role::Document));
}

#[test]
fn a_note_is_resolved_from_its_card() {
    let mut window = beside(vec![chased(), split()]);
    let edits = window.ui().edits();
    // The first card is the comment's.
    window.click(Role::Button, "Resolve");
    let resolved: Vec<bool> = window.ui().notes().notes.iter().map(|note| note.resolved).collect();
    assert_eq!((resolved, window.ui().edits()), (vec![true, false], edits + 1));
    assert_eq!(cards(&mut window).len(), 1);
    assert!(!marked(&mut window, "a", QUOTE), "and its passage is marked no more");

    // Not on a document that takes no comments.
    window.ui().refuse_comments(Some("This document's notes cannot be changed here."));
    assert!(window.find(Role::Button, "Resolve").is_some_and(|resolve| resolve.disabled));
}

#[test]
fn the_cards_keep_inside_the_window_and_lie_over_the_page_where_it_leaves_no_room_beside_it() {
    // How wide the window is, whether the notes have cards, and whether those lie over the page.
    let cases = [(1280.0, true, false), (960.0, true, false), (880.0, true, true), (780.0, false, false)];
    for (width, carded, over) in cases {
        let mut window = beside(vec![chased()]);
        window.resize(width, WIDE.1);
        let (page, all) = (area(&mut window), cards(&mut window));
        assert_eq!((all.len(), markers(&mut window).len()), (usize::from(carded), usize::from(!carded)), "{width}");
        assert_eq!(window.find(Role::Button, "Hide comments").is_some(), over, "{width}: only those can be shut");
        let Some(card) = all.first() else { continue };
        assert!(card.bounds.x + card.bounds.width < page.x + page.width, "{width}: {:?}", card.bounds);
        let start = caret_at(&mut window, "a", 0);
        assert!(start.x >= page.x && card.bounds.x > start.x, "{width}: the page shows from its left margin");
    }
}

#[test]
fn cards_that_lie_over_the_page_can_be_shut_to_read_it_and_opened_again() {
    let mut window = beside(vec![chased()]);
    window.resize(880.0, WIDE.1);
    let before = caret_at(&mut window, "a", 0);
    window.click(Role::Button, "Hide comments");
    assert!(cards(&mut window).is_empty(), "nothing lies over the page");
    assert_eq!(markers(&mut window).len(), 1, "and the note is marked in the margin");
    assert_eq!(caret_at(&mut window, "a", 0), before, "the page stands where it did");

    // They stay shut while the window is one they would lie over the page in.
    window.resize(900.0, WIDE.1);
    assert!(cards(&mut window).is_empty());
    window.click(Role::Button, "Show comments");
    assert_eq!(cards(&mut window).len(), 1);
    assert!(markers(&mut window).is_empty());

    // With room beside the page there is nothing to shut.
    window.click(Role::Button, "Hide comments");
    window.resize(WIDE.0, WIDE.1);
    assert_eq!(cards(&mut window).len(), 1);
    assert!(window.find(Role::Button, "Show comments").is_none());
}

#[test]
fn the_cards_go_with_the_page_as_it_scrolls_and_the_wheel_works_over_them() {
    let line = |n: usize| Block::text(format!("p{n}").as_str(), BlockKind::Paragraph, format!("Item {n} of the works"));
    let mut window = Harness::open(Document { blocks: (0..60).map(line).collect(), ..Document::default() });
    let notes = vec![
        note("near", NoteKind::Comment, Anchor::range("p3", 0..4), "Done?"),
        note("far", NoteKind::Comment, Anchor::range("p50", 0..4), "Still to do?"),
    ];
    window.ui().set_notes(Notes { notes, ..Notes::default() });
    let (line, before) = (caret_at(&mut window, "p3", 0), cards(&mut window));
    assert_eq!(before.len(), 1, "only the card whose passage is near shows");

    window.scroll_at(before[0].center(), 60.0);
    let (moved, after) = (caret_at(&mut window, "p3", 0), cards(&mut window));
    assert!(moved.y < line.y, "the wheel over a card scrolls the page");
    assert_eq!(after[0].bounds.y - before[0].bounds.y, moved.y - line.y, "and the card goes with its line");
}

#[test]
fn the_card_of_a_note_on_a_table_or_a_part_of_it_stands_beside_the_table() {
    let mut window = Harness::doc(
        "Costs so far.\n\nOne.\n\nTwo.\n\nThree.\n\nFour.\n\nAnd as agreed:\n\n| Item | Cost |\n| --- | --- |\n| Slates | 900 |",
    );
    let doc = window.editor().doc().clone();
    let table = doc.blocks.last().expect("a table");
    let row = &table.body.blocks()[1];
    let (head, cells) = (&table.body.blocks()[0].body.blocks()[0], row.body.blocks());
    // What the note is on, and the text it is level with: the table, a row of it, and a cell of that row.
    let cases = [(&table.id, &head.id), (&row.id, &cells[0].id), (&cells[1].id, &cells[1].id)];
    let first = caret_at(&mut window, doc.blocks[0].id.as_str(), 0);
    for (on, level) in cases {
        let top = note("top", NoteKind::Comment, Anchor::block(doc.blocks[0].id.as_str()), "How much?");
        let split = note("split", NoteKind::Import, Anchor::block(on.as_str()), "Merged table cells were split");
        window.ui().set_notes(Notes { notes: vec![top, split], ..Notes::default() });
        let all = cards(&mut window);
        let line = caret_at(&mut window, level.as_str(), 0);
        // As far under the first card as its line is under the first line of the page.
        let (card, down) = (all[1].bounds.y - all[0].bounds.y, line.y - first.y);
        assert!((card - down).abs() < line.height, "{on:?}: the card is level with it: {card} {down}");
    }
}
