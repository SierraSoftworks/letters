//! Two versions of a document on one page: where the page is cut, how the versions share a window, and how
//! places in each version map to the window.

use letters_layout::{Arrangement, DocLayout, Piece, Side, Split, Window};
use letters_model::{Align, BlockId, Document, Pos};
use letters_render::{DisplayList, Item, Point, Rect};

mod common;
use common::*;

const MARGIN: f32 = 30.0;
const HEAD: f32 = 24.0;
/// What one version needs of a window: its text and a margin on both sides.
const VERSION: f32 = WIDTH + 2.0 * MARGIN;
/// A window with room for both versions, one that pins the pair, and one that stacks it.
const WINDOWS: [f32; 3] = [2.0 * VERSION + 80.0, 1.5 * VERSION, VERSION];

/// Paragraphs `a`, `b`, a table `t` of four cells, and paragraphs `d` and `e`. Each block in `rewritten`
/// reads otherwise, and at another length, than in a version that does not name it.
fn version(rewritten: &[&str]) -> Document {
    let text = |id: &str| if rewritten.contains(&id) { PROSE.repeat(2) } else { format!("Block {id} as it was.") };
    let said = |id: &str| paragraph(id, &text(id));
    let cells = ["t00", "t01", "t10", "t11"].map(text);
    let cells = cells.each_ref().map(String::as_str);
    let rows: [&[&str]; 2] = [&cells[..2], &cells[2..]];
    doc(vec![said("a"), said("b"), table("t", &[Align::Start, Align::Start], false, &rows), said("d"), said("e")])
}

/// A version laid out with every height exact, as a split takes the heights it finds.
fn set(rewritten: &[&str]) -> DocLayout {
    let mut layout = laid(&version(rewritten));
    settled(&mut layout);
    layout
}

fn window(width: f32, active: Side) -> Window {
    Window { width, margin: MARGIN, head: HEAD, active, slide: 1.0 }
}

fn ids(blocks: &[&str]) -> Vec<BlockId> {
    blocks.iter().map(|id| BlockId::from(*id)).collect()
}

/// The two versions of a block in a split.
fn pair<'a>(split: &'a Split, block: &str) -> [&'a Piece; 2] {
    let of = |side: Side| {
        let shows = |piece: &&Piece| piece.side == side && piece.block == Some(block.into());
        split.pieces.iter().find(shows).unwrap_or_else(|| panic!("{block} is split"))
    };
    [of(Side::First), of(Side::Second)]
}

fn right(rect: Rect) -> f32 {
    rect.x + rect.width
}

fn bottom(rect: Rect) -> f32 {
    rect.y + rect.height
}

/// How many glyphs a layout paints in a stretch of its column.
fn glyphs_in(layout: &DocLayout, band: (f32, f32)) -> usize {
    placed(&painted(layout, Rect::new(0.0, band.0, WIDTH, band.1 - band.0))).len()
}

#[test]
fn a_window_shows_two_versions_as_its_width_allows() {
    let (first, second) = (set(&[]), set(&["b"]));
    let cases = [
        ("room for both", 2.0 * VERSION, Arrangement::Whole),
        ("room to spare", 2.0 * VERSION + 300.0, Arrangement::Whole),
        ("just short of both", 2.0 * VERSION - 1.0, Arrangement::Pinned),
        ("one and a quarter", 1.25 * VERSION, Arrangement::Pinned),
        ("under one and a quarter", 1.25 * VERSION - 1.0, Arrangement::Stacked),
        ("one page", VERSION, Arrangement::Stacked),
    ];
    for (name, width, arrangement) in cases {
        assert_eq!(Arrangement::of(width, VERSION), arrangement, "{name}");
        for active in [Side::First, Side::Second] {
            let split = Split::new(&first, &second, &ids(&["b"]), &window(width, active));
            assert_eq!(split.arrangement, arrangement, "{name}");
            let [before, after] = pair(&split, "b").map(|piece| piece.rect);
            let whole = |rect: Rect| rect.x >= -0.5 && right(rect) <= width + 0.5;
            if arrangement == Arrangement::Stacked {
                assert!(bottom(before) <= after.y, "{name}: the first is above the second");
                assert!(whole(before) && whole(after), "{name}: each within the window");
                continue;
            }
            assert!(before.y == after.y && right(before) <= after.x, "{name}: they are side by side");
            assert!(before.width >= VERSION && after.width >= VERSION, "{name}: each at the page's measure");
            let (shown, other) = if active == Side::First { (before, after) } else { (after, before) };
            assert!(whole(shown), "{name}: the active version shows whole: {shown:?}");
            assert!(other.x < width && right(other) > 0.0, "{name}: the other shows as far as the window reaches");
            assert_eq!(whole(other), arrangement == Arrangement::Whole, "{name}: whole only with room for both");
            if arrangement == Arrangement::Pinned {
                let gap = if active == Side::First { before.x } else { width - right(after) };
                assert!(gap.abs() < 0.5, "{name}: the active version stands against its margin");
            }
        }
    }
}

#[test]
fn a_pinned_pair_slides_to_the_version_that_becomes_active() {
    let (first, second) = (set(&[]), set(&["b"]));
    let left = |active: Side, slide: f32| {
        let window = Window { slide, ..window(1.5 * VERSION, active) };
        pair(&Split::new(&first, &second, &ids(&["b"]), &window), "b")[0].rect.x
    };
    assert_eq!(left(Side::Second, 0.0), left(Side::First, 1.0), "it sets out from where the first was active");
    assert_eq!(left(Side::First, 0.0), left(Side::Second, 1.0), "and comes back from where the second was");
    let steps = [0.0, 0.25, 0.5, 0.75, 1.0].map(|slide| left(Side::Second, slide));
    assert!(steps.windows(2).all(|step| step[1] < step[0]), "step by step across the window: {steps:?}");
}

/// What a case is called, the blocks asked for, and the pieces down the page: a block's name for each version
/// of it, and nothing for a stretch that both versions share.
type Cut = (&'static str, &'static [&'static str], &'static [&'static str]);

#[test]
fn the_page_is_cut_around_what_both_versions_hold_and_differ_in() {
    let (first, second) = (set(&["e"]), set(&["b", "t01", "t10", "e"]));
    let cases: [Cut; 7] = [
        ("one block", &["b"], &["", "b", "b", ""]),
        ("a cell of a table", &["t01"], &["", "t01", "t01", ""]),
        ("two cells of one table", &["t10", "t01"], &["", "t10", "t10", ""]),
        ("blocks that touch", &["t01", "b"], &["", "b", "b", "t01", "t01", ""]),
        ("the last block", &["e"], &["", "e", "e"]),
        ("a block only one version holds", &["missing"], &[""]),
        ("nothing", &[], &[""]),
    ];
    for (name, blocks, pieces) in cases {
        for width in WINDOWS {
            let split = Split::new(&first, &second, &ids(blocks), &window(width, Side::Second));
            let shows: Vec<Option<BlockId>> = split.pieces.iter().map(|piece| piece.block.clone()).collect();
            let wanted: Vec<Option<BlockId>> =
                pieces.iter().map(|id| (!id.is_empty()).then(|| BlockId::from(*id))).collect();
            assert_eq!(shows, wanted, "{name} in {width}");
            let sides: Vec<Side> =
                split.pieces.iter().filter(|piece| piece.block.is_some()).map(|piece| piece.side).collect();
            assert!(sides.chunks(2).all(|pair| pair == [Side::First, Side::Second]), "{name}: first, then second");

            // The pieces go down the page with no gap, and between them show all of the second version.
            let (mut down, mut row) = (0.0, 0.0);
            for piece in &split.pieces {
                let beside = piece.side == Side::Second && piece.rect.y == row;
                assert!(piece.rect.y == down || beside, "{name}: a piece at {:?} leaves a gap", piece.rect);
                (down, row) = (bottom(piece.rect).max(down), piece.rect.y);
            }
            assert_eq!(split.height, down, "{name}: the page ends with its last piece");
            let seconds = split.pieces.iter().filter(|piece| piece.side == Side::Second);
            let bands: Vec<(f32, f32)> = seconds.map(|piece| piece.band).collect();
            assert!(bands.windows(2).all(|pair| pair[0].1 == pair[1].0), "{name}: nothing is left out: {bands:?}");
            assert_eq!((bands[0].0, bands[bands.len() - 1].1), (0.0, second.height()), "{name}: from top to bottom");
        }
    }
}

#[test]
fn a_place_in_either_version_is_where_its_piece_shows_it() {
    let (first, second) = (set(&[]), set(&["b"]));
    for width in WINDOWS {
        let split = Split::new(&first, &second, &ids(&["b"]), &window(width, Side::Second));
        let [before, after] = pair(&split, "b");
        // Where the window shows a place, and what a point there is taken for.
        let shown = |side: Side, layout: &DocLayout, pos: &Pos| {
            let caret = layout.caret(pos).unwrap();
            let origin = split.origin(side, caret.y)?;
            Some(Rect::new(caret.x + origin.x, caret.y + origin.y, caret.width, caret.height))
        };
        let taken = |at: Rect| split.hit(&first, &second, Point { x: at.x + 1.0, y: at.y + at.height / 2.0 });

        for (side, layout, piece) in [(Side::First, &first, before), (Side::Second, &second, after)] {
            for offset in [0, 9, 18] {
                let pos = Pos::new("b", offset);
                let at = shown(side, layout, &pos).expect("each version of the block shows");
                let rect = piece.rect;
                let inside = at.x >= rect.x && right(at) <= right(rect) && bottom(at) <= bottom(rect);
                assert!(inside && at.y >= rect.y + HEAD, "{width}: {pos:?} is in its piece, under its head: {at:?}");
                assert_eq!(taken(at), Some((side, pos)), "{width}: and a point there is that place");
            }
            let head = Point { x: piece.rect.x + piece.rect.width / 2.0, y: piece.rect.y + HEAD / 2.0 };
            let on = split.hit(&first, &second, head).map(|(side, pos)| (side, pos.block));
            assert_eq!(on, Some((side, "b".into())), "{width}: a point on a head is in the block under it");
        }

        let pair = (before.rect.y.min(after.rect.y), bottom(before.rect).max(bottom(after.rect)));
        for (id, above) in [("a", true), ("d", false), ("e", false)] {
            let pos = Pos::new(id, 3);
            let at = shown(Side::Second, &second, &pos).expect("what both share shows as the second has it");
            assert!(if above { bottom(at) <= pair.0 } else { at.y >= pair.1 }, "{width}: {id} is clear of the pair");
            assert!((at.x + at.width / 2.0 - width / 2.0).abs() < WIDTH / 2.0, "{width}: on the page in the middle");
            assert_eq!(taken(at), Some((Side::Second, pos.clone())), "{width}: a point on {id} is there");
            assert_eq!(shown(Side::First, &first, &pos), None, "{width}: and it shows once");
        }
        assert!(split.height >= second.height() + HEAD, "{width}: the page makes room for the pair");
    }
}

#[test]
fn each_piece_paints_its_own_stretch_of_its_version() {
    let (first, second) = (set(&[]), set(&["b"]));
    for width in WINDOWS {
        let split = Split::new(&first, &second, &ids(&["b"]), &window(width, Side::Second));
        let [before, after] = pair(&split, "b");
        let paint = |viewport: Rect| {
            let mut list = DisplayList::default();
            split.paint(&first, &second, viewport, &mut list);
            list.items
        };

        let items = paint(Rect::new(-width, 0.0, 3.0 * width, split.height));
        let both = glyphs_in(&second, (0.0, second.height())) + glyphs_in(&first, before.band);
        assert_eq!(placed(&items).len(), both, "{width}: all of the second version, and the first's block");

        // Each piece is cut to its place under its head, and what it paints is drawn there.
        let mut cuts = Vec::new();
        for item in &items {
            match item {
                Item::PushClip { rect, .. } => cuts.push(*rect),
                Item::Glyphs { origin, .. } => {
                    let cut = cuts.last().expect("text is painted inside a piece");
                    let inside =
                        origin.x >= cut.x && origin.x <= right(*cut) && origin.y >= cut.y && origin.y <= bottom(*cut);
                    assert!(inside, "{width}: text at {origin:?} is in its piece at {cut:?}");
                }
                _ => {}
            }
        }
        let places: Vec<Rect> = split.pieces.iter().map(|piece| piece.rect).collect();
        assert_eq!(cuts.len(), places.len(), "{width}: one cut for each piece");
        for (cut, place) in cuts.iter().zip(&places) {
            let within = cut.x >= place.x && right(*cut) <= right(*place) && cut.y >= place.y;
            assert!(within && (bottom(*cut) - bottom(*place)).abs() < 0.01, "{width}: {cut:?} is cut to {place:?}");
        }
        let pushes = items.iter().filter(|item| matches!(item, Item::PushClip { .. })).count();
        let pops = items.iter().filter(|item| matches!(item, Item::PopClip)).count();
        assert_eq!(pushes, pops, "{width}: every cut ends");

        // A viewport on the pair alone paints the pair alone, and asks only that of each version.
        let (top, end) = (before.rect.y.min(after.rect.y), bottom(before.rect).max(bottom(after.rect)));
        let on_pair = Rect::new(-width, top, 3.0 * width, end - top);
        let pair = glyphs_in(&first, before.band) + glyphs_in(&second, after.band);
        assert_eq!(placed(&paint(on_pair)).len(), pair, "{width}: only the two versions of the block");
        for (side, piece) in [(Side::First, before), (Side::Second, after)] {
            let shown = split.shown(side, on_pair).expect("each version shows");
            let all = (shown.y - piece.band.0).abs() < 0.01 && (bottom(shown) - piece.band.1).abs() < 0.01;
            assert!(all, "{width}: all of {side:?}'s block is asked for: {shown:?} of {:?}", piece.band);
        }
        let above = Rect::new(0.0, 0.0, width, top / 2.0);
        assert_eq!(split.shown(Side::First, above), None, "{width}: over the pair, nothing of the first shows");
        let shown = split.shown(Side::Second, above).expect("the page above shows");
        assert_eq!((shown.y, shown.height), (0.0, top / 2.0), "{width}: and the top of the second");
    }
}
