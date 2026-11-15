//! Shared fixtures: small documents, a layout of them and ways to read what was painted.
#![allow(dead_code)]

use letters_layout::{DocLayout, Fonts, Palette, Scheme};
use letters_model::{Align, Block, BlockKind, Body, Document, Mark, Page, Run, Style};
use letters_render::{Color, DisplayList, Item, Rect};

pub const WIDTH: f32 = 600.0;
pub const PROSE: &str = "Rates change on the first of January, and every household on the scheme should check the \
    notice that arrives with the December statement before the new figures take effect.";

pub fn fonts() -> Fonts {
    Fonts { faces: letters_fonts::bundled() }
}

pub fn doc(blocks: Vec<Block>) -> Document {
    Document { blocks, ..Document::default() }
}

pub fn paragraph(id: &str, text: &str) -> Block {
    Block::text(id, BlockKind::Paragraph, text)
}

/// A paragraph of runs, each plain or marked.
pub fn rich(id: &str, runs: &[(&str, &[Mark])]) -> Block {
    let runs = runs.iter().map(|(text, marks)| Run::marked(*text, marks.iter().cloned())).collect();
    Block { body: Body::Text(runs), ..paragraph(id, "") }
}

/// A formula set as a block, from its TeX.
pub fn formula(id: &str, tex: &str) -> Block {
    Block::text(id, BlockKind::Formula, tex)
}

pub fn container(id: &str, kind: BlockKind, children: Vec<Block>) -> Block {
    Block { body: Body::Blocks(children), ..Block::new(id, kind) }
}

/// A figure of `size` points with a caption. Its picture is the asset `assets/<id>.png`.
pub fn figure(id: &str, size: (f32, f32), caption: &str) -> Block {
    let (asset, alt) = (Some(format!("assets/{id}.png")), format!("Picture {id}"));
    let kind = BlockKind::Figure { asset, src: None, alt, width: size.0, height: size.1, numbered: false };
    Block::text(id, kind, caption)
}

/// The same figure with its picture still a link to `src`.
pub fn remote(mut figure: Block, src: &str) -> Block {
    if let BlockKind::Figure { asset, src: link, .. } = &mut figure.kind {
        (*asset, *link) = (None, Some(src.into()));
    }
    figure
}

/// The same figure or gallery, numbered.
pub fn numbered(mut block: Block) -> Block {
    if let BlockKind::Figure { numbered, .. } | BlockKind::Gallery { numbered } = &mut block.kind {
        *numbered = true;
    }
    block
}

/// A gallery of figures. Its caption, when it has one, is the block `<id>c`.
pub fn gallery(id: &str, mut figures: Vec<Block>, caption: Option<&str>) -> Block {
    figures.extend(caption.map(|text| Block::text(format!("{id}c").as_str(), BlockKind::Caption, text)));
    container(id, BlockKind::Gallery { numbered: false }, figures)
}

/// A table of text cells. Row `r` is the block `<id><r>`, and its cell in column `c` is `<id><r><c>`.
pub fn table(id: &str, columns: &[Align], header: bool, rows: &[&[&str]]) -> Block {
    let cell = |row: usize, (column, text): (usize, &&str)| {
        Block::text(format!("{id}{row}{column}").as_str(), BlockKind::TableCell, *text)
    };
    let row = |(index, cells): (usize, &&[&str])| {
        let cells = cells.iter().enumerate().map(|at| cell(index, at)).collect();
        container(&format!("{id}{index}"), BlockKind::TableRow, cells)
    };
    let kind = BlockKind::Table { columns: columns.to_vec(), header };
    container(id, kind, rows.iter().enumerate().map(row).collect())
}

/// A sheet of paper whose text area is `column` wide and `room` tall, in logical pixels.
pub fn paper(column: f32, room: f32) -> Page {
    let (margin, points) = (30.0, 72.0 / 96.0);
    Page {
        width: column * points + 2.0 * margin,
        height: room * points + 2.0 * margin,
        margin_top: margin,
        margin_right: margin,
        margin_bottom: margin,
        margin_left: margin,
        ..Page::default()
    }
}

/// What a document's sheets spell, in order, when it is paged at `WIDTH` with all the room it wants.
pub fn spelt(layout: &DocLayout) -> String {
    let pages = layout.paginate(&paper(WIDTH, 1.0e6));
    let mut list = DisplayList::default();
    for index in 0..pages.sheets.len() {
        layout.paint_page(&pages, index, &mut list);
    }
    let texts = list.items.iter().filter_map(|item| match item {
        Item::Glyphs { text: Some(run), .. } => Some(run.text.to_string()),
        _ => None,
    });
    texts.collect()
}

/// `count` paragraphs of wrapping prose, named `p0`, `p1` and so on.
pub fn prose(count: usize) -> Document {
    doc((0..count).map(|i| paragraph(&format!("p{i}"), PROSE)).collect())
}

pub fn laid(doc: &Document) -> DocLayout {
    laid_at(doc, WIDTH)
}

pub fn laid_at(doc: &Document, width: f32) -> DocLayout {
    let mut layout = DocLayout::new(fonts());
    layout.update(doc, &Style::default(), width, None);
    layout
}

pub fn everything() -> Rect {
    Rect::new(0.0, -1.0e6, WIDTH, 2.0e6)
}

pub fn painted(layout: &DocLayout, viewport: Rect) -> Vec<Item> {
    let mut list = DisplayList::default();
    layout.paint(viewport, &mut list);
    list.items
}

/// The vertical extent of an item, loosely: glyphs are given their ascent and descent around the baseline.
pub fn extent(item: &Item) -> Option<(f32, f32)> {
    match item {
        Item::Fill { rect, .. } | Item::Stroke { rect, .. } | Item::Image { rect, .. } => {
            Some((rect.y, rect.y + rect.height))
        }
        Item::Glyphs { origin, size, .. } => Some((origin.y - size, origin.y + size / 4.0)),
        _ => None,
    }
}

pub fn color_of(item: &Item) -> Option<Color> {
    match item {
        Item::Fill { color, .. } | Item::Stroke { color, .. } | Item::Glyphs { color, .. } => Some(*color),
        _ => None,
    }
}

/// A dark page: the default palette with yewi's dark surface and text.
pub fn dark() -> Palette {
    let (page, text) = (Color::rgb(0x22, 0x22, 0x24), Color::rgb(0xd0, 0xd0, 0xd1));
    Palette { scheme: Scheme::Dark, page, text, ..Palette::LIGHT }
}

/// Every glyph painted and where it sits on the page, whatever items carry it.
pub fn placed(items: &[Item]) -> Vec<(u32, f32, f32)> {
    let glyphs = glyph_runs(items).flat_map(|item| match item {
        Item::Glyphs { origin, glyphs, .. } => glyphs.iter().map(|g| (g.id, origin.x + g.x, origin.y + g.y)).collect(),
        _ => Vec::new(),
    });
    glyphs.collect()
}

/// The glyphs painted in `color`.
pub fn inked(items: &[Item], color: Color) -> Vec<u32> {
    let runs = glyph_runs(items).filter(|item| color_of(item) == Some(color));
    runs.flat_map(
        |item| if let Item::Glyphs { glyphs, .. } = item { glyphs.iter().map(|g| g.id).collect() } else { vec![] },
    )
    .collect()
}

pub fn glyph_runs(items: &[Item]) -> impl Iterator<Item = &Item> {
    items.iter().filter(|item| matches!(item, Item::Glyphs { .. }))
}

pub fn settled(layout: &mut DocLayout) {
    while layout.settle(Rect::new(0.0, 0.0, WIDTH, 800.0), 1 << 20) {}
}
