//! One leaf's text: shaping it with Parley, painting it and answering where things are inside it.

use std::ops::Range;

use letters_render::{Color, DisplayList, Item, Point, Rect};
use parley::{
    Affinity, Alignment, AlignmentOptions, Cursor, Language, Layout, PositionedLayoutItem, Selection as Span,
};

use crate::engine::{Brush, Engine, Spec};
use crate::flow::{Leaf, Marker, Role, Type, Types, What, marks};
use crate::theme;

/// A leaf's layout, kept while the leaf is near the viewport.
pub(crate) struct Laid {
    pub text: Layout<Brush>,
    /// The number of an ordered item or the tick of a done task, set beside the first line.
    pub sign: Option<Layout<Brush>>,
}

/// Shapes a leaf's text and breaks it to the leaf's width.
pub(crate) fn shape(
    engine: &mut Engine,
    types: &Types,
    leaf: &Leaf,
    locale: Option<Language>,
    layout: &mut Layout<Brush>,
) {
    let (of, code) = (types.get(leaf.role), types.get(Role::Code));
    let color = if leaf.what == What::Label { theme::MUTED } else { of.color };
    let spec = |bits: u8| marked(of, code, leaf.role == Role::Code, color, bits);
    let mut specs = Vec::with_capacity(leaf.spans.len().max(1));
    let mut runs = Vec::with_capacity(leaf.spans.len().max(1));
    if leaf.spans.is_empty() {
        specs.push(spec(0));
        if !leaf.text.is_empty() {
            runs.push((0, 0..leaf.text.len()));
        }
    } else {
        // One spec per distinct set of marks, however many spans share it.
        let mut slots = [u16::MAX; 128];
        let mut start = 0;
        for span in &leaf.spans {
            let slot = &mut slots[usize::from(span.marks)];
            if *slot == u16::MAX {
                *slot = specs.len() as u16;
                specs.push(spec(span.marks));
            }
            runs.push((*slot, start..span.end));
            start = span.end;
        }
    }
    engine.build(&leaf.text, &specs, &runs, locale, layout);
    let align = if leaf.boxed() { Alignment::Start } else { of.align };
    layout.set_alignment(align, AlignmentOptions::default());
    layout.break_all_lines(Some(leaf.width));
}

/// How text of type `of` looks under a set of marks.
fn marked<'a>(of: &'a Type, code: &'a Type, mono: bool, color: Color, bits: u8) -> Spec<'a> {
    let mut spec = of.spec(mono, Brush { color, back: None });
    if bits & marks::BOLD != 0 {
        // Bold must still stand out in a type that is already heavy.
        spec.weight = if of.weight >= 600.0 { (of.weight + 200.0).min(900.0) } else { 700.0 };
    }
    if bits & marks::ITALIC != 0 {
        // Emphasis inside italic text turns upright, as in print.
        spec.italic = !spec.italic;
    }
    if bits & marks::CODE != 0 {
        (spec.family, spec.mono) = (&code.family, true);
        spec.size = of.size * theme::INLINE_CODE_SCALE;
        spec.brush.back = Some(theme::CODE_INLINE);
    }
    if bits & marks::HIGHLIGHT != 0 {
        spec.brush.back = Some(theme::HIGHLIGHT);
    }
    if bits & marks::LINK != 0 {
        spec.brush.color = theme::LINK;
    }
    spec.underline = bits & (marks::UNDERLINE | marks::LINK) != 0;
    spec.strike = bits & marks::STRIKE != 0;
    spec
}

/// The height a leaf takes, in whole pixels so that the rows below stay on the pixel grid.
pub(crate) fn height(layout: &Layout<Brush>) -> f32 {
    layout.height().round().max(1.0)
}

/// Lays a leaf out in full, with the sign its marker needs.
pub(crate) fn lay(engine: &mut Engine, types: &Types, leaf: &Leaf, locale: Option<Language>) -> Laid {
    let mut text = Layout::new();
    shape(engine, types, leaf, locale, &mut text);
    let of = types.get(leaf.role);
    let sign = match leaf.marker {
        Marker::Number(number) => Some((format!("{number}."), of.color, of.size)),
        Marker::Task(true) => Some(("\u{2713}".to_string(), theme::ON_ACCENT, theme::CHECKBOX * 0.8)),
        _ => None,
    };
    let sign = sign.map(|(label, color, size)| {
        let mut spec = of.spec(false, Brush { color, back: None });
        (spec.size, spec.italic) = (size, false);
        let mut layout = Layout::new();
        engine.build(&label, &[spec], &[(0, 0..label.len())], None, &mut layout);
        layout.break_all_lines(None);
        layout
    });
    Laid { text, sign }
}

/// Emits the lines of a leaf that fall between `top` and `bottom`, with its box and marker.
pub(crate) fn paint(
    engine: &mut Engine,
    types: &Types,
    leaf: &Leaf,
    laid: &Laid,
    (top, bottom): (f32, f32),
    out: &mut DisplayList,
) {
    let at = Point { x: leaf.x, y: leaf.y };
    if leaf.boxed() {
        let rect = Rect::new(
            leaf.x - leaf.pad,
            leaf.y - theme::BOX_PAD_Y,
            leaf.width + 2.0 * leaf.pad,
            leaf.height + 2.0 * theme::BOX_PAD_Y,
        );
        out.push(match leaf.what {
            What::Label => Item::Stroke { rect, color: theme::LINE_STRONG, width: 1.0, radius: theme::BOX_RADIUS },
            _ => Item::Fill { rect, color: theme::CODE_BLOCK, radius: theme::BOX_RADIUS },
        });
    }
    if let Some(first) = laid.text.get(0)
        && leaf.y + first.metrics().block_min_coord < bottom
        && leaf.y + first.metrics().block_max_coord > top
    {
        marker(engine, types.get(leaf.role), leaf, laid, first.metrics().baseline, out);
    }
    for line in laid.text.lines() {
        let metrics = line.metrics();
        if leaf.y + metrics.block_max_coord <= top || leaf.y + metrics.block_min_coord >= bottom {
            continue;
        }
        // Fills go under every glyph of the line, so one run's fill never covers its neighbour's overhang.
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else { continue };
            let Some(color) = run.style().brush.back else { continue };
            let font = run.run().font_metrics();
            let rect = Rect::new(
                at.x + run.offset() - 2.0,
                at.y + run.baseline() - font.ascent,
                run.advance() + 4.0,
                font.ascent + font.descent,
            );
            out.push(Item::Fill { rect, color, radius: theme::INLINE_RADIUS });
        }
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else { continue };
            let style = run.style();
            out.items.extend(engine.glyphs(&run, at, style.brush.color));
            let font = run.run().font_metrics();
            let strokes = [
                (&style.underline, font.underline_offset, font.underline_size),
                (&style.strikethrough, font.strikethrough_offset, font.strikethrough_size),
            ];
            for (stroke, offset, size) in strokes {
                let Some(stroke) = stroke else { continue };
                let size = stroke.size.unwrap_or(size).max(1.0);
                let y = at.y + run.baseline() - stroke.offset.unwrap_or(offset);
                let rect = Rect::new(at.x + run.offset(), y, run.advance(), size);
                out.push(Item::Fill { rect, color: stroke.brush.color, radius: 0.0 });
            }
        }
    }
}

/// Draws a list item's marker in the indent, level with the first line.
fn marker(engine: &mut Engine, of: &Type, leaf: &Leaf, laid: &Laid, baseline: f32, out: &mut DisplayList) {
    let right = leaf.x - theme::MARKER_GAP;
    // The middle of a lower-case letter, which is where a bullet or checkbox looks centred.
    let middle = leaf.y + baseline - of.size * 0.34;
    match leaf.marker {
        Marker::None => {}
        Marker::Bullet(depth) => {
            let side = (of.size * 0.36).round().max(3.0);
            let rect = Rect::new(right - side, (middle - side / 2.0).round(), side, side);
            out.push(match depth % 3 {
                0 => Item::Fill { rect, color: of.color, radius: side / 2.0 },
                1 => Item::Stroke { rect, color: of.color, width: 1.0, radius: side / 2.0 },
                _ => Item::Fill { rect, color: of.color, radius: 0.0 },
            });
        }
        Marker::Number(_) => {
            let Some(sign) = &laid.sign else { return };
            let at = Point { x: right - sign.full_width(), y: leaf.y };
            glyphs(engine, sign, at, out);
        }
        Marker::Task(checked) => {
            let side = theme::CHECKBOX;
            let rect = Rect::new(right - side, (middle - side / 2.0).round(), side, side);
            if !checked {
                out.push(Item::Stroke { rect, color: theme::LINE_STRONG, width: 1.5, radius: 3.0 });
                return;
            }
            out.push(Item::Fill { rect, color: theme::ACCENT, radius: 3.0 });
            let Some(sign) = &laid.sign else { return };
            let at = Point {
                x: rect.x + (side - sign.full_width()) / 2.0,
                y: rect.y + ((side - sign.height()) / 2.0).round(),
            };
            glyphs(engine, sign, at, out);
        }
    }
}

fn glyphs(engine: &mut Engine, layout: &Layout<Brush>, at: Point, out: &mut DisplayList) {
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else { continue };
            out.items.extend(engine.glyphs(&run, at, run.style().brush.color));
        }
    }
}

/// The byte offset nearest to a point given from the leaf's top left.
///
/// A [`letters_model::Pos`] has no affinity, and an offset at a wrap draws at the start of the next line. So a
/// point past the end of a wrapped line lands before the space the line wraps at, never after it.
pub(crate) fn hit(laid: &Laid, x: f32, y: f32) -> usize {
    let cursor = Cursor::from_point(&laid.text, x, y);
    if cursor.affinity() == Affinity::Upstream
        && cursor.index() < laid.text.text_len()
        && let Some(last) = cursor.upstream_cluster(&laid.text)
        && last.is_soft_line_break()
    {
        return last.text_range().start;
    }
    cursor.index()
}

/// The line holding a byte offset.
pub(crate) fn line_of(laid: &Laid, offset: usize) -> usize {
    let at = Cursor::from_byte_index(&laid.text, offset, Affinity::Downstream).geometry(&laid.text, 0.0);
    let middle = ((at.y0 + at.y1) / 2.0) as f32;
    let line = laid.text.lines().position(|line| middle < line.metrics().block_max_coord);
    line.unwrap_or(laid.text.len().saturating_sub(1))
}

/// The byte offset on a line nearest to `x`, given from the leaf's left edge.
pub(crate) fn on_line(laid: &Laid, line: usize, x: f32) -> usize {
    let Some(line) = laid.text.get(line) else { return 0 };
    let metrics = line.metrics();
    hit(laid, x, (metrics.block_min_coord + metrics.block_max_coord) / 2.0)
}

/// Where a caret at a byte offset is drawn, in page coordinates. It spans the line's text, not its leading.
pub(crate) fn caret(leaf: &Leaf, laid: &Laid, offset: usize) -> Rect {
    let at = Cursor::from_byte_index(&laid.text, offset, Affinity::Downstream).geometry(&laid.text, theme::CARET);
    let (mut top, mut bottom) = (at.y0 as f32, at.y1 as f32);
    if let Some(line) = laid.text.get(line_of(laid, offset)) {
        let metrics = line.metrics();
        (top, bottom) = (metrics.content_block_min_coord, metrics.content_block_max_coord);
    }
    Rect::new(leaf.x + at.x0 as f32, leaf.y + top, theme::CARET, bottom - top)
}

/// Adds the rectangles that highlight a byte range, in page coordinates. `newline` also shows the block's end
/// as selected, for a selection that runs on into the next block.
pub(crate) fn selection(leaf: &Leaf, laid: &Laid, range: Range<usize>, newline: bool, out: &mut Vec<Rect>) {
    let layout = &laid.text;
    let first = out.len();
    let start = Cursor::from_byte_index(layout, range.start, Affinity::Downstream);
    let end = Cursor::from_byte_index(layout, range.end, Affinity::Upstream);
    Span::new(start, end).geometry_with(layout, |area, _| {
        let (x, y) = (leaf.x + area.x0 as f32, leaf.y + area.y0 as f32);
        out.push(Rect::new(x, y, area.width() as f32, area.height() as f32));
    });
    if !newline {
        return;
    }
    let Some(line) = layout.lines().next_back() else { return };
    let metrics = line.metrics();
    let end = Cursor::from_byte_index(layout, layout.text_len(), Affinity::Downstream).geometry(layout, 0.0);
    let (x, y) = (leaf.x + end.x0 as f32, leaf.y + metrics.block_min_coord);
    let width = (metrics.line_height * theme::NEWLINE).round();
    match out[first..].last_mut() {
        Some(last) if last.y == y && (last.x + last.width - x).abs() < 0.5 => last.width += width,
        _ => out.push(Rect::new(x, y, width, metrics.block_max_coord - metrics.block_min_coord)),
    }
}
