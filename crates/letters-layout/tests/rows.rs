//! Figures, galleries and tables: a picture over its caption, pictures tiled in rows, rows of cells, and the
//! numbers and references that follow the figures of a document.

use letters_layout::{DocLayout, FigureBox, Palette, figure_label, figure_numbers};
use letters_model::{Align, Block, BlockId, BlockKind, Document, Mark, Model, Op, Pos, ReplicaId, Selection, Style};
use letters_render::{Font, Item, Point, Rect};

mod common;
use common::*;

/// The pictures a page shows, each with the entry of the document it is read from.
fn images(items: &[Item]) -> Vec<(&str, Rect)> {
    let mut images = Vec::new();
    for item in items {
        if let Item::Image { rect, asset } = item {
            images.push((asset.as_str(), *rect));
        }
    }
    images
}

fn middle(rect: Rect) -> Point {
    Point { x: rect.x + rect.width / 2.0, y: rect.y + rect.height / 2.0 }
}

fn bottom(rect: Rect) -> f32 {
    rect.y + rect.height
}

fn right(rect: Rect) -> f32 {
    rect.x + rect.width
}

/// How far a picture drawn at `rect` is from the shape of one stored at `size`, as a share of that shape.
fn stretch(rect: Rect, size: (f32, f32)) -> f32 {
    ((rect.width / rect.height) / (size.0 / size.1) - 1.0).abs()
}

fn picture(id: &str, caption: &str) -> Block {
    figure(id, (150.0, 100.0), caption)
}

/// The pictures of [`tiles`], of differing shapes: `g0`, `g1` and so on, each with its size.
fn named(count: usize) -> Vec<(String, (f32, f32))> {
    const SHAPES: [(f32, f32); 4] = [(300.0, 200.0), (200.0, 300.0), (400.0, 400.0), (320.0, 180.0)];
    (0..count).map(|at| (format!("g{at}"), SHAPES[at % 4])).collect()
}

/// A gallery `g` of `count` pictures.
fn tiles(count: usize, caption: Option<&str>) -> Block {
    gallery("g", named(count).iter().map(|(id, size)| figure(id, *size, "")).collect(), caption)
}

/// What the cells of [`rates`] hold. One wraps at any width.
const RATES: [[&str; 3]; 3] = [["Household", "Band", "Rate"], [PROSE, "B", "12.5%"], ["Mill cottage", "C", "9%"]];

/// A table `t` of households, their bands and their rates, every column set from its start.
fn rates(header: bool) -> Block {
    table("t", &[Align::Start; 3], header, &[&RATES[0], &RATES[1], &RATES[2]])
}

#[test]
fn a_figure_is_its_picture_over_its_caption() {
    // Sizes in points: two that fit the column, one wider than it, and one tall and thin with no caption.
    let cases = [
        ((150.0, 100.0), "The harbour office"),
        ((300.0, 200.0), PROSE),
        ((1200.0, 400.0), "The harbour office"),
        ((60.0, 240.0), ""),
    ];
    for (size, text) in cases {
        let blocks = vec![paragraph("p", "Above"), figure("f", size, text), paragraph("z", "End")];
        let layout = laid(&doc(blocks));
        let items = painted(&layout, everything());
        let FigureBox { image, caption, badge } = layout.figure(&"f".into()).expect("a figure has a place");
        assert_eq!(images(&items), [("assets/f.png", image)], "{size:?}: its picture is drawn in its place");
        assert_eq!(badge, None, "{size:?}: a picture the document holds has no cloud");
        assert!(stretch(image, size) < 0.03, "{size:?}: the picture keeps its shape: {image:?}");
        assert!(image.x >= 0.0 && right(image) <= WIDTH + 0.5, "{size:?}: inside the column: {image:?}");
        let stored = size.0 * 96.0 / 72.0;
        assert!((image.width - stored.min(WIDTH)).abs() <= 1.0, "{size:?}: at its size, or the column's: {image:?}");

        let caret = |block: &str, offset| layout.caret(&Pos::new(block, offset)).unwrap();
        let (above, first, last, below) = (caret("p", 0), caret("f", 0), caret("f", text.len()), caret("z", 0));
        assert!(bottom(above) <= image.y, "{size:?}: under the block above");
        assert!(bottom(image) <= first.y, "{size:?}: the caption is under the picture");
        assert!(caption.y <= first.y + 0.5, "{size:?}: where the figure says it is: {caption:?}");
        if !text.is_empty() {
            assert!(bottom(last) <= bottom(caption) + 0.5, "{size:?}: all of it: {caption:?}");
        }
        // With no caption, its place is where one would start, in the space under the picture.
        let figure = if text.is_empty() { bottom(image) } else { bottom(last) };
        assert!(figure <= below.y, "{size:?}: and the block below is under the figure");
    }
}

#[test]
fn a_picture_still_a_link_shows_where_it_comes_from() {
    // A link, the host it names, and something else in it that is nobody's to read off the page.
    let cases = [
        ("https://images.example.org/harbour/office.png?size=large", "images.example.org", "office.png"),
        ("https://reader:secret@cdn.example.net/a.png#top", "cdn.example.net", "secret"),
    ];
    for (src, host, private) in cases {
        let linked = remote(figure("f", (300.0, 200.0), "The harbour"), src);
        let layout = laid(&doc(vec![linked, paragraph("z", "End")]));
        let items = painted(&layout, everything());
        let FigureBox { image, badge, .. } = layout.figure(&"f".into()).unwrap();
        assert!(images(&items).is_empty(), "{src}: nothing is fetched to draw it");
        let frame = |item: &Item| matches!(item, Item::Stroke { rect, .. } if *rect == image);
        assert!(items.iter().any(frame), "{src}: a frame stands where the picture will");
        let cloud = badge.expect("a cloud marks it as a link");
        let within = cloud.x >= image.x && cloud.y >= image.y;
        assert!(within && right(cloud) <= right(image), "{src}: on the picture: {cloud:?}");
        let drawn =
            |item: &Item| matches!(item, Item::Path { origin, .. } if (origin.x, origin.y) == (cloud.x, cloud.y));
        assert!(items.iter().any(drawn), "{src}: and is drawn there");
        let said = spelt(&layout);
        assert!(said.contains(host), "{src}: it names {host}: {said}");
        assert!(!said.contains(private), "{src}: and no more of the link than that: {said}");
    }
    let tiny = remote(figure("f", (12.0, 12.0), ""), "https://example.org/a.png");
    assert_eq!(laid(&doc(vec![tiny])).figure(&"f".into()).unwrap().badge, None, "a cloud needs room");
}

#[test]
fn numbered_figures_count_through_the_document() {
    let inside = vec![numbered(picture("g0", "")), picture("g1", "")];
    let document = doc(vec![
        numbered(picture("a", "Alpha")),
        picture("b", "Bravo"),
        numbered(gallery("g", inside, Some("Golf"))),
        container("q", BlockKind::Quote, vec![numbered(picture("c", "Charlie"))]),
        numbered(picture("d", "")),
    ]);
    let numbers = [("a", 1), ("g", 2), ("c", 3), ("d", 4)].map(|(id, number)| (BlockId::from(id), number));
    assert_eq!(figure_numbers(&document), numbers, "in the document's order, a gallery as one");

    let mut german = Style::default();
    german.types.get_mut("caption").unwrap().extra.insert("label".into(), "Abbildung".into());
    for (style, word) in [(Style::default(), "Figure"), (german, "Abbildung")] {
        let mut layout = DocLayout::new(fonts());
        layout.update(&document, &style, WIDTH, None);
        let said = spelt(&layout);
        assert_eq!(said.matches(word).count(), 4, "{word}: only numbered figures are labelled: {said}");
        for (number, caption) in [(1, "Alpha"), (2, "Golf"), (3, "Charlie"), (4, "")] {
            let label = figure_label(&style, number);
            assert!(label.starts_with(word) && label.contains(&number.to_string()), "{label} is {word} {number}");
            let at = said.find(&label).unwrap_or_else(|| panic!("{label} is on the page: {said}"));
            let after = &said[at + label.len()..];
            assert!(after.trim_start().starts_with(caption), "{label} opens its caption: {said}");
        }

        let start = |block: &str| layout.caret(&Pos::new(block, 0)).unwrap();
        assert!(start("a").x > start("b").x + 10.0, "{word}: a caption's start is after its label");
        let label = Point { x: start("b").x + 2.0, y: middle(start("a")).y };
        assert_eq!(layout.hit(label), Some(Pos::new("a", 0)), "{word}: and a point on the label is at that start");
    }
}

#[test]
fn a_gallery_tiles_its_pictures_in_rows() {
    // How many pictures each row takes: up to three, and the last is never alone.
    let rows: [&[usize]; 8] = [&[1], &[2], &[3], &[2, 2], &[3, 2], &[3, 3], &[3, 2, 2], &[3, 3, 2]];
    for (expected, width) in rows.into_iter().flat_map(|rows| [(rows, WIDTH), (rows, 320.0)]) {
        let count: usize = expected.iter().sum();
        let (block, named) = (tiles(count, Some("Along the harbour")), named(count));
        let layout = laid_at(&doc(vec![paragraph("p", "Above"), block, paragraph("z", "End")]), width);
        let items = painted(&layout, everything());
        let shown = images(&items);
        let assets: Vec<String> = named.iter().map(|(id, _)| format!("assets/{id}.png")).collect();
        assert!(shown.iter().map(|(asset, _)| *asset).eq(assets.iter()), "{count}: every picture, in order");

        // Pictures that start at one height are a row.
        let mut tiled: Vec<Vec<Rect>> = Vec::new();
        for (_, rect) in &shown {
            match tiled.last_mut().filter(|row| row[0].y == rect.y) {
                Some(row) => row.push(*rect),
                None => tiled.push(vec![*rect]),
            }
        }
        let sizes: Vec<usize> = tiled.iter().map(Vec::len).collect();
        assert_eq!(sizes, expected, "{count} pictures at {width} px");
        for row in &tiled {
            let (first, last) = (row[0], row[row.len() - 1]);
            let fills = first.x.abs() < 0.5 && (last.x + last.width - width).abs() < 0.5;
            assert!(fills, "{count}: a row fills the column at {width} px: {row:?}");
            assert!(row.iter().all(|tile| tile.height == first.height), "{count}: at one height: {row:?}");
            let apart = row.windows(2).all(|pair| pair[0].x + pair[0].width < pair[1].x);
            assert!(apart, "{count}: with space between its pictures: {row:?}");
        }
        assert!(tiled.windows(2).all(|pair| bottom(pair[0][0]) < pair[1][0].y), "{count}: rows stack, apart");
        for ((id, size), (_, rect)) in named.iter().zip(&shown) {
            assert!(stretch(*rect, *size) < 0.04, "{count}: {id} keeps its shape at {width} px: {rect:?}");
            let found = layout.figure(&id.as_str().into()).map(|place| place.image);
            assert_eq!(found, Some(*rect), "{count}: {id} is found where it is drawn");
        }

        let all = layout.figure(&"g".into()).expect("a gallery has a place").image;
        let (top, end) = (tiled[0][0].y, bottom(tiled[tiled.len() - 1][0]));
        assert!((all.y - top).abs() < 0.5 && (bottom(all) - end).abs() < 0.5, "{count}: its rows are at {all:?}");
        let caret = |block: &str| layout.caret(&Pos::new(block, 0)).unwrap();
        assert!(bottom(caret("p")) <= top, "{count}: under the block above");
        assert!(end <= caret("gc").y, "{count}: the caption is under the last row");
        assert!(bottom(caret("gc")) <= caret("z").y, "{count}: and the block below is under that");
    }
}

#[test]
fn positions_in_a_gallery_are_on_its_pictures() {
    let around = |gallery: Block| laid(&doc(vec![paragraph("p", PROSE), gallery, paragraph("z", PROSE)]));
    let (block, named) = (tiles(5, Some("Along the harbour")), named(5));
    let layout = around(block);
    let place = |id: &str| layout.figure(&id.into()).unwrap().image;
    for (id, _) in &named {
        let (id, tile) = (id.as_str(), place(id));
        assert_eq!(layout.hit(middle(tile)), Some(Pos::new(id, 0)), "a point on {id} is at it");
        let caret = |offset| layout.caret(&Pos::new(id, offset)).unwrap();
        let (before, after) = (caret(0), caret(1));
        let sides = (before.x - tile.x).abs() <= 2.0 && (right(after) - right(tile)).abs() <= 2.0;
        assert!(sides, "the caret stands at either side of {id}: {before:?} and {after:?} by {tile:?}");
        assert!(
            [before, after].iter().all(|caret| (caret.y, caret.height) == (tile.y, tile.height)),
            "as tall as {id}"
        );
    }

    let chosen = |from: Pos, to: Pos| layout.selection(&Selection { anchor: from, head: to });
    let across = chosen(Pos::new("p", 0), Pos::new("z", 4));
    assert!(
        named.iter().all(|(id, _)| across.contains(&place(id))),
        "a selection across a gallery covers its pictures"
    );
    let some = chosen(Pos::new("g1", 0), Pos::new("g3", 0));
    assert_eq!(some, [place("g1"), place("g2")], "one inside it covers the pictures it runs over and no more");
    assert_eq!(chosen(Pos::new("g3", 0), Pos::new("g1", 0)), some, "whichever way it was made");

    // Down and back up the column, under the second picture of the first row.
    let x = middle(place("g1")).x;
    let under = ["g3", "g4"].into_iter().find(|id| x < place(id).x + place(id).width).unwrap();
    let ids = |names: [&str; 4]| names.map(BlockId::from).to_vec();
    let mut at = Pos::new("p", PROSE.len());
    let mut step = |down: bool| {
        at = layout.line_step(&at, x, down).expect("a line that way");
        at.clone()
    };
    let down: Vec<Pos> = (0..4).map(|_| step(true)).collect();
    assert_eq!(down[..2], [Pos::new("g1", 0), Pos::new(under, 0)], "a line down is the next row of pictures");
    let blocks: Vec<BlockId> = down.into_iter().map(|pos| pos.block).collect();
    assert_eq!(blocks, ids(["g1", under, "gc", "z"]), "then the caption, then the block below");
    let up: Vec<BlockId> = (0..4).map(|_| step(false).block).collect();
    assert_eq!(up, ids(["gc", under, "g1", "p"]), "and back up the same way");

    let bare = around(tiles(5, None));
    let last = bare.line_step(&Pos::new("z", 0), 10.0, false);
    assert_eq!(last, Some(Pos::new("g3", 0)), "a gallery with no caption is entered from below by its last row");
}

#[test]
fn a_table_shares_its_width_by_what_its_columns_hold() {
    for width in [WIDTH, 320.0] {
        let layout = laid_at(&doc(vec![paragraph("p", "Above"), rates(false), paragraph("z", "End")]), width);
        let caret = |block: &str, offset| layout.caret(&Pos::new(block, offset)).unwrap();
        let cell = |row: usize, column: usize| caret(&format!("t{row}{column}"), 0);
        for at in 0..3 {
            assert!((0..3).all(|row| cell(row, at).x == cell(0, at).x), "{width}: column {at} lines up");
            assert!((0..3).all(|column| cell(at, column).y == cell(at, 0).y), "{width}: row {at} starts level");
        }
        let (first, second, third) = (cell(0, 0).x, cell(0, 1).x, cell(0, 2).x);
        assert!(0.0 <= first && first < second && second < third && third < width, "{width}: columns stand apart");
        assert!(second - first > third - second, "{width}: the column with the most to say is the widest");

        let end = caret("t10", PROSE.len());
        assert!(end.y > cell(1, 0).y, "{width}: a cell wraps");
        assert!((0..=PROSE.len()).all(|offset| caret("t10", offset).x < second), "{width}: inside its column");
        assert_eq!(cell(1, 1).y, cell(1, 0).y, "{width}: beside the first line of its row's other cells");
        assert!(bottom(end) <= cell(2, 0).y, "{width}: and the next row is under all of it");
        assert!(bottom(caret("p", 0)) <= cell(0, 0).y, "{width}: the table is under the block above");
        assert!(bottom(cell(2, 0)) <= caret("z", 0).y, "{width}: and over the block below");
    }
}

#[test]
fn a_column_is_set_the_way_it_is_aligned() {
    // An alignment, and how far along a cell's text the cells of its column meet.
    let cases = [(Align::Start, 0.0), (Align::Center, 0.5), (Align::End, 1.0)];
    let (long, short) = ("Households here", "Mill");
    let layout = laid(&doc(vec![table("t", &cases.map(|(align, _)| align), false, &[&[long; 3], &[short; 3]])]));
    for (column, (align, share)) in cases.into_iter().enumerate() {
        let along = |row: usize, text: &str| {
            let caret = |offset| layout.caret(&Pos::new(format!("t{row}{column}").as_str(), offset)).unwrap().x;
            caret(0) + (caret(text.len()) - caret(0)) * share
        };
        let (wide, narrow) = (along(0, long), along(1, short));
        assert!((wide - narrow).abs() <= 1.0, "{align:?}: a long cell and a short one meet: {wide} and {narrow}");
    }
}

/// The face of each run of text on the line a caret stands on, and whether it is thickened.
fn faces(items: &[Item], line: Rect) -> Vec<(Font, bool)> {
    let face = |item: &Item| match item {
        Item::Glyphs { origin, font, embolden, .. } if origin.y > line.y && origin.y <= bottom(line) + 1.0 => {
            Some((font.clone(), *embolden))
        }
        _ => None,
    };
    items.iter().filter_map(face).collect()
}

#[test]
fn a_table_header_stands_out_from_its_rows() {
    let palette = Palette::LIGHT;
    for header in [true, false] {
        let strong = rich("b", &[("Band", &[Mark::Bold])]);
        let layout = laid(&doc(vec![strong, paragraph("n", "Band"), rates(header)]));
        let items = painted(&layout, everything());
        let line = |block: &str| layout.caret(&Pos::new(block, 0)).unwrap();
        let (bold, plain) = (faces(&items, line("b")), faces(&items, line("n")));
        assert!(bold[0] != plain[0], "bold text is set in another face");
        let (first, second) = (faces(&items, line("t00")), faces(&items, line("t20")));
        assert_eq!(first.len(), 3, "a row sets each of its cells");
        let like = if header { &bold[0] } else { &plain[0] };
        assert!(first.iter().all(|face| face == like), "header {header}: the first row is set to match");
        assert!(second.iter().all(|face| *face == plain[0]), "header {header}: the rows under it are plain");

        // A line under each row, from edge to edge.
        let rule = |item: &Item| match item {
            Item::Fill { rect, color, .. } if rect.width > WIDTH / 2.0 && rect.height <= 2.0 => Some((rect.y, *color)),
            _ => None,
        };
        let rules: Vec<(f32, _)> = items.iter().filter_map(rule).collect();
        assert_eq!(rules.len(), 3, "header {header}: a line under each row");
        for (row, (y, _)) in rules.iter().enumerate() {
            let text = line(&format!("t{row}0"));
            let next = if row < 2 { line(&format!("t{}0", row + 1)).y } else { layout.height() };
            assert!(text.y < *y && *y <= next, "header {header}: the line of row {row} is under its text");
        }
        assert_eq!(rules[1].1, palette.line, "header {header}: a row's line is light");
        assert_eq!(rules[0].1 != rules[1].1, header, "header {header}: and only a header's is heavier");
    }
}

#[test]
fn positions_in_a_table_are_in_its_cells() {
    let layout = laid(&doc(vec![paragraph("p", PROSE), rates(true), paragraph("z", PROSE)]));
    let caret = |block: &str, offset| layout.caret(&Pos::new(block, offset)).unwrap();
    for (row, texts) in RATES.iter().enumerate() {
        for (column, text) in texts.iter().enumerate() {
            let id = format!("t{row}{column}");
            for offset in [0, text.len().min(3), text.len()] {
                let found = layout.hit(middle(caret(&id, offset)));
                assert_eq!(found, Some(Pos::new(id.as_str(), offset)), "a point at a caret in {id} is that position");
            }
        }
    }

    let chosen = layout.selection(&Selection { anchor: Pos::new("t00", 0), head: Pos::new("t22", 2) });
    for id in (0..3).flat_map(|row| (0..3).map(move |column| format!("t{row}{column}"))) {
        let start = caret(&id, 0);
        let covers = |rect: &Rect| {
            let across = rect.x <= start.x + 1.0 && start.x + 1.0 < rect.x + rect.width;
            across && rect.y < bottom(start) && bottom(*rect) > start.y
        };
        assert!(chosen.iter().any(covers), "a selection across the table covers {id}");
    }

    for column in 0..3 {
        let cells = [0, 1, 2].map(|row| format!("t{row}{column}"));
        let x = caret(&cells[0], 0).x + 2.0;
        let walk = |from: Pos, down: bool| {
            let mut through = Vec::new();
            let mut at = from;
            while let Some(next) = layout.line_step(&at, x, down) {
                through.push(next.block.clone());
                at = next;
            }
            through.dedup();
            through
        };
        let mut down: Vec<BlockId> = cells.iter().map(|id| BlockId::from(id.as_str())).collect();
        down.push("z".into());
        assert_eq!(walk(Pos::new("p", PROSE.len()), true), down, "lines go down column {column}");
        let mut up: Vec<BlockId> = down[..3].iter().rev().cloned().collect();
        up.push("p".into());
        assert_eq!(walk(Pos::new("z", 0), false), up, "and back up it");
    }
}

#[test]
fn what_a_merge_leaves_out_of_place_still_shows() {
    let cell = |id: &str, text: &str| Block::text(id, BlockKind::TableCell, text);
    let row = |id: &str, cells: Vec<Block>| container(id, BlockKind::TableRow, cells);
    let caption = |id: &str, text: &str| Block::text(id, BlockKind::Caption, text);
    let odd = |id: &str| Block::new(id, BlockKind::Unknown { type_name: "chart".into() });
    let holding = |kind: BlockKind, parts: Vec<Block>| container("x", kind, parts);
    let (two, album) =
        (BlockKind::Table { columns: vec![Align::Start; 2], header: true }, BlockKind::Gallery { numbered: false });
    // What is left where, the text blocks that must still take the caret, and the words that must still show.
    let cases: Vec<(&str, Block, Vec<&str>, Vec<&str>)> = vec![
        (
            "a row short of cells",
            table("t", &[Align::Start; 2], true, &[&["Alpha", "Bravo"], &["Charlie"]]),
            vec!["t00", "t01", "t10"],
            vec!["Alpha", "Bravo", "Charlie"],
        ),
        (
            "a row with cells to spare",
            table("t", &[Align::End; 2], false, &[&["Alpha", "Bravo", "Charlie"], &["Delta", "Echo"]]),
            vec!["t00", "t01", "t02", "t10", "t11"],
            vec!["Alpha", "Bravo", "Charlie", "Delta", "Echo"],
        ),
        (
            "a row with no cells",
            holding(two.clone(), vec![row("r", vec![]), row("s", vec![cell("a", "Alpha")])]),
            vec!["a"],
            vec!["Alpha"],
        ),
        ("a table with no rows", holding(two.clone(), vec![]), vec![], vec![]),
        (
            "a row outside a table",
            row("r", vec![cell("a", "Alpha"), cell("b", "Bravo")]),
            vec!["a", "b"],
            vec!["Alpha", "Bravo"],
        ),
        ("a cell outside a row", cell("a", "Alpha"), vec!["a"], vec!["Alpha"]),
        (
            "a cell straight in a table",
            holding(two.clone(), vec![cell("a", "Alpha"), row("r", vec![cell("b", "Bravo")])]),
            vec!["a", "b"],
            vec!["Alpha", "Bravo"],
        ),
        (
            "a block of an unknown kind in a row",
            holding(two, vec![row("r", vec![cell("a", "Alpha"), odd("u"), cell("b", "Bravo")])]),
            vec!["a", "b"],
            vec!["Alpha", "Bravo", "chart"],
        ),
        ("a caption outside a gallery", caption("c", "Alpha"), vec!["c"], vec!["Alpha"]),
        (
            "a gallery with two captions",
            holding(
                album.clone(),
                vec![caption("c", "Alpha"), picture("f", ""), picture("h", ""), caption("d", "Bravo")],
            ),
            vec!["c", "d", "f", "h"],
            vec!["Alpha", "Bravo"],
        ),
        (
            "a block of an unknown kind in a gallery",
            holding(album.clone(), vec![picture("f", ""), odd("u"), caption("c", "Alpha")]),
            vec!["c", "f"],
            vec!["Alpha", "chart"],
        ),
        ("a gallery of a caption alone", holding(album.clone(), vec![caption("c", "Alpha")]), vec!["c"], vec!["Alpha"]),
        ("a gallery of nothing", holding(album, vec![]), vec![], vec![]),
    ];
    for (name, block, holds, says) in cases {
        let layout = laid(&doc(vec![paragraph("p", "Above"), block, paragraph("z", "Zulu")]));
        let said = spelt(&layout);
        for word in says.iter().chain(&["Above", "Zulu"]) {
            assert!(said.contains(word), "{name}: {word} shows: {said}");
        }
        let mut places: Vec<Rect> = Vec::new();
        for id in holds.iter().chain(&["p", "z"]) {
            let pos = Pos::new(*id, 0);
            let place = layout.caret(&pos).unwrap_or_else(|| panic!("{name}: {id} has a place"));
            assert_eq!(layout.hit(middle(place)), Some(pos), "{name}: {id} is found at its place");
            assert!(!places.contains(&place), "{name}: {id} has a place of its own");
            places.push(place);
        }
        let height = layout.height();
        for item in painted(&layout, everything()) {
            let Some((top, end)) = extent(&item) else { continue };
            assert!(top >= -1.0 && end <= height + 1.0, "{name}: everything is inside the page: {item:?}");
        }
    }
}

/// A reference to a figure, with words that are no longer its label.
const STALE: &str = "Figure 9";

/// A table `s` of one row: a cell that refers to the figure `to`, and a cell beside it.
fn pointing(to: &str) -> Block {
    let mark = [Mark::Ref(to.into())];
    let refers = Block { kind: BlockKind::TableCell, ..rich("s0", &[("As ", &[]), (STALE, &mark)]) };
    let cells = vec![refers, Block::text("s1", BlockKind::TableCell, "Harbour")];
    let grid = BlockKind::Table { columns: vec![Align::Start; 2], header: false };
    container("s", grid, vec![container("sr", BlockKind::TableRow, cells)])
}

#[test]
fn a_reference_reads_as_its_figure_is_numbered() {
    let to = |id: &str| [Mark::Ref(id.into())];
    let (two, gone, bare) = (to("f2"), to("nowhere"), to("u"));
    let runs: Vec<(&str, &[Mark])> =
        vec![("See ", &[]), (STALE, &two), (" and ", &[]), (STALE, &gone), (" or ", &[]), (STALE, &bare), (".", &[])];
    let figures = || vec![numbered(picture("f1", "Harbour")), numbered(picture("f2", "School")), picture("u", "Mill")];
    let mut blocks = vec![rich("a", &runs), pointing("f2")];
    blocks.extend(figures());
    let layout = laid(&doc(blocks));
    let said = spelt(&layout);
    assert!(!said.contains(STALE), "a reference never reads as its own words: {said}");
    assert_eq!(said.matches("Figure 2").count(), 3, "it reads as its figure's label does, in a cell too: {said}");
    assert_eq!(said.matches("Figure ?").count(), 2, "and as unknown with no numbered figure to name: {said}");

    let warning = Palette::LIGHT.warn.accent;
    assert!(!inked(&painted(&layout, everything()), warning).is_empty(), "one that names nothing warns");
    let mut blocks = vec![rich("a", &runs[..2])];
    blocks.extend(figures());
    assert!(inked(&painted(&laid(&doc(blocks)), everything()), warning).is_empty(), "one that names a figure does not");

    let (start, end) = ("See ".len(), "See ".len() + STALE.len());
    let caret = |offset| layout.caret(&Pos::new("a", offset)).unwrap();
    assert!(caret(start).x < caret(end).x, "the caret steps over a reference");
    assert!((start + 1..end).all(|inside| caret(inside) == caret(end)), "and is never inside one");
    assert!(caret(end).x < caret(end + 2).x, "the text after it runs on from its end");
}

/// Figures, a gallery and tables, with a paragraph and a cell that refer to them.
fn album() -> Document {
    let (second, third) = ([Mark::Ref("f2".into())], [Mark::Ref("g".into())]);
    let refers: Vec<(&str, &[Mark])> =
        vec![("See ", &[]), (STALE, &second), (" and ", &[]), (STALE, &third), (", then read on.", &[])];
    doc(vec![
        rich("a", &refers),
        numbered(picture("f1", "The harbour office")),
        numbered(remote(picture("f2", "The old school"), "https://images.example.org/school.png")),
        numbered(tiles(4, Some("Along the harbour"))),
        rates(true),
        pointing("f2"),
        paragraph("z", PROSE),
    ])
}

#[test]
fn an_edit_to_a_figure_or_a_table_gives_the_same_layout_as_starting_over() {
    let typing =
        |block: &str, offset: usize, text: &str| Op::InsertText { at: Pos::new(block, offset), text: text.into() };
    let cutting = |block: &str, len: usize| Op::DeleteText { block: block.into(), range: 0..len };
    let removing = |block: &str| Op::RemoveBlock { block: block.into() };
    let adding = |parent: Option<&str>, index: usize, block: Block| Op::InsertBlock {
        parent: parent.map(BlockId::from),
        index,
        block,
    };
    // Makes a block the kind that `like` is.
    let retyping = |like: Block| Op::SetKind { block: like.id.clone(), kind: like.kind };
    let referring = |block: &str, to: &str, on: bool| Op::SetMark {
        block: block.into(),
        range: 4..4 + STALE.len(),
        mark: Mark::Ref(to.into()),
        on,
    };
    let grid = |columns: [Align; 3], header| Block::new("t", BlockKind::Table { columns: columns.to_vec(), header });
    let row = table("n", &[], false, &[&["Quay house", "D", "14%"]]).body.blocks()[0].clone();
    let edits: Vec<(&str, Vec<Op>)> = vec![
        ("typing in a caption", vec![typing("f1", 4, "new ")]),
        ("a caption long enough to wrap", vec![typing("f1", 0, PROSE)]),
        ("emptying a caption", vec![cutting("f1", "The harbour office".len())]),
        ("typing in a gallery's caption", vec![typing("gc", 0, PROSE)]),
        ("typing in a cell", vec![typing("t11", 1, PROSE)]),
        ("emptying a cell", vec![cutting("t10", PROSE.len())]),
        ("removing a figure", vec![removing("f1")]),
        ("removing a figure that is referred to", vec![removing("f2")]),
        ("adding a numbered figure above the rest", vec![adding(None, 0, numbered(picture("n", "New")))]),
        ("moving a figure down", vec![removing("f1"), adding(None, 2, numbered(picture("f1", "The harbour office")))]),
        ("taking a figure's number away", vec![retyping(picture("f1", ""))]),
        ("resizing a picture", vec![retyping(numbered(figure("f1", (300.0, 100.0), "")))]),
        ("fetching a linked picture", vec![retyping(numbered(picture("f2", "")))]),
        ("taking a picture out of a gallery", vec![removing("g1")]),
        ("adding a picture to a gallery", vec![adding(Some("g"), 2, figure("n", (200.0, 200.0), ""))]),
        ("taking a gallery's caption away", vec![removing("gc")]),
        ("taking a gallery's number away", vec![retyping(gallery("g", vec![], None))]),
        ("removing a row", vec![removing("t1")]),
        ("adding a row", vec![adding(Some("t"), 1, row)]),
        ("removing a cell", vec![removing("t11")]),
        ("turning a header into a row", vec![retyping(grid([Align::Start; 3], false))]),
        ("setting columns another way", vec![retyping(grid([Align::End, Align::Center, Align::Start], true))]),
        ("typing before a reference", vec![typing("a", 0, "Now ")]),
        ("making a reference", vec![referring("z", "f1", true)]),
        ("unmaking a reference", vec![referring("a", "f2", false)]),
    ];
    for (name, ops) in edits {
        let mut model = Model::new(album(), ReplicaId::from(1));
        let mut layout = laid(model.doc());
        let before = painted(&layout, everything());
        let change = model.apply(ops).unwrap_or_else(|error| panic!("{name}: {error:?}"));
        layout.update(model.doc(), &Style::default(), WIDTH, Some(&change));

        let fresh = laid(model.doc());
        let after = painted(&layout, everything());
        assert!(after != before, "{name} shows");
        assert!(after == painted(&fresh, everything()), "{name}: the same page as starting over");
        assert_eq!(layout.height(), fresh.height(), "{name}: at the same height");
        assert_eq!(spelt(&layout), spelt(&fresh), "{name}: and the same sheets");
    }
}
