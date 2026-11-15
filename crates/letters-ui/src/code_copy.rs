//! CodeCopy a code block: the button in the corner of its box.
//!
//! It is yewi's copy button, as its code block has one: a small subtle button with the copy icon, which
//! shows a check mark for two seconds once the code is on the clipboard. The page is drawn on a canvas, so
//! the button is drawn there too and the window asks this module what a press is on.
//!
//! A code block has no header to keep the button clear of its text. Where the first line runs as far as
//! the corner, the button shows only while the pointer is over the block, and covers the end of that line.

use letters_model::BlockId;
use letters_render::{Color, DisplayList, Item, Point, Rect};

use crate::color::resolve;
use crate::icon::{self, GRID, Icon};
use crate::tokens::{self, Scheme};

/// What the button is called.
pub(crate) const LABEL: &str = "Copy code";

/// The side of the button, and of the icon in it: those of yewi's small icon button.
const SIDE: f32 = tokens::CONTROL_HEIGHT_SM;
const ICON: f32 = 14.0;
/// How far the button keeps from the top and right of the box.
const INSET: f32 = 6.0;
/// The room text keeps from the button for the two not to meet.
pub(crate) const CLEAR: f32 = 8.0;
/// How long the check mark shows, in milliseconds: as long as yewi's copy button shows it.
const SHOWN_MS: u64 = 2000;

/// Where the button is in the box of a code block.
pub(crate) fn button(code: Rect) -> Rect {
    Rect::new(code.x + code.width - INSET - SIDE, code.y + INSET, SIDE, SIDE)
}

/// What the pointer is doing with the copy buttons on the page.
#[derive(Default)]
pub(crate) struct CodeCopy {
    /// The block whose box the pointer rests on, and whether it rests on the button there.
    pointed: Option<(BlockId, bool)>,
    /// The block whose button the pointer went down on.
    pressed: Option<BlockId>,
    /// The block that was just copied, and when.
    copied: Option<(BlockId, u64)>,
}

impl CodeCopy {
    /// Takes the block whose box the pointer rests on now, and whether it is on the button. Returns
    /// whether either is news.
    pub fn point(&mut self, pointed: Option<(BlockId, bool)>) -> bool {
        let news = pointed != self.pointed;
        self.pointed = pointed;
        news
    }

    /// Whether the pointer rests on a block's box.
    pub fn over(&self, block: &BlockId) -> bool {
        self.pointed.as_ref().is_some_and(|(pointed, _)| pointed == block)
    }

    /// Whether the pointer rests on a button.
    pub fn pointed(&self) -> bool {
        matches!(self.pointed, Some((_, true)))
    }

    /// Notes the block whose button the pointer went down on. `None` for a press anywhere else.
    pub fn press(&mut self, block: Option<BlockId>) {
        self.pressed = block;
    }

    /// Whether a press that began on a button is still held, so that what the pointer does is the button's.
    pub fn held(&self) -> bool {
        self.pressed.is_some()
    }

    /// The block to copy, when the pointer comes up on the button it went down on.
    pub fn release(&mut self, block: Option<&BlockId>) -> Option<BlockId> {
        self.pressed.take().filter(|pressed| block == Some(pressed))
    }

    /// Shows the check mark on a block's button from `now_ms`.
    pub fn copied(&mut self, block: BlockId, now_ms: u64) {
        self.copied = Some((block, now_ms));
    }

    /// Whether a block's button shows that it has copied.
    pub fn shows_copied(&self, block: &BlockId) -> bool {
        self.copied.as_ref().is_some_and(|(copied, _)| copied == block)
    }

    /// Takes the check mark away when its time is up. Returns when to call again, or `None` while no
    /// button shows one.
    pub fn animate(&mut self, now_ms: u64) -> Option<u64> {
        let until = self.copied.as_ref()?.1 + SHOWN_MS;
        if now_ms >= until {
            self.copied = None;
        }
        self.copied.is_some().then_some(until)
    }

    /// Draws a block's button at `at`. `behind` is what the box shows under it, bottom first, for a block
    /// whose first line reaches the button: the end of that line is covered while the button shows, which
    /// is then only while the pointer is over the block or the check mark is up.
    pub fn paint(&self, block: &BlockId, at: Rect, behind: Option<[Color; 2]>, scheme: Scheme, out: &mut DisplayList) {
        let over = self.pointed.as_ref().filter(|(pointed, _)| pointed == block).map(|(_, on)| *on);
        let down = self.pressed.as_ref() == Some(block);
        let copied = self.shows_copied(block);
        if behind.is_some() && over.is_none() && !down && !copied {
            return;
        }
        let radius = tokens::RADIUS_CONTROL;
        out.items.extend(behind.into_iter().flatten().map(|color| Item::Fill { rect: at, color, radius }));
        let on = over == Some(true) || down;
        if on {
            // The fill of a subtle button under the pointer, and a little more of it while it is held.
            let share = if down { 1.5 } else { 1.0 };
            out.push(Item::Fill { rect: at, color: resolve(tokens::NEUTRAL_SOFT, scheme, share), radius });
        }
        let (shown, ink): (Icon, _) = match (copied, on) {
            (true, _) => (icon::CHECK, tokens::OK),
            (false, true) => (icon::COPY, tokens::FG),
            (false, false) => (icon::COPY, tokens::FG_MUTED),
        };
        out.push(Item::Path {
            origin: Point { x: at.x + (at.width - ICON) / 2.0, y: at.y + (at.height - ICON) / 2.0 },
            scale: ICON / GRID,
            path: shown.outline(),
            color: resolve(ink, scheme, 1.0),
            stroke: (!shown.is_filled()).then_some(tokens::ICON_STROKE),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_copies_when_it_ends_on_the_button_it_began_on() {
        let (a, b) = (BlockId::new("a"), BlockId::new("b"));
        // (pressed on, released on, copied)
        let cases = [
            (Some(&a), Some(&a), Some(&a)),
            (Some(&a), Some(&b), None),
            (Some(&a), None, None),
            (None, Some(&a), None),
        ];
        for (down, up, copied) in cases {
            let mut copy = CodeCopy::default();
            copy.press(down.cloned());
            assert_eq!(copy.held(), down.is_some());
            assert_eq!(copy.release(up).as_ref(), copied, "{down:?} then {up:?}");
            assert!(!copy.held(), "the press is over");
        }
    }

    #[test]
    fn the_check_mark_shows_for_two_seconds() {
        let mut copy = CodeCopy::default();
        let block = BlockId::new("a");
        assert_eq!(copy.animate(0), None, "nothing moves until something is copied");
        copy.copied(block.clone(), 5000);
        assert_eq!(copy.animate(5001), Some(5000 + SHOWN_MS));
        assert!(copy.shows_copied(&block) && !copy.shows_copied(&BlockId::new("b")));
        assert_eq!(copy.animate(5000 + SHOWN_MS - 1), Some(5000 + SHOWN_MS));
        assert_eq!(copy.animate(5000 + SHOWN_MS), None);
        assert!(!copy.shows_copied(&block));
    }

    #[test]
    fn the_button_sits_inside_the_top_right_corner_of_the_box() {
        let code = Rect::new(40.0, 100.0, 600.0, 80.0);
        let at = button(code);
        assert!(at.x > code.x + code.width / 2.0 && at.x + at.width < code.x + code.width, "{at:?}");
        assert!(at.y > code.y && at.y + at.height < code.y + code.height / 2.0 + at.height, "{at:?}");
    }
}
