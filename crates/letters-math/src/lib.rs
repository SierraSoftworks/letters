//! Maths: a formula's TeX source in, a box and what to draw in it out.
//!
//! The source is the LaTeX that KaTeX accepts. [`typeset`] turns it into a [`Formula`], whose items go into
//! the same display list as everything else, so one renderer draws maths on every backend. Layout keeps a
//! [`Cache`], since a formula is set far more often than it changes. [`commands`] and [`environments`] list
//! what the engine understands, for completion while typing. [`typeset_mapped`] also traces what is drawn
//! to the source, to light the term at the caret. [`Formula::svg`] draws a formula where there is no
//! renderer, as on an exported page. [`mathml`] gives a formula's structure to the formats that hold maths
//! as a tree.
//!
//! A formula holds only glyphs, rules and outlines, so nothing in one can load, and there is no command for
//! an image. `\href` and `\url` are drawn as links only where [`Style::links`] allows it. The address is not
//! kept either way, so a formula cannot yet be clicked through.
//!
//! The engine is RaTeX and the faces are KaTeX's, compiled in. Neither shows in this interface.

mod cache;
mod commands;
mod engine;
mod fonts;
mod mathml;
mod parts;
mod svg;
mod table;

use std::fmt;
use std::ops::Range;

use letters_render::{Color, DisplayList, Item, Point, Size};

pub use crate::cache::Cache;
pub use crate::commands::{Command, commands, environments};
pub use crate::fonts::{FILES as FONT_FILES, provide as provide_fonts};

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
    /// Lets `\href` and `\url` link. Otherwise each is set as its plain argument: the text of `\href`, the
    /// address of `\url`. Only a document that is trusted may link.
    pub links: bool,
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
    /// Which bytes of the source the items are set from, in the order they are drawn. Empty unless the
    /// formula is from [`typeset_mapped`], and for the few formulas that cannot be traced.
    pub parts: Vec<Part>,
}

/// A stretch of a formula's source, and what draws it. One stretch is in several parts when what draws it
/// is not drawn all at once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    /// The bytes of the source it is set from.
    pub source: Range<usize>,
    /// Which of the formula's items draw it.
    pub items: Range<usize>,
}

impl fmt::Debug for Formula {
    /// Counts the items instead of listing them: each glyph run would print its whole font.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Formula")
            .field("size", &self.size)
            .field("baseline", &self.baseline)
            .field("items", &self.items.len())
            .field("parts", &self.parts.len())
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

    /// The term a caret at the byte `caret` of the source is at, as its bytes of the source: the one to
    /// light while it is written. A term the caret is in or just after comes before one it is just before,
    /// and a shorter one before a longer.
    pub fn term(&self, caret: usize) -> Option<Range<usize>> {
        let sources = self.parts.iter().map(|part| &part.source);
        let near = sources.filter(|source| source.start <= caret && caret <= source.end);
        near.min_by_key(|source| (source.start == caret, source.len())).cloned()
    }

    /// The formula with what is set from the bytes `source` of its source in `color`: a term from
    /// [`term`](Self::term), or any stretch, such as a selection. Only a part that is all in the stretch is
    /// coloured.
    pub fn tinted(&self, source: &Range<usize>, color: Color) -> Formula {
        let mut tinted = self.clone();
        let within = |part: &&Part| source.start <= part.source.start && part.source.end <= source.end;
        for part in self.parts.iter().filter(within) {
            for item in tinted.items.get_mut(part.items.clone()).into_iter().flatten() {
                if let Item::Fill { color: ink, .. } | Item::Glyphs { color: ink, .. } | Item::Path { color: ink, .. } =
                    item
                {
                    *ink = color;
                }
            }
        }
        tinted
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

/// Typesets `source` as [`typeset`] does, and traces what it draws to the source in [`Formula::parts`], so
/// that the term at a caret can be lit: see [`Formula::term`] and [`Formula::tinted`]. The formula draws the
/// same and takes about three times as long to set, so this is for the one formula being written, each time
/// its source changes.
///
/// A part is a symbol, an operator such as `\sum`, what a macro of the engine's draws, such as `\ne`, or
/// what is set as one whole: a run in a font, a line of text. What a macro defined in the source draws is
/// traced to that definition, where it is written. What a command draws of its own is in no part: the bar
/// of a fraction, a root sign, an accent, a bracket that `\left` sizes.
pub fn typeset_mapped(source: &str, style: &Style) -> Result<Formula, Error> {
    engine::typeset_mapped(source, style)
}

/// Writes `source` as presentation MathML, for the formats and tools that take a formula as its structure
/// and not as a drawing. The result is one `math` element, set as a block of its own when `display` is set.
///
/// Fails as [`typeset`] does on a source that does not parse, and on the few things MathML has no element
/// for: a commutative diagram and a proof tree.
pub fn mathml(source: &str, display: bool) -> Result<String, Error> {
    mathml::write(source, display)
}
