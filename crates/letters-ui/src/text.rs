//! Interface text: what a label looks like, and the shaper that measures and positions it.
//!
//! Widgets never touch fonts. They ask a [`TextShaper`] for a [`Shaped`] label and paint its glyph runs, so the
//! window can run on the bundled fonts or, in fast tests, on fixed metrics with no font at all.

use std::sync::{Arc, LazyLock};

use letters_layout::{FontSpec, Fonts, Shaper};
use letters_render::{Color, DisplayList, Font, Glyph, Item, Point, Size};

/// Glyphs in one font, positioned from the text's top left.
pub use letters_layout::ShapedRun as GlyphRun;

use crate::tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    /// Body and control text (Inter).
    Sans,
    /// Titles (Montserrat).
    Display,
    /// Code and shortcuts (Fira Code).
    Mono,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub family: Family,
    /// The font size in logical pixels.
    pub size: f32,
    pub weight: u16,
    pub italic: bool,
    /// The line height as a multiple of the size.
    pub leading: f32,
}

impl TextStyle {
    /// Control text: 14 px Inter on a tight line.
    pub const fn control() -> Self {
        Self { family: Family::Sans, size: tokens::TEXT_MD, weight: 400, italic: false, leading: tokens::LEADING_TIGHT }
    }

    /// Reading text in a panel or popover.
    pub const fn body() -> Self {
        Self { leading: tokens::LEADING, ..Self::control() }
    }

    /// A panel or dialog title.
    pub const fn title() -> Self {
        Self { family: Family::Display, weight: tokens::HEADING_WEIGHT_MD, ..Self::control() }
    }

    pub const fn size(self, size: f32) -> Self {
        Self { size, ..self }
    }

    pub const fn weight(self, weight: u16) -> Self {
        Self { weight, ..self }
    }

    pub const fn family(self, family: Family) -> Self {
        Self { family, ..self }
    }

    pub const fn leading(self, leading: f32) -> Self {
        Self { leading, ..self }
    }

    pub const fn italic(self) -> Self {
        Self { italic: true, ..self }
    }
}

/// A place the caret may rest: a byte offset into the text and its distance from the left edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    pub offset: usize,
    pub x: f32,
}

/// Measured and positioned text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shaped {
    pub size: Size,
    /// The first baseline, from the top.
    pub baseline: f32,
    pub runs: Vec<GlyphRun>,
    /// Caret stops on the first line, ordered by offset, from 0 to the text's length.
    pub stops: Vec<Stop>,
}

impl Shaped {
    /// The caret's distance from the left edge at a byte offset.
    pub fn x_at(&self, offset: usize) -> f32 {
        self.stops.iter().rev().find(|stop| stop.offset <= offset).map_or(0.0, |stop| stop.x)
    }

    /// The caret stop nearest to a distance from the left edge.
    pub fn offset_at(&self, x: f32) -> usize {
        let nearest = self.stops.iter().min_by(|a, b| (a.x - x).abs().total_cmp(&(b.x - x).abs()));
        nearest.map_or(0, |stop| stop.offset)
    }

    /// The stop after `offset`, or `offset` at the end.
    pub fn next(&self, offset: usize) -> usize {
        self.stops.iter().map(|stop| stop.offset).find(|&stop| stop > offset).unwrap_or(offset)
    }

    /// The stop before `offset`, or `offset` at the start.
    pub fn previous(&self, offset: usize) -> usize {
        self.stops.iter().rev().map(|stop| stop.offset).find(|&stop| stop < offset).unwrap_or(offset)
    }

    pub(crate) fn paint(&self, at: Point, color: Color, out: &mut DisplayList) {
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

/// Measures and positions interface text.
pub trait TextShaper {
    /// Shapes `text` on one line, or wrapped to `max_width` when one is given.
    fn shape(&self, text: &str, style: &TextStyle, max_width: Option<f32>) -> Shaped;
}

/// Shapes with the bundled fonts, through the engine that lays out documents.
pub struct FontShaper(Shaper);

impl Default for FontShaper {
    fn default() -> Self {
        Self(Shaper::new(Fonts { faces: letters_fonts::bundled() }))
    }
}

impl FontShaper {
    /// Adds faces that arrived after the shaper was made. Returns whether any was new, when a label shaped
    /// before may have been set without it.
    pub fn add_fonts(&self, fonts: &Fonts) -> bool {
        self.0.add_fonts(fonts)
    }
}

impl TextShaper for FontShaper {
    fn shape(&self, text: &str, style: &TextStyle, max_width: Option<f32>) -> Shaped {
        let family = match style.family {
            Family::Sans => letters_fonts::SANS,
            Family::Display => letters_fonts::DISPLAY,
            Family::Mono => letters_fonts::MONO,
        };
        let font = FontSpec {
            family,
            size: style.size,
            weight: style.weight,
            italic: style.italic,
            line_height: style.leading,
        };
        let shaped = self.0.shape(text, &font, max_width);
        let mut stops: Vec<Stop> = shaped.stops.iter().map(|&(offset, x)| Stop { offset, x }).collect();
        // The engine lists stops left to right, which is not offset order in right-to-left text.
        stops.sort_by_key(|stop| stop.offset);
        Shaped {
            size: Size { width: shaped.width, height: shaped.height },
            baseline: shaped.baseline,
            runs: shaped.runs,
            stops,
        }
    }
}

/// A shaper with no fonts: every character is half the font size wide.
///
/// It keeps widget tests fast and independent of font files. Its glyph ids are character codes, so what it
/// paints is only meaningful to assertions.
#[derive(Clone, Copy, Debug, Default)]
pub struct FixedShaper;

impl FixedShaper {
    /// The advance of one character at a font size.
    pub const fn advance(size: f32) -> f32 {
        size * 0.5
    }
}

static NO_FONT: LazyLock<Font> = LazyLock::new(|| Font::new(Arc::from(Vec::new()), 0));

impl TextShaper for FixedShaper {
    fn shape(&self, text: &str, style: &TextStyle, max_width: Option<f32>) -> Shaped {
        let advance = Self::advance(style.size);
        let line_height = (style.size * style.leading).round();
        let baseline = (line_height - style.size) / 2.0 + style.size * 0.8;
        let fits = max_width.map_or(usize::MAX, |width| ((width / advance).floor() as usize).max(1));

        let mut shaped = Shaped { baseline, ..Shaped::default() };
        let mut lines = 0;
        for line in wrap(text, fits) {
            let glyphs = line.chars().enumerate().map(|(i, c)| Glyph { id: c as u32, x: i as f32 * advance, y: 0.0 });
            let glyphs: Vec<Glyph> = glyphs.collect();
            shaped.size.width = shaped.size.width.max(glyphs.len() as f32 * advance);
            if lines == 0 {
                let ends = line.char_indices().map(|(at, c)| at + c.len_utf8());
                let offsets = std::iter::once(0).chain(ends);
                shaped.stops = offsets.enumerate().map(|(i, offset)| Stop { offset, x: i as f32 * advance }).collect();
            }
            if !glyphs.is_empty() {
                shaped.runs.push(GlyphRun {
                    origin: Point { x: 0.0, y: baseline + lines as f32 * line_height },
                    font: NO_FONT.clone(),
                    size: style.size,
                    skew: 0.0,
                    embolden: false,
                    glyphs,
                });
            }
            lines += 1;
        }
        shaped.size.height = lines.max(1) as f32 * line_height;
        if shaped.stops.is_empty() {
            shaped.stops.push(Stop { offset: 0, x: 0.0 });
        }
        shaped
    }
}

/// Breaks at spaces so no line holds more than `fits` characters, where a word allows it.
fn wrap(text: &str, fits: usize) -> Vec<&str> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let (mut start, mut count, mut last_space) = (0, 0, None);
        for (at, c) in paragraph.char_indices() {
            if count == fits
                && c != ' '
                && let Some(space) = last_space.take()
            {
                lines.push(&paragraph[start..space]);
                start = space + 1;
                count = paragraph[start..at].chars().count();
            }
            if c == ' ' {
                last_space = Some(at);
            }
            count += 1;
        }
        lines.push(&paragraph[start..]);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(text: &str, max_width: Option<f32>) -> Shaped {
        FixedShaper.shape(text, &TextStyle::control().size(10.0).leading(1.0), max_width)
    }

    #[test]
    fn one_line_measures_five_pixels_a_character() {
        let shaped = shape("Share", None);
        assert_eq!(shaped.size, Size { width: 25.0, height: 10.0 });
        assert_eq!(shaped.runs.len(), 1);
    }

    #[test]
    fn caret_stops_follow_characters_not_bytes() {
        let shaped = shape("né!", None);
        let offsets: Vec<usize> = shaped.stops.iter().map(|stop| stop.offset).collect();
        assert_eq!(offsets, [0, 1, 3, 4]);
        assert_eq!(shaped.x_at(3), 10.0);
        assert_eq!(shaped.offset_at(11.0), 3);
        assert_eq!((shaped.next(1), shaped.previous(3)), (3, 1));
        assert_eq!((shaped.next(4), shaped.previous(0)), (4, 0), "the ends hold still");
    }

    #[test]
    fn wrapping_breaks_at_spaces() {
        let shaped = shape("one two three", Some(40.0));
        assert_eq!(shaped.runs.len(), 2, "eight characters fit: 'one two' then 'three'");
        assert_eq!(shaped.size, Size { width: 35.0, height: 20.0 });
    }

    #[test]
    fn empty_text_still_has_a_line_and_a_stop() {
        let shaped = shape("", None);
        assert_eq!(shaped.size.height, 10.0);
        assert_eq!(shaped.stops.len(), 1);
    }
}
