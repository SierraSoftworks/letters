//! Tables of contents: the lines they list, where those lead, and the page numbers a sheet gives them.

use letters_layout::{ContentsBox, DocLayout, Palette};
use letters_model::{BlockId, BlockKind, Document, Model, Op, Pos, ReplicaId, Style};
use letters_render::{Point, Rect};

mod common;
use common::*;

const HEBREW: &str = "\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{5e2}\u{5d5}\u{5dc}\u{5dd}";

/// A report with a title, a table of contents that lists down to `levels`, and headings of three levels
/// with a paragraph under each.
fn report(levels: u8) -> Document {
    let headings = [
        ("a", 1, "What changes"),
        ("b", 2, "For tenants"),
        ("d", 3, "In shared houses"),
        ("e", 2, "For owners"),
        ("f", 1, "When it starts"),
    ];
    let mut blocks = vec![heading("t", 1, "Household rates"), contents("c", levels)];
    for (id, level, text) in headings {
        blocks.extend([heading(id, level, text), paragraph(&format!("p{id}"), PROSE)]);
    }
    doc(blocks)
}

/// The one table of contents a layout shows.
fn table(layout: &DocLayout) -> ContentsBox {
    let mut tables = layout.contents(everything());
    assert_eq!(tables.len(), 1, "the document has one table of contents");
    tables.remove(0)
}

fn level(doc: &Document, heading: &BlockId) -> u8 {
    match doc.block(heading).map(|block| &block.kind) {
        Some(BlockKind::Heading { level }) => *level,
        kind => panic!("{heading:?} is no heading: {kind:?}"),
    }
}

#[test]
fn a_table_lists_its_headings_set_in_from_the_shallowest() {
    let deep = doc(vec![contents("c", 4), heading("a", 2, "For tenants"), heading("b", 3, "In shared houses")]);
    let cases =
        [("every level", report(4)), ("two levels", report(2)), ("one level", report(1)), ("no first level", deep)];
    for (name, doc) in cases {
        let layout = laid(&doc);
        let (table, items) = (table(&layout), painted(&layout, everything()));
        let listed: Vec<BlockId> = doc.contents(&"c".into()).into_iter().map(|heading| heading.id.clone()).collect();
        let lines: Vec<BlockId> = table.lines.iter().map(|(heading, _)| heading.clone()).collect();
        assert!(!lines.is_empty() && lines == listed, "{name}: the lines are the model's, in order: {lines:?}");

        let starts: Vec<(u8, f32, Rect)> = table
            .lines
            .iter()
            .map(|(heading, rect)| (level(&doc, heading), reach(&items, *rect).expect("a line has text").0, *rect))
            .collect();
        let shallowest = starts.iter().map(|(level, ..)| *level).min().unwrap();
        for pair in starts.windows(2) {
            let ((above, left, over), (below, right, under)) = (pair[0], pair[1]);
            assert!(over.y + over.height <= under.y + 0.5, "{name}: the lines stand one under the other");
            let set_in = (f32::from(below) - f32::from(above)).signum();
            assert_eq!((right - left).round().signum(), set_in, "{name}: a deeper heading is set further in");
        }
        for (level, left, rect) in starts {
            let inside = rect.y >= table.rect.y && rect.y + rect.height <= table.rect.y + table.rect.height + 0.5;
            assert!(inside, "{name}: a line is inside its table");
            let edge = level == shallowest;
            assert_eq!(left - table.rect.x < 4.0, edge, "{name}: only the shallowest level starts at the edge");
        }
    }
}

#[test]
fn a_table_follows_what_its_headings_say() {
    let start = || {
        let mut doc = report(2);
        doc.blocks.push(preview("v", "https://example.org/notice", [None, None, None]));
        doc
    };
    let titled = BlockKind::LinkPreview {
        url: "https://example.org/notice".into(),
        title: Some("The notice".into()),
        description: Some(PROSE.into()),
        image: None,
    };
    let edits: Vec<(&str, Vec<Op>)> = vec![
        ("typing in a heading", vec![Op::InsertText { at: Pos::new("b", 4), text: "all the ".into() }]),
        ("emptying a heading", vec![Op::DeleteText { block: "e".into(), range: 0..10 }]),
        ("typing a heading long enough to wrap", vec![Op::InsertText { at: Pos::new("a", 0), text: PROSE.into() }]),
        ("typing under a heading", vec![Op::InsertText { at: Pos::new("pa", 0), text: "Soon. ".into() }]),
        ("raising a heading", vec![Op::SetKind { block: "d".into(), kind: BlockKind::Heading { level: 2 } }]),
        (
            "making a heading of a paragraph",
            vec![Op::SetKind { block: "pb".into(), kind: BlockKind::Heading { level: 1 } }],
        ),
        ("removing a heading", vec![Op::RemoveBlock { block: "e".into() }]),
        ("adding a heading", vec![Op::InsertBlock { parent: None, index: 3, block: heading("n", 2, "New") }]),
        ("listing deeper", vec![Op::SetKind { block: "c".into(), kind: BlockKind::TableOfContents { levels: 3 } }]),
        (
            "removing every heading it lists",
            "abdef".chars().map(|id| Op::RemoveBlock { block: id.to_string().as_str().into() }).collect(),
        ),
        ("fetching a preview", vec![Op::SetKind { block: "v".into(), kind: titled }]),
    ];
    for (name, ops) in edits {
        let mut model = Model::new(start(), ReplicaId::from(1));
        let mut layout = laid(model.doc());
        painted(&layout, everything());
        let change = model.apply(ops).unwrap();
        layout.update(model.doc(), &Style::default(), WIDTH, Some(&change));

        let fresh = laid(model.doc());
        assert_eq!(painted(&layout, everything()), painted(&fresh, everything()), "{name}");
        assert_eq!(layout.contents(everything()), fresh.contents(everything()), "{name}");
        assert_eq!(layout.previews(everything()), fresh.previews(everything()), "{name}");
        assert_eq!(layout.height(), fresh.height(), "{name}");

        let change = model.undo().unwrap();
        layout.update(model.doc(), &Style::default(), WIDTH, Some(&change));
        assert_eq!(painted(&layout, everything()), painted(&laid(&start()), everything()), "undoing {name}");
    }
}

#[test]
fn a_sheet_gives_each_line_its_page_and_leads_to_its_heading() {
    let doc = report(3);
    let layout = laid_at(&doc, 320.0);
    let pages = layout.paginate(&paper(320.0, 300.0));
    assert!(pages.sheets.len() > 2, "the report runs over several sheets");
    let outline = layout.outline(&pages);
    let listed: Vec<_> = outline.iter().filter(|heading| heading.block != "t".into()).collect();

    let (items, origin) = (sheet(&layout, &pages, 0), pages.origin(0).unwrap());
    let leads: Vec<_> = layout.page_links(&pages, 0);
    let places: Vec<(usize, Point)> = leads.iter().filter_map(|link| link.place).collect();
    let expected: Vec<(usize, Point)> = listed.iter().map(|heading| (heading.page, heading.at)).collect();
    assert_eq!(places, expected, "each line leads to where the outline has its heading");
    assert!(leads.iter().all(|link| link.target.is_empty()), "and nowhere out of the document");
    assert!(places.iter().any(|(page, _)| *page > 0), "some headings are on later sheets");

    let spelt = links(&items);
    let parts: Vec<Option<u64>> = leads.iter().map(|link| link.part).collect();
    assert_eq!(parts, spelt.iter().map(|(id, _)| Some(*id)).collect::<Vec<_>>(), "each line is one link of the sheet");
    for ((_, text), heading) in spelt.iter().zip(&listed) {
        let number = (heading.page + 1).to_string();
        let reads = text.starts_with(heading.title.as_str()) && text.ends_with(&number);
        assert!(reads, "a line reads as its heading and the number of its sheet: {text:?}");
    }

    // A number stands at the end of its line, in the room every line keeps for it.
    let table = table(&layout);
    let screen = painted(&layout, everything());
    for ((_, line), link) in table.lines.iter().zip(&leads) {
        let on_sheet = Rect { x: line.x + origin.x, y: line.y + origin.y, ..*line };
        assert_eq!(link.rect, on_sheet, "the link is the line, across the table");
        let (written, numbered) = (reach(&screen, *line).unwrap().1, reach(&items, on_sheet).unwrap().1 - origin.x);
        assert!(numbered > written + 8.0, "only a sheet numbers a line: {written} on screen, {numbered} on paper");
        assert!(numbered > line.x + line.width - 16.0, "the number ends the line");
    }
}

#[test]
fn a_line_that_reads_from_the_right_ends_at_the_left() {
    let doc = doc(vec![contents("c", 2), heading("a", 1, HEBREW), heading("b", 2, HEBREW), paragraph("p", PROSE)]);
    let layout = laid(&doc);
    let pages = layout.paginate(&paper(WIDTH, 800.0));
    let (items, origin, table) = (sheet(&layout, &pages, 0), pages.origin(0).unwrap(), table(&layout));
    let ends: Vec<(f32, f32)> = table
        .lines
        .iter()
        .map(|(_, line)| reach(&items, Rect { x: line.x + origin.x, y: line.y + origin.y, ..*line }).unwrap())
        .map(|(left, right)| (left - origin.x, right - origin.x))
        .collect();
    let (first, second) = (ends[0], ends[1]);
    assert!(first.0 < 4.0 && second.0 < 4.0, "the numbers stand at the left: {ends:?}");
    assert!(first.1 > WIDTH - 24.0, "a line starts at the right edge: {first:?}");
    assert!(second.1 < first.1 - 12.0, "and a deeper one is set in from there: {ends:?}");
}

#[test]
fn a_table_with_nothing_to_list_says_so_on_screen_alone() {
    let doc = doc(vec![heading("t", 1, "Household rates"), contents("c", 3), paragraph("p", "Nothing yet.")]);
    let layout = laid(&doc);
    let table = table(&layout);
    assert!(table.lines.is_empty() && table.rect.height > 0.0, "it has a line of its own and leads nowhere");
    let items = painted(&layout, table.rect);
    let muted = inked(&items, Palette::LIGHT.marker);
    assert!(!muted.is_empty() && muted.len() == placed(&items).len(), "which is muted");
    let below = layout.caret(&Pos::new("p", 0)).unwrap();
    assert!(below.y >= table.rect.y + table.rect.height, "the text after it stands under it");

    assert_eq!(spelt(&layout), "Household ratesNothing yet.1", "paper shows the document, where it lists nothing");
    let pages = layout.paginate(&paper(WIDTH, 800.0));
    assert!(layout.page_links(&pages, 0).is_empty(), "and it leads nowhere there");
}

#[test]
fn a_long_table_runs_over_pages_between_its_lines() {
    let mut blocks = vec![contents("c", 1)];
    blocks.extend((0..40).map(|at| heading(&format!("h{at}"), 1, &format!("Part {at}"))));
    let layout = laid_at(&doc(blocks), 320.0);
    let pages = layout.paginate(&paper(320.0, 300.0));
    let (top, bottom) = (pages.area.y, pages.area.y + pages.area.height);
    let mut sheets = Vec::new();
    for index in 0..pages.sheets.len() {
        for link in layout.page_links(&pages, index) {
            let whole = link.rect.y >= top - 0.5 && link.rect.y + link.rect.height <= bottom + 0.5;
            assert!(whole, "a line is whole on its sheet: {:?} on sheet {index}", link.rect);
            sheets.push(index);
        }
    }
    assert_eq!(sheets.len(), 40, "every line is on one sheet");
    assert!(sheets.last() > sheets.first(), "and the table is on more than one");
}

#[test]
fn no_position_is_inside_a_table_or_a_preview() {
    let card = preview("v", "https://example.org/notice", [Some("The notice"), Some(PROSE), Some("assets/v.jpg")]);
    let blocks =
        vec![paragraph("a", "Above."), contents("c", 2), card, heading("h", 1, "Rates"), paragraph("z", "Below.")];
    for width in [40.0, 120.0, WIDTH] {
        let layout = laid_at(&doc(blocks.clone()), width);
        let view = Rect::new(0.0, -1.0e6, width, 2.0e6);
        painted(&layout, view);
        let (table, card) = (layout.contents(view).remove(0).rect, layout.previews(view).remove(0).rect);
        for (id, rect) in [("c", table), ("v", card)] {
            assert!(
                rect.height > 0.0 && rect.x >= 0.0 && rect.x + rect.width <= width + 0.5,
                "{id} at {width}: {rect:?}"
            );
            let middle = Point { x: rect.x + rect.width / 2.0, y: rect.y + rect.height / 2.0 };
            let hit = layout.hit(middle).expect("the document has text");
            assert!(["a", "h"].map(BlockId::from).contains(&hit.block), "a point on {id} finds text beside it");
            assert_eq!(layout.caret(&Pos::new(id, 0)), None, "no caret stands in {id}");
            let band = layout.band(&id.into()).expect("the block has its stretch of the column");
            assert!(band.0 <= rect.y && band.1 >= rect.y + rect.height, "{id} at {width}: {band:?} holds {rect:?}");
        }
        let down = layout.line_step(&Pos::new("a", "Above.".len()), 0.0, true).unwrap();
        assert_eq!(down.block, "h".into(), "a step down from the line above them lands under them at {width}");
    }
}
