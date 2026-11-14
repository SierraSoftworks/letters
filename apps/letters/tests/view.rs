use letters::platform::Desktop;
use letters::view::View;
use letters_editor::{Editor, InputEvent, Key, KeyEvent, Modifiers};
use letters_layout::{Palette, Scheme};
use letters_model::{Block, BlockKind, Document, Style};
use letters_render::{Color, DisplayList, Item, Point, Size};

const WINDOW: Size = Size { width: 800.0, height: 600.0 };
/// A point over the text, a little below the top of the window.
const IN_TEXT: Point = Point { x: 400.0, y: 300.0 };

/// Paragraphs of uneven length in wide letters, so that a guess at their heights from the amount of text
/// comes out short.
fn document(blocks: usize) -> Document {
    let block = |n: usize| {
        Block::text(format!("p{n}").as_str(), BlockKind::Paragraph, "WARM MAMMOTHS SWIM. ".repeat(1 + n % 9))
    };
    Document { blocks: (0..blocks).map(block).collect(), ..Document::default() }
}

/// A view of that document in a window, with its first frame painted.
fn view(blocks: usize) -> View {
    let mut view = View::new(Editor::new(document(blocks)), Style::default());
    view.resize(WINDOW, 1.0);
    paint(&mut view);
    view
}

fn paint(view: &mut View) -> DisplayList {
    let mut list = DisplayList::default();
    view.paint(&mut list);
    list
}

/// Where each run of text was painted.
fn text(list: &DisplayList) -> Vec<Point> {
    let origin = |item: &Item| match item {
        Item::Glyphs { origin, .. } => Some(*origin),
        _ => None,
    };
    list.items.iter().filter_map(origin).collect()
}

#[test]
fn a_frame_holds_only_what_the_window_shows() {
    let (mut fills_window, mut long) = (view(40), view(4000));
    let (short, long) = (text(&paint(&mut fills_window)), text(&paint(&mut long)));
    assert!(!short.is_empty(), "the text is painted");
    assert!(long.len() <= short.len(), "{} runs of a long document, {} of a short one", long.len(), short.len());
}

#[test]
fn typing_goes_in_at_the_caret_and_asks_for_a_frame() {
    let mut view = view(3);
    let caret = view.caret_rect().expect("a document opens with a caret");
    assert!(!view.needs_paint(), "nothing is owed after a frame");

    view.input(InputEvent::Insert("Hello".into()), &mut Desktop::new());
    assert!(view.needs_paint());
    assert_eq!(view.edits(), 1);
    assert!(view.editor().doc().blocks[0].plain_text().starts_with("HelloWARM"));
    assert!(view.caret_rect().unwrap().x > caret.x, "the caret follows the text");
}

#[test]
fn a_click_puts_the_caret_there_and_a_drag_selects() {
    let mut view = view(40);
    let platform = &mut Desktop::new();
    let start = view.editor().selection().cloned();

    view.pointer_down(IN_TEXT, Modifiers::default(), platform);
    let clicked = view.editor().selection().cloned().unwrap();
    assert!(clicked.is_caret());
    assert_ne!(Some(&clicked), start.as_ref(), "the caret moved to the click");
    assert!(view.needs_paint());

    view.pointer_move(Point { x: IN_TEXT.x + 120.0, y: IN_TEXT.y + 90.0 }, platform);
    let dragged = view.editor().selection().cloned().unwrap();
    assert_eq!(dragged.anchor, clicked.head);
    assert!(!dragged.is_caret());

    view.pointer_up(IN_TEXT, platform);
    view.pointer_move(Point { x: 300.0, y: 100.0 }, platform);
    assert_eq!(view.editor().selection(), Some(&dragged), "a pointer that is up selects nothing");
    assert_eq!(view.edits(), 0, "selecting is not an edit");
}

#[test]
fn the_page_scrolls_within_the_document() {
    let top = Point { x: 0.0, y: 0.0 };
    let mut short = view(1);
    short.scroll(top, Point { x: 0.0, y: 500.0 });
    assert!(!short.needs_paint(), "a document that fits has nowhere to scroll");

    let mut long = view(400);
    let first = text(&paint(&mut long));
    long.scroll(top, Point { x: 0.0, y: 500.0 });
    assert!(long.needs_paint());
    assert_ne!(text(&paint(&mut long)), first);
    long.scroll(top, Point { x: 0.0, y: -5000.0 });
    assert_eq!(text(&paint(&mut long)), first, "scrolling stops at the top");
}

#[test]
fn idle_time_lays_out_the_rest_and_holds_the_page_still() {
    let mut view = view(4000);
    view.scroll(Point { x: 0.0, y: 0.0 }, Point { x: 0.0, y: 30_000.0 });
    let before = text(&paint(&mut view));

    // The clock stands still, so the caret's blink asks for nothing.
    let now_ms = 1;
    let mut steps = 0;
    while view.animate(now_ms).is_some_and(|wake| wake <= now_ms) {
        steps += 1;
        assert!(steps < 10_000, "laying out the rest never ends");
    }
    assert!(steps > 1, "a long document is not laid out all at once");

    let after = text(&paint(&mut view));
    assert_eq!(after.len(), before.len());
    for (after, before) in after.iter().zip(&before) {
        assert!((after.y - before.y).abs() < 0.5 && after.x == before.x, "text moved from {before:?} to {after:?}");
    }
}

#[test]
fn a_new_palette_recolours_the_page_at_the_next_frame() {
    let mut view = view(3);
    let page = Color::rgb(13, 17, 23);
    view.set_palette(Palette { scheme: Scheme::Dark, page, ..Palette::LIGHT });
    assert!(view.needs_paint());

    let list = paint(&mut view);
    assert!(matches!(list.items.first(), Some(Item::Fill { color, .. }) if *color == page), "{:?}", list.items.first());
    assert_eq!(view.palette().page, page, "the window is cleared to the same colour");
}

#[test]
fn keys_move_the_caret_along_and_between_the_laid_out_lines() {
    let mut view = view(40);
    let platform = &mut Desktop::new();
    let mut press = |view: &mut View, key: Key, primary: bool| {
        let used = view.key(KeyEvent { key, mods: Modifiers { primary, ..Modifiers::default() } }, platform);
        (used, view.caret_rect().unwrap())
    };
    view.pointer_down(IN_TEXT, Modifiers::default(), &mut Desktop::new());
    let clicked = view.caret_rect().unwrap();

    let (_, end) = press(&mut view, Key::End, false);
    assert!(end.x > clicked.x && end.y == clicked.y, "End went from {clicked:?} to {end:?}");
    let (_, home) = press(&mut view, Key::Home, false);
    assert!(home.x < clicked.x && home.y == clicked.y, "Home went from {end:?} to {home:?}");
    let (_, down) = press(&mut view, Key::Down, false);
    assert!(down.y > home.y, "Down went from {home:?} to {down:?}");
    let (_, up) = press(&mut view, Key::Up, false);
    assert_eq!(up.y, home.y);

    let (used, _) = press(&mut view, Key::Char('s'), true);
    assert!(!used, "a key the editor has no use for is left to the shell");
    assert_eq!(view.edits(), 0, "moving is not an edit");
}

#[test]
fn a_second_click_takes_the_word_and_a_third_the_block() {
    let mut view = view(40);
    let platform = &mut Desktop::new();
    let mut click = |view: &mut View, at: Point| {
        view.pointer_down(at, Modifiers::default(), platform);
        view.pointer_up(at, platform);
        let selection = view.editor().selection().cloned().unwrap();
        let (from, to) =
            (selection.anchor.offset.min(selection.head.offset), selection.anchor.offset.max(selection.head.offset));
        assert_eq!(selection.anchor.block, selection.head.block);
        (view.editor().doc().block(&selection.head.block).unwrap().plain_text(), from..to)
    };

    let (_, caret) = click(&mut view, IN_TEXT);
    assert!(caret.is_empty());
    let (text, word) = click(&mut view, IN_TEXT);
    assert!(!word.is_empty() && !text[word.clone()].contains(' '), "selected {:?}", &text[word]);
    let (text, block) = click(&mut view, IN_TEXT);
    assert_eq!(block, 0..text.len());

    let (_, elsewhere) = click(&mut view, Point { x: IN_TEXT.x + 60.0, y: IN_TEXT.y });
    assert!(elsewhere.is_empty(), "a click somewhere else starts again");
}

#[test]
fn text_being_composed_is_underlined_until_it_is_committed() {
    let mut view = view(3);
    let platform = &mut Desktop::new();
    let ink = view.palette().text;
    let underlines = |view: &mut View| {
        let underline =
            |item: &Item| matches!(item, Item::Fill { rect, color, .. } if *color == ink && rect.height <= 2.0);
        paint(view).items.iter().filter(|item| underline(item)).count()
    };
    assert_eq!(underlines(&mut view), 0);

    view.input(InputEvent::SetComposition { text: "ni".into(), selection: None }, platform);
    assert_eq!(underlines(&mut view), 1);
    view.input(InputEvent::Commit("你".into()), platform);
    assert_eq!(underlines(&mut view), 0);
    assert!(view.editor().doc().blocks[0].plain_text().starts_with("你WARM"));
}
