//! Fixtures shared by the maths tests.
#![allow(dead_code, reason = "each test file uses its own part")]

use std::path::PathBuf;

use letters_math::{Formula, Style, typeset};
use letters_render::cpu::{CpuRenderer, Frame};
use letters_render::image::Images;
use letters_render::snapshot::{self, Shot};
use letters_render::{Color, DisplayList, Point};

pub const WHITE: Color = Color::rgb(255, 255, 255);
pub const INK: Color = Color::rgb(24, 28, 40);

/// Room left around a formula when it is drawn, in logical pixels.
pub const MARGIN: f32 = 16.0;

/// The inline style at a size, as a document that is not trusted sets it.
pub fn inline(size: f32) -> Style {
    Style { size, color: INK, display: false, links: false }
}

/// The display style at a size.
pub fn display(size: f32) -> Style {
    Style { display: true, ..inline(size) }
}

/// Typesets a source that is known to be good.
pub fn set(source: &str, style: &Style) -> Formula {
    typeset(source, style).unwrap_or_else(|error| panic!("{source}: {error}"))
}

/// Formulas one under the other with [`MARGIN`] around each, and the logical size they take up.
pub fn stack(formulas: &[Formula]) -> (DisplayList, f32, f32) {
    let mut list = DisplayList::default();
    let (mut width, mut y) = (0.0_f32, MARGIN);
    for formula in formulas {
        formula.paint(Point { x: MARGIN, y }, &mut list);
        width = width.max(formula.size.width);
        y += formula.size.height + MARGIN;
    }
    (list, width + MARGIN * 2.0, y)
}

/// Draws stacked formulas over white.
pub fn render(formulas: &[Formula], scale: f32) -> Frame {
    let (list, width, height) = stack(formulas);
    let mut frame = Frame::new(pixels(width, scale), pixels(height, scale));
    CpuRenderer::new().render(&list, &Images::default(), scale, WHITE, &mut frame);
    frame
}

/// Checks stacked formulas against `tests/snapshots/<name>.png`. Run with `LETTERS_BLESS=1` to accept what
/// is drawn as the new snapshot.
pub fn assert_snapshot(name: &str, formulas: &[Formula], scale: f32) {
    let (list, width, height) = stack(formulas);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots").join(format!("{name}.png"));
    let size = (pixels(width, scale), pixels(height, scale));
    snapshot::assert(&path, &Shot { list: &list, images: &Images::default(), size, scale, background: WHITE });
}

fn pixels(length: f32, scale: f32) -> u32 {
    (length * scale).ceil() as u32
}

/// The smallest rectangle of physical pixels holding everything that is not white, as its left, top, right
/// and bottom edges. `None` for a blank frame.
pub fn ink_bounds(frame: &Frame) -> Option<[u32; 4]> {
    let pixels = (0..frame.height()).flat_map(|y| (0..frame.width()).map(move |x| (x, y)));
    let inked = pixels.filter(|(x, y)| frame.pixel(*x, *y).is_some_and(|pixel| pixel != WHITE));
    inked.fold(None, |bounds, (x, y)| {
        let [left, top, right, bottom] = bounds.unwrap_or([x, y, x + 1, y + 1]);
        Some([left.min(x), top.min(y), right.max(x + 1), bottom.max(y + 1)])
    })
}

/// The colour of the darkest pixel: what a formula's solid ink is drawn in.
pub fn darkest(frame: &Frame) -> Color {
    let pixels = (0..frame.height()).flat_map(|y| (0..frame.width()).map(move |x| (x, y)));
    let weight = |color: &Color| u32::from(color.r) + u32::from(color.g) + u32::from(color.b);
    pixels.filter_map(|(x, y)| frame.pixel(x, y)).min_by_key(weight).unwrap_or(WHITE)
}
