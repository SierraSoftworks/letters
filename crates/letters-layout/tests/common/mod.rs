//! Shared fixtures: small documents, a layout of them and ways to read what was painted.
#![allow(dead_code)]

use letters_layout::{DocLayout, Fonts, Palette, Scheme};
use letters_model::{Block, BlockKind, Body, Document, Mark, Run, Style};
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
        Item::Fill { rect, .. } | Item::Stroke { rect, .. } => Some((rect.y, rect.y + rect.height)),
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
