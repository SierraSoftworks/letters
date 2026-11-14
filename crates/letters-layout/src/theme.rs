//! The measures layout adds on top of a document's style. Colours come from the [`crate::Palette`].

use letters_render::Color;

/// Logical pixels per point.
pub(crate) const PT: f32 = 96.0 / 72.0;

/// How far each list level sits in from the one above.
pub(crate) const LIST_INDENT: f32 = 24.0;
/// The gap between a list marker and its text.
pub(crate) const MARKER_GAP: f32 = 8.0;
/// The side of a task item's checkbox.
pub(crate) const CHECKBOX: f32 = 14.0;
/// The width of the bar beside a quote, and of the accent inside a callout.
pub(crate) const BAR: f32 = 3.0;
/// How far a quote's text sits in from its bar's outer edge.
pub(crate) const QUOTE_INSET: f32 = 19.0;
/// The padding inside a code block or callout.
pub(crate) const BOX_PAD_X: f32 = 16.0;
pub(crate) const BOX_PAD_Y: f32 = 12.0;
pub(crate) const BOX_RADIUS: f32 = 6.0;
/// The corner radius of a fill behind inline text.
pub(crate) const INLINE_RADIUS: f32 = 3.0;
/// A block is never squeezed narrower than this by indents.
pub(crate) const MIN_WIDTH: f32 = 48.0;
pub(crate) const RULE: f32 = 1.0;
pub(crate) const CARET: f32 = 1.5;
/// The width that shows a selected paragraph end, as a share of the line height.
pub(crate) const NEWLINE: f32 = 0.25;

/// Inline code is set a little smaller than the text around it.
pub(crate) const INLINE_CODE_SCALE: f32 = 0.9;

/// Reads `#rrggbb`.
pub(crate) fn color(hex: &str) -> Option<Color> {
    let digits = hex.strip_prefix('#').unwrap_or(hex);
    match (digits.len(), u32::from_str_radix(digits, 16)) {
        (6, Ok(rgb)) => Some(Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)),
        _ => None,
    }
}
