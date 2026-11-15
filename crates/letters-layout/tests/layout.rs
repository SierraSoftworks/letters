//! Document layout through its public interface, with the bundled fonts.

use std::sync::Arc;

use letters_layout::{Callout, DocLayout, FontSpec, Fonts, Palette, Shaper, Token, TokenKind, WordList};
use letters_model::{
    Align, Block, BlockKind, Body, Document, ListKind, Mark, Markers, Model, Op, Pos, ReplicaId, Run, Selection, Style,
    TextStyle, Tone,
};
use letters_render::{Color, DisplayList, Font, Item, Point, Rect};

mod common;
use common::*;

#[test]
fn every_kind_of_block_paints_inside_the_column() {
    let cases: Vec<(&str, Block)> = vec![
        ("paragraph", paragraph("a", PROSE)),
        ("heading", Block::text("a", BlockKind::Heading { level: 1 }, "Rates")),
        ("bullet", Block::text("a", BlockKind::ListItem { list: ListKind::Bullet, depth: 0 }, PROSE)),
        ("number", Block::text("a", BlockKind::ListItem { list: ListKind::Ordered, depth: 1 }, PROSE)),
        ("task", Block::text("a", BlockKind::ListItem { list: ListKind::Task { checked: true }, depth: 0 }, PROSE)),
        ("code", Block::text("a", BlockKind::Code { language: Some("rust".into()) }, "fn main() {\n    run();\n}")),
        ("quote", container("q", BlockKind::Quote, vec![paragraph("a", PROSE)])),
        ("callout", container("c", BlockKind::callout(Tone::Warn), vec![paragraph("a", PROSE)])),
        ("empty callout", container("c", BlockKind::callout(Tone::Info), vec![])),
        ("divider", Block::new("d", BlockKind::Divider)),
        ("unknown", Block::new("u", BlockKind::Unknown { type_name: "chart".into() })),
        ("empty paragraph", paragraph("a", "")),
        ("figure", figure("f", (300.0, 200.0), PROSE)),
        ("linked figure", remote(figure("f", (1200.0, 200.0), ""), "https://example.org/a.png")),
        (
            "gallery",
            gallery("g", (0..4).map(|at| figure(&format!("g{at}"), (300.0, 200.0), "")).collect(), Some(PROSE)),
        ),
        ("table", table("t", &[Align::Start, Align::End], true, &[&["Household", "Rate"], &[PROSE, "9%"]])),
    ];
    for (name, block) in cases {
        let layout = laid(&doc(vec![block]));
        let items = painted(&layout, everything());
        let height = layout.height();
        assert!(height > 0.0, "{name} takes up room");
        assert!(!items.is_empty() || name == "empty paragraph", "{name} draws something");
        for item in &items {
            let Some((top, bottom)) = extent(item) else { continue };
            assert!(top >= -1.0 && bottom <= height + 1.0, "{name} stays inside its height: {item:?}");
            if let Item::Fill { rect, .. } | Item::Stroke { rect, .. } | Item::Image { rect, .. } = item {
                assert!(rect.x >= -0.5 && rect.x + rect.width <= WIDTH + 2.5, "{name} stays inside the column");
            }
        }
    }
}

#[test]
fn marks_change_how_text_is_set() {
    let plain = painted(&laid(&doc(vec![paragraph("a", "Rates change")])), everything());
    let cases = [
        ("bold", Mark::Bold),
        ("italic", Mark::Italic),
        ("underline", Mark::Underline),
        ("strike", Mark::Strike),
        ("highlight", Mark::Highlight),
        ("code", Mark::Code),
    ];
    for (name, mark) in cases {
        let block = Block { body: Body::Text(vec![Run::marked("Rates change", [mark])]), ..paragraph("a", "") };
        let marked = painted(&laid(&doc(vec![block])), everything());
        assert_ne!(plain, marked, "{name} shows");
    }
    let linked = Run { link: Some("https://example.com".into()), ..Run::plain("Rates change") };
    let block = Block { body: Body::Text(vec![linked]), ..paragraph("a", "") };
    assert_ne!(plain, painted(&laid(&doc(vec![block])), everything()), "a link shows");
}

#[test]
fn text_wraps_to_the_width() {
    let doc = prose(1);
    let mut layout = laid(&doc);
    let wide = layout.height();
    layout.update(&doc, &Style::default(), WIDTH / 2.0, None);
    assert!(layout.height() > wide, "a narrower column is taller");
    for item in painted(&layout, everything()) {
        let Item::Glyphs { origin, glyphs, .. } = item else { continue };
        let right = origin.x + glyphs.last().map_or(0.0, |glyph| glyph.x);
        assert!(right <= WIDTH / 2.0 + 1.0, "no glyph starts past the edge");
    }
}

#[test]
fn style_decides_size_and_spacing() {
    let doc = prose(3);
    let mut layout = laid(&doc);
    let before = layout.height();
    let mut style = Style::default();
    let body = TextStyle { size: 16.0, space_after: 24.0, ..style.types["body"].clone() };
    style.types.insert("body".into(), body);
    layout.update(&doc, &style, WIDTH, None);
    assert!(layout.height() > before);
}

#[test]
fn fonts_fall_back_and_may_be_missing() {
    let doc = doc(vec![paragraph("a", "Rates"), Block::text("b", BlockKind::Code { language: None }, "let x;")]);
    let mut style = Style::default();
    for kind in style.types.values_mut() {
        kind.family = "No Such Family".into();
    }
    let mut layout = DocLayout::new(fonts());
    layout.update(&doc, &style, WIDTH, None);
    let items = painted(&layout, everything());
    let fonts: Vec<_> = glyph_runs(&items)
        .map(|item| if let Item::Glyphs { font, .. } = item { &font.data } else { unreachable!() })
        .collect();
    assert_eq!(fonts.len(), 2, "both blocks are drawn with a bundled face");
    assert!(!fonts[0].same(fonts[1]), "code falls back to a monospace face, prose does not");

    let mut bare = DocLayout::new(Fonts::default());
    bare.update(&doc, &Style::default(), WIDTH, None);
    assert!(bare.height() > 0.0, "a layout with no fonts still has lines");
    assert!(bare.caret(&Pos::new("a", 2)).is_some());
}

#[test]
fn paint_emits_only_what_the_viewport_shows() {
    let layout = laid(&prose(400));
    let viewport = Rect::new(0.0, 3000.0, WIDTH, 700.0);
    let items = painted(&layout, viewport);
    assert!(glyph_runs(&items).count() > 10);
    for item in &items {
        let (top, bottom) = extent(item).unwrap();
        assert!(bottom > viewport.y - 40.0 && top < viewport.y + viewport.height + 40.0, "{item:?}");
    }
    assert!(items.len() < 400, "one screen of items, not a document's worth");
}

#[test]
fn opening_lays_out_the_viewport_first() {
    let mut layout = laid(&prose(2000));
    let viewport = Rect::new(0.0, 0.0, WIDTH, 800.0);
    let estimate = layout.height();
    assert!(layout.settle(viewport, 0), "most of the document is still an estimate after one screen");
    assert!(!painted(&layout, viewport).is_empty());

    let mut rounds = 0;
    while layout.settle(viewport, 64 * 1024) {
        rounds += 1;
        assert!(rounds < 10_000, "settling ends");
    }
    assert!(rounds > 1, "settling is spread over many calls");
    let exact = layout.height();
    assert!((estimate - exact).abs() / exact < 0.1, "the estimate {estimate} is near the real height {exact}");
}

#[test]
fn settling_never_moves_what_the_viewport_shows() {
    let mut layout = laid(&prose(300));
    let viewport = Rect::new(0.0, 4000.0, WIDTH, 800.0);
    let anchor = layout.hit(Point { x: 10.0, y: 4010.0 }).unwrap();
    let before = layout.caret(&anchor).unwrap();
    layout.settle(viewport, 0);
    assert_eq!(layout.caret(&anchor).unwrap(), before, "laying out the viewport keeps its top in place");
    settled(&mut layout);
    let after = layout.caret(&anchor).unwrap();
    assert_eq!((after.x, after.height), (before.x, before.height), "blocks above only move it up or down");
}

#[test]
fn hit_and_caret_agree() {
    let doc = doc(vec![
        Block::text("h", BlockKind::Heading { level: 2 }, "Rates"),
        paragraph("a", PROSE),
        container("q", BlockKind::Quote, vec![paragraph("b", PROSE)]),
        Block::text("c", BlockKind::Code { language: None }, "one\ntwo\n\nfour"),
        Block::new("d", BlockKind::Divider),
        Block::text("e", BlockKind::ListItem { list: ListKind::Bullet, depth: 2 }, PROSE),
    ]);
    let layout = laid(&doc);
    for block in doc.walk().filter(|block| matches!(block.body, Body::Text(_))) {
        let text = block.plain_text();
        // A space that a line wraps at draws at the start of the next line, so only test inside words.
        for offset in (0..=text.len()).filter(|at| text.is_char_boundary(*at) && !text[*at..].starts_with(' ')) {
            let pos = Pos::new(block.id.clone(), offset);
            let caret = layout.caret(&pos).unwrap();
            let middle = Point { x: caret.x + 0.25, y: caret.y + caret.height / 2.0 };
            assert_eq!(layout.hit(middle), Some(pos), "at {caret:?}");
        }
    }
}

#[test]
fn caret_follows_the_text() {
    let layout = laid(&prose(2));
    let at = |offset| layout.caret(&Pos::new("p0", offset)).unwrap();
    assert!(at(0).x < at(1).x && at(1).x < at(5).x, "the caret moves right along a line");
    assert_eq!(at(0).y, at(5).y);
    assert!(at(PROSE.len()).y > at(0).y, "and down onto wrapped lines");
    assert!(layout.caret(&Pos::new("p1", 0)).unwrap().y > at(PROSE.len()).y, "blocks stack");
    assert_eq!(at(PROSE.len() + 50), at(PROSE.len()), "an offset past the end sits at the end");
    assert_eq!(layout.caret(&Pos::new("missing", 0)), None);
}

#[test]
fn hit_finds_text_from_anywhere() {
    let layout = laid(&doc(vec![paragraph("a", "First"), Block::new("d", BlockKind::Divider), paragraph("b", "Last")]));
    let height = layout.height();
    let cases = [
        ("above the page", Point { x: 50.0, y: -500.0 }, "a"),
        ("below the page", Point { x: 50.0, y: height + 500.0 }, "b"),
        ("left of the text", Point { x: -80.0, y: 5.0 }, "a"),
        ("right of the text", Point { x: WIDTH * 3.0, y: height - 5.0 }, "b"),
    ];
    for (name, point, block) in cases {
        assert_eq!(layout.hit(point).map(|pos| pos.block.to_string()), Some(block.to_string()), "{name}");
    }
    assert_eq!(layout.hit(Point { x: WIDTH * 3.0, y: height - 5.0 }).unwrap().offset, 4);
    assert_eq!(laid(&Document::default()).hit(Point::default()), None, "an empty document has no positions");
    assert_eq!(laid(&doc(vec![Block::new("d", BlockKind::Divider)])).hit(Point::default()), None);
}

#[test]
fn selection_covers_each_line_it_touches() {
    let doc = prose(3);
    let layout = laid(&doc);
    let line = layout.caret(&Pos::new("p0", 0)).unwrap();
    let lines = |block: &str| {
        let end = layout.caret(&Pos::new(block, PROSE.len())).unwrap();
        let start = layout.caret(&Pos::new(block, 0)).unwrap();
        ((end.y - start.y) / 22.0).round() as usize + 1
    };
    let select = |from: (&str, usize), to: (&str, usize)| {
        layout.selection(&Selection { anchor: Pos::new(from.0, from.1), head: Pos::new(to.0, to.1) })
    };

    assert!(select(("p0", 4), ("p0", 4)).is_empty(), "a caret selects nothing");
    let word = select(("p0", 0), ("p0", 5));
    assert_eq!(word.len(), 1);
    assert!(word[0].width > 20.0 && word[0].y <= line.y && word[0].height >= line.height);
    assert_eq!(select(("p0", 5), ("p0", 0)), word, "direction does not matter");
    assert_eq!(select(("p0", 0), ("p0", PROSE.len())).len(), lines("p0"));
    let all = select(("p0", 0), ("p2", PROSE.len()));
    assert_eq!(all.len(), lines("p0") + lines("p1") + lines("p2"));
    assert!(all.windows(2).all(|pair| pair[0].y < pair[1].y), "one rectangle per line, top to bottom");
    assert!(select(("p0", 0), ("missing", 2)).is_empty());

    let viewport = Rect::new(0.0, 0.0, WIDTH, 30.0);
    let shown = layout.selection_in(&Selection { anchor: Pos::new("p0", 0), head: Pos::new("p2", 9) }, viewport);
    assert!(!shown.is_empty() && shown.len() < all.len(), "only the lines in view");
    assert!(shown.iter().all(|rect| rect.intersects(&Rect::new(0.0, -30.0, WIDTH, 90.0))));
}

#[test]
fn a_selection_shows_the_paragraph_ends_it_crosses() {
    let doc = doc(vec![paragraph("a", "One"), paragraph("b", ""), paragraph("c", "Three")]);
    let layout = laid(&doc);
    let within = layout.selection(&Selection { anchor: Pos::new("a", 0), head: Pos::new("a", 3) });
    let across = layout.selection(&Selection { anchor: Pos::new("a", 0), head: Pos::new("c", 2) });
    assert_eq!(across.len(), 3, "the empty paragraph between shows as selected");
    assert!(across[0].width > within[0].width);
}

#[test]
fn line_step_moves_one_visual_line() {
    let doc = doc(vec![
        paragraph("a", PROSE),
        Block::new("d", BlockKind::Divider),
        container("q", BlockKind::Quote, vec![paragraph("b", PROSE)]),
    ]);
    let layout = laid(&doc);
    // Near the left edge, so that every line reaches this far, even a paragraph's short last one.
    let start = Pos::new("a", 3);
    let x = layout.caret(&start).unwrap().x;
    let mut pos = start.clone();
    let mut ys = vec![layout.caret(&pos).unwrap().y];
    while let Some(next) = layout.line_step(&pos, x, true) {
        let caret = layout.caret(&next).unwrap();
        assert!((caret.x - x).abs() < 12.0, "the caret keeps to its column: {} for {x}", caret.x);
        ys.push(caret.y);
        pos = next;
        assert!(ys.len() < 50);
    }
    assert!(ys.windows(2).all(|pair| pair[0] < pair[1]), "each step goes down a line: {ys:?}");
    assert_eq!(pos.block.as_str(), "b", "stepping crosses the divider into the quote");
    assert_eq!(layout.caret(&pos).unwrap().y, layout.caret(&Pos::new("b", PROSE.len())).unwrap().y, "to the last line");

    let mut back = 0;
    while let Some(previous) = layout.line_step(&pos, x, false) {
        pos = previous;
        back += 1;
    }
    assert_eq!(back, ys.len() - 1, "and the same number of steps back up");
    assert_eq!(pos, start);
}

#[test]
fn line_step_reaches_short_lines() {
    let doc = doc(vec![paragraph("a", PROSE), paragraph("b", "Short"), paragraph("c", PROSE)]);
    let layout = laid(&doc);
    let far = layout.hit(Point { x: WIDTH - 20.0, y: 5.0 }).unwrap();
    let x = layout.caret(&far).unwrap().x;
    let mut pos = far;
    while pos.block.as_str() == "a" {
        pos = layout.line_step(&pos, x, true).unwrap();
    }
    assert_eq!(pos, Pos::new("b", 5), "a line shorter than the column is entered at its end");
    let below = layout.line_step(&pos, x, true).unwrap();
    assert!((layout.caret(&below).unwrap().x - x).abs() < 12.0, "and the column is picked up again below");
}

#[test]
fn home_and_end_follow_the_way_a_line_reads() {
    let cases = [("English", PROSE.to_owned(), false), ("Hebrew", [HEBREW; 16].join(" "), true)];
    for (name, text, rtl) in cases {
        let layout = laid(&doc(vec![paragraph("a", &text)]));
        let at = |offset: usize| Pos::new("a", offset);
        let caret = |pos: &Pos| layout.caret(pos).unwrap();
        let middle = text.char_indices().nth(8).unwrap().0;
        let (home, end) = (layout.line_edge(&at(middle), false).unwrap(), layout.line_edge(&at(middle), true).unwrap());
        assert!(home.offset < middle && middle < end.offset, "{name}: Home goes back in the text and End on");
        assert!(end.offset < text.len(), "{name}: the text wraps");
        assert_eq!((caret(&home).y, caret(&end).y), (caret(&at(middle)).y, caret(&at(middle)).y), "{name}");
        assert_eq!(caret(&end).x < caret(&home).x, rtl, "{name}: the line ends at the side it is read toward");
    }
}

/// Texts whose lines wrap with no space to wrap at, each with an `x` far past the end of its lines.
fn unspaced() -> Vec<(&'static str, String, f32)> {
    vec![
        ("a long word", "abcdefghij".repeat(20), WIDTH * 2.0),
        ("hyphens", "well-known-".repeat(20), WIDTH * 2.0),
        ("Japanese", "今日は天気がいいので散歩に行きます".repeat(6), WIDTH * 2.0),
        ("a long Hebrew word", "שלום".repeat(50), -WIDTH),
    ]
}

#[test]
fn a_line_that_wraps_with_no_space_ends_after_its_last_character() {
    for (name, text, past) in unspaced() {
        let layout = laid(&doc(vec![paragraph("a", &text)]));
        let at = |offset: usize| Pos::new("a", offset);
        let first = layout.caret(&at(0)).unwrap();
        let (end, before_wrap) = layout.line_edge_sided(&at(0), false, true).unwrap();
        let last = text[..end.offset].char_indices().next_back().unwrap().0;
        let second = layout.caret(&end).unwrap();
        assert!(before_wrap, "{name}: the line ends where the next begins");
        assert_eq!(layout.caret(&at(last)).unwrap().y, first.y, "{name}: after its last character");
        assert!(second.y > first.y, "{name}: which a caret from the other side starts the next line at");
        let beside = Point { x: past, y: first.y + first.height / 2.0 };
        assert_eq!(layout.hit_sided(beside), Some((end.clone(), true)), "{name}: a point past the end is there");

        let ended = layout.caret_sided(&end, true).unwrap();
        assert_eq!(ended.y, first.y, "{name}: the caret stays on the line it ends");
        assert!((ended.x - first.x).abs() > (second.x - first.x).abs(), "{name}: at its far side");
        let edge = |before_wrap: bool, end_of: bool| layout.line_edge_sided(&end, before_wrap, end_of).unwrap();
        assert_eq!(edge(true, true), (end.clone(), true), "{name}: End pressed twice stays put");
        assert_eq!(edge(true, false), (at(0), false), "{name}: and Home goes back along the line");
        assert_eq!(edge(false, false), (end.clone(), false), "{name}: from the other side it is the line's start");
        let (below, sided) = layout.line_step_sided(&end, true, ended.x, true).unwrap();
        assert_eq!(layout.caret_sided(&below, sided).unwrap().y, second.y, "{name}: a step down is one line");
        assert_eq!(
            layout.line_step_sided(&below, sided, ended.x, false),
            Some((end.clone(), true)),
            "{name}: and back"
        );

        // A position alone says nothing of its side, so it is kept off an end that needs one.
        let plain = [layout.line_edge(&at(0), true), layout.hit(beside)];
        assert_eq!(plain, [Some(at(last)), Some(at(last))], "{name}: without a side");
    }
}

#[test]
fn a_caret_has_a_side_only_where_a_line_wraps_with_no_space() {
    let layout = laid(&prose(1));
    let at = |offset: usize| Pos::new("p0", offset);
    let (end, before_wrap) = layout.line_edge_sided(&at(0), false, true).unwrap();
    assert_eq!((&PROSE[end.offset..=end.offset], before_wrap), (" ", false), "a wrap at a space ends before it");
    let top = layout.caret(&at(0)).unwrap().y;
    let beside = layout.hit_sided(Point { x: WIDTH * 2.0, y: top + 5.0 });
    assert_eq!(beside, Some((end.clone(), false)), "also for a point past the line's end");
    for offset in [0, 3, end.offset, end.offset + 1, PROSE.len()] {
        let pos = at(offset);
        assert_eq!(layout.caret_sided(&pos, true), layout.caret(&pos), "a side left over moves no caret at {offset}");
        let edges = [false, true].map(|end| layout.line_edge_sided(&pos, true, end).map(|(pos, _)| pos));
        assert_eq!(edges, [false, true].map(|end| layout.line_edge(&pos, end)), "nor a line's edges at {offset}");
    }
}

#[test]
fn an_edit_gives_the_same_layout_as_starting_over() {
    let start = || {
        doc(vec![
            Block::text("h", BlockKind::Heading { level: 1 }, "Rates"),
            paragraph("a", PROSE),
            Block::text("i", BlockKind::ListItem { list: ListKind::Ordered, depth: 0 }, "One"),
            Block::text("j", BlockKind::ListItem { list: ListKind::Ordered, depth: 0 }, "Two"),
            container("c", BlockKind::callout(Tone::Tip), vec![paragraph("b", PROSE)]),
            container("q", BlockKind::Quote, vec![paragraph("d", "Quoted"), paragraph("e", PROSE)]),
            paragraph("z", "End"),
        ])
    };
    let edits: Vec<(&str, Vec<Op>)> = vec![
        ("typing", vec![Op::InsertText { at: Pos::new("a", 5), text: " really".into() }]),
        ("typing a paragraph's worth", vec![Op::InsertText { at: Pos::new("b", 0), text: PROSE.repeat(3) }]),
        ("deleting", vec![Op::DeleteText { block: "a".into(), range: 0..PROSE.len() - 3 }]),
        ("marking", vec![Op::SetMark { block: "a".into(), range: 0..40, mark: Mark::Bold, on: true }]),
        ("linking", vec![Op::SetLink { block: "b".into(), range: 2..9, link: Some("https://example.com".into()) }]),
        ("splitting", vec![Op::Split { at: Pos::new("a", 20), new_id: "n".into() }]),
        ("joining", vec![Op::Join { block: "j".into() }]),
        ("retyping", vec![Op::SetKind { block: "a".into(), kind: BlockKind::Heading { level: 3 } }]),
        ("renumbering", vec![Op::RemoveBlock { block: "i".into() }]),
        ("retoning", vec![Op::SetKind { block: "c".into(), kind: BlockKind::Quote }]),
        ("removing a container", vec![Op::RemoveBlock { block: "c".into() }]),
        ("inserting", vec![Op::InsertBlock { parent: Some("c".into()), index: 0, block: paragraph("n", "New") }]),
        ("turning an item to read from the right", vec![Op::InsertText { at: Pos::new("i", 0), text: HEBREW.into() }]),
        ("turning a quote to read from the right", vec![Op::InsertText { at: Pos::new("d", 0), text: HEBREW.into() }]),
        (
            "several at once",
            vec![
                Op::InsertText { at: Pos::new("z", 3), text: "ing".into() },
                Op::Split { at: Pos::new("b", 30), new_id: "n".into() },
                Op::SetKind { block: "i".into(), kind: BlockKind::Paragraph },
            ],
        ),
    ];
    for (name, ops) in edits {
        let mut model = Model::new(start(), ReplicaId::from(1));
        let mut layout = laid(model.doc());
        painted(&layout, everything());
        let change = model.apply(ops).unwrap();
        layout.update(model.doc(), &Style::default(), WIDTH, Some(&change));

        let fresh = laid(model.doc());
        assert_eq!(painted(&layout, everything()), painted(&fresh, everything()), "{name}");
        assert_eq!(layout.height(), fresh.height(), "{name}");

        let change = model.undo().unwrap();
        layout.update(model.doc(), &Style::default(), WIDTH, Some(&change));
        assert_eq!(painted(&layout, everything()), painted(&laid(&start()), everything()), "undoing {name}");
    }
}

#[test]
fn an_edit_is_exact_at_once() {
    let mut model = Model::new(prose(200), ReplicaId::from(1));
    let mut layout = laid(model.doc());
    let viewport = Rect::new(0.0, 0.0, WIDTH, 600.0);
    painted(&layout, viewport);
    let below = layout.caret(&Pos::new("p3", 0)).unwrap();
    let change = model.apply([Op::InsertText { at: Pos::new("p1", 0), text: PROSE.repeat(2) }]).unwrap();
    layout.update(model.doc(), &Style::default(), WIDTH, Some(&change));
    let moved = layout.caret(&Pos::new("p3", 0)).unwrap();
    assert!(moved.y > below.y + 40.0, "the blocks below a grown paragraph move down with no paint between");
    assert_eq!(painted(&layout, viewport), painted(&laid(model.doc()), viewport));
}

#[test]
fn lists_number_and_indent() {
    let item = |id: &str, list, depth| Block::text(id, BlockKind::ListItem { list, depth }, "Item");
    let doc = doc(vec![
        item("a", ListKind::Ordered, 0),
        item("b", ListKind::Ordered, 1),
        item("c", ListKind::Bullet, 2),
        paragraph("p", "Item"),
    ]);
    let layout = laid(&doc);
    let x = |id: &str| layout.caret(&Pos::new(id, 0)).unwrap().x;
    assert!(x("p") < x("a") && x("a") < x("b") && x("b") < x("c"), "each level sits further in");
    let items = painted(&layout, everything());
    let left_of_text = |from: f32, to: f32| {
        items.iter().any(|item| match item {
            Item::Glyphs { origin, .. } => origin.x >= from && origin.x < to,
            Item::Fill { rect, .. } | Item::Stroke { rect, .. } => rect.x >= from && rect.x + rect.width <= to,
            _ => false,
        })
    };
    assert!(left_of_text(x("p"), x("a") - 1.0), "the first item has a marker in its indent");
    assert!(left_of_text(x("a"), x("b") - 1.0) && left_of_text(x("b"), x("c") - 1.0));
}

const HEBREW: &str = "שלום עולם";
const ARABIC: &str = "مرحبا بالعالم";

/// How far across the page an item reaches, loosely: glyphs from the first of them to the last.
fn across(item: &Item) -> Option<(f32, f32)> {
    match item {
        Item::Fill { rect, .. } | Item::Stroke { rect, .. } => Some((rect.x, rect.x + rect.width)),
        Item::Glyphs { origin, glyphs, .. } => {
            let at = glyphs.iter().map(|glyph| origin.x + glyph.x);
            Some((at.clone().fold(f32::MAX, f32::min), at.fold(f32::MIN, f32::max)))
        }
        _ => None,
    }
}

#[test]
fn a_text_reads_from_the_side_its_first_letter_does() {
    let item = |block: Block| Block { kind: BlockKind::ListItem { list: ListKind::Bullet, depth: 0 }, ..block };
    let cases: Vec<(&str, Block, bool)> = vec![
        ("Latin", paragraph("a", "Hello"), false),
        ("Hebrew", paragraph("a", HEBREW), true),
        ("Arabic", paragraph("a", ARABIC), true),
        ("a heading", Block::text("a", BlockKind::Heading { level: 2 }, HEBREW), true),
        ("Hebrew after a number", paragraph("a", "12. שלום"), true),
        ("Hebrew after Latin", paragraph("a", "A שלום"), false),
        ("Hebrew after a formula", item(rich("a", &[("x", &[Mark::Math]), (" שלום", &[])])), true),
        ("code", Block::text("a", BlockKind::Code { language: None }, HEBREW), false),
    ];
    for (name, block, right) in cases {
        let mut layout = laid(&doc(vec![block]));
        let leftmost = |layout: &DocLayout| {
            let glyphs = placed(&painted(layout, everything()));
            glyphs.iter().map(|glyph| glyph.1).fold(f32::MAX, f32::min)
        };
        assert_eq!(leftmost(&layout) > WIDTH / 2.0, right, "{name}");
        // A formula shows its source while the caret is in it, which is no letter of the text either.
        layout.reveal(Some((&Pos::new("a", 1), &Markers::default())));
        assert_eq!(leftmost(&layout) > WIDTH / 2.0, right, "{name}, with the caret in it");
    }
}

/// Thai as it is written, without spaces: words a word list keeps whole.
const THAI: [&str; 9] = ["ภาษา", "ไทย", "เป็น", "ภาษา", "ราชการ", "ของ", "ประเทศ", "ไทย", "และ"];

/// How many lines of a paragraph of [`THAI`] start inside a word, and how many lines it has.
fn broken_words(layout: &DocLayout, text: &str) -> (usize, usize) {
    let line = |offset: usize| layout.caret(&Pos::new("a", offset)).unwrap().y;
    let offsets: Vec<usize> = text.char_indices().map(|(at, _)| at).collect();
    let starts = offsets.windows(2).filter(|pair| line(pair[1]) > line(pair[0])).map(|pair| pair[1]);
    let starts: Vec<usize> = starts.collect();
    let mut between = vec![0];
    while between[between.len() - 1] < text.len() {
        between.push(between[between.len() - 1] + THAI[(between.len() - 1) % THAI.len()].len());
    }
    (starts.iter().filter(|start| !between.contains(start)).count(), starts.len() + 1)
}

#[test]
fn a_line_of_thai_ends_between_words_once_its_word_list_is_here() {
    let text = THAI.concat().repeat(4);
    let page = doc(vec![paragraph("a", &text)]);
    let mut layout = laid_at(&page, 300.0);
    painted(&layout, everything());
    // A face for Thai comes first, as it does where the lists are fetched: the text then lacks only its list.
    let thai = Font::new(Arc::from(font_test_data::LAST_RESORT_HE), 0);
    assert!(layout.add_fallback_fonts(&Fonts { faces: vec![thai.clone()] }), "the text has its glyphs");
    painted(&layout, everything());
    let list = WordList::Thai.data();
    assert!(broken_words(&layout, &text).1 > 3, "the text runs over several lines");

    // A build either has the lists in it, or is handed each when a document turns out to need it.
    if !WordList::BUILT_IN {
        assert!(broken_words(&layout, &text).0 > 0, "without the list a line ends wherever it fills");
        assert_eq!(layout.take_unlisted(), [WordList::Thai], "the script is told of");
        assert_eq!(layout.take_unlisted(), [], "and only once");
        // A file that is not the list would be read as words it does not hold.
        let damaged: [(&str, Vec<u8>); 4] = [
            ("nothing", vec![]),
            ("a file cut short", list[..list.len() / 2].to_vec()),
            ("a changed file", list.iter().map(|byte| byte.wrapping_add(1)).collect()),
            ("another script's list", WordList::Lao.data()),
        ];
        for (name, data) in damaged {
            assert!(!layout.add_word_list(WordList::Thai, &data), "{name} is refused");
        }
        assert!(layout.add_word_list(WordList::Thai, &list), "the page may look different with the list");
    }
    assert_eq!(layout.take_unlisted(), [], "a script is not told of where its list is");
    assert!(!layout.add_word_list(WordList::Thai, &list), "a list that is here already changes nothing");
    assert_eq!(broken_words(&layout, &text).0, 0, "no line starts inside a word");

    let mut fresh = DocLayout::new(fonts());
    fresh.add_fallback_fonts(&Fonts { faces: vec![thai] });
    fresh.add_word_list(WordList::Thai, &list);
    fresh.update(&page, &Style::default(), 300.0, None);
    assert_eq!(painted(&layout, everything()), painted(&fresh, everything()), "as if the list had always been here");
    assert_eq!(layout.height(), fresh.height());
}

#[test]
fn each_script_written_without_spaces_asks_for_its_own_word_list() {
    let cases = [
        (WordList::Thai, "ภาษาไทย"),
        (WordList::Lao, "ພາສາລາວ"),
        (WordList::Khmer, "ភាសាខ្មែរ"),
        (WordList::Burmese, "မြန်မာဘာသာ"),
    ];
    for (list, text) in cases {
        let page = doc(vec![paragraph("a", "日本語 and English"), paragraph("b", text)]);
        let mut layout = laid(&page);
        painted(&layout, everything());
        let asked = layout.take_unlisted();
        assert_eq!(asked, if WordList::BUILT_IN { vec![] } else { vec![list] }, "{list:?}");
        assert!(list.data().len() > 100_000, "{list:?} has a list to hand over");
        assert_eq!(layout.add_word_list(list, &list.data()), !WordList::BUILT_IN, "{list:?} is taken");

        let mut ahead = laid(&page);
        painted(&ahead, everything());
        ahead.add_word_list(list, &list.data());
        assert_eq!(ahead.take_unlisted(), [], "{list:?} is not asked for once it is here");
    }
}

#[test]
fn a_list_item_is_marked_and_set_in_at_the_side_it_starts_at() {
    const LOUD: Color = Color::rgb(1, 200, 3);
    let palette = Palette { marker: LOUD, accent: LOUD, on_accent: LOUD, line_strong: LOUD, ..Palette::LIGHT };
    let lists = [
        ("a bullet", ListKind::Bullet),
        ("a number", ListKind::Ordered),
        ("a ticked box", ListKind::Task { checked: true }),
        ("an unticked box", ListKind::Task { checked: false }),
    ];
    for (list, kind) in lists {
        for (script, text, right) in [("Latin", "Hello", false), ("Hebrew", HEBREW, true), ("Arabic", ARABIC, true)] {
            let item = Block::text("a", BlockKind::ListItem { list: kind, depth: 1 }, text);
            let mut layout = laid(&doc(vec![paragraph("p", text), item]));
            layout.set_palette(palette);
            let start = |id: &str| layout.caret(&Pos::new(id, 0)).unwrap().x;
            let (edge, inner) = (start("p"), start("a"));
            assert_eq!(inner < edge, right, "{list} in {script}: it is set in from where a paragraph starts");

            let items = painted(&layout, everything());
            let marks: Vec<(f32, f32)> =
                items.iter().filter(|item| color_of(item) == Some(LOUD)).filter_map(across).collect();
            assert!(!marks.is_empty(), "{list} in {script}: it has a marker");
            let (low, high) = (edge.min(inner), edge.max(inner));
            assert!(
                marks.iter().all(|(from, to)| low <= *from && *to <= high),
                "{list} in {script}: the marker stands in that indent, {marks:?} in {low}..{high}"
            );
        }
    }
}

#[test]
fn a_number_reads_the_way_its_item_does() {
    const LOUD: Color = Color::rgb(1, 200, 3);
    let number = |text: &str| {
        let item = Block::text("a", BlockKind::ListItem { list: ListKind::Ordered, depth: 0 }, text);
        let mut layout = laid(&doc(vec![item]));
        layout.set_palette(Palette { marker: LOUD, ..Palette::LIGHT });
        let mut marker: Vec<Item> = painted(&layout, everything());
        marker.retain(|item| color_of(item) == Some(LOUD));
        let mut glyphs = placed(&marker);
        glyphs.sort_by(|a, b| a.1.total_cmp(&b.1));
        glyphs.into_iter().map(|glyph| glyph.0).collect::<Vec<u32>>()
    };
    let (left, mut right) = (number("Hello"), number(HEBREW));
    assert!(left.len() > 1, "a number has its stop");
    right.reverse();
    assert_eq!(left, right, "the stop is between the number and the text on either side");
}

#[test]
fn a_quotes_bar_stands_where_its_first_text_starts() {
    const LOUD: Color = Color::rgb(1, 200, 3);
    let quote = |id: &str, inside: Vec<Block>| container(id, BlockKind::Quote, inside);
    let cases: Vec<(&str, Vec<Block>, bool)> = vec![
        ("Latin", vec![paragraph("a", "Hello")], false),
        ("Hebrew", vec![paragraph("a", HEBREW)], true),
        ("Arabic", vec![paragraph("a", ARABIC)], true),
        ("Hebrew after Latin", vec![paragraph("a", "Hello"), paragraph("b", HEBREW)], false),
        ("Latin after Hebrew", vec![paragraph("a", HEBREW), paragraph("b", "Hello")], true),
        ("a quote in Hebrew", vec![quote("r", vec![paragraph("a", HEBREW)])], true),
        ("code in Hebrew", vec![Block::text("a", BlockKind::Code { language: None }, HEBREW)], false),
    ];
    for (name, inside, right) in cases {
        let mut layout = laid(&doc(vec![quote("q", inside)]));
        layout.set_palette(Palette { line_strong: LOUD, ..Palette::LIGHT });
        let items = painted(&layout, everything());
        let (bars, rest): (Vec<&Item>, Vec<&Item>) = items.iter().partition(|item| color_of(item) == Some(LOUD));
        let reach = |items: &[&Item]| {
            let spans = items.iter().filter_map(|item| across(item));
            spans.fold((f32::MAX, f32::MIN), |all, span| (all.0.min(span.0), all.1.max(span.1)))
        };
        let (bar, text) = (reach(&bars), reach(&rest));
        let start = layout.caret(&Pos::new("a", 0)).unwrap();
        assert!(bar.0 >= 0.0 && bar.1 <= WIDTH, "{name}: the bar is in the column");
        match right {
            true => {
                assert!(text.1 <= bar.0, "{name}: the bar is right of all it holds, {text:?} and {bar:?}");
                assert!(start.x + start.width <= bar.0, "{name}: and its text starts clear of the bar");
            }
            false => {
                assert!(bar.1 <= text.0, "{name}: the bar is left of all it holds, {bar:?} and {text:?}");
                assert!(bar.1 <= start.x, "{name}: and its text starts clear of the bar");
            }
        }
    }
}

#[test]
fn a_callout_is_barred_and_headed_where_its_first_text_starts() {
    const OWN: Color = Color::rgb(0xaa, 0x33, 0x66);
    let cases: Vec<(&str, Vec<Block>, bool)> = vec![
        ("Latin", vec![paragraph("a", "Hello")], false),
        ("Hebrew", vec![paragraph("a", HEBREW)], true),
        ("Arabic", vec![paragraph("a", ARABIC)], true),
        ("Hebrew after Latin", vec![paragraph("a", "Hello"), paragraph("b", HEBREW)], false),
        ("Latin after Hebrew", vec![paragraph("a", HEBREW), paragraph("b", "Hello")], true),
    ];
    for (script, inside, right) in cases {
        for (head, title) in [("its tone's title", None), ("an icon alone", Some(String::new()))] {
            let name = format!("{script} under {head}");
            let titled = title.is_none();
            let kind = BlockKind::Callout { tone: Tone::Warn, title, icon: None, color: Some("#aa3366".into()) };
            let layout = laid(&doc(vec![container("c", kind, inside.clone())]));
            let items = painted(&layout, everything());
            let own = |item: &&Item| color_of(item) == Some(OWN);
            let reach = |spans: Vec<(f32, f32)>| {
                spans.into_iter().fold((f32::MAX, f32::MIN), |all, span| (all.0.min(span.0), all.1.max(span.1)))
            };
            let bar = reach(
                items.iter().filter(|item| matches!(item, Item::Fill { .. })).filter(own).filter_map(across).collect(),
            );
            let written = reach(
                items
                    .iter()
                    .filter(|item| matches!(item, Item::Glyphs { .. }))
                    .filter(own)
                    .filter_map(across)
                    .collect(),
            );
            let icons: Vec<f32> = items
                .iter()
                .filter_map(|item| match item {
                    Item::Path { origin, .. } => Some(origin.x),
                    _ => None,
                })
                .collect();
            let [icon] = icons.as_slice() else { panic!("{name}: one icon, not {icons:?}") };
            let start = layout.caret(&Pos::new("a", 0)).unwrap();
            let heads = layout.callouts(everything());
            let [(_, pressed)] = heads.as_slice() else { panic!("{name}: one head, not {heads:?}") };
            let (from, to) = (pressed.x, pressed.x + pressed.width);

            assert!(bar.0 >= 0.0 && bar.1 <= WIDTH && from >= 0.0 && to <= WIDTH, "{name}: all of it is in the column");
            assert!(
                (from - 0.5..=to).contains(icon),
                "{name}: a press on the icon is on the head, {icon} in {pressed:?}"
            );
            match right {
                true => {
                    assert!(start.x + start.width <= bar.0, "{name}: the bar is right of the text, {bar:?}");
                    assert!(from > WIDTH / 2.0 && to <= bar.0, "{name}: the head is at the bar's side, {pressed:?}");
                }
                false => {
                    assert!(bar.1 <= start.x, "{name}: the bar is left of the text, {bar:?}");
                    assert!(to < WIDTH / 2.0 && bar.1 <= from, "{name}: the head is at the bar's side, {pressed:?}");
                }
            }
            if titled {
                assert!(from - 0.5 <= written.0 && written.1 <= to, "{name}: and one on the title, {written:?}");
                assert_eq!(*icon > written.1, right, "{name}: the icon leads the title, {icon} and {written:?}");
            } else {
                assert_eq!(start.x <= *icon, right, "{name}: the text starts past the icon, {icon} and {start:?}");
            }
        }
    }
}

#[test]
fn plain_strings_are_shaped_for_widgets() {
    let shaper = Shaper::new(fonts());
    let font = FontSpec::new(letters_fonts::SANS, 13.0);
    let text = "Save changes";
    let label = shaper.shape(text, &font, None);
    assert!(label.width > 40.0 && label.height >= 13.0);
    assert!(label.baseline > 0.0 && label.baseline < label.height);
    assert_eq!(label.stops.first(), Some(&(0, 0.0)));
    let (end, right) = *label.stops.last().unwrap();
    assert_eq!(end, text.len());
    assert!((right - label.width).abs() < 0.5, "the last stop is the right edge");
    assert!(label.stops.windows(2).all(|pair| pair[0].0 < pair[1].0 && pair[0].1 <= pair[1].1));

    assert!(shaper.shape(text, &FontSpec { weight: 700, ..font }, None).width > label.width, "weight is applied");
    let wrapped = shaper.shape(text, &font, Some(label.width / 2.0));
    assert!(wrapped.height > label.height && wrapped.width < label.width, "a width wraps the text");
    let empty = shaper.shape("", &font, None);
    assert!(empty.runs.is_empty() && empty.height > 0.0, "an empty string keeps a line's height");
    assert_eq!(empty.stops, [(0, 0.0)]);

    let mut list = DisplayList::default();
    label.paint(Point { x: 10.0, y: 20.0 }, Color::rgb(0, 0, 0), &mut list);
    let [Item::Glyphs { origin, glyphs, .. }] = &list.items[..] else { panic!("one run of glyphs") };
    assert_eq!((origin.x, origin.y), (10.0, 20.0 + label.baseline));
    assert_eq!(glyphs.len(), text.len());
}

#[test]
fn the_palette_colours_what_the_style_does_not() {
    const LOUD: Color = Color::rgb(1, 200, 3);
    let light = Palette::LIGHT;
    let item = |list| Block::text("a", BlockKind::ListItem { list, depth: 0 }, "Item");
    let run = |run: Run| Block { body: Body::Text(vec![run]), ..paragraph("a", "") };
    let warning = || container("c", BlockKind::callout(Tone::Warn), vec![paragraph("a", "Rates")]);
    let cases: Vec<(&str, Palette, Block)> = vec![
        (
            "a link",
            Palette { link: LOUD, ..light },
            run(Run { link: Some("https://a.example".into()), ..Run::plain("a") }),
        ),
        ("a bullet", Palette { marker: LOUD, ..light }, item(ListKind::Bullet)),
        ("a number", Palette { marker: LOUD, ..light }, item(ListKind::Ordered)),
        ("a ticked box", Palette { accent: LOUD, ..light }, item(ListKind::Task { checked: true })),
        ("a tick", Palette { on_accent: LOUD, ..light }, item(ListKind::Task { checked: true })),
        ("an unticked box", Palette { line_strong: LOUD, ..light }, item(ListKind::Task { checked: false })),
        ("a divider", Palette { line: LOUD, ..light }, Block::new("d", BlockKind::Divider)),
        (
            "a quote",
            Palette { line_strong: LOUD, ..light },
            container("q", BlockKind::Quote, vec![paragraph("a", "Rates")]),
        ),
        (
            "a code block",
            Palette { code: LOUD, ..light },
            Block::text("a", BlockKind::Code { language: None }, "let x;"),
        ),
        ("inline code", Palette { inline_code: LOUD, ..light }, run(Run::marked("x", [Mark::Code]))),
        ("a highlight", Palette { highlight: LOUD, ..light }, run(Run::marked("x", [Mark::Highlight]))),
        ("a callout's bar", Palette { warn: Callout { accent: LOUD, ..light.warn }, ..light }, warning()),
        ("a callout's fill", Palette { warn: Callout { fill: LOUD, ..light.warn }, ..light }, warning()),
        (
            "an unsupported block",
            Palette { marker: LOUD, ..light },
            Block::new("u", BlockKind::Unknown { type_name: "chart".into() }),
        ),
    ];
    for (name, palette, block) in cases {
        let mut layout = laid(&doc(vec![block]));
        let before = painted(&layout, everything());
        let height = layout.height();
        assert!(!before.iter().any(|item| color_of(item) == Some(LOUD)), "{name} starts in the light palette");
        layout.set_palette(palette);
        let after = painted(&layout, everything());
        assert!(after.iter().any(|item| color_of(item) == Some(LOUD)), "{name} takes the palette's colour");
        assert_eq!((after.len(), layout.height()), (before.len(), height), "{name} keeps its layout");
    }
}

#[test]
fn a_dark_page_recolours_the_style() {
    let (light, dark) = (Palette::LIGHT, dark());
    let near = |a: Color, b: Color| [(a.r, b.r), (a.g, b.g), (a.b, b.b)].iter().all(|(a, b)| a.abs_diff(*b) <= 2);
    let brightness = |color: Color| u32::from(color.r) + u32::from(color.g) + u32::from(color.b);
    let (navy, brick) = (Color::rgb(0x1a, 0x3c, 0x8e), Color::rgb(0xa0, 0x1c, 0x1c));

    let cases = [("the light page's text", light.text, dark.text), ("the light page", light.page, dark.page)];
    for (name, written, shown) in cases {
        assert_eq!(light.recolor(written), written, "a light page shows {name} as written");
        assert!(near(dark.recolor(written), shown), "{name} becomes {:?}", dark.recolor(written));
    }
    let greys: Vec<u32> = (0..=255).step_by(15).map(|v| brightness(dark.recolor(Color::rgb(v, v, v)))).collect();
    assert!(greys.windows(2).all(|pair| pair[0] >= pair[1]), "darker ink is lighter text, in order: {greys:?}");
    let (blue, red) = (dark.recolor(navy), dark.recolor(brick));
    assert!(blue.b > blue.r && blue.b > blue.g && brightness(blue) > brightness(navy), "navy stays blue: {blue:?}");
    assert!(red.r > red.g && red.r > red.b && brightness(red) > brightness(brick), "brick stays red: {red:?}");
    assert_eq!(dark.recolor(Color::rgba(10, 20, 30, 77)).a, 77, "transparency is kept");

    let mut style = Style::default();
    style.types.get_mut("body").unwrap().color = "#1a3c8e".into();
    style.types.get_mut("heading1").unwrap().color = "not a colour".into();
    let blocks = vec![Block::text("h", BlockKind::Heading { level: 1 }, "Rates"), paragraph("a", "Rates")];
    let mut layout = DocLayout::new(fonts());
    layout.update(&doc(blocks), &style, WIDTH, None);
    for (palette, body) in [(light, navy), (dark, blue)] {
        layout.set_palette(palette);
        let items = painted(&layout, everything());
        let colors: Vec<_> = glyph_runs(&items).filter_map(color_of).collect();
        assert_eq!(colors, [palette.text, body], "a colour that cannot be read falls back to the palette's text");
    }
}

const CODE: &str =
    "fn main() {\n    // rates change on the first of January, and every household should check the notice first\n}";

fn code() -> Document {
    doc(vec![Block::text("c", BlockKind::Code { language: Some("rust".into()) }, CODE), paragraph("p", "Rates")])
}

fn token(word: &str, kind: TokenKind) -> Token {
    let start = CODE.find(word).unwrap();
    Token { range: start..start + word.len(), kind }
}

#[test]
fn tokens_colour_code_and_move_nothing() {
    use TokenKind::*;
    let syntax = Palette::LIGHT.syntax;
    let mut layout = laid(&code());
    let plain = painted(&layout, everything());
    let height = layout.height();
    let comment = &CODE[CODE.find("//").unwrap()..CODE.rfind('\n').unwrap()];

    let kinds = [
        (Comment, syntax.comment),
        (Keyword, syntax.keyword),
        (Literal, syntax.number),
        (Number, syntax.number),
        (String, syntax.string),
        (Type, syntax.type_name),
        (Function, syntax.function),
        (Property, syntax.property),
        (Tag, syntax.property),
        (Variable, syntax.property),
        (Meta, syntax.number),
        (Inserted, syntax.string),
        (Deleted, syntax.property),
    ];
    for (kind, color) in kinds {
        layout.set_tokens(&"c".into(), vec![token("main", kind)]);
        let items = painted(&layout, everything());
        assert_eq!(inked(&items, color).len(), 4, "{kind:?} colours the four letters of its token");
        assert_eq!(placed(&items), placed(&plain), "{kind:?} moves no glyph");
    }

    // A comment long enough to wrap, between two other tokens.
    layout.set_tokens(&"c".into(), vec![token(comment, Comment), token("main", Function), token("fn", Keyword)]);
    let items = painted(&layout, everything());
    assert_eq!(placed(&items), placed(&plain), "tokens move no glyph, across wrapped lines too");
    assert_eq!(layout.height(), height);
    assert_eq!(inked(&items, syntax.keyword).len(), 2, "tokens may be given in any order");
    assert_eq!(inked(&items, syntax.comment).len(), comment.len(), "every letter of the comment, on both lines");
    let slanted = |items: &[Item], color| {
        glyph_runs(items)
            .filter(|item| color_of(item) == Some(color))
            .all(|item| matches!(item, Item::Glyphs { skew, .. } if *skew > 0.0))
    };
    assert!(slanted(&items, syntax.comment) && !slanted(&items, syntax.keyword), "only comments are set in italics");

    layout.set_tokens(&"c".into(), Vec::new());
    assert_eq!(painted(&layout, everything()), plain, "no tokens is plain code again");
    layout.set_tokens(&"p".into(), vec![Token { range: 0..5, kind: Keyword }]);
    layout.set_tokens(&"missing".into(), vec![Token { range: 0..5, kind: Keyword }]);
    assert_eq!(painted(&layout, everything()), plain, "only code blocks take tokens");
}

#[test]
fn tokens_follow_the_text_through_edits() {
    let keyword = Palette::LIGHT.syntax.keyword;
    let cases: Vec<(&str, Vec<Op>, usize)> = vec![
        ("typing before it", vec![Op::InsertText { at: Pos::new("c", 0), text: "pub ".into() }], 4),
        ("typing after it", vec![Op::InsertText { at: Pos::new("c", 9), text: "x".into() }], 4),
        ("typing at its end", vec![Op::InsertText { at: Pos::new("c", 7), text: "ly".into() }], 6),
        ("typing inside it", vec![Op::InsertText { at: Pos::new("c", 5), text: "x".into() }], 5),
        ("deleting part of it", vec![Op::DeleteText { block: "c".into(), range: 2..5 }], 2),
        ("deleting all of it", vec![Op::DeleteText { block: "c".into(), range: 0..9 }], 0),
        ("splitting the block after it", vec![Op::Split { at: Pos::new("c", 11), new_id: "n".into() }], 4),
    ];
    for (name, ops, coloured) in cases {
        let mut model = Model::new(code(), ReplicaId::from(1));
        let mut layout = laid(model.doc());
        layout.set_tokens(&"c".into(), vec![token("main", TokenKind::Keyword)]);
        let before = inked(&painted(&layout, everything()), keyword);
        let change = model.apply(ops).unwrap();
        layout.update(model.doc(), &Style::default(), WIDTH, Some(&change));
        let after = inked(&painted(&layout, everything()), keyword);
        assert_eq!(after.len(), coloured, "{name}");
        if coloured == before.len() {
            assert_eq!(after, before, "{name} leaves the same letters coloured");
        }
        layout.update(model.doc(), &Style::default(), WIDTH / 2.0, None);
        assert_eq!(inked(&painted(&layout, everything()), keyword), after, "{name}, then a new width");
    }
}

#[test]
fn the_code_blocks_in_view_are_told_with_the_boxes_drawn_behind_them() {
    let filler = (0..80).map(|n| paragraph(&format!("p{n}"), PROSE));
    let mut blocks = vec![paragraph("a", "Rates"), Block::text("first", BlockKind::Code { language: None }, "let x;")];
    blocks.extend(filler);
    blocks.push(Block::text("last", BlockKind::Code { language: Some("rust".into()) }, CODE));
    blocks.push(Block::new("f", BlockKind::Formula));
    let layout = laid(&doc(blocks));

    let all = layout.code_boxes(everything());
    let ids: Vec<&str> = all.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["first", "last"], "code alone, in order");
    let fills: Vec<Rect> = painted(&layout, everything())
        .iter()
        .filter_map(|item| match item {
            Item::Fill { rect, color, .. } if *color == Palette::LIGHT.code => Some(*rect),
            _ => None,
        })
        .collect();
    for (id, rect) in &all {
        assert!(fills.contains(rect), "{id}: {rect:?} is a box that is painted: {fills:?}");
        let text = layout.caret(&Pos::new(id.as_str(), 0)).expect("a caret");
        assert!(rect.contains(Point { x: text.x, y: text.y + 1.0 }), "{id}: the box holds its text");
    }

    let top = Rect::new(0.0, 0.0, WIDTH, 200.0);
    let shown: Vec<_> = layout.code_boxes(top).into_iter().map(|(id, _)| id).collect();
    assert_eq!(shown.iter().map(|id| id.as_str()).collect::<Vec<_>>(), ["first"], "only what the view shows");
}

#[test]
fn text_is_read_in_the_language_named_nearest_to_it() {
    // Romanian sets its ş and ţ another way than other languages do, and no bundled face has Han.
    const TEXT: &str = "Şţ şi ţară 漢";
    let glyphs =
        |layout: &DocLayout| placed(&painted(layout, everything())).iter().map(|glyph| glyph.0).collect::<Vec<_>>();
    // The glyphs a page is set in from the start, and the language its Han is told with.
    let read = |page: &Document| {
        let mut layout = laid(page);
        settled(&mut layout);
        let told: Vec<_> = layout.take_uncovered().into_iter().map(|missing| missing.lang).collect();
        (glyphs(&layout), told)
    };
    let quoted = || doc(vec![container("q", BlockKind::Quote, vec![paragraph("x", TEXT)])]);
    let tabled = || doc(vec![table("t", &[Align::Start], false, &[&[TEXT]])]);
    let named = |block: &str, lang: &str| {
        let lang = Some(lang.to_owned());
        Op::SetBlockProps { block: block.into(), page_break: false, lang }
    };
    let stretch = |block: &str, lang: &str| {
        let lang = Some(lang.to_owned());
        Op::SetLang { block: block.into(), range: 0..TEXT.len(), lang }
    };
    // An English page, what then names a language in it, and whether its text is Romanian for that.
    let cases = [
        ("the stretch itself", quoted(), vec![stretch("x", "ro")], true),
        ("its block", quoted(), vec![named("x", "ro")], true),
        ("a block around it", quoted(), vec![named("q", "ro")], true),
        ("a stretch of a cell", tabled(), vec![stretch("t00", "ro")], true),
        ("its cell", tabled(), vec![named("t00", "ro")], true),
        ("its row", tabled(), vec![named("t0", "ro")], true),
        ("its table", tabled(), vec![named("t", "ro")], true),
        ("a stretch, over its block", quoted(), vec![named("x", "ro"), stretch("x", "en")], false),
        ("a block, over the one around it", quoted(), vec![named("q", "ro"), named("x", "en")], false),
        ("a tag that is no language", quoted(), vec![named("x", "not a language")], false),
    ];
    for (name, page, ops, foreign) in cases {
        let (english, romanian) = (read(&page), read(&Document { language: "ro".into(), ..page.clone() }));
        assert!(english.0 != romanian.0, "{name}: a language sets its letters its own way");
        assert_eq!(romanian.1, [Some("ro".to_owned())], "{name}: and a character without a face is told with it");
        let wanted = if foreign { romanian } else { english };

        let mut model = Model::new(page, ReplicaId::from(1));
        let mut layout = laid(model.doc());
        settled(&mut layout);
        let change = model.apply(ops).expect("a language can be named");
        layout.update(model.doc(), &Style::default(), WIDTH, Some(&change));
        settled(&mut layout);
        assert!(glyphs(&layout) == wanted.0, "{name}: the text is set again for the language");
        assert!(read(model.doc()) == wanted, "{name}: as on a page that starts with it, Han told with it");
    }
}

#[test]
fn a_callout_is_headed_by_its_icon_and_title_in_its_colour() {
    const OWN: Color = Color::rgb(0xaa, 0x33, 0x66);
    let chosen = |title: Option<&str>, icon: Option<&str>, color: Option<&str>| {
        let own = |text: Option<&str>| text.map(str::to_owned);
        let kind = BlockKind::Callout { tone: Tone::Warn, title: own(title), icon: own(icon), color: own(color) };
        laid(&doc(vec![container("c", kind, vec![paragraph("a", "Rates")])]))
    };
    let icons = |layout: &DocLayout| {
        let items = painted(layout, everything());
        items.iter().filter(|item| matches!(item, Item::Path { .. })).count()
    };
    let caret = |layout: &DocLayout| layout.caret(&Pos::new("a", 0)).expect("the text has a caret");

    let plain = chosen(None, None, None);
    let (titled, untitled) = (chosen(Some("Before the roofers arrive"), None, None), chosen(Some(""), None, None));
    let bare = chosen(Some(""), Some("none"), None);
    // The tone gives a title and an icon. A title has a line of its own, and an icon alone stands beside the text.
    assert_eq!(spelt(&plain).matches("Warning").count(), 1, "the tone's title shows: {}", spelt(&plain));
    assert!(spelt(&titled).contains("Before the roofers arrive") && !spelt(&titled).contains("Warning"));
    assert_eq!((icons(&plain), icons(&untitled), icons(&bare)), (1, 1, 0));
    assert_eq!(titled.height(), plain.height(), "a title takes one line, whatever it says");
    assert!(untitled.height() < plain.height() && bare.height() == untitled.height());
    assert!(caret(&untitled).x > caret(&bare).x, "the text makes room for an icon beside it");
    assert!((caret(&plain).x - caret(&bare).x).abs() < 0.01, "and for none above it");

    // What a press on the callout's head is on: its icon and title, inside its box and above its text.
    for (name, layout) in [("the tone's", &plain), ("a title", &titled), ("an icon alone", &untitled)] {
        let heads = layout.callouts(everything());
        let [(block, head)] = heads.as_slice() else { panic!("{name}: one callout has one head, not {heads:?}") };
        assert_eq!(block.as_str(), "c");
        assert!(head.width > 0.0 && head.x >= 0.0 && head.x + head.width <= WIDTH, "{name}: {head:?}");
        assert!(head.y < caret(layout).y + caret(layout).height, "{name}: {head:?}");
    }
    assert!(bare.callouts(everything()).is_empty(), "nothing to press where nothing shows");

    // A colour of its own colours the bar and the title, and a dark page turns it over to be read.
    let mut own = chosen(None, None, Some("#aa3366"));
    let light = painted(&own, everything());
    let in_own = light.iter().filter(|item| color_of(item) == Some(OWN)).count();
    assert!(in_own >= 2, "the bar and the title take the colour: {in_own}");
    assert!(!light.iter().any(|item| color_of(item) == Some(Palette::LIGHT.warn.accent)));
    own.set_palette(dark());
    let dim = painted(&own, everything());
    assert!(
        !dim.iter().any(|item| color_of(item) == Some(OWN)),
        "a colour for paper is not drawn as it is on a dark page"
    );
}
