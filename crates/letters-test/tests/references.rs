//! References to figures on the page: what the pointer does with "Figure 2".

use letters_model::{Block, BlockId, BlockKind, Body, Document, Mark, Pos, Run, Selection};
use letters_render::{DisplayList, Item, Point, Rect};
use letters_test::{Harness, Role};
use letters_ui::Cursor;

/// How long the light takes to cross a figure, in milliseconds, and a little more.
const PASS_MS: u64 = 1700;

fn picture(id: &str, numbered: bool, caption: &str) -> Block {
    let kind = BlockKind::Figure {
        asset: Some(format!("assets/{id}.png")),
        src: None,
        alt: String::new(),
        width: 300.0,
        height: 180.0,
        numbered,
    };
    Block::text(id, kind, caption)
}

/// A paragraph that says "See Figure N." of a block, with the reference from byte 4.
fn see(id: &str, target: &str) -> Block {
    let reference = Run::marked("Figure N", [Mark::Ref(BlockId::new(target))]);
    Block {
        body: Body::Text(vec![Run::plain("See "), reference, Run::plain(".")]),
        ..Block::new(id, BlockKind::Paragraph)
    }
}

fn filler(id: &str) -> impl Iterator<Item = Block> {
    (0..60).map(move |n| Block::text(format!("{id}{n}").as_str(), BlockKind::Paragraph, "One more line of the survey."))
}

/// A survey that opens on its first figure, `roof`, under paragraphs that refer to each figure. Its second
/// figure is the gallery `gutters`, far below, and under that a last paragraph refers back to the roof.
/// Paragraph `to-x` refers to block `x`: `plain` is a figure without a number, and `gone` is no block.
fn survey() -> Harness {
    let gallery = Block {
        body: Body::Blocks(vec![
            picture("before", false, ""),
            picture("after", false, ""),
            Block::text("said", BlockKind::Caption, "The gutters, before and after"),
        ]),
        ..Block::new("gutters", BlockKind::Gallery { numbered: true })
    };
    let mut blocks =
        vec![see("to-roof", "roof"), see("to-gutters", "gutters"), see("to-plain", "plain"), see("to-gone", "gone")];
    blocks.push(picture("roof", true, "The roof from the south"));
    blocks.push(picture("plain", false, "A slate"));
    blocks.extend(filler("a"));
    blocks.push(gallery);
    blocks.extend(filler("b"));
    blocks.push(see("back-to-roof", "roof"));
    Harness::open(Document { blocks, ..Document::default() })
}

/// The page area.
fn page(window: &mut Harness) -> Rect {
    window.controls().into_iter().find(|found| found.role == Role::Document).expect("the page").bounds
}

/// A point on the reference of a paragraph made by [`see`], which is brought into view for it.
fn reference(window: &mut Harness, block: &str) -> Point {
    // The window is laid out before anything can be brought into view.
    window.paint();
    window.ui().select(Selection::caret(Pos::new(block, 4)));
    window.paint();
    let start = window.ui().caret_rect().expect("the page shows a caret");
    Point { x: start.x + 12.0, y: start.y + start.height / 2.0 }
}

/// Where a frame draws the picture of an asset.
fn drawn(frame: &DisplayList, asset: &str) -> Option<Rect> {
    frame.items.iter().find_map(|item| match item {
        Item::Image { rect, asset: drawn } if drawn.ends_with(asset) => Some(*rect),
        _ => None,
    })
}

fn within(inner: Rect, outer: Rect) -> bool {
    let (right, bottom) = (inner.x + inner.width, inner.y + inner.height);
    inner.x >= outer.x && inner.y >= outer.y && right <= outer.x + outer.width && bottom <= outer.y + outer.height
}

/// What a frame draws over a picture that lets the picture show through: the middle of the brightest of it.
fn light(frame: &DisplayList, picture: Rect) -> Option<f32> {
    let lit = frame.items.iter().filter_map(|item| match item {
        Item::Fill { rect, color, .. } if color.a > 0 && color.a < 255 && rect.intersects(&picture) => {
            Some((color.a, rect.x + rect.width / 2.0))
        }
        _ => None,
    });
    lit.max_by_key(|(alpha, _)| *alpha).map(|(_, middle)| middle)
}

/// The line a frame draws around a picture.
fn marked(frame: &DisplayList, picture: Rect) -> bool {
    frame.items.iter().any(|item| matches!(item, Item::Stroke { rect, .. } if within(picture, *rect)))
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

/// Paints the frame a moment later, as a shell does while something moves.
fn later(window: &mut Harness, ms: u64) -> DisplayList {
    window.advance(ms).paint()
}

#[test]
fn a_press_on_a_reference_brings_its_figure_into_view() {
    // (paragraph pressed, an asset of its figure, whether the page has to move for it)
    for (from, asset, moves) in
        [("to-gutters", "before.png", true), ("back-to-roof", "roof.png", true), ("to-roof", "roof.png", false)]
    {
        let mut window = survey();
        let at = reference(&mut window, from);
        let before = drawn(&window.paint(), asset);
        assert_eq!(before.is_none(), moves, "{from}: {before:?}");

        let frame = window.click_at(at).paint();
        let (figure, page) = (drawn(&frame, asset).expect("the figure shows"), page(&mut window));
        assert!(within(figure, page), "{from}: the whole picture is on the page: {figure:?}");
        assert!(moves || Some(figure) == before, "{from}: a figure in view stays where it is");
        let head = window.editor().selection().expect("a caret").head.block.clone();
        assert_eq!(head.as_str(), from, "the caret waits at the reference");
    }
}

#[test]
fn a_figure_pressed_for_has_the_light_run_over_it_once() {
    let mut window = survey();
    let at = reference(&mut window, "to-gutters");
    window.click_at(at).advance(0);
    let figure = drawn(&later(&mut window, PASS_MS / 2), "before.png").expect("the gallery shows");
    assert!(light(&later(&mut window, 0), figure).is_some(), "the light is on its way over the gallery");
    assert!(light(&later(&mut window, PASS_MS), figure).is_none(), "and gone after one pass");
}

#[test]
fn a_press_that_selects_or_misses_follows_nothing() {
    let mut window = survey();
    let at = reference(&mut window, "to-gutters");
    let beside = Point { x: at.x + 400.0, ..at };
    let roof = drawn(&window.paint(), "roof.png").expect("the roof shows");
    // (where the button goes down, where it comes up)
    for (down, up) in [(at, beside), (beside, at), (beside, beside)] {
        let frame = window.press_at(down).release_at(up).paint();
        assert_eq!(drawn(&frame, "roof.png"), Some(roof), "{down:?} to {up:?} leaves the page where it is");
    }
}

#[test]
fn only_a_reference_to_a_numbered_figure_takes_the_pointer() {
    // (paragraph, whether its reference leads to a figure)
    for (block, leads) in [("to-roof", true), ("to-gutters", true), ("to-plain", false), ("to-gone", false)] {
        let mut window = survey();
        let at = reference(&mut window, block);
        window.hover_at(at);
        let cursor = window.ui().cursor();
        assert_eq!(cursor == Cursor::Pointer, leads, "{block}: {cursor:?}");
        // The words beside it are text to write in, as anywhere on the page.
        window.hover_at(Point { x: at.x - 24.0, ..at });
        assert_eq!(window.ui().cursor(), Cursor::Text, "{block}");
    }
}

#[test]
fn light_runs_over_a_figure_in_view_while_the_pointer_is_on_a_reference_to_it() {
    let mut window = survey();
    let at = reference(&mut window, "to-roof");
    let figure = drawn(&window.paint(), "roof.png").expect("the roof shows");
    assert!(light(&window.paint(), figure).is_none());

    window.hover_at(at).advance(0);
    let early = light(&later(&mut window, PASS_MS / 3), figure).expect("light over the roof");
    let late = light(&later(&mut window, PASS_MS / 3), figure).expect("light over the roof");
    assert!(early < late, "the light goes from left to right: {early} then {late}");
    assert!(light(&later(&mut window, 3 * PASS_MS), figure).is_some(), "and comes again while the pointer stays");
    assert!(
        window.find(Role::Tooltip, "Figure 1: The roof from the south").is_none(),
        "a figure in view needs no preview"
    );

    // The pass it is on is its last once the pointer has gone, and the window comes to rest.
    window.ui().pointer_leave();
    assert!(light(&later(&mut window, 2 * PASS_MS), figure).is_none());
    assert!(rests(&mut window), "nothing is left moving");
}

#[test]
fn a_figure_is_marked_without_motion_for_someone_who_asked_for_less() {
    let mut window = survey();
    window.ui().set_reduced_motion(true);
    let at = reference(&mut window, "to-roof");
    let figure = drawn(&window.paint(), "roof.png").expect("the roof shows");
    assert!(!marked(&window.paint(), figure));

    window.hover_at(at).advance(0);
    assert!(rests(&mut window), "nothing moves");
    let frame = later(&mut window, PASS_MS / 3);
    assert!(marked(&frame, figure) && light(&frame, figure).is_none(), "a line around the roof, and no light");

    window.ui().pointer_leave();
    assert!(!marked(&window.paint(), figure), "the line goes with the pointer");
}

#[test]
fn a_figure_out_of_view_shows_small_beside_the_reference_with_its_caption() {
    // (paragraph pointed at, what its preview says, the assets it shows)
    let cases = [
        ("to-gutters", "Figure 2: The gutters, before and after", vec!["before.png", "after.png"]),
        ("back-to-roof", "Figure 1: The roof from the south", vec!["roof.png"]),
    ];
    for (from, caption, assets) in cases {
        let mut window = survey();
        let at = reference(&mut window, from);
        assert!(window.find(Role::Tooltip, caption).is_none());

        window.hover_at(at);
        let (preview, page) = (window.find(Role::Tooltip, caption).expect("a preview").bounds, page(&mut window));
        assert!(within(preview, page), "{from}: the preview is on the page: {preview:?}");
        let beside = (preview.y - at.y).abs().min((preview.y + preview.height - at.y).abs());
        assert!(beside < 24.0 && !preview.contains(at), "{from}: beside the reference, not over it: {preview:?}");
        let frame = window.paint();
        for asset in assets {
            let small = drawn(&frame, asset).expect("the picture shows");
            assert!(
                within(small, preview) && small.width < 300.0,
                "{from}: {asset} is small in the preview: {small:?}"
            );
        }
        assert!(later(&mut window, PASS_MS).items == window.paint().items, "{from}: nothing moves while it shows");

        window.hover_at(Point { x: at.x - 24.0, ..at });
        assert!(window.find(Role::Tooltip, caption).is_none(), "{from}: the preview goes with the pointer");
    }
}

#[test]
fn a_preview_goes_when_the_person_writes_or_the_figure_comes_into_view() {
    let caption = "Figure 2: The gutters, before and after";
    let mut window = survey();
    let at = reference(&mut window, "to-gutters");
    window.hover_at(at).assert_open(Role::Tooltip, caption);
    window.type_text("x").assert_closed(Role::Tooltip, caption);

    // The pointer has to move again to say where it is.
    window.hover_at(Point { x: at.x + 1.0, ..at }).assert_open(Role::Tooltip, caption);
    window.click_at(at).assert_closed(Role::Tooltip, caption);
}

#[test]
fn a_caption_too_long_for_a_preview_is_cut_short() {
    let long = "A gutter ".repeat(40);
    let blocks =
        vec![see("to-figure", "figure")].into_iter().chain(filler("a")).chain([picture("figure", true, &long)]);
    let mut window = Harness::open(Document { blocks: blocks.collect(), ..Document::default() });
    let at = reference(&mut window, "to-figure");
    window.hover_at(at);
    let said = window.controls().into_iter().find(|found| found.role == Role::Tooltip).expect("a preview").name;
    assert!(said.starts_with("Figure 1: A gutter A gutter") && said.ends_with('…'), "{said}");
    assert!(said.chars().count() < long.len() / 2, "{said}");
}
