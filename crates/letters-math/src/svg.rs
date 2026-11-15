//! A formula as SVG, for a page that no renderer of ours draws, such as an exported one.

use std::fmt::Write;

use letters_render::{Color, Font, Glyph, Item, PathEl, Point, Rect};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::types::F2Dot14;
use skrifa::{FontRef, GlyphId, MetadataProvider};

use crate::Formula;

impl Formula {
    /// The formula's drawing as SVG: what goes inside an `svg` element, in logical pixels with the top left
    /// of the box at the origin. The caller writes that element, sized from [`size`](Self::size).
    ///
    /// Glyphs are written as outlines, so the page needs no font. There are only `path` and `rect`
    /// elements, with nothing in them that could load, link or run. Whatever is coloured `ink` is written
    /// as `currentColor`, so that it follows the text of the page around it.
    pub fn svg(&self, ink: Color) -> String {
        let mut svg = Svg { text: String::new(), places: places(&self.items), ink };
        for item in &self.items {
            match item {
                Item::Fill { rect, color, radius } => svg.rect(rect, *color, *radius),
                Item::Glyphs { origin, font, size, color, glyphs, .. } => {
                    svg.glyphs(*origin, font, *size, *color, glyphs)
                }
                Item::Path { origin, scale, path, color, stroke } => svg.path(*origin, *scale, path, *color, *stroke),
                // A formula holds nothing else.
                _ => {}
            }
        }
        svg.text
    }
}

/// How many decimal places lengths are written to: enough to tell apart the units the largest glyphs are
/// designed in, a thousand to the em.
fn places(items: &[Item]) -> usize {
    let scale = |item: &Item| match item {
        Item::Glyphs { size, .. } => *size,
        Item::Path { scale, .. } => *scale,
        _ => 0.0,
    };
    let em = items.iter().map(scale).filter(|scale| scale.is_finite()).fold(0.0, f32::max);
    if em > 0.0 { (1000.0 / em).log10().ceil().clamp(0.0, 6.0) as usize } else { 2 }
}

struct Svg {
    text: String,
    places: usize,
    ink: Color,
}

impl Svg {
    fn rect(&mut self, rect: &Rect, color: Color, radius: f32) {
        // Written so that a side that is not a number is dropped too.
        let visible = rect.width > 0.0 && rect.height > 0.0;
        if !visible {
            return;
        }
        self.text.push_str("<rect");
        for (name, length) in [("x", rect.x), ("y", rect.y), ("width", rect.width), ("height", rect.height)] {
            self.attribute(name, length);
        }
        if radius > 0.0 {
            self.attribute("rx", radius);
        }
        self.paint("fill", color);
        self.text.push_str("/>");
    }

    /// A run of glyphs as one path of their outlines. Fonts wind their outlines for the non-zero rule,
    /// which is the one SVG fills by.
    fn glyphs(&mut self, origin: Point, font: &Font, size: f32, color: Color, glyphs: &[Glyph]) {
        // As on screen, a run with no size is not drawn. Written so that a size that is not a number is
        // dropped too.
        let sized = size > 0.0;
        let Some(face) = FontRef::from_index(&font.data, font.index).ok().filter(|_| sized) else { return };
        let coords: Vec<F2Dot14> = font.coords.iter().map(|bits| F2Dot14::from_bits(*bits)).collect();
        let location = LocationRef::new(&coords);
        // Outlines are read in the font's own units and scaled here: scaled by the font, a small size
        // would lose their detail.
        let scale = size / f32::from(face.metrics(Size::unscaled(), location).units_per_em);
        let outlines = face.outline_glyphs();
        let mut shape = Shape { text: String::new(), places: self.places, origin, scale };
        for glyph in glyphs {
            let Some(outline) = outlines.get(GlyphId::new(glyph.id)) else { continue };
            shape.origin = Point { x: origin.x + glyph.x, y: origin.y + glyph.y };
            // A glyph the font cannot draw is skipped, and the rest of the run still shows.
            let _ = outline.draw(DrawSettings::unhinted(Size::unscaled(), location), &mut shape);
        }
        if !shape.text.is_empty() {
            self.text.push_str("<path");
            self.paint("fill", color);
            let _ = write!(self.text, " d=\"{}\"/>", shape.text);
        }
    }

    fn path(&mut self, origin: Point, scale: f32, path: &[PathEl], color: Color, stroke: Option<f32>) {
        let mut shape = Shape { text: String::new(), places: self.places, origin, scale };
        for element in path {
            match *element {
                PathEl::MoveTo(to) => shape.command('M', &[to]),
                PathEl::LineTo(to) => shape.command('L', &[to]),
                PathEl::QuadTo(control, to) => shape.command('Q', &[control, to]),
                PathEl::CurveTo(first, second, to) => shape.command('C', &[first, second, to]),
                PathEl::Close => shape.command('Z', &[]),
            }
        }
        let sized = scale > 0.0;
        if shape.text.is_empty() || !sized {
            return;
        }
        self.text.push_str("<path");
        match stroke {
            // The renderer fills a path by the even-odd rule, which only shows where it has several outlines.
            None => {
                self.paint("fill", color);
                if path.iter().filter(|element| matches!(element, PathEl::MoveTo(_))).count() > 1 {
                    self.text.push_str(" fill-rule=\"evenodd\"");
                }
            }
            Some(width) => {
                self.text.push_str(" fill=\"none\"");
                self.paint("stroke", color);
                self.attribute("stroke-width", width * scale);
                self.text.push_str(" stroke-linecap=\"round\" stroke-linejoin=\"round\"");
            }
        }
        let _ = write!(self.text, " d=\"{}\"/>", shape.text);
    }

    fn attribute(&mut self, name: &str, length: f32) {
        let _ = write!(self.text, " {name}=\"");
        number(&mut self.text, length, self.places);
        self.text.push('"');
    }

    /// Sets a `fill` or a `stroke` to a colour.
    fn paint(&mut self, name: &str, color: Color) {
        if color == self.ink {
            let _ = write!(self.text, " {name}=\"currentColor\"");
            return;
        }
        let _ = write!(self.text, " {name}=\"#{:02x}{:02x}{:02x}\"", color.r, color.g, color.b);
        if color.a < u8::MAX {
            // Not as `#rrggbbaa`, which fewer readers of SVG know.
            let _ = write!(self.text, " {name}-opacity=\"");
            number(&mut self.text, f32::from(color.a) / 255.0, 3);
            self.text.push('"');
        }
    }
}

/// The data of a path. Points arrive in a shape's own units, with y up for a glyph and down for a path, and
/// are written in the formula's.
struct Shape {
    text: String,
    places: usize,
    origin: Point,
    scale: f32,
}

impl Shape {
    fn command(&mut self, letter: char, points: &[Point]) {
        self.text.push(letter);
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                self.text.push(' ');
            }
            number(&mut self.text, self.origin.x + point.x * self.scale, self.places);
            self.text.push(' ');
            number(&mut self.text, self.origin.y + point.y * self.scale, self.places);
        }
    }

    /// A glyph's point, which has y up from the baseline.
    fn up(x: f32, y: f32) -> Point {
        Point { x, y: -y }
    }
}

impl OutlinePen for Shape {
    fn move_to(&mut self, x: f32, y: f32) {
        self.command('M', &[Self::up(x, y)]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.command('L', &[Self::up(x, y)]);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.command('Q', &[Self::up(cx0, cy0), Self::up(x, y)]);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.command('C', &[Self::up(cx0, cy0), Self::up(cx1, cy1), Self::up(x, y)]);
    }

    fn close(&mut self) {
        self.command('Z', &[]);
    }
}

/// Writes a number to at most `places` decimal places, as short as it goes. One that is not a number is
/// written as 0, so that what is written is always well formed.
fn number(text: &mut String, value: f32, places: usize) {
    let start = text.len();
    let _ = write!(text, "{:.places$}", if value.is_finite() { value } else { 0.0 });
    if text[start..].contains('.') {
        text.truncate(text.trim_end_matches('0').trim_end_matches('.').len());
    }
    if &text[start..] == "-0" {
        text.replace_range(start.., "0");
    }
}
