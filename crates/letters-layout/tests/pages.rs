//! Pagination for Print view and export: where pages break, and what each one paints.

use std::collections::HashMap;
use std::ops::Range;

use letters_layout::{DocLayout, PageLink, Pages, Palette, Token, TokenKind};
use letters_model::{
    Align, Block, BlockId, BlockKind, Body, Document, ListKind, Mark, Markers, Model, Op, Page, Pos, ReplicaId, Run,
    Style, Tone,
};
use letters_render::{DisplayList, Item, Point, Rect, Role};
use proptest::prelude::*;

mod common;
use common::*;

/// The text width of [`paper`], in logical pixels.
const COLUMN: f32 = 320.0;

/// A sheet whose text area is [`COLUMN`] wide and `room` logical pixels tall, with margins all round.
fn paper(room: f32) -> Page {
    common::paper(COLUMN, room)
}

fn paged(doc: &Document, room: f32) -> (DocLayout, Pages) {
    let layout = laid_at(doc, COLUMN);
    let pages = layout.paginate(&paper(room));
    (layout, pages)
}

/// A block that starts a page of print.
fn breaking(mut block: Block) -> Block {
    block.page_break = true;
    block
}

fn code(id: &str, text: &str) -> Block {
    Block::text(id, BlockKind::Code { language: None }, text)
}

/// One of every kind of block, long enough to fill several small pages.
fn mixed() -> Document {
    let item = |id: &str, list| Block::text(id, BlockKind::ListItem { list, depth: 0 }, PROSE);
    doc(vec![
        Block::text("h", BlockKind::Heading { level: 1 }, "Household rates"),
        paragraph("a", PROSE),
        paragraph("b", &PROSE.repeat(3)),
        code("c", "fn main() {\n    let rate = 0.125;\n    println!(\"{rate}\");\n}\n\nfn other() {}\n// the end"),
        item("i", ListKind::Ordered),
        item("j", ListKind::Task { checked: true }),
        container("w", BlockKind::Callout { tone: Tone::Warn }, vec![paragraph("d", PROSE), paragraph("e", PROSE)]),
        Block::new("r", BlockKind::Divider),
        container("q", BlockKind::Quote, vec![paragraph("f", &PROSE.repeat(2))]),
        Block::new("u", BlockKind::Unknown { type_name: "chart".into() }),
        Block::text("k", BlockKind::Heading { level: 2 }, "What changes"),
        paragraph("z", PROSE),
    ])
}

/// What one page paints: where its text is clipped to, what is drawn inside that, and what beneath it.
struct Painted {
    clip: Rect,
    text: Vec<Item>,
    beneath: Vec<Item>,
}

fn page(layout: &DocLayout, pages: &Pages, index: usize) -> Painted {
    let mut list = DisplayList::default();
    layout.paint_page(pages, index, &mut list);
    let Some(Item::PushClip { rect, .. }) = list.items.first() else { panic!("page {index} clips its text") };
    let close = list.items.iter().rposition(|item| matches!(item, Item::PopClip)).expect("the clip ends");
    Painted { clip: *rect, text: list.items[1..close].to_vec(), beneath: list.items[close + 1..].to_vec() }
}

/// The lines of each block on each page it is on, in page order.
fn parts(pages: &Pages) -> HashMap<BlockId, Vec<(usize, Range<usize>)>> {
    let mut parts: HashMap<BlockId, Vec<(usize, Range<usize>)>> = HashMap::new();
    for (index, sheet) in pages.sheets.iter().enumerate() {
        for (block, lines) in &sheet.blocks {
            parts.entry(block.clone()).or_default().push((index, lines.clone()));
        }
    }
    parts
}

/// Where a block's first line starts in the column, how tall a line of it is, and how many it has.
fn lines_of(layout: &DocLayout, block: &str, len: usize) -> (f32, f32, usize) {
    let count = parts(&layout.paginate(&paper(1.0e6)))[&BlockId::from(block)][0].1.len();
    let caret = |offset| layout.caret(&Pos::new(block, offset)).unwrap();
    let (first, last) = (caret(0), caret(len));
    let line = (last.y - first.y) / (count - 1) as f32;
    (first.y - (line - first.height) / 2.0, line, count)
}

/// The blocks that start a page: those marked to, and the first inside a container that is. A gallery is
/// one block to a page, whatever it holds.
fn starts(doc: &Document) -> Vec<BlockId> {
    let first = |block: &Block| match block.kind {
        BlockKind::Gallery { .. } => block.id.clone(),
        _ => block.body.blocks().first().unwrap_or(block).id.clone(),
    };
    doc.blocks.iter().filter(|block| block.page_break).map(first).collect()
}

/// Checks what holds for any document: every line is on one page, in order, inside the page's text area.
fn check_pages(doc: &Document, layout: &DocLayout, pages: &Pages) -> Result<(), String> {
    let glyphs = |items: &[Item]| placed(items).iter().map(|glyph| glyph.0).collect::<Vec<u32>>();
    let whole = glyphs(&painted(layout, everything()));
    let mut seen = Vec::new();
    for index in 0..pages.sheets.len() {
        let Painted { clip, text, .. } = page(layout, pages, index);
        let (top, bottom) = (pages.area.y, pages.area.y + pages.area.height);
        if clip.y < top - 0.01 || clip.y + clip.height > bottom + 0.01 {
            return Err(format!("page {index} shows {clip:?}, past its text area"));
        }
        if placed(&text).iter().any(|glyph| glyph.2 < clip.y || glyph.2 > clip.y + clip.height) {
            return Err(format!("page {index} sets a line outside {clip:?}"));
        }
        if pages.sheets[index].blocks.is_empty() {
            return Err(format!("page {index} is empty"));
        }
        seen.extend(glyphs(&text));
    }
    if seen != whole {
        return Err(format!(
            "the pages paint {} glyphs and the document {}, or in another order",
            seen.len(),
            whole.len()
        ));
    }

    // Blocks come in the document's order, and the lines of each run on from page to page.
    let order: Vec<&BlockId> = pages.sheets.iter().flat_map(|sheet| &sheet.blocks).map(|block| &block.0).collect();
    let mut expected: Vec<&BlockId> = order.clone();
    expected.dedup();
    let known: Vec<&BlockId> = doc.walk().map(|block| &block.id).filter(|id| expected.contains(id)).collect();
    if expected != known {
        return Err("blocks are out of order, or one is on two pages that are not neighbours".into());
    }
    for (block, parts) in parts(pages) {
        let runs_on = parts.windows(2).all(|pair| pair[0].1.end == pair[1].1.start && pair[0].0 + 1 == pair[1].0);
        if parts[0].1.start != 0 || !runs_on {
            return Err(format!("the lines of {block:?} do not run on: {parts:?}"));
        }
    }
    Ok(())
}

/// What the rows of [`album`]'s table hold. Two of them wrap.
const RATES: [[&str; 2]; 6] = [
    ["Household", "Rate"],
    [PROSE, "12.5%"],
    ["Mill cottage", "9%"],
    ["The old school", "exempt"],
    [PROSE, "10%"],
    ["Harbour office", "11%"],
];

/// A figure `f`, a gallery `g` of seven pictures and a table `t` among prose, each part of them small enough
/// for a small page.
fn album() -> Document {
    let tiles = (0..7).map(|at| figure(&format!("g{at}"), (90.0 + 30.0 * (at % 3) as f32, 90.0), ""));
    let rows: Vec<&[&str]> = RATES.iter().map(|row| &row[..]).collect();
    doc(vec![
        paragraph("a", PROSE),
        figure("f", (120.0, 80.0), "The harbour office, seen from the quay"),
        paragraph("b", PROSE),
        gallery("g", tiles.collect(), Some("Along the harbour")),
        paragraph("c", PROSE),
        table("t", &[Align::Start, Align::End], true, &rows),
        paragraph("z", PROSE),
    ])
}

#[test]
fn a_page_never_cuts_a_picture_or_a_row() {
    let doc = album();
    let pictures: Vec<String> = (0..7).map(|at| format!("assets/g{at}.png")).collect();
    let (mut gallery_split, mut table_split) = (false, false);
    for room in [200.0, 260.0, 340.0, 480.0] {
        let (layout, pages) = paged(&doc, room);
        assert_eq!(check_pages(&doc, &layout, &pages), Ok(()), "at {room} px");
        // The page each picture is on, in the order they are met.
        let mut shown: Vec<(String, usize)> = Vec::new();
        for index in 0..pages.sheets.len() {
            let Painted { clip, text, .. } = page(&layout, &pages, index);
            for item in &text {
                let Item::Image { rect, asset } = item else { continue };
                let (top, end, last) = (rect.y, rect.y + rect.height, clip.y + clip.height);
                if end > clip.y && top < last {
                    assert!(top >= clip.y - 0.01 && end <= last + 0.01, "at {room} px page {index} cuts {asset}");
                    shown.push((asset.clone(), index));
                }
            }
        }
        let met: Vec<&str> = shown.iter().map(|(asset, _)| asset.as_str()).collect();
        let all: Vec<&str> = ["assets/f.png"].into_iter().chain(pictures.iter().map(String::as_str)).collect();
        assert_eq!(met, all, "at {room} px each picture is on one page");

        let on = |block: &str, offset: usize| layout.page_of(&pages, &Pos::new(block, offset)).unwrap();
        assert_eq!(
            (on("f", 0), on("f", 30)),
            (shown[0].1, shown[0].1),
            "at {room} px a caption is on its picture's page"
        );
        assert_eq!(on("gc", 0), shown[7].1, "at {room} px a gallery's caption is on its last row's page");
        for (row, [home, rate]) in RATES.iter().enumerate() {
            let cell = |column: usize, offset: usize| on(&format!("t{row}{column}"), offset);
            let ends = [cell(0, 0), cell(0, home.len()), cell(1, rate.len())];
            assert!(ends.iter().all(|page| *page == ends[0]), "at {room} px row {row} is on pages {ends:?}");
        }
        assert_eq!(on("t00", 0), on("t10", 0), "at {room} px a header is on its first row's page");
        gallery_split |= shown[1].1 != shown[7].1;
        table_split |= on("t00", 0) != on("t50", 0);
    }
    assert!(gallery_split && table_split, "a gallery and a table go on to the next page between their rows");
}

/// Prose with formulas in its lines, some taller than a line of text, and formulas as blocks.
fn sums() -> Document {
    let tall = r"\frac{\frac{a}{b}}{\frac{c}{d}}";
    let runs: Vec<(&str, &[Mark])> =
        vec![(PROSE, &[]), (tall, &[Mark::Math]), (PROSE, &[]), ("x^2", &[Mark::Math]), (PROSE, &[])];
    doc(vec![
        paragraph("a", PROSE),
        rich("m", &runs),
        formula("f", r"\sum_{i=0}^{n} \frac{1}{i^2}"),
        rich("n", &runs),
        formula("g", "E = mc^2"),
        paragraph("z", PROSE),
    ])
}

#[test]
fn lines_with_formulas_go_to_one_page_each() {
    let sums = sums();
    for room in [160.0, 240.0, 500.0, 5000.0] {
        let (layout, pages) = paged(&sums, room);
        assert_eq!(check_pages(&sums, &layout, &pages), Ok(()), "with {room} px of room");
        // A formula is never cut by the edge of a page.
        for (block, offset) in [("m", PROSE.len()), ("f", 0), ("n", PROSE.len()), ("g", 0)] {
            let at = Pos::new(block, offset);
            let (place, sheet) = (layout.formula_rect(&at).unwrap(), layout.page_of(&pages, &at).unwrap());
            let top = place.y + pages.origin(sheet).unwrap().y;
            let Painted { clip, .. } = page(&layout, &pages, sheet);
            assert!(clip.y <= top + 0.01 && top + place.height <= clip.y + clip.height + 0.01, "{block} at {room} px");
        }
    }
}

#[test]
fn every_line_is_on_one_page_in_order() {
    let mixed = mixed();
    for room in [160.0, 240.0, 500.0, 5000.0] {
        let (layout, pages) = paged(&mixed, room);
        assert_eq!(check_pages(&mixed, &layout, &pages), Ok(()), "with {room} px of room");
        let fewest = (layout.height() / room).ceil() as usize;
        assert!(pages.sheets.len() >= fewest && pages.sheets.len() <= fewest * 2, "{} pages", pages.sheets.len());
        assert!((pages.area.width - COLUMN).abs() < 0.01 && (pages.area.height - room).abs() < 0.01);
    }
    let (_, pages) = paged(&doc(vec![]), 500.0);
    assert_eq!(pages.sheets.len(), 1, "an empty document still has a page");
}

#[test]
fn a_marked_block_starts_a_page() {
    let marked = doc(vec![
        paragraph("a", "One"),
        breaking(paragraph("b", "Two")),
        paragraph("c", "Three"),
        breaking(container("w", BlockKind::Callout { tone: Tone::Tip }, vec![paragraph("d", "Four")])),
        breaking(Block::new("r", BlockKind::Divider)),
    ]);
    let (_, pages) = paged(&marked, 500.0);
    let firsts: Vec<&str> = pages.sheets.iter().map(|sheet| sheet.blocks[0].0.as_str()).collect();
    assert_eq!(firsts, ["a", "b", "d", "r"]);
    assert_eq!(pages.sheets[1].blocks.len(), 2, "what follows stays on the page");

    let (_, pages) = paged(&doc(vec![breaking(paragraph("a", "One")), paragraph("b", "Two")]), 500.0);
    assert_eq!(pages.sheets.len(), 1, "the first block starts the first page as it is");
}

#[test]
fn a_heading_goes_with_the_block_after_it() {
    let doc = doc(vec![
        paragraph("a", PROSE),
        Block::text("h", BlockKind::Heading { level: 2 }, "What changes"),
        Block::text("s", BlockKind::Heading { level: 3 }, "For households"),
        paragraph("b", PROSE),
    ]);
    // Room for both headings and one line of the paragraph, which may not stand alone.
    let (top, line, _) = lines_of(&laid_at(&doc, COLUMN), "b", PROSE.len());
    let (layout, pages) = paged(&doc, top + line + 1.0);
    let on = |block: &str| layout.page_of(&pages, &Pos::new(block, 0));
    assert_eq!((on("a"), on("h"), on("s"), on("b")), (Some(0), Some(1), Some(1), Some(1)));

    let (layout, pages) = paged(&doc, top + 2.0 * line + 1.0);
    assert_eq!(layout.page_of(&pages, &Pos::new("h", 0)), Some(0), "with room for two lines they all stay");
    assert_eq!(pages.sheets[1].blocks[0].0.as_str(), "b");
}

#[test]
fn a_block_splits_with_two_lines_on_each_side() {
    let doc = doc(vec![paragraph("a", &PROSE.repeat(3)), paragraph("b", &PROSE.repeat(2))]);
    let (top, line, lines) = lines_of(&laid_at(&doc, COLUMN), "b", PROSE.len() * 2);
    assert!(lines >= 6, "the paragraph has lines to split");
    for fit in 0..=lines + 1 {
        // Room for the first paragraph and `fit` lines of the second.
        let (_, pages) = paged(&doc, top + fit as f32 * line + 1.0);
        let parts = &parts(&pages)[&BlockId::from("b")];
        assert!(parts.iter().all(|part| part.1.len() >= 2), "{fit} lines fit: no line stands alone in {parts:?}");
        let stayed = if parts[0].0 == 0 { parts[0].1.len() } else { 0 };
        let most = if fit >= lines { lines } else { fit.min(lines - 2) };
        assert_eq!(stayed, if most < 2 { 0 } else { most }, "{fit} lines fit: the page takes what the rule allows");
    }
}

#[test]
fn a_line_of_code_never_splits() {
    let long = "let assessment = council.rates().for_household(scheme).quarterly().before(december_statement);";
    let text = [long, "run();", long, long, "check();", long, "done();", long, long].join("\n");
    let doc = doc(vec![paragraph("a", PROSE), code("c", &text)]);
    for room in [130.0, 150.0, 170.0, 190.0, 210.0] {
        let (layout, pages) = paged(&doc, room);
        assert_eq!(check_pages(&doc, &layout, &pages), Ok(()), "with {room} px of room");
        assert!(parts(&pages)[&BlockId::from("c")].len() >= 2, "the code runs over a page end");
        let mut start = 0;
        for line in text.split('\n') {
            let on = |offset| layout.page_of(&pages, &Pos::new("c", offset));
            assert_eq!(on(start), on(start + line.len()), "the line at {start} is on one page with {room} px");
            start += line.len() + 1;
        }
    }
}

#[test]
fn a_position_maps_to_its_page() {
    let doc = mixed();
    let (layout, pages) = paged(&doc, 240.0);
    let mut last = 0;
    for block in doc.walk().filter(|block| matches!(block.body, Body::Text(_))) {
        for offset in [0, block.plain_text().len()] {
            let on = layout.page_of(&pages, &Pos::new(block.id.clone(), offset)).expect("a page");
            assert!(on >= last && on < pages.sheets.len(), "pages follow the document");
            assert!(pages.sheets[on].blocks.iter().any(|(id, _)| *id == block.id), "{:?} is on page {on}", block.id);
            last = on;

            // The page's origin carries a caret onto the sheet, and a point on the sheet back to the text.
            let (caret, origin) =
                (layout.caret(&Pos::new(block.id.clone(), offset)).unwrap(), pages.origin(on).unwrap());
            let (top, area) = (caret.y + origin.y, pages.area);
            assert!(top > area.y - 1.0 && top + caret.height < area.y + area.height + 1.0, "its caret is on the page");
            let point = Point { x: caret.x + origin.x, y: top + caret.height / 2.0 };
            let hit = layout.hit(Point { x: point.x - origin.x, y: point.y - origin.y }).unwrap();
            assert_eq!(layout.page_of(&pages, &hit), Some(on), "a click there finds the page's text");
        }
    }
    assert_eq!(last, pages.sheets.len() - 1, "the document ends on the last page");
    assert_eq!(layout.page_of(&pages, &Pos::new("missing", 0)), None);
    assert_eq!(pages.origin(pages.sheets.len()), None);
}

#[test]
fn a_page_paints_its_text_in_the_text_area_and_its_number_beneath() {
    let doc = mixed();
    let (layout, pages) = paged(&doc, 240.0);
    for index in [0, pages.sheets.len() - 1] {
        let Painted { clip, text, beneath } = page(&layout, &pages, index);
        let area = pages.area;
        assert_eq!(clip.y, area.y, "page {index} starts at the top of its text area");
        let left = placed(&text).iter().map(|glyph| glyph.1).fold(f32::MAX, f32::min);
        assert!(left >= area.x - 0.5 && left < area.x + 40.0, "and at its left edge");

        let number = placed(&beneath);
        assert!(!number.is_empty() && number.len() <= 2, "the page number is beneath");
        for (_, x, y) in number {
            assert!(y > area.y + area.height && y < pages.height, "in the bottom margin");
            assert!((x - (area.x + area.width / 2.0)).abs() < 12.0, "in the middle");
        }
    }
    let mut list = DisplayList::default();
    layout.paint_page(&pages, pages.sheets.len(), &mut list);
    assert!(list.items.is_empty(), "there is no page past the last");

    let bare = Page { margin_bottom: 0.0, ..paper(240.0) };
    let pages = layout.paginate(&bare);
    assert!(page(&layout, &pages, 0).beneath.is_empty(), "a sheet with no bottom margin has no room for a number");
}

#[test]
fn print_shows_the_document_and_nothing_of_the_editor() {
    let doc = mixed();
    let (plain, pages) = paged(&doc, 240.0);
    let (mut layout, _) = paged(&doc, 240.0);
    layout.set_spelling(&"a".into(), vec![0..5, 10..20]);
    layout.set_composition(Some((&"a".into(), 30..40)));
    let revealed = letters_model::Markers { block: Some(("h".into(), "#".into())), ..Default::default() };
    layout.reveal(Some((&Pos::new("h", 2), &revealed)));
    for index in 0..pages.sheets.len() {
        assert!(page(&layout, &pages, index).text == page(&plain, &pages, index).text, "page {index}");
    }
}

#[test]
fn a_sheet_typesets_the_formula_the_caret_is_in() {
    let (sums, none) = (sums(), Markers::default());
    let (plain, pages) = paged(&sums, 240.0);
    let (mut layout, _) = paged(&sums, 240.0);
    for (block, offset) in [("m", PROSE.len() + 2), ("f", 1)] {
        let caret = Pos::new(block, offset);
        assert!(layout.reveal(Some((&caret, &none))), "the formula in {block} opens");
        let screen = painted(&layout, everything());
        assert!(layout.paginate(&paper(240.0)) == pages, "the pages are the same with the caret in {block}");
        for index in 0..pages.sheets.len() {
            assert!(page(&layout, &pages, index).text == page(&plain, &pages, index).text, "page {index}, {block}");
        }
        assert_eq!(layout.page_of(&pages, &caret), plain.page_of(&pages, &caret), "{block} is on the same page");
        assert!(painted(&layout, everything()) == screen, "and the screen still shows the source in {block}");
    }
}

/// A part a sheet tells of: what it is, and what the part it lies in is.
type Part = (Role, Option<Role>);

/// One block of each kind a reader is told of, long enough to fill several small pages, with its parts in
/// reading order.
fn kinds() -> (Document, Vec<Part>) {
    let item = |id: &str, list, depth| Block::text(id, BlockKind::ListItem { list, depth }, PROSE);
    let rule = |id: &str| Block::new(id, BlockKind::Divider);
    let said = |numbered, checked, depth| Role::ListItem { depth, numbered, checked };
    let maths = |tex: &str| Role::Formula { alt: tex.into() };
    let shown = |id: &str| Role::Figure { alt: format!("Picture {id}").as_str().into() };
    let picture = |id: &str, wide: f32, caption: &str| figure(id, (wide, 60.0), caption);
    let blocks = vec![
        Block::text("h", BlockKind::Heading { level: 1 }, "Household rates"),
        paragraph("a", &PROSE.repeat(3)),
        code("c", "fn main() {\n    let rate = 0.125;\n}"),
        item("i", ListKind::Ordered, 0),
        item("j", ListKind::Bullet, 1),
        item("t", ListKind::Task { checked: false }, 0),
        item("u", ListKind::Task { checked: true }, 0),
        container("w", BlockKind::Callout { tone: Tone::Warn }, vec![paragraph("d", PROSE)]),
        container("q", BlockKind::Quote, vec![paragraph("e", PROSE), rule("r"), code("x", "let x = 1;")]),
        Block::text("k", BlockKind::Heading { level: 2 }, "What changes"),
        rich("m", &[(PROSE, &[]), ("x^2", &[Mark::Math]), (PROSE, &[]), ("a + b", &[Mark::Math])]),
        formula("g", "E = mc^2"),
        picture("p", 90.0, "The harbour office"),
        gallery("v", vec![picture("v0", 90.0, ""), picture("v1", 120.0, "")], Some("Along the harbour")),
        table("s", &[Align::Start, Align::End], true, &[&["Band", "Rate"], &["B", "12.5%"]]),
    ];
    let parts = vec![
        (Role::Heading(1), None),
        (Role::Paragraph, None),
        (Role::Code, None),
        (said(true, None, 0), None),
        (Role::Label, Some(said(true, None, 0))),
        (said(false, None, 1), None),
        (Role::Label, Some(said(false, None, 1))),
        (said(false, Some(false), 0), None),
        (Role::Label, Some(said(false, Some(false), 0))),
        (said(false, Some(true), 0), None),
        (Role::Label, Some(said(false, Some(true), 0))),
        (Role::Note, None),
        (Role::Paragraph, Some(Role::Note)),
        (Role::Quote, None),
        (Role::Paragraph, Some(Role::Quote)),
        (Role::Code, Some(Role::Quote)),
        (Role::Heading(2), None),
        (Role::Paragraph, None),
        (maths("x^2"), Some(Role::Paragraph)),
        (maths("a + b"), Some(Role::Paragraph)),
        (maths("E = mc^2"), None),
        (shown("p"), None),
        (Role::Caption, Some(shown("p"))),
        (shown("v0"), None),
        (shown("v1"), None),
        (Role::Caption, None),
        (Role::Table, None),
        (Role::Row, Some(Role::Table)),
        (Role::Cell { header: true }, Some(Role::Row)),
        (Role::Cell { header: true }, Some(Role::Row)),
        (Role::Row, Some(Role::Table)),
        (Role::Cell { header: false }, Some(Role::Row)),
        (Role::Cell { header: false }, Some(Role::Row)),
    ];
    (doc(blocks), parts)
}

#[test]
fn a_sheet_says_what_its_parts_are() {
    let (doc, parts) = kinds();
    let told = |item: &Item| matches!(item, Item::Open { .. } | Item::Close);
    // The box behind code and the lines of a divider and under the rows of a table, which are decoration.
    let palette = Palette::LIGHT;
    let lines = [palette.code, palette.line, palette.line_strong];
    let plain = |item: &Item| matches!(item, Item::Fill { color, .. } if lines.contains(color));
    let pictured = |part: Option<&&Role>| matches!(part, Some(Role::Figure { .. }));
    for room in [160.0, 240.0, 5000.0] {
        let (layout, pages) = paged(&doc, room);
        // Each part where it is first met, by its name.
        let mut met: Vec<(u64, Part)> = Vec::new();
        let mut decoration = 0;
        for index in 0..pages.sheets.len() {
            let Painted { text, beneath, .. } = page(&layout, &pages, index);
            let mut open: Vec<&Role> = Vec::new();
            for item in &text {
                match item {
                    Item::Open { role, id } => {
                        let part = (role.clone(), open.last().copied().cloned());
                        match met.iter().find(|known| known.0 == *id) {
                            Some((_, known)) => assert_eq!(known, &part, "a part runs on as what it was"),
                            None => met.push((*id, part)),
                        }
                        open.push(role);
                    }
                    Item::Close => drop(open.pop().expect("a part closes only once open")),
                    Item::Glyphs { .. } => assert!(!open.is_empty(), "text on page {index} is part of something"),
                    Item::Image { .. } => assert!(pictured(open.last()), "a picture on page {index} is a figure"),
                    item if plain(item) => {
                        assert!(open.is_empty(), "decoration on page {index} is part of nothing: {open:?}");
                        decoration += 1;
                    }
                    _ => {}
                }
            }
            assert!(open.is_empty(), "every part of page {index} closes on it at {room} px");
            assert!(!beneath.iter().any(told), "the page number is part of nothing");
        }
        assert!(decoration >= 3, "the boxes and the rule are drawn at {room} px");
        let met: Vec<Part> = met.into_iter().map(|(_, part)| part).collect();
        assert!(met == parts, "at {room} px the parts are {met:#?}");
        assert!(!painted(&layout, everything()).iter().any(told), "the screen is told of none");
    }
}

#[test]
fn a_sheet_is_white_paper_whatever_the_theme() {
    let heading = Block::text("h", BlockKind::Heading { level: 1 }, "Household rates");
    let (mut layout, pages) = paged(&doc(vec![heading, paragraph("a", PROSE), paragraph("b", PROSE)]), 240.0);
    // What a sheet says of its parts paints nothing.
    let glyphs = |items: &[Item]| {
        let text = |item: &Item| matches!(item, Item::Glyphs { .. } | Item::Open { .. } | Item::Close);
        glyph_runs(items).next().is_some() && items.iter().all(text)
    };
    let light = page(&layout, &pages, 0);
    assert!(glyphs(&light.text) && glyphs(&light.beneath), "nothing is painted behind the text of a sheet");
    assert!(glyphs(&painted(&layout, everything())), "nor behind the text on screen");

    layout.set_palette(dark());
    let sheet = page(&layout, &pages, 0);
    assert!(sheet.text == light.text && sheet.beneath == light.beneath, "a dark theme changes nothing on paper");
    let colors = |items: &[Item]| items.iter().filter_map(color_of).collect::<Vec<_>>();
    assert!(colors(&painted(&layout, everything())) != colors(&light.text), "while the screen takes its colours");
}

#[test]
fn a_sheet_says_what_its_glyphs_spell() {
    let marked = Run { marks: [Mark::Bold].into(), ..Run::plain("office finds") };
    let runs = vec![Run::plain("The "), marked, Run::plain(" it fine.")];
    let mut blocks = mixed().blocks;
    blocks.push(Block { body: Body::Text(runs), ..paragraph("m", "") });
    let doc = doc(blocks);
    let (mut layout, _) = paged(&doc, 240.0);
    layout.set_tokens(&"c".into(), vec![Token { range: 0..2, kind: TokenKind::Keyword }]);
    let pages = layout.paginate(&paper(240.0));

    // What a list of items spells, with each run checked on the way.
    let read = |items: &[Item]| {
        let mut spelt = String::new();
        for item in glyph_runs(items) {
            let Item::Glyphs { glyphs, text, .. } = item else { continue };
            let run = text.as_ref().expect("a run on a sheet carries its text");
            assert_eq!(run.clusters.len(), glyphs.len(), "one cluster for each glyph of {:?}", run.text);
            assert!(run.clusters.iter().all(|at| run.text.is_char_boundary(*at as usize)), "{:?}", run.text);
            spelt.push_str(&run.text);
        }
        spelt
    };
    let (mut spelt, mut numbers) = (String::new(), Vec::new());
    for index in 0..pages.sheets.len() {
        let Painted { text, beneath, .. } = page(&layout, &pages, index);
        spelt.push_str(&read(&text));
        numbers.push(read(&beneath));
    }
    assert_eq!(numbers, (1..=pages.sheets.len()).map(|number| number.to_string()).collect::<Vec<_>>());
    // A space at a line's end may draw nothing, so only what shows is compared.
    let shown = |text: &str| text.split_whitespace().collect::<String>();
    let spelt = shown(&spelt);
    for block in doc.walk().filter(|block| !block.plain_text().is_empty()) {
        assert!(spelt.contains(&shown(&block.plain_text())), "block {} can be read from its sheets", block.id);
    }

    let screen = painted(&layout, everything());
    assert!(glyph_runs(&screen).all(|item| matches!(item, Item::Glyphs { text: None, .. })), "the screen keeps none");
}

#[test]
fn a_sheet_lists_its_links() {
    // A link long enough to wrap, part of it bold, and a second one further down the document.
    let (before, first, second) = ("Read ", "the notice that arrives with the December ", "statement before January");
    let link = |text: &str, url: &str| Run { link: Some(url.into()), ..Run::plain(text) };
    let (notice, rates) = ("https://example.org/notice", "https://example.org/rates");
    let bold = Run { marks: [Mark::Bold].into(), ..link(second, notice) };
    let linked = Block { body: Body::Text(vec![Run::plain(before), link(first, notice), bold]), ..paragraph("a", "") };
    let later = Block { body: Body::Text(vec![link("Rates", rates), Run::plain(" change.")]), ..paragraph("z", "") };
    let doc = doc(vec![linked, paragraph("b", &PROSE.repeat(4)), paragraph("c", &PROSE.repeat(4)), later]);
    let (layout, pages) = paged(&doc, 240.0);
    let on = |block: &str| layout.page_of(&pages, &Pos::new(block, 0)).unwrap();
    assert!(on("z") > on("a"), "the links are on different sheets");

    let links = layout.page_links(&pages, on("a"));
    assert!(links.len() >= 2 && links.iter().all(|link| link.target == notice), "one link, a rectangle a line");
    // The link's ends, as carets on the sheet, are at the ends of its rectangles.
    let caret = |block: &str, offset| {
        let (caret, origin) = (layout.caret(&Pos::new(block, offset)).unwrap(), pages.origin(on(block)).unwrap());
        Point { x: caret.x + origin.x, y: caret.y + origin.y + caret.height / 2.0 }
    };
    let (start, end) = (caret("a", before.len()), caret("a", before.len() + first.len() + second.len()));
    let (head, tail) = (links[0].rect, links[links.len() - 1].rect);
    assert!((head.x - start.x).abs() < 1.0 && head.y < start.y && head.y + head.height > start.y, "from its start");
    assert!((tail.x + tail.width - end.x).abs() < 2.0 && tail.y < end.y && tail.y + tail.height > end.y, "to its end");
    let area = pages.area;
    for PageLink { rect, .. } in &links {
        assert!(rect.x >= area.x - 0.5 && rect.x + rect.width <= area.x + area.width + 0.5, "inside the text area");
        assert!(rect.y >= area.y - 0.5 && rect.y + rect.height <= area.y + area.height + 0.5);
    }

    let last = layout.page_links(&pages, on("z"));
    assert_eq!(last.iter().map(|link| link.target.as_str()).collect::<Vec<_>>(), [rates]);
    assert!(layout.page_links(&pages, pages.sheets.len()).is_empty(), "a sheet that is not there lists none");
}

#[test]
fn the_outline_lists_headings_with_their_sheets() {
    let mut blocks = mixed().blocks;
    blocks.push(Block::text("e", BlockKind::Heading { level: 3 }, "  "));
    let doc = doc(blocks);
    let (layout, pages) = paged(&doc, 240.0);
    let outline = layout.outline(&pages);
    let listed: Vec<(&str, &str, u8)> =
        outline.iter().map(|heading| (heading.block.as_str(), heading.title.as_str(), heading.level)).collect();
    assert_eq!(listed, [("h", "Household rates", 1), ("k", "What changes", 2)], "in order, without the empty one");
    for heading in &outline {
        let pos = Pos::new(heading.block.clone(), 0);
        assert_eq!(Some(heading.page), layout.page_of(&pages, &pos), "{:?} is on its sheet", heading.block);
        let (caret, origin) = (layout.caret(&pos).unwrap(), pages.origin(heading.page).unwrap());
        assert!((heading.at.x - (caret.x + origin.x)).abs() < 1.0, "at the left of its text");
        assert!(heading.at.y <= caret.y + origin.y && heading.at.y >= pages.area.y - 0.5, "and at its top");
    }
}

#[test]
fn pages_follow_edits() {
    let edits: Vec<(&str, Vec<Op>)> = vec![
        ("typing a page's worth", vec![Op::InsertText { at: Pos::new("a", 5), text: PROSE.repeat(4) }]),
        ("deleting", vec![Op::DeleteText { block: "b".into(), range: 0..PROSE.len() * 2 }]),
        ("splitting", vec![Op::Split { at: Pos::new("b", 40), new_id: "n".into() }]),
        ("removing", vec![Op::RemoveBlock { block: "w".into() }]),
        (
            "adding a page break",
            vec![
                Op::RemoveBlock { block: "i".into() },
                Op::InsertBlock { parent: None, index: 4, block: breaking(paragraph("i", PROSE)) },
            ],
        ),
    ];
    for (name, ops) in edits {
        let mut model = Model::new(mixed(), ReplicaId::from(1));
        let mut layout = laid_at(model.doc(), COLUMN);
        let before = layout.paginate(&paper(240.0));
        let change = model.apply(ops).unwrap();
        layout.update(model.doc(), &Style::default(), COLUMN, Some(&change));
        let after = layout.paginate(&paper(240.0));
        assert!(after != before, "{name} changes the pages");
        assert!(after == laid_at(model.doc(), COLUMN).paginate(&paper(240.0)), "{name}: as paginating afresh");
        assert_eq!(check_pages(model.doc(), &layout, &after), Ok(()), "{name}");
    }
}

const WORDS: [&str; 8] = ["rates", "change", "on", "the", "first", "of", "January", "household"];

/// Documents of every kind of block, some marked to start a page.
fn documents() -> impl Strategy<Value = Document> {
    let words = |most: usize| prop::collection::vec(prop::sample::select(WORDS.to_vec()), 1..most);
    let lines = prop::collection::vec(words(14).prop_map(|words| words.join(".")), 1..12);
    let block = (0..14u8, words(90), lines, prop::bool::weighted(0.1));
    prop::collection::vec(block, 1..14).prop_map(|blocks| {
        let blocks = blocks.into_iter().enumerate().map(|(index, (kind, words, lines, breaks))| {
            let (id, text) = (format!("b{index}"), words.join(" "));
            let short = words[..words.len().min(6)].join(" ");
            let inside = |kind| {
                container(
                    &format!("c{index}"),
                    kind,
                    vec![paragraph(&id, &text), paragraph(&format!("d{index}"), &text)],
                )
            };
            let block = match kind {
                0 => Block::text(
                    id.as_str(),
                    BlockKind::Heading { level: 1 + (index % 3) as u8 },
                    words[..words.len().min(6)].join(" "),
                ),
                1 => code(&id, &lines.join("\n")),
                2 => Block::text(
                    id.as_str(),
                    BlockKind::ListItem { list: ListKind::Ordered, depth: (index % 2) as u8 },
                    text,
                ),
                3 => inside(BlockKind::Callout { tone: Tone::Info }),
                4 => inside(BlockKind::Quote),
                5 => Block::new(id.as_str(), BlockKind::Divider),
                6 => rich(&id, &[(&text, &[]), (r"\frac{a}{\frac{b}{c}}", &[Mark::Math]), (&text, &[])]),
                7 => formula(&id, r"\sum_{i=0}^{n} i"),
                8 => figure(&id, (120.0, 80.0), &short),
                9 => {
                    // Wide pictures, so that one alone in its row is still shorter than a page.
                    let tile = |at: usize| figure(&format!("{id}x{at}"), (120.0 + 30.0 * (at % 3) as f32, 60.0), "");
                    gallery(&format!("c{index}"), (0..1 + index % 5).map(tile).collect(), Some(&short))
                }
                10 => {
                    let rows: [&[&str]; 2] = [&[&short, "Rate"], &[&short, "9%"]];
                    table(&format!("t{index}x"), &[Align::Start, Align::End], index % 2 == 0, &rows)
                }
                _ => paragraph(&id, &text),
            };
            if breaks { breaking(block) } else { block }
        });
        doc(blocks.collect())
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Every line lands on exactly one page, in order, no page overflows its text area, and the rules of
    /// where a page may break hold.
    #[test]
    fn pages_hold_every_line_once(doc in documents(), room in 320.0f32..900.0) {
        let (layout, pages) = paged(&doc, room);
        prop_assert_eq!(check_pages(&doc, &layout, &pages), Ok(()));

        let blocks: Vec<&Block> = doc.walk().collect();
        let block = |id: &BlockId| blocks.iter().position(|block| block.id == *id).unwrap();
        let heading = |id: &BlockId| matches!(blocks[block(id)].kind, BlockKind::Heading { .. });
        let starts = starts(&doc);
        for (index, sheet) in pages.sheets.iter().enumerate() {
            let (last, _) = sheet.blocks.last().unwrap();
            let next = pages.sheets.get(index + 1).map(|next| block(&next.blocks[0].0));
            if let Some(next) = next.filter(|next| *next != block(last)) {
                let alone = sheet.blocks.iter().all(|(id, _)| heading(id));
                prop_assert!(!heading(last) || alone || starts.contains(&blocks[next].id), "page {} ends with the heading {:?}", index, last);
            }
        }
        for (id, parts) in parts(&pages) {
            let source = blocks[block(&id)];
            // A row of pictures may stand alone on a page, as no line of text does.
            let rows = matches!(source.kind, BlockKind::Gallery { .. });
            prop_assert!(rows || parts.len() == 1 || parts.iter().all(|part| part.1.len() >= 2), "{:?} leaves a line alone: {:?}", id, parts);
            if starts.contains(&id) {
                prop_assert!(pages.sheets[parts[0].0].blocks[0].0 == id, "{:?} starts a page", id);
            }
            if matches!(source.kind, BlockKind::Code { .. }) {
                let (text, mut start) = (source.plain_text(), 0);
                for line in text.split('\n') {
                    let on = |offset| layout.page_of(&pages, &Pos::new(id.clone(), offset));
                    prop_assert_eq!(on(start), on(start + line.len()), "a line of {:?} is split", id);
                    start += line.len() + 1;
                }
            }
        }
    }
}
