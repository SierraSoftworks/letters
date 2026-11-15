//! The line under a picture: where a caption is offered, and the switch at its end to the picture's alt text.
//!
//! The page is drawn on a canvas, so the hint and the switch are drawn there too and the window asks this
//! module what a press is on. With the switch on, the line is a field on a sheet over the page: alt text is
//! never shown on the page itself.

use letters_model::{Align as TextAlign, BlockId};
use letters_render::{DisplayList, Item, Point, Rect};

use crate::color::resolve;
use crate::element::Element;
use crate::style::{Align, Edges};
use crate::text::{TextShaper, TextStyle};
use crate::theme::accent;
use crate::tokens::{self, Scheme};
use crate::widgets::{Button, ControlSize, Popover, TextField};
use crate::window::Message;

/// What the hint under a picture without a caption says.
const HINT: &str = "Add a caption";

/// What the switch says, and what the line is called while it is on.
const SWITCH: &str = "Alt";
pub(crate) const LABEL: &str = "Alt text";

/// How tall the switch is and how far its word keeps from its sides: yewi's small button, made to fit the
/// line of a caption.
const HEIGHT: f32 = 24.0;
const PADDING: f32 = tokens::SPACE_SM;

/// How far round the switch a press still counts as on it.
const REACH: f32 = 4.0;

/// How far the sheet of the alt text reaches beyond the line it covers, across and down.
const SHEET: (f32, f32) = (tokens::SPACE_SM, tokens::SPACE_XS);

/// What the field of a picture's alt text asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Altering {
    /// Turns the line under the caret's picture to its alt text, as a press on the switch does.
    Open,
    /// What is typed into the field, before it is taken.
    Draft(String),
    /// Takes what was typed as the picture's alt text, and gives the line back to the caption.
    Apply,
    /// Drops what was typed.
    Drop,
}

/// The line under a picture, as the pointer and the switch have it.
#[derive(Default)]
pub(crate) struct Captions {
    /// The picture whose alt text the line shows, and the text as typed so far.
    pub alt: Option<(BlockId, String)>,
    /// Whether the pointer rests on the switch.
    pub over: bool,
    /// The block the selection was in when the layout was last told whose caption is being written.
    pub named: Option<BlockId>,
}

/// Where the hint that a caption can be written stands on `line`: where a caption's first letter would,
/// in the size and alignment the document gives captions. `first` is the caret of the empty caption, where
/// there is one to ask for, and `inset` is how far a caption being written keeps from the end of its
/// line, where the switch stands.
pub(crate) fn hint_at(
    line: Rect,
    first: Option<Rect>,
    inset: f32,
    size: f32,
    align: TextAlign,
    shaper: &dyn TextShaper,
) -> Rect {
    let shaped = shaper.shape(HINT, &TextStyle::control().size(size), None).size;
    let inset = inset.min(((line.width - shaped.width) / 2.0).max(0.0));
    // How much of the hint lies before the place the caption is set from, and that place on a bare line.
    let (before, from) = match align {
        TextAlign::Start | TextAlign::Justify => (0.0, line.x),
        TextAlign::Center => (0.5, line.x + line.width / 2.0),
        TextAlign::End => (1.0, line.x + line.width - inset),
    };
    let x = (first.map_or(from, |caret| caret.x) - before * shaped.width).round();
    let (top, tall) = first.map_or((line.y, line.height), |caret| (caret.y, caret.height));
    Rect::new(x, top + ((tall - shaped.height) / 2.0).round(), shaped.width, shaped.height)
}

/// Draws the hint at the place [`hint_at`] gave it.
pub(crate) fn hint(at: Rect, size: f32, shaper: &dyn TextShaper, scheme: Scheme, out: &mut DisplayList) {
    let shaped = shaper.shape(HINT, &TextStyle::control().size(size), None);
    shaped.paint(Point { x: at.x, y: at.y }, resolve(tokens::FG_SUBTLE, scheme, 1.0), out);
}

/// The style the switch says its word in.
fn word() -> TextStyle {
    TextStyle::control().size(tokens::TEXT_SM).weight(500)
}

/// Where the switch stands: at the end of `line`, level with the text of its first line, which is `first`
/// tall from the top it gives.
pub(crate) fn switch_at(line: Rect, first: (f32, f32), shaper: &dyn TextShaper) -> Rect {
    // Its word, the room round it and its border, as a button measures.
    let wide = (shaper.shape(SWITCH, &word(), None).size.width + 2.0 * (PADDING + 1.0)).round();
    let top = (first.0 + (first.1 - HEIGHT) / 2.0).round();
    Rect::new(line.x + line.width - wide, top, wide, HEIGHT)
}

/// Whether a press at `at` is on the switch at `switch`: on it, or as near as a finger comes.
pub(crate) fn pressed(switch: Rect, at: Point) -> bool {
    Rect::new(switch.x - REACH, switch.y - REACH, switch.width + 2.0 * REACH, switch.height + 2.0 * REACH).contains(at)
}

/// Draws the switch while it is off, as yewi's button: `over` is whether the pointer rests on it.
pub(crate) fn switch(at: Rect, over: bool, shaper: &dyn TextShaper, scheme: Scheme, out: &mut DisplayList) {
    let radius = tokens::RADIUS_CONTROL;
    let ground = if over { tokens::TONE_1 } else { tokens::SURFACE };
    out.push(Item::Fill { rect: at, color: resolve(ground, scheme, 1.0), radius });
    out.push(Item::Stroke { rect: at, color: resolve(tokens::LINE_STRONG, scheme, 1.0), width: 1.0, radius });
    let shaped = shaper.shape(SWITCH, &word(), None);
    let origin = Point {
        x: at.x + ((at.width - shaped.size.width) / 2.0).round(),
        y: at.y + (at.height - shaped.size.height) / 2.0,
    };
    shaped.paint(origin, resolve(tokens::FG, scheme, 1.0), out);
}

/// The line switched to a picture's alt text: a field on a sheet that covers `line`, with the switch
/// pressed where it stood at `switch`. `text` is the alt text as typed so far.
pub(crate) fn sheet(text: &str, line: Rect, switch: Rect) -> Popover<Message> {
    let tag = Element::text(LABEL, TextStyle::control().size(tokens::TEXT_XS).weight(600)).fg(accent().text);
    let field = TextField::new(LABEL, text)
        .size(ControlSize::Small)
        .placeholder("What the picture shows, for people who cannot see it")
        .on_input(|text| Message::Alt(Altering::Draft(text)))
        .on_submit(Message::Alt(Altering::Apply))
        .on_cancel(Message::Alt(Altering::Drop));
    let back = Button::new(SWITCH).small().pressed(true).on_press(Message::Alt(Altering::Apply));
    let row = Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_SM)
        .width(line.width)
        .height(HEIGHT)
        .child(tag)
        .child(Element::from(field).height(HEIGHT).grow(1.0))
        .child(Element::from(back).height(HEIGHT).padding(Edges::xy(PADDING, 0.0)));
    // The sheet covers the caption, however many lines it has.
    let tall = (line.y + line.height - switch.y).max(HEIGHT);
    let over = Element::column().padding(Edges::xy(SHEET.0, SHEET.1)).height(tall + 2.0 * SHEET.1).child(row);
    let from = Rect::new(line.x - SHEET.0, switch.y - SHEET.1, line.width + 2.0 * SHEET.0, 0.0);
    Popover::new(over)
        .label(LABEL)
        .flush()
        .flat()
        .max_width(f32::INFINITY)
        .gap(0.0)
        .at(from)
        .open(true)
        .on_close(Message::Alt(Altering::Apply))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::FixedShaper;

    #[test]
    fn the_hint_stands_where_a_caption_would_start() {
        let line = Rect::new(40.0, 300.0, 400.0, 20.0);
        let wide = hint_at(line, None, 0.0, 12.0, TextAlign::Start, &FixedShaper).width;
        // How captions are aligned, and how far in from the left of a bare line the hint starts.
        let cases = [
            (TextAlign::Start, 0.0),
            (TextAlign::Justify, 0.0),
            (TextAlign::Center, ((400.0 - wide) / 2.0).round()),
            (TextAlign::End, 400.0 - 44.0 - wide),
        ];
        for (align, left) in cases {
            let at = hint_at(line, None, 44.0, 12.0, align, &FixedShaper);
            assert_eq!(at.x - line.x, left, "{align:?}");
            assert!(at.y >= line.y && at.y + at.height <= line.y + line.height, "{align:?}: {at:?}");
        }
    }

    #[test]
    fn the_hint_follows_the_caret_of_an_empty_caption() {
        let line = Rect::new(40.0, 300.0, 400.0, 20.0);
        let caret = Rect::new(200.0, 340.0, 1.0, 16.0);
        let wide = hint_at(line, None, 0.0, 12.0, TextAlign::Start, &FixedShaper).width;
        // How captions are aligned, and where the hint starts against a caret at 200.
        let cases = [
            (TextAlign::Start, 200.0),
            (TextAlign::Center, (200.0 - wide / 2.0).round()),
            (TextAlign::End, 200.0 - wide),
        ];
        for (align, left) in cases {
            let at = hint_at(line, Some(caret), 44.0, 12.0, align, &FixedShaper);
            assert_eq!(at.x, left, "{align:?}");
            assert!((at.y + at.height / 2.0 - 348.0).abs() <= 1.0, "level with the caret: {at:?}");
        }
    }

    #[test]
    fn the_switch_stands_at_the_end_of_the_line_level_with_its_first_line_of_text() {
        let line = Rect::new(40.0, 300.0, 400.0, 60.0);
        let at = switch_at(line, (300.0, 20.0), &FixedShaper);
        assert_eq!(at.x + at.width, line.x + line.width);
        assert_eq!(at.y + at.height / 2.0, 310.0, "{at:?}");
    }
}
