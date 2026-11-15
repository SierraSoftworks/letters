//! The copy button in the corner of a code block.

use accesskit::{Action, ActionRequest, TreeId};
use letters_model::{Block, BlockKind, Document, Pos, Selection};
use letters_render::{DisplayList, Item, PathEl, Point, Rect};
use letters_test::{Harness, Role};
use letters_ui::Cursor;
use std::sync::Arc;

const SHORT: &str = "fn main() {\n    run();\n}";

/// A note with one code block, `code`, between two paragraphs. The caret waits in the first.
fn note(code: &str, language: Option<&str>) -> Harness {
    let blocks = vec![
        Block::text("before", BlockKind::Paragraph, "Run this once."),
        Block::text("code", BlockKind::Code { language: language.map(str::to_owned) }, code),
        Block::text("after", BlockKind::Paragraph, "Then restart."),
    ];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.paint();
    window.ui().select(Selection::caret(Pos::new("before", 3)));
    window
}

/// The page area.
fn page(window: &mut Harness) -> Rect {
    window.controls().into_iter().find(|found| found.role == Role::Document).expect("the page").bounds
}

/// The icons a frame draws on the page: where each is, and its shape.
fn icons(window: &mut Harness) -> Vec<(Point, Arc<[PathEl]>)> {
    let (frame, page): (DisplayList, Rect) = (window.paint(), page(window));
    let on_page = frame.items.into_iter().filter_map(|item| match item {
        Item::Path { origin, path, stroke: Some(_), .. } if page.contains(origin) => Some((origin, path)),
        _ => None,
    });
    on_page.collect()
}

/// A point on the one copy button the page shows, and the shape of its icon.
fn button(window: &mut Harness) -> (Point, Arc<[PathEl]>) {
    let icons = icons(window);
    assert_eq!(icons.len(), 1, "one code block has one button");
    let (origin, shape) = icons.into_iter().next().expect("a button");
    (Point { x: origin.x + 4.0, y: origin.y + 4.0 }, shape)
}

/// A point in the text of the code block.
fn in_code(window: &mut Harness) -> Point {
    let held = window.editor().selection().cloned().expect("a caret");
    window.ui().select(Selection::caret(Pos::new("code", 1)));
    window.paint();
    let caret = window.ui().caret_rect().expect("the page shows a caret");
    window.ui().select(held);
    window.paint();
    Point { x: caret.x, y: caret.y + caret.height / 2.0 }
}

/// Whether the window has come to rest: once the work it keeps for idle time is done, it asks for no frame
/// soon.
fn rests(window: &mut Harness) -> bool {
    let now = window.platform.now_ms;
    for _ in 0..10_000 {
        match window.ui().animate(now) {
            Some(next) if next == now => _ = window.paint(),
            next => return next.is_none_or(|next| next > now + 100),
        }
    }
    false
}

fn copied(window: &Harness) -> Option<String> {
    window.platform.clipboard.as_ref().map(|clip| clip.text.clone())
}

#[test]
fn a_press_on_the_button_copies_the_code_as_written() {
    let cases = [
        (SHORT, Some("rust")),
        ("echo hello", None),
        ("  indented:\n\n    - two\n", Some("yaml")),
        ("# not a heading\n**nor bold**", Some("markdown")),
    ];
    for (code, language) in cases {
        let mut window = note(code, language);
        let before = (window.markdown(), window.editor().selection().cloned());
        let (at, _) = button(&mut window);
        window.click_at(at);
        assert_eq!(copied(&window).as_deref(), Some(code), "{language:?}");
        let clip = window.platform.clipboard.clone().expect("a clip");
        assert_eq!(clip.html, None, "code is copied as plain text");
        assert_eq!((window.markdown(), window.editor().selection().cloned()), before, "nothing else changes");
    }
}

#[test]
fn the_button_sits_in_the_top_right_corner_of_the_block() {
    let mut window = note(SHORT, Some("rust"));
    let ((at, _), text, page) = (button(&mut window), in_code(&mut window), page(&mut window));
    assert!(at.x > page.x + page.width / 2.0 && at.x > text.x, "to the right: {at:?}");
    assert!((at.y - text.y).abs() < 24.0, "beside the first line: {at:?} {text:?}");
}

#[test]
fn the_button_says_it_copied_for_two_seconds() {
    let mut window = note(SHORT, None);
    let (at, copy) = button(&mut window);
    window.click_at(at);
    let (_, check) = button(&mut window);
    assert_ne!(check, copy, "the icon changes once the code is copied");
    let (_, still) = button(window.advance(1900));
    assert_eq!(still, check, "it stays for a while");
    let (_, after) = button(window.advance(200));
    assert_eq!(after, copy, "and then the copy icon is back");
    assert!(rests(&mut window), "nothing is left to move");
}

#[test]
fn a_press_that_does_not_end_on_the_button_copies_nothing() {
    let mut window = note(SHORT, None);
    let ((at, _), text) = (button(&mut window), in_code(&mut window));
    // (pressed at, released at)
    for (down, up) in [(at, text), (text, at)] {
        window.press_at(down).release_at(up);
        assert_eq!(copied(&window), None, "{down:?} to {up:?}");
    }
    assert_eq!(window.markdown(), note(SHORT, None).markdown(), "and writes nothing");
}

#[test]
fn the_pointer_says_the_button_can_be_pressed() {
    let mut window = note(SHORT, None);
    let ((at, _), text) = (button(&mut window), in_code(&mut window));
    assert_eq!(window.hover_at(at).ui().cursor(), Cursor::Pointer);
    assert_ne!(window.hover_at(text).ui().cursor(), Cursor::Pointer);
}

#[test]
fn a_first_line_that_reaches_the_corner_has_the_button_only_under_the_pointer() {
    let long = format!("{}\nshort", "long ".repeat(60));
    let mut window = note(&long, None);
    assert_eq!(icons(&mut window).len(), 0, "the button is not drawn over the code");

    let text = in_code(&mut window);
    window.hover_at(text);
    let (at, _) = button(&mut window);
    window.click_at(at);
    assert_eq!(copied(&window), Some(long));

    let away = Point { x: text.x, y: page(&mut window).y + 4.0 };
    // A shell asks what moves with every frame it paints.
    window.hover_at(away).advance(100).paint();
    window.advance(2000);
    assert_eq!(icons(&mut window).len(), 0, "it goes again with the pointer");
}

#[test]
fn assistive_technology_finds_the_button_and_presses_it() {
    let mut window = note(SHORT, Some("rust"));
    assert!(window.find(Role::Button, "Copy code").is_some(), "the block has a button named for what it does");
    let tree = window.ui().access();
    let named = |node: &accesskit::Node| node.role() == Role::Button && node.label() == Some("Copy code");
    let target_node = tree.nodes.iter().find(|(_, node)| named(node)).expect("the button").0;
    window.ask(ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node, data: None });
    assert_eq!(copied(&window).as_deref(), Some(SHORT));
}
