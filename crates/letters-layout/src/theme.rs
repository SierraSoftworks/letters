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
/// Half the length of one wave of the line under a misspelled word, and how far it swings either way.
pub(crate) const WAVE: f32 = 2.0;
pub(crate) const WAVE_SWING: f32 = 1.0;

/// The gap between a picture and the caption under it.
pub(crate) const CAPTION_GAP: f32 = 8.0;
/// How far in from the right a caption is set while [`crate::DocLayout::set_caption_open`] names its figure:
/// room at the end of its lines for what a window stands there. A centred caption is as far in from the left.
pub const CAPTION_INSET: f32 = 44.0;
/// How many lines of caption still fit a page under a picture that is as tall as layout lets it be.
pub(crate) const CAPTION_LINES: f32 = 3.0;
/// The gap between the pictures of a gallery.
pub(crate) const TILE_GAP: f32 = 8.0;
/// The side of the cloud on a picture that is still a link, and how far it sits in from the corner.
pub(crate) const BADGE: f32 = 16.0;
pub(crate) const BADGE_INSET: f32 = 8.0;
/// The padding inside a table cell.
pub(crate) const CELL_PAD_X: f32 = 10.0;
pub(crate) const CELL_PAD_Y: f32 = 6.0;
/// A display formula too wide for the column is set smaller, down to this share of its size.
pub(crate) const FORMULA_SHRINK: f32 = 0.4;

/// The gap between the lines of a table of contents, and between the texts of a link preview.
pub(crate) const ENTRY_GAP: f32 = 3.0;
/// The room at the end of the lines of a table of contents that a sheet sets a page number in, in ems of
/// their type: three digits and a gap.
pub(crate) const PAGE_NUMBER: f32 = 2.6;
/// The slot a link preview shows its picture in, whatever the picture's size.
pub(crate) const PREVIEW_PICTURE: (f32, f32) = (120.0, 80.0);
/// The gap between that slot and the text beside it.
pub(crate) const PREVIEW_GAP: f32 = 16.0;
/// The picture shows only where it leaves the text this much room.
pub(crate) const PREVIEW_TEXT: f32 = 160.0;
/// The room at the end of a link preview that has only its address, for what a window stands there.
pub(crate) const PREVIEW_CONTROL: f32 = 128.0;

/// A callout's revealed tag is set small enough to sit in the padding above its text.
pub(crate) const TAG_SCALE: f32 = 0.7;

/// A callout's title is at least this heavy.
pub(crate) const TITLE_WEIGHT: f32 = 600.0;

/// Reads `#rrggbb`.
pub(crate) fn color(hex: &str) -> Option<Color> {
    let digits = hex.strip_prefix('#').unwrap_or(hex);
    match (digits.len(), u32::from_str_radix(digits, 16)) {
        (6, Ok(rgb)) => Some(Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)),
        _ => None,
    }
}
