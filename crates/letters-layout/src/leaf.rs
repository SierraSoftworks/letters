//! One leaf's text: shaping it with Parley, painting it and answering where things are inside it.

use std::ops::Range;

use letters_render::{Color, DisplayList, Glyph, Item, Point, Rect};
use parley::{
    Affinity, Alignment, AlignmentOptions, Cursor, GlyphRun, Language, Layout, Line, PositionedLayoutItem,
    Selection as Span,
};

use crate::engine::{Engine, Spec};
use crate::flow::{Leaf, Marker, Role, Type, Types, What, marks};
use crate::ink::{Back, Brush, Ink, Inks};
use crate::reveal::{Shapes, Shifts};
use crate::theme;
use crate::tokens::{self, Token, TokenKind};

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
    let ink = if leaf.what == What::Label { Ink::Marker } else { Ink::Style(leaf.role) };
    let spec = |bits: u8| marked(of, code, leaf.role == Role::Code, ink, bits);
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
fn marked<'a>(of: &'a Type, code: &'a Type, mono: bool, ink: Ink, bits: u8) -> Spec<'a> {
    let mut spec = of.spec(mono, Brush { ink, back: None });
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
        spec.brush.back = Some(Back::Code);
    }
    if bits & marks::HIGHLIGHT != 0 {
        spec.brush.back = Some(Back::Highlight);
    }
    if bits & marks::LINK != 0 {
        spec.brush.ink = Ink::Link;
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
        Marker::Number(number) => Some((format!("{number}."), Ink::Marker, of.size)),
        Marker::Task(true) => Some(("\u{2713}".to_string(), Ink::OnAccent, theme::CHECKBOX * 0.8)),
        _ => None,
    };
    let sign = sign.map(|(label, ink, size)| {
        let mut spec = of.spec(false, Brush { ink, back: None });
        (spec.size, spec.italic) = (size, false);
        let mut layout = Layout::new();
        engine.build(&label, &[spec], &[(0, 0..label.len())], None, &mut layout);
        layout.break_all_lines(None);
        layout
    });
    Laid { text, sign }
}

/// Glyphs of one style and one kind of token, gathered along a line.
struct Stretch {
    style: u16,
    kind: Option<TokenKind>,
    ink: Ink,
    /// Where the first glyph starts, from the line's left edge.
    start: f32,
    glyphs: Vec<Glyph>,
}

/// What is drawn over a leaf's text without being part of it.
pub(crate) struct Over<'a> {
    /// The markers revealed in the leaf.
    pub shifts: &'a Shifts,
    pub shapes: &'a Shapes,
    /// The text an input method is composing, which is underlined.
    pub composing: Option<Range<usize>>,
}

/// What painting draws with and into.
pub(crate) struct Painter<'a> {
    pub engine: &'a mut Engine,
    pub types: &'a Types,
    pub inks: &'a Inks,
    /// The top and bottom of what is being painted. Lines outside it are skipped.
    pub band: (f32, f32),
    pub out: &'a mut DisplayList,
}

impl Painter<'_> {
    /// Emits the lines of a leaf that fall inside the band, with its box and marker. `tokens` colour the
    /// text of a code block.
    pub fn leaf(&mut self, leaf: &Leaf, laid: &Laid, tokens: Option<&[Token]>, over: &Over<'_>) {
        let at = Point { x: leaf.x, y: leaf.y };
        let palette = &self.inks.palette;
        let (top, bottom) = self.band;
        if leaf.boxed() {
            let rect = Rect::new(
                leaf.x - leaf.pad,
                leaf.y - theme::BOX_PAD_Y,
                leaf.width + 2.0 * leaf.pad,
                leaf.height + 2.0 * theme::BOX_PAD_Y,
            );
            self.out.push(match leaf.what {
                What::Label => Item::Stroke { rect, color: palette.line_strong, width: 1.0, radius: theme::BOX_RADIUS },
                _ => Item::Fill { rect, color: palette.code, radius: theme::BOX_RADIUS },
            });
        }
        if let Some(first) = laid.text.get(0)
            && leaf.y + first.metrics().block_min_coord < bottom
            && leaf.y + first.metrics().block_max_coord > top
        {
            let baseline = first.metrics().baseline;
            self.marker(leaf, laid, baseline);
            // A heading's marker hangs in the margin. Where the margin is too narrow, the line makes room.
            let mut right = leaf.x - theme::MARKER_GAP + over.shifts.indent;
            for (shape, level) in over.shifts.hung.iter().rev() {
                let Some(shape) = over.shapes.layout(*shape) else { continue };
                right -= shape.full_width();
                self.sign(shape, Point { x: right, y: leaf.y + baseline }, *level);
            }
        }
        for (index, line) in laid.text.lines().enumerate() {
            let metrics = line.metrics();
            if leaf.y + metrics.block_max_coord <= top || leaf.y + metrics.block_min_coord >= bottom {
                continue;
            }
            let pushed = over.shifts.touches(index);
            // Fills go under every glyph of the line, so one run's fill never covers its neighbour's overhang.
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let Some(back) = run.style().brush.back else { continue };
                let font = run.run().font_metrics();
                let (lead, wider) = over.shifts.spread(index, run.offset(), run.offset() + run.advance());
                let rect = Rect::new(
                    at.x + run.offset() + lead - 2.0,
                    at.y + run.baseline() - font.ascent,
                    run.advance() + wider + 4.0,
                    font.ascent + font.descent,
                );
                self.out.push(Item::Fill { rect, color: self.inks.back(back), radius: theme::INLINE_RADIUS });
            }
            if let Some(tokens) = tokens {
                self.tokens(&line, at, tokens);
            }
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let style = run.style();
                let color = self.inks.ink(style.brush.ink);
                match (tokens, pushed) {
                    (Some(_), _) => {}
                    (None, true) => self.pushed(&run, at, color, index, over.shifts),
                    (None, false) => self.out.items.extend(self.engine.glyphs(&run, at, color)),
                }
                let font = run.run().font_metrics();
                let (lead, wider) = over.shifts.spread(index, run.offset(), run.offset() + run.advance());
                let strokes = [
                    (&style.underline, font.underline_offset, font.underline_size),
                    (&style.strikethrough, font.strikethrough_offset, font.strikethrough_size),
                ];
                for (stroke, offset, size) in strokes {
                    let Some(stroke) = stroke else { continue };
                    let size = stroke.size.unwrap_or(size).max(1.0);
                    let y = at.y + run.baseline() - stroke.offset.unwrap_or(offset);
                    let rect = Rect::new(at.x + run.offset() + lead, y, run.advance() + wider, size);
                    self.out.push(Item::Fill { rect, color: self.inks.ink(stroke.brush.ink), radius: 0.0 });
                }
            }
            // Each marker goes in the room pushed open for it.
            let mut room = 0.0;
            for shown in over.shifts.on(index) {
                if let Some(shape) = over.shapes.layout(shown.shape) {
                    self.sign(shape, Point { x: at.x + shown.x + room, y: at.y + metrics.baseline }, shown.level);
                }
                room += shown.width;
            }
        }
        if let Some(range) = &over.composing {
            self.composing(leaf, laid, range, over.shifts);
        }
    }

    /// Emits a run's glyphs on a line that holds markers, each pushed by the markers before it.
    fn pushed(&mut self, run: &GlyphRun<'_, Brush>, at: Point, color: Color, line: usize, shifts: &Shifts) {
        // Along the run, as when nothing is pushed, so that glyphs before the first marker land where they were.
        let mut pen = 0.0;
        let mut open: Option<(f32, f32, Vec<Glyph>)> = None;
        let mut close = |open: Option<(f32, f32, Vec<Glyph>)>| {
            let Some((push, start, glyphs)) = open else { return };
            let origin = Point { x: at.x + run.offset() + start + push, y: at.y + run.baseline() };
            self.out.items.extend(self.engine.item(run.run(), origin, color, false, glyphs));
        };
        for glyph in run.glyphs() {
            let push = shifts.push(line, run.offset() + pen);
            if open.as_ref().is_none_or(|(was, ..)| *was != push) {
                close(open.take());
                open = Some((push, pen, Vec::new()));
            }
            if let Some((_, start, glyphs)) = &mut open {
                glyphs.push(Glyph { id: glyph.id, x: pen - *start + glyph.x, y: glyph.y });
            }
            pen += glyph.advance;
        }
        close(open);
    }

    /// Draws a revealed marker with the left end of its baseline at `at`. `level` is how much of it shows.
    pub fn sign(&mut self, shape: &Layout<Brush>, at: Point, level: f32) {
        let Some(line) = shape.get(0) else { return };
        let Color { r, g, b, a } = self.inks.palette.marker;
        let color = Color::rgba(r, g, b, (f32::from(a) * level.clamp(0.0, 1.0)).round() as u8);
        let at = Point { x: at.x, y: at.y - line.metrics().baseline };
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else { continue };
            self.out.items.extend(self.engine.glyphs(&run, at, color));
        }
    }

    /// Underlines the text an input method is composing.
    fn composing(&mut self, leaf: &Leaf, laid: &Laid, range: &Range<usize>, shifts: &Shifts) {
        let (of, layout) = (self.types.get(leaf.role), &laid.text);
        let (size, color) = ((of.size / 12.0).round().max(1.0), self.inks.ink(Ink::Style(leaf.role)));
        let start = Cursor::from_byte_index(layout, range.start, Affinity::Downstream);
        let end = Cursor::from_byte_index(layout, range.end, Affinity::Upstream);
        Span::new(start, end).geometry_with(layout, |area, line| {
            let Some(baseline) = layout.get(line).map(|line| line.metrics().baseline) else { return };
            let (lead, wider) = shifts.spread(line, area.x0 as f32, area.x1 as f32);
            let y = (leaf.y + baseline + of.size * 0.12).round();
            if y + size <= self.band.0 || y >= self.band.1 {
                return;
            }
            let rect = Rect::new(leaf.x + area.x0 as f32 + lead, y, area.width() as f32 + wider, size);
            self.out.push(Item::Fill { rect, color, radius: 0.0 });
        });
    }

    /// Emits a line's glyphs with each token in its colour. A token can start anywhere inside one of Parley's
    /// glyph runs, so this walks the clusters and starts an item wherever the style or the token changes.
    fn tokens(&mut self, line: &Line<'_, Brush>, at: Point, tokens: &[Token]) {
        let metrics = line.metrics();
        let mut pen = metrics.inline_min_coord + metrics.offset;
        for run in line.runs() {
            let mut open: Option<Stretch> = None;
            let mut close = |open: Option<Stretch>| {
                let Some(Stretch { kind, ink, start, glyphs, .. }) = open else { return };
                let origin = Point { x: at.x + start, y: at.y + metrics.baseline };
                let color = kind.map_or(self.inks.ink(ink), |kind| self.inks.palette.syntax.color(kind));
                let slant = kind == Some(TokenKind::Comment);
                self.out.items.extend(self.engine.item(&run, origin, color, slant, glyphs));
            };
            for cluster in run.visual_clusters() {
                let (style, kind) = (cluster.style_index(), tokens::at(tokens, cluster.text_range().start));
                for glyph in cluster.glyphs() {
                    if open.as_ref().is_none_or(|open| (open.style, open.kind) != (style, kind)) {
                        close(open.take());
                        let ink = cluster.style().brush.ink;
                        open = Some(Stretch { style, kind, ink, start: pen, glyphs: Vec::new() });
                    }
                    if let Some(open) = &mut open {
                        open.glyphs.push(Glyph { id: glyph.id, x: pen - open.start + glyph.x, y: glyph.y });
                    }
                    pen += glyph.advance;
                }
            }
            close(open.take());
        }
    }

    /// Draws a list item's marker in the indent, level with the first line.
    fn marker(&mut self, leaf: &Leaf, laid: &Laid, baseline: f32) {
        let (of, palette) = (self.types.get(leaf.role), &self.inks.palette);
        let right = leaf.x - theme::MARKER_GAP;
        // The middle of a lower-case letter, which is where a bullet or checkbox looks centred.
        let middle = leaf.y + baseline - of.size * 0.34;
        match leaf.marker {
            Marker::None => {}
            Marker::Bullet(depth) => {
                let side = (of.size * 0.36).round().max(3.0);
                let rect = Rect::new(right - side, (middle - side / 2.0).round(), side, side);
                self.out.push(match depth % 3 {
                    0 => Item::Fill { rect, color: palette.marker, radius: side / 2.0 },
                    1 => Item::Stroke { rect, color: palette.marker, width: 1.0, radius: side / 2.0 },
                    _ => Item::Fill { rect, color: palette.marker, radius: 0.0 },
                });
            }
            Marker::Number(_) => {
                let Some(sign) = &laid.sign else { return };
                self.glyphs(sign, Point { x: right - sign.full_width(), y: leaf.y });
            }
            Marker::Task(checked) => {
                let side = theme::CHECKBOX;
                let rect = Rect::new(right - side, (middle - side / 2.0).round(), side, side);
                if !checked {
                    self.out.push(Item::Stroke { rect, color: palette.line_strong, width: 1.5, radius: 3.0 });
                    return;
                }
                self.out.push(Item::Fill { rect, color: palette.accent, radius: 3.0 });
                let Some(sign) = &laid.sign else { return };
                let at = Point {
                    x: rect.x + (side - sign.full_width()) / 2.0,
                    y: rect.y + ((side - sign.height()) / 2.0).round(),
                };
                self.glyphs(sign, at);
            }
        }
    }

    /// Emits a whole small layout with its top left at `at`.
    fn glyphs(&mut self, layout: &Layout<Brush>, at: Point) {
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                self.out.items.extend(self.engine.glyphs(&run, at, self.inks.ink(run.style().brush.ink)));
            }
        }
    }
}

/// The byte offset nearest to a point given from the leaf's top left.
///
/// A [`letters_model::Pos`] has no affinity, and an offset at a wrap draws at the start of the next line. So a
/// point past the end of a wrapped line lands before the space the line wraps at, never after it.
pub(crate) fn hit(laid: &Laid, shifts: &Shifts, x: f32, y: f32) -> usize {
    let x = if shifts.is_empty() { x } else { shifts.pull(line_at(laid, y), x) };
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

/// The line at a height given from the leaf's top, or the nearest one.
fn line_at(laid: &Laid, y: f32) -> usize {
    let line = laid.text.lines().position(|line| y < line.metrics().block_max_coord);
    line.unwrap_or(laid.text.len().saturating_sub(1))
}

/// The line a byte offset is on and where along it, from the layout's left edge. The affinity decides which
/// line an offset at a wrap belongs to.
pub(crate) fn spot(layout: &Layout<Brush>, offset: usize, affinity: Affinity) -> (usize, f32) {
    let at = Cursor::from_byte_index(layout, offset, affinity).geometry(layout, 0.0);
    let middle = ((at.y0 + at.y1) / 2.0) as f32;
    let line = layout.lines().position(|line| middle < line.metrics().block_max_coord);
    (line.unwrap_or(layout.len().saturating_sub(1)), at.x0 as f32)
}

/// The line holding a byte offset.
pub(crate) fn line_of(laid: &Laid, offset: usize) -> usize {
    spot(&laid.text, offset, Affinity::Downstream).0
}

/// The byte offset on a line nearest to `x`, given from the leaf's left edge.
pub(crate) fn on_line(laid: &Laid, shifts: &Shifts, line: usize, x: f32) -> usize {
    let Some(line) = laid.text.get(line) else { return 0 };
    let metrics = line.metrics();
    hit(laid, shifts, x, (metrics.block_min_coord + metrics.block_max_coord) / 2.0)
}

/// Where a caret at a byte offset is drawn, in page coordinates. It spans the line's text, not its leading.
pub(crate) fn caret(leaf: &Leaf, laid: &Laid, shifts: &Shifts, offset: usize) -> Rect {
    let at = Cursor::from_byte_index(&laid.text, offset, Affinity::Downstream).geometry(&laid.text, theme::CARET);
    let (mut top, mut bottom) = (at.y0 as f32, at.y1 as f32);
    let line = line_of(laid, offset);
    if let Some(line) = laid.text.get(line) {
        let metrics = line.metrics();
        (top, bottom) = (metrics.content_block_min_coord, metrics.content_block_max_coord);
    }
    Rect::new(leaf.x + at.x0 as f32 + shifts.caret(line, offset), leaf.y + top, theme::CARET, bottom - top)
}

/// Adds the rectangles that highlight a byte range, in page coordinates. `newline` also shows the block's end
/// as selected, for a selection that runs on into the next block.
pub(crate) fn selection(
    leaf: &Leaf,
    laid: &Laid,
    shifts: &Shifts,
    range: Range<usize>,
    newline: bool,
    out: &mut Vec<Rect>,
) {
    let layout = &laid.text;
    let first = out.len();
    let start = Cursor::from_byte_index(layout, range.start, Affinity::Downstream);
    let end = Cursor::from_byte_index(layout, range.end, Affinity::Upstream);
    Span::new(start, end).geometry_with(layout, |area, line| {
        let (lead, wider) = shifts.spread(line, area.x0 as f32, area.x1 as f32);
        let (x, y) = (leaf.x + area.x0 as f32 + lead, leaf.y + area.y0 as f32);
        out.push(Rect::new(x, y, area.width() as f32 + wider, area.height() as f32));
    });
    if !newline {
        return;
    }
    let Some(line) = layout.lines().next_back() else { return };
    let metrics = line.metrics();
    let end = Cursor::from_byte_index(layout, layout.text_len(), Affinity::Downstream).geometry(layout, 0.0);
    let push = shifts.push(layout.len().saturating_sub(1), end.x0 as f32);
    let (x, y) = (leaf.x + end.x0 as f32 + push, leaf.y + metrics.block_min_coord);
    let width = (metrics.line_height * theme::NEWLINE).round();
    match out[first..].last_mut() {
        Some(last) if last.y == y && (last.x + last.width - x).abs() < 0.5 => last.width += width,
        _ => out.push(Rect::new(x, y, width, metrics.block_max_coord - metrics.block_min_coord)),
    }
}
