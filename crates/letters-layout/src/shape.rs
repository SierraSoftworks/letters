//! Plain strings for widgets: labels, buttons and text fields share the document's fonts and shaping.

use std::cell::RefCell;

use letters_render::{Color, DisplayList, Font, Glyph, Item, Point};
use parley::{Layout, LineHeight, PositionedLayoutItem};

use crate::Fonts;
use crate::engine::{Engine, Spec, Uncovered};
use crate::ink::Brush;

/// The face a plain string is set in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontSpec<'a> {
    /// A family among the fonts the shaper was given. An unknown one falls back to the first of them.
    pub family: &'a str,
    /// In logical pixels.
    pub size: f32,
    pub weight: u16,
    pub italic: bool,
    /// A multiple of the font size.
    pub line_height: f32,
}

impl<'a> FontSpec<'a> {
    /// Regular weight, upright, with a line height of 1.5.
    pub fn new(family: &'a str, size: f32) -> Self {
        Self { family, size, weight: 400, italic: false, line_height: 1.5 }
    }
}

/// Shapes plain strings. Keep one and share it, since it holds the parsed fonts and reuses its scratch space.
pub struct Shaper {
    state: RefCell<(Engine, Layout<Brush>)>,
}

impl Shaper {
    pub fn new(fonts: Fonts) -> Self {
        Self { state: RefCell::new((Engine::new(&fonts), Layout::new())) }
    }

    /// Adds faces to the fonts given at the start, as [`crate::DocLayout::add_fonts`] does. Strings shaped
    /// before are not changed: shape again whatever named a family or an italic that was not there.
    pub fn add_fonts(&self, fonts: &Fonts) -> bool {
        self.state.borrow_mut().0.add(fonts)
    }

    /// Adds faces that only back up the fonts, as [`crate::DocLayout::add_fallback_fonts`] does. Returns
    /// whether a character of a string shaped before has a glyph now: shape again whatever may hold one.
    pub fn add_fallback_fonts(&self, fonts: &Fonts) -> bool {
        self.state.borrow_mut().0.back(fonts)
    }

    /// The characters of the strings shaped since this was last asked that no face has a glyph for, as
    /// [`crate::DocLayout::take_uncovered`] tells them. A plain string has no language.
    pub fn take_uncovered(&self) -> Vec<Uncovered> {
        self.state.borrow_mut().0.take_uncovered()
    }

    /// Shapes `text`. With `max_width` it wraps to that width, and without one only line breaks in the text
    /// start a new line.
    pub fn shape(&self, text: &str, font: &FontSpec, max_width: Option<f32>) -> ShapedText {
        let (engine, layout) = &mut *self.state.borrow_mut();
        let spec = Spec {
            family: font.family,
            mono: false,
            size: font.size,
            weight: f32::from(font.weight),
            italic: font.italic,
            line: LineHeight::FontSizeRelative(font.line_height),
            brush: Brush::default(),
            underline: false,
            strike: false,
        };
        let runs = if text.is_empty() { Vec::new() } else { vec![(0, 0..text.len())] };
        engine.build(text, &[spec], &runs, None, layout);
        layout.break_all_lines(max_width);

        let mut shaped = ShapedText {
            width: layout.full_width(),
            height: layout.height(),
            baseline: layout.first_baseline().unwrap_or(0.0),
            ..ShapedText::default()
        };
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let Some(Item::Glyphs { origin, font, size, skew, embolden, glyphs, .. }) =
                    engine.glyphs(&run, Point::default(), Color::default())
                else {
                    continue;
                };
                shaped.runs.push(ShapedRun { origin, font, size, skew, embolden, glyphs });
            }
        }
        if let Some(line) = layout.get(0) {
            let mut x = line.metrics().offset;
            let mut end = line.text_range().start;
            for cluster in line.runs().flat_map(|run| run.visual_clusters()) {
                let range = cluster.text_range();
                shaped.stops.push((if cluster.is_rtl() { range.end } else { range.start }, x));
                x += cluster.advance();
                end = if cluster.is_rtl() { range.start } else { range.end };
            }
            shaped.stops.push((end, x));
        }
        shaped
    }
}

/// A shaped string, measured from its top left.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShapedText {
    /// The width of the widest line, trailing spaces included.
    pub width: f32,
    pub height: f32,
    /// The first line's baseline.
    pub baseline: f32,
    pub runs: Vec<ShapedRun>,
    /// Where a caret can sit on the first line, left to right: a byte offset and its x.
    pub stops: Vec<(usize, f32)>,
}

/// Glyphs in one font, as [`Item::Glyphs`] takes them.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedRun {
    /// The run's start on its baseline.
    pub origin: Point,
    pub font: Font,
    pub size: f32,
    pub skew: f32,
    pub embolden: bool,
    pub glyphs: Vec<Glyph>,
}

impl ShapedText {
    /// Emits the text with its top left at `at`.
    pub fn paint(&self, at: Point, color: Color, out: &mut DisplayList) {
        for run in &self.runs {
            out.push(Item::Glyphs {
                origin: Point { x: at.x + run.origin.x, y: at.y + run.origin.y },
                font: run.font.clone(),
                size: run.size,
                color,
                skew: run.skew,
                embolden: run.embolden,
                glyphs: run.glyphs.clone(),
                text: None,
            });
        }
    }
}
