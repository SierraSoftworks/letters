//! Formulas: typeset in a line of text or as a block of their own, shown as their source at the caret, and
//! shown as written when they cannot be typeset.

use letters_layout::Palette;
use letters_model::{BlockKind, Document, Mark, Markers, Model, Op, Pos, ReplicaId, Style};
use letters_render::{Color, Item, Point, Rect};

mod common;
use common::*;

const BEFORE: &str = "Mass and energy, ";
const AFTER: &str = ", are one and the same.";
const TEX: &str = r"\frac{a}{b}";

/// A paragraph with `tex` in its line under `mark`, and a paragraph after it.
fn line(tex: &str, mark: Mark) -> Document {
    doc(vec![rich("a", &[(BEFORE, &[]), (tex, &[mark]), (AFTER, &[])]), paragraph("z", "End")])
}

/// `tex` as a block between two paragraphs: a formula, or a paragraph of code to compare its source with.
fn block(tex: &str, typeset: bool) -> Document {
    let middle = if typeset { formula("f", tex) } else { rich("f", &[(tex, &[Mark::Code])]) };
    doc(vec![paragraph("p", "Above"), middle, paragraph("z", "End")])
}

/// A formula somewhere in a document.
struct Case {
    name: &'static str,
    doc: Document,
    /// The same document with the formula's source set as code.
    as_code: Document,
    /// The block the formula is in, and the offsets its source starts and ends at.
    block: &'static str,
    ends: (usize, usize),
    /// A position clear of the formula.
    outside: Pos,
}

/// The same formula in a line of block `a`, and as block `f`.
fn both(tex: &str) -> [Case; 2] {
    let ends = (BEFORE.len(), BEFORE.len() + tex.len());
    let (doc, as_code) = (line(tex, Mark::Math), line(tex, Mark::Code));
    let in_line = Case { name: "in a line", doc, as_code, block: "a", ends, outside: Pos::new("a", 2) };
    let (doc, as_code) = (block(tex, true), block(tex, false));
    let alone = Case { name: "as a block", doc, as_code, block: "f", ends: (0, tex.len()), outside: Pos::new("p", 0) };
    [in_line, alone]
}

/// The items anchored inside `place`: a run by the start of its baseline, a box by its corner, a path by its
/// origin.
fn within(items: &[Item], place: Rect) -> Vec<&Item> {
    let anchor = |item: &Item| match item {
        Item::Glyphs { origin, .. } | Item::Path { origin, .. } => Some(*origin),
        Item::Fill { rect, .. } | Item::Stroke { rect, .. } => Some(Point { x: rect.x, y: rect.y }),
        _ => None,
    };
    let inside = |at: Point| {
        let across = at.x > place.x - 2.0 && at.x < place.x + place.width - 0.5;
        across && at.y > place.y - 2.0 && at.y < place.y + place.height + 2.0
    };
    items.iter().filter(|item| anchor(item).is_some_and(inside)).collect()
}

fn colour(item: &Item) -> Option<Color> {
    match item {
        Item::Path { color, .. } => Some(*color),
        other => color_of(other),
    }
}

/// Each glyph painted and how far across the page it sits.
fn across(items: &[Item]) -> Vec<(u32, f32)> {
    placed(items).into_iter().map(|(glyph, x, _)| (glyph, x)).collect()
}

#[test]
fn a_formula_in_a_line_is_typeset_in_place_of_its_source() {
    for tex in ["E = mc^2", r"\frac{a}{b}", r"\sqrt{x^2 + y^2}", r"\sum_{i=0}^{n} i"] {
        let layout = laid(&line(tex, Mark::Math));
        let (start, end) = (BEFORE.len(), BEFORE.len() + tex.len());
        let place = layout.formula_rect(&Pos::new("a", start)).expect("a formula has a place");
        let items = painted(&layout, everything());
        assert!(!within(&items, place).is_empty(), "{tex}: it is drawn in its place");
        let source = painted(&laid(&line(tex, Mark::Code)), everything());
        assert_ne!(across(&items), across(&source), "{tex}: and not as its source");

        let caret = |offset: usize| layout.caret(&Pos::new("a", offset)).unwrap();
        assert!((caret(start).x - place.x).abs() < 1.0, "{tex}: the text before it ends where it starts");
        assert!((caret(end).x - place.x - place.width).abs() < 1.0, "{tex}: the text after it starts where it ends");
        assert!(caret(end + 2).x > caret(end).x, "{tex}: and runs on from there");
        for inside in start + 1..end {
            assert_eq!(caret(inside), caret(end), "{tex}: offset {inside} of its source is at its end");
            assert_eq!(layout.formula_rect(&Pos::new("a", inside)), Some(place), "{tex}: and still at it");
        }
        assert_eq!(layout.formula_rect(&Pos::new("a", 2)), None, "{tex}: other text is at no formula");

        let middle = place.y + place.height / 2.0;
        let hit = |share: f32| layout.hit(Point { x: place.x + place.width * share, y: middle });
        assert_eq!(hit(0.25), Some(Pos::new("a", start)), "{tex}: a point on its left half is before it");
        assert_eq!(hit(0.75), Some(Pos::new("a", end)), "{tex}: a point on its right half is after it");
    }
}

#[test]
fn a_formula_makes_room_in_its_line() {
    let tall = r"\frac{\frac{a}{b}}{\frac{c}{d}}";
    let (plain, set) = (laid(&line("ab", Mark::Code)), laid(&line(tall, Mark::Math)));
    let place = set.formula_rect(&Pos::new("a", BEFORE.len())).unwrap();
    let next = |layout: &letters_layout::DocLayout| layout.caret(&Pos::new("z", 0)).unwrap();
    assert!(next(&set).y > next(&plain).y, "a tall formula pushes the next block down");
    assert!(place.y >= 0.0 && place.y + place.height <= next(&set).y, "and stays clear of it: {place:?}");
    let beside = set.caret(&Pos::new("a", 2)).unwrap();
    assert!(place.y <= beside.y && beside.y + beside.height <= place.y + place.height + 1.0, "the text sits beside it");
}

#[test]
fn a_formula_block_stands_in_the_middle_of_the_column() {
    for tex in ["E = mc^2", r"\int_0^1 x^2 \, dx = \frac{1}{3}"] {
        let layout = laid(&block(tex, true));
        let place = layout.formula_rect(&Pos::new("f", 0)).expect("a formula has a place");
        assert!((place.x + place.width / 2.0 - WIDTH / 2.0).abs() <= 1.0, "{tex}: centred, at {place:?}");
        assert!(!within(&painted(&layout, everything()), place).is_empty(), "{tex}: it is drawn in its place");
        let (above, below) = (layout.caret(&Pos::new("p", 0)).unwrap(), layout.caret(&Pos::new("z", 0)).unwrap());
        assert!(above.y + above.height <= place.y, "{tex}: under the block above");
        assert!(place.y + place.height <= below.y, "{tex}: over the block below");
    }
}

#[test]
fn a_formula_too_wide_for_the_column_is_set_smaller() {
    let wide = (0..30).map(|term| format!("a_{{{term}}}")).collect::<Vec<_>>().join(" + ");
    let place = |width: f32| laid_at(&block(&wide, true), width).formula_rect(&Pos::new("f", 0)).unwrap();
    let (roomy, tight) = (place(4000.0), place(WIDTH));
    assert!(roomy.width > WIDTH, "the formula wants {} px", roomy.width);
    assert!(tight.width <= WIDTH + 1.0, "and is set to fit the column: {} px", tight.width);
    assert!(tight.height < roomy.height, "by setting it smaller");
}

#[test]
fn a_source_that_cannot_be_typeset_shows_as_written() {
    let warning = Palette::LIGHT.warn.accent;
    for tex in [r"\frac{a}{", "x^{2"] {
        for Case { name, doc, as_code, .. } in both(tex) {
            let items = painted(&laid(&doc), everything());
            let code = painted(&laid(&as_code), everything());
            assert_eq!(across(&items), across(&code), "{tex} {name}: it reads as code does");
            assert!(!inked(&items, warning).is_empty(), "{tex} {name}: in the colour of a warning");
            assert!(inked(&code, warning).is_empty(), "{tex} {name}: which code is not");
        }
    }
    let fine = painted(&laid(&line(TEX, Mark::Math)), everything());
    assert!(inked(&fine, warning).is_empty(), "a formula that is typeset warns of nothing");
}

#[test]
fn a_formula_shows_its_source_while_the_caret_is_in_it() {
    let none = Markers::default();
    for Case { name, doc, as_code, block: id, ends: (start, end), outside } in both(TEX) {
        let mut layout = laid(&doc);
        let typeset = painted(&layout, everything());
        let height = layout.height();
        let source = across(&painted(&laid(&as_code), everything()));

        for offset in [start, start + 3, end] {
            layout.reveal(Some((&outside, &none)));
            assert!(layout.reveal(Some((&Pos::new(id, offset), &none))), "{name}: it opens at {offset}");
            assert_eq!(across(&painted(&layout, everything())), source, "{name}: its source shows at {offset}");
        }
        assert!(!layout.reveal(Some((&Pos::new(id, start + 1), &none))), "{name}: and stays as the caret moves in it");
        let xs: Vec<f32> = (start..=end).map(|offset| layout.caret(&Pos::new(id, offset)).unwrap().x).collect();
        assert!(xs.windows(2).all(|pair| pair[0] < pair[1]), "{name}: the caret steps through the source: {xs:?}");

        assert!(layout.reveal(Some((&outside, &none))), "{name}: it closes when the caret leaves");
        assert_eq!(painted(&layout, everything()), typeset, "{name}: and is typeset as before");
        assert_eq!(layout.height(), height, "{name}: at its height");

        layout.reveal(Some((&Pos::new(id, end), &none)));
        assert!(layout.reveal(None), "{name}: it closes with the caret gone");
        assert_eq!(painted(&layout, everything()), typeset, "{name}: typeset again");
    }
}

#[test]
fn a_formula_stays_typeset_where_its_source_shows_elsewhere() {
    let none = Markers::default();
    for Case { name, doc, block: id, ends: (start, end), .. } in both(TEX) {
        let mut layout = laid(&doc);
        let typeset = painted(&layout, everything());
        let place = layout.formula_rect(&Pos::new(id, start));
        assert!(place.is_some(), "{name}: the formula has a place");

        layout.set_formula_source(false);
        for offset in [start, start + 3, end] {
            let caret = Pos::new(id, offset);
            assert!(!layout.reveal(Some((&caret, &none))), "{name}: nothing opens at {offset}");
            assert_eq!(painted(&layout, everything()), typeset, "{name}: it stays typeset at {offset}");
            assert_eq!(layout.formula_rect(&caret), place, "{name}: in its place at {offset}");
        }

        layout.set_formula_source(true);
        assert!(layout.reveal(Some((&Pos::new(id, start + 1), &none))), "{name}: it opens again when allowed");
        assert_ne!(painted(&layout, everything()), typeset, "{name}: as its source");
        layout.set_formula_source(false);
        assert_eq!(painted(&layout, everything()), typeset, "{name}: and closes when that ends");
    }
}

/// What an edit is called, what it does, and where the caret is before and after it when the page shows one.
type Edit = (&'static str, Vec<Op>, Option<(Pos, Pos)>);

#[test]
fn an_edit_to_a_formula_gives_the_same_layout_as_starting_over() {
    let start = || {
        doc(vec![
            rich("a", &[(BEFORE, &[]), (TEX, &[Mark::Math]), (AFTER, &[])]),
            formula("f", TEX),
            paragraph("z", "End"),
        ])
    };
    let (at, len) = (BEFORE.len(), TEX.len());
    let typing = |block: &str, offset: usize, text: &str| {
        vec![Op::InsertText { at: Pos::new(block, offset), text: text.into() }]
    };
    let cutting = |block: &str, range| vec![Op::DeleteText { block: block.into(), range }];
    let marking = |range, on| vec![Op::SetMark { block: "a".into(), range, mark: Mark::Math, on }];
    let retyping = |block: &str, kind| vec![Op::SetKind { block: block.into(), kind }];
    let edits: Vec<Edit> = vec![
        ("typing in a line's formula", typing("a", at + 1, "x + "), None),
        ("typing in a formula block", typing("f", 0, "x + "), None),
        ("breaking a formula", cutting("f", len - 1..len), None),
        ("emptying a formula", cutting("f", 0..len), None),
        ("cutting a formula out of its line", cutting("a", at..at + len), None),
        ("marking text as a formula", marking(0..4, true), None),
        ("unmarking a formula", marking(at..at + len, false), None),
        ("turning a paragraph into a formula", retyping("z", BlockKind::Formula), None),
        ("turning a formula into a paragraph", retyping("f", BlockKind::Paragraph), None),
        (
            "typing at the caret in a line",
            typing("a", at + 6, "^2"),
            Some((Pos::new("a", at + 6), Pos::new("a", at + 8))),
        ),
        ("typing at the caret in a block", typing("f", len, "+1"), Some((Pos::new("f", len), Pos::new("f", len + 2)))),
        (
            "deleting back from a formula's end",
            cutting("a", at + len - 1..at + len),
            Some((Pos::new("a", at + len), Pos::new("a", at + len - 1))),
        ),
        (
            "leaving a formula for the text after it",
            typing("a", at + len + 2, "so"),
            Some((Pos::new("a", at + len), Pos::new("a", at + len + 4))),
        ),
    ];
    let none = Markers::default();
    for (name, ops, carets) in edits {
        let mut model = Model::new(start(), ReplicaId::from(1));
        let mut layout = laid(model.doc());
        let (before, after) = carets.unzip();
        layout.reveal(before.as_ref().map(|caret| (caret, &none)));
        painted(&layout, everything());
        let change = model.apply(ops).unwrap();
        layout.update(model.doc(), &Style::default(), WIDTH, Some(&change));
        layout.reveal(after.as_ref().map(|caret| (caret, &none)));

        let mut fresh = laid(model.doc());
        fresh.reveal(after.as_ref().map(|caret| (caret, &none)));
        assert_eq!(painted(&layout, everything()), painted(&fresh, everything()), "{name}: the same page");
        assert_eq!(layout.height(), fresh.height(), "{name}: at the same height");
    }
}

#[test]
fn a_formula_takes_the_colour_of_its_text() {
    for Case { name, doc, block: id, ends: (start, _), .. } in both(TEX) {
        let mut layout = laid(&doc);
        for palette in [Palette::LIGHT, dark()] {
            layout.set_palette(palette);
            let place = layout.formula_rect(&Pos::new(id, start)).unwrap();
            let items = painted(&layout, everything());
            let text = glyph_runs(&items).next().and_then(color_of).expect("the text before it");
            let drawn = within(&items, place);
            assert!(!drawn.is_empty(), "{name}: it is drawn");
            assert!(drawn.iter().all(|item| colour(item) == Some(text)), "{name}: in the colour of the text beside it");
        }
    }
}

#[test]
fn a_formula_links_only_where_that_is_allowed() {
    let page = |tex: &str, links: bool| {
        let mut layout = laid(&line(tex, Mark::Math));
        painted(&layout, everything());
        layout.set_maths_links(links);
        painted(&layout, everything())
    };
    let linked = r"\href{https://example.org}{x}";
    assert_ne!(page(linked, true), page(linked, false), "a link is drawn as one only when formulas may link");
    assert_eq!(
        page(linked, false),
        painted(&laid(&line(linked, Mark::Math)), everything()),
        "and they may not at first"
    );
    assert_eq!(page(TEX, true), page(TEX, false), "a formula with no link is the same either way");
}
