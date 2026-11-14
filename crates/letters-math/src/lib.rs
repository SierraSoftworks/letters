//! Maths: a formula's TeX source in, a box and what to draw in it out.
//!
//! The source is the LaTeX that KaTeX accepts. [`typeset`] turns it into a [`Formula`], whose items go into
//! the same display list as everything else, so one renderer draws maths on every backend.
//!
//! A formula holds only glyphs, rules and outlines. Nothing in one can load or link: `\href` sets its text
//! and drops the address, and there is no command for an image.
//!
//! The engine is RaTeX and the faces are KaTeX's, compiled in. Neither shows in this interface.

mod engine;
mod fonts;

use std::fmt;
use std::ops::Range;

use letters_render::{Color, DisplayList, Item, Point, Size};

/// How much larger than the text beside it a formula is set. The maths faces are small on the body, so at
/// the same size a formula looks slight next to prose. This is the ratio KaTeX uses.
pub const TEXT_RATIO: f32 = 1.21;

/// How a formula is set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// The formula's em, in logical pixels. Beside text, that is the text's size times [`TEXT_RATIO`].
    pub size: f32,
    /// The colour of everything the source does not colour itself.
    pub color: Color,
    /// Sets the formula as a block of its own: full-size fractions, and limits above and below their
    /// operators. Otherwise it is set tight, to sit in a line of text.
    pub display: bool,
}

/// A typeset formula. All lengths are logical pixels.
#[derive(Clone, PartialEq)]
pub struct Formula {
    /// The box the formula takes up in its line or block.
    pub size: Size,
    /// How far below the top of the box its baseline lies, to line it up with the text beside it.
    pub baseline: f32,
    /// What to draw, with the top left of the box at the origin. As in TeX, ink may reach a little outside
    /// the box.
    pub items: Vec<Item>,
}

impl fmt::Debug for Formula {
    /// Counts the items instead of listing them: each glyph run would print its whole font.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Formula")
            .field("size", &self.size)
            .field("baseline", &self.baseline)
            .field("items", &self.items.len())
            .finish()
    }
}

impl Formula {
    /// Emits the formula with the top left of its box at `origin`.
    pub fn paint(&self, origin: Point, out: &mut DisplayList) {
        let from = out.items.len();
        out.items.extend(self.items.iter().cloned());
        out.translate(from, origin);
    }
}

/// Why a source could not be typeset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// What is wrong, in the engine's words.
    pub message: String,
    /// The bytes of the source at fault, when the engine can tell. Empty at the end of the source when
    /// something is missing there.
    pub span: Option<Range<usize>>,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// Typesets `source`, which is TeX with no `$` around it.
pub fn typeset(source: &str, style: &Style) -> Result<Formula, Error> {
    engine::typeset(source, style)
}
