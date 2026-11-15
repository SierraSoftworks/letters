//! A link in text: what the document keeps of the page it leads to shows in a small card while the
//! pointer rests on the link, as a link preview on a line of its own shows it.
//!
//! Showing the card loads nothing. It is made of what a fetch found once, and its picture is one of the
//! document's own.

use letters_model::Preview;
use letters_render::{DisplayList, Item, Point, Rect, Size};

use crate::color::resolve;
use crate::layout::position;
use crate::pictures::host;
use crate::style::Align;
use crate::text::{TextShaper, TextStyle};
use crate::tokens::{self, Scheme};
use crate::widgets::{Placement, Side};

/// How long the pointer rests on a link before its card shows, in milliseconds: long enough that a
/// pointer on its way over the text raises nothing.
const REST_MS: u64 = 350;
/// The room a card's picture takes, and how wide its words run.
const PICTURE: Size = Size { width: 96.0, height: 72.0 };
const WORDS: f32 = 232.0;
/// The most characters of a title and of a description that a card shows: about two lines and three.
const TITLE: usize = 64;
const DESCRIPTION: usize = 110;
const TITLE_TEXT: TextStyle = TextStyle::control().size(tokens::TEXT_SM).weight(600).leading(1.35);
const BODY_TEXT: TextStyle = TextStyle::control().size(tokens::TEXT_SM).leading(1.4);
const HOST_TEXT: TextStyle = TextStyle::control().size(tokens::TEXT_XS).leading(1.4);
const PADDING: f32 = tokens::SPACE_SM;
/// The room between a link and its card.
const GAP: f32 = 6.0;

/// A link in text that the pointer rests on.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Peek {
    /// The address the link leads to, as the document writes it.
    pub url: String,
    /// The line of the link that the pointer is on, in the layout.
    pub line: Rect,
}

/// The link in text under the pointer, and whether its card shows yet.
#[derive(Default)]
pub(crate) struct Peeks {
    over: Option<Peek>,
    /// When the pointer came to rest there, once a frame has said so.
    since: Option<u64>,
    shown: bool,
}

impl Peeks {
    /// Takes the link the pointer rests on now. Returns whether a frame is owed: a card goes, or the
    /// rest on a link begins and is to be timed.
    pub fn point(&mut self, found: Option<Peek>) -> bool {
        if found == self.over {
            return false;
        }
        let owed = self.shown || found.is_some();
        (self.over, self.since, self.shown) = (found, None, false);
        owed
    }

    /// Shows the card once the pointer has rested long enough. Returns when to call again, or `None`
    /// while nothing waits.
    pub fn animate(&mut self, now_ms: u64) -> Option<u64> {
        if self.over.is_none() || self.shown {
            return None;
        }
        let until = *self.since.get_or_insert(now_ms) + REST_MS;
        self.shown = now_ms >= until;
        (!self.shown).then_some(until)
    }

    /// The link whose card shows.
    pub fn shown(&self) -> Option<&Peek> {
        self.over.as_ref().filter(|_| self.shown)
    }
}

/// A text on one line of spaces, cut short when it runs on.
fn cut(text: &str, most: usize) -> String {
    let said = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if said.chars().count() <= most {
        return said;
    }
    let cut: String = said.chars().take(most - 1).collect();
    format!("{}…", cut.trim_end())
}

/// Where a picture of a given size is drawn to fill a slot at its own shape, with what does not fit cut
/// off alike at both sides: as a link preview on the page fills its slot.
fn cover(slot: Rect, (width, height): (u32, u32)) -> Rect {
    let (width, height) = (width.max(1) as f32, height.max(1) as f32);
    let scale = (slot.width / width).max(slot.height / height);
    let (width, height) = (width * scale, height * scale);
    Rect::new(slot.x + (slot.width - width) / 2.0, slot.y + (slot.height - height) / 2.0, width, height)
}

/// The card of a link in text.
pub(crate) struct Card<'a> {
    /// The address the link leads to.
    pub url: &'a str,
    /// What the document keeps of the page there.
    pub kept: &'a Preview,
    /// The size of the kept picture in pixels, once the window has it to draw.
    pub picture: Option<(u32, u32)>,
    /// The line of the link that the pointer is on, in the page area.
    pub anchor: Rect,
    /// The size of the page area, which the card stays inside.
    pub room: Size,
}

impl Card<'_> {
    /// The lines of the card: the page's title, what it says of itself, and the host it is on.
    fn lines(&self) -> [Option<String>; 3] {
        let said =
            |text: &Option<String>, most| text.as_deref().map(|text| cut(text, most)).filter(|text| !text.is_empty());
        let host = Some(host(self.url).to_owned()).filter(|host| !host.is_empty());
        [said(&self.kept.title, TITLE), said(&self.kept.description, DESCRIPTION), host]
    }

    /// What the card says, for someone who cannot see it.
    pub fn said(&self) -> String {
        self.lines().into_iter().flatten().collect::<Vec<_>>().join("\n")
    }

    /// Draws the card as a tooltip is drawn, under the link or over it when there is no room below.
    /// Returns where it is.
    pub fn paint(&self, shaper: &dyn TextShaper, scheme: Scheme, out: &mut DisplayList) -> Rect {
        let color = |ink| resolve(ink, scheme, 1.0);
        let styles = [(TITLE_TEXT, tokens::FG), (BODY_TEXT, tokens::FG_MUTED), (HOST_TEXT, tokens::FG_MUTED)];
        let lines = self.lines().into_iter().zip(styles);
        let words: Vec<_> =
            lines.filter_map(|(line, (style, ink))| Some((shaper.shape(&line?, &style, Some(WORDS)), ink))).collect();
        let wide = words.iter().map(|(line, _)| line.size.width).fold(0.0, f32::max);
        let tall: f32 = words.iter().map(|(line, _)| line.size.height).sum::<f32>()
            + tokens::SPACE_XS * words.len().saturating_sub(1) as f32;
        let slot = self.kept.image.as_ref().map(|_| PICTURE);
        let beside = slot.map_or(0.0, |slot| slot.width + tokens::SPACE_MD);
        // The card's border is inside its size, as a tooltip's is.
        let frame = 2.0 * (PADDING + 1.0);
        let size =
            Size { width: beside + wide + frame, height: tall.max(slot.map_or(0.0, |slot| slot.height)) + frame };
        let placement = Placement { side: Side::Bottom, align: Align::Start, gap: GAP, at: None };
        let rect = position(placement, self.anchor, size, self.room);

        let radius = tokens::RADIUS_CONTROL;
        for shadow in tokens::SHADOW_CARD {
            let offset = Point { x: shadow.x, y: shadow.y };
            out.push(Item::Shadow { rect, color: color(shadow.color), blur: shadow.blur, offset, radius });
        }
        out.push(Item::Fill { rect, color: color(tokens::SURFACE_RAISED), radius });
        out.push(Item::Stroke { rect, color: color(tokens::CARD_LINE), width: 1.0, radius });

        let inner = Point { x: rect.x + PADDING + 1.0, y: rect.y + PADDING + 1.0 };
        if let (Some(slot), Some(asset)) = (slot, &self.kept.image) {
            let at = Rect::new(inner.x, inner.y, slot.width, slot.height);
            // Until the picture is here its size is not known, and its place is kept at the slot's shape.
            let drawn = self.picture.map_or(at, |size| cover(at, size));
            out.push(Item::PushClip { rect: at, radius: tokens::RADIUS_SM });
            out.push(Item::Image { rect: drawn, asset: asset.clone() });
            out.push(Item::PopClip);
        }
        let mut y = inner.y;
        for (line, ink) in words {
            line.paint(Point { x: inner.x + beside, y }, color(ink), out);
            y += line.size.height + tokens::SPACE_XS;
        }
        rect
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peek(url: &str) -> Option<Peek> {
        Some(Peek { url: url.to_owned(), line: Rect::new(40.0, 100.0, 120.0, 24.0) })
    }

    #[test]
    fn a_card_shows_once_the_pointer_has_rested_on_its_link() {
        let mut peeks = Peeks::default();
        assert!(peeks.point(peek("a")), "a frame is owed, which times the rest");
        assert!(!peeks.point(peek("a")), "the pointer moving along the link changes nothing");
        assert_eq!(peeks.animate(1_000), Some(1_000 + REST_MS));
        assert_eq!(peeks.shown(), None, "not while the pointer is on its way");
        assert_eq!(peeks.animate(1_000 + REST_MS), None);
        assert_eq!(peeks.shown(), peek("a").as_ref());

        // Another link is rested on from the start, and the card goes with the pointer.
        assert!(peeks.point(peek("b")));
        assert_eq!(peeks.shown(), None);
        assert_eq!(peeks.animate(5_000), Some(5_000 + REST_MS));
        assert!(!peeks.point(None), "no card showed, so no frame is owed for its going");
        assert_eq!((peeks.animate(9_000), peeks.shown()), (None, None));
    }

    #[test]
    fn a_text_that_runs_on_is_cut_short_on_one_line_of_spaces() {
        // A text, the most it may take, and what is shown of it.
        let cases = [
            ("Welsh slate", 20, "Welsh slate"),
            ("  Slate and\n lead.  ", 20, "Slate and lead."),
            ("Where Welsh slate is cut", 12, "Where Welsh…"),
        ];
        for (text, most, shown) in cases {
            assert_eq!(cut(text, most), shown);
        }
    }

    #[test]
    fn a_picture_fills_its_slot_at_its_own_shape() {
        let slot = Rect::new(10.0, 20.0, 96.0, 72.0);
        for size in [(400, 200), (200, 400), (96, 72)] {
            let drawn = cover(slot, size);
            let covers =
                drawn.x <= slot.x && drawn.y <= slot.y && drawn.width >= slot.width && drawn.height >= slot.height;
            assert!(covers, "{size:?}: {drawn:?}");
            let shape = drawn.width / drawn.height - size.0 as f32 / size.1 as f32;
            assert!(shape.abs() < 0.01, "{size:?} keeps its shape: {drawn:?}");
        }
    }
}
