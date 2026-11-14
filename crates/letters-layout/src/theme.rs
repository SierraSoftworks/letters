//! The measures and colours layout adds on top of a document's style. They follow yewi's light theme, and a
//! screen backend recolours them for dark mode along with the style's own colours.

use letters_model::Tone;
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

pub(crate) const TEXT: Color = Color::rgb(0x1f, 0x23, 0x28);
pub(crate) const MUTED: Color = Color::rgb(0x5e, 0x5e, 0x62);
pub(crate) const LINE: Color = Color::rgb(0xe3, 0xe4, 0xe8);
pub(crate) const LINE_STRONG: Color = Color::rgb(0xca, 0xcc, 0xd1);
pub(crate) const LINK: Color = Color::rgb(0x1a, 0x63, 0xc6);
pub(crate) const ACCENT: Color = Color::rgb(0x1f, 0x75, 0xd9);
pub(crate) const ON_ACCENT: Color = Color::rgb(0xff, 0xff, 0xff);
// Translucent, so they also read on a callout's tint.
pub(crate) const CODE_BLOCK: Color = Color::rgba(0x15, 0x17, 0x1a, 10);
pub(crate) const CODE_INLINE: Color = Color::rgba(0x15, 0x17, 0x1a, 16);
pub(crate) const HIGHLIGHT: Color = Color::rgba(0xff, 0xd6, 0x0a, 96);

/// A callout's accent and the soft fill behind it.
pub(crate) fn tone(tone: Tone) -> (Color, Color) {
    let (r, g, b, soft) = match tone {
        Tone::Tip => (0x14, 0x6f, 0x43, 31),
        Tone::Info => (0x2a, 0x63, 0xb5, 31),
        Tone::Warn => (0x85, 0x59, 0x00, 33),
        Tone::Danger => (0xb8, 0x2c, 0x24, 31),
    };
    (Color::rgb(r, g, b), Color::rgba(r, g, b, soft))
}

/// Reads `#rrggbb`, falling back to the default text colour.
pub(crate) fn color(hex: &str) -> Color {
    let digits = hex.strip_prefix('#').unwrap_or(hex);
    match (digits.len(), u32::from_str_radix(digits, 16)) {
        (6, Ok(rgb)) => Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8),
        _ => TEXT,
    }
}
