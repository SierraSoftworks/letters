//! A link in text: what the document keeps of the page it leads to shows in a small card while the
//! pointer rests on the link, as a link preview on a line of its own shows it.
//!
//! Showing the card loads nothing. It is made of what a fetch found once, and its picture is one of the
//! document's own. Where the document keeps nothing of a web page, the card offers the fetch, as a link
//! preview that shows its address alone does.

use letters_model::{BlockId, Preview};
use letters_render::{DisplayList, Item, Point, Rect, Size};

use crate::color::resolve;
use crate::layout::position;
use crate::pictures::host;
use crate::previews;
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
    /// The block the link is written in.
    pub block: BlockId,
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
    /// Where the card that shows was last drawn, and the button on it when it has one, in the page area.
    drawn: Option<(Rect, Option<Rect>)>,
    /// Whether the pointer is on that button.
    aimed: bool,
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
        (self.drawn, self.aimed) = (None, false);
        owed
    }

    /// Takes where the card that shows was drawn, and the button on it.
    pub fn drew(&mut self, drawn: Option<(Rect, Option<Rect>)>) {
        self.drawn = drawn;
    }

    /// The link whose card a point of the page area is on, or between it and the link: a card with a
    /// button stays for the pointer to reach it.
    pub fn held(&self, at: Point) -> Option<&Peek> {
        let (card, _) = self.drawn.filter(|(_, button)| button.is_some())?;
        let reach = Rect::new(card.x - GAP, card.y - GAP, card.width + 2.0 * GAP, card.height + 2.0 * GAP);
        self.shown().filter(|_| reach.contains(at))
    }

    /// The button of the card that shows, in the page area.
    pub fn button(&self) -> Option<Rect> {
        self.shown().and(self.drawn).and_then(|(_, button)| button)
    }

    /// Takes where the pointer is in the page area. Returns whether a frame is owed: it came onto the
    /// card's button, or left it.
    pub fn aim(&mut self, at: Option<Point>) -> bool {
        let aimed = at.zip(self.button()).is_some_and(|(at, button)| button.contains(at));
        std::mem::replace(&mut self.aimed, aimed) != aimed
    }

    /// Whether the pointer is on the card's button.
    pub fn aimed(&self) -> bool {
        self.aimed
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

/// The fetch that a card offers of a page the document keeps nothing of.
pub(crate) struct Offer {
    /// What its button says, which stands as words alone while the page is fetched.
    pub says: &'static str,
    /// Whether the page is being fetched, so that there is nothing to press.
    pub waits: bool,
    /// What is said of a fetch that found nothing.
    pub failed: Option<String>,
    /// Whether the pointer is on the button.
    pub aimed: bool,
}

/// The card of a link in text.
pub(crate) struct Card<'a> {
    /// The address the link leads to.
    pub url: &'a str,
    /// What the document keeps of the page there, when it keeps anything.
    pub kept: Option<&'a Preview>,
    /// The size of the kept picture in pixels, once the window has it to draw.
    pub picture: Option<(u32, u32)>,
    /// The fetch the card offers, where nothing is kept and the page can be fetched.
    pub offer: Option<Offer>,
    /// The line of the link that the pointer is on, in the page area.
    pub anchor: Rect,
    /// The size of the page area, which the card stays inside.
    pub room: Size,
}

impl Card<'_> {
    /// The lines of the card: the page's title, what it says of itself, the host it is on, and what is
    /// said of a fetch that found nothing.
    fn lines(&self) -> [Option<String>; 4] {
        let said =
            |text: &Option<String>, most| text.as_deref().map(|text| cut(text, most)).filter(|text| !text.is_empty());
        let kept = |line: fn(&Preview) -> &Option<String>, most| self.kept.and_then(|kept| said(line(kept), most));
        let host = Some(host(self.url).to_owned()).filter(|host| !host.is_empty());
        let failed = self.offer.as_ref().and_then(|offer| offer.failed.clone());
        [kept(|kept| &kept.title, TITLE), kept(|kept| &kept.description, DESCRIPTION), host, failed]
    }

    /// What the card says, for someone who cannot see it.
    pub fn said(&self) -> String {
        self.lines().into_iter().flatten().collect::<Vec<_>>().join("\n")
    }

    /// Draws the card as a tooltip is drawn, under the link or over it when there is no room below.
    /// Returns where it is, and where the button of its offer is: the words that stand for the button
    /// while the page is fetched.
    pub fn paint(&self, shaper: &dyn TextShaper, scheme: Scheme, out: &mut DisplayList) -> (Rect, Option<Rect>) {
        let color = |ink| resolve(ink, scheme, 1.0);
        let quiet = (BODY_TEXT, tokens::FG_MUTED);
        let styles = [(TITLE_TEXT, tokens::FG), quiet, (HOST_TEXT, tokens::FG_MUTED), quiet];
        let lines = self.lines().into_iter().zip(styles);
        let words: Vec<_> =
            lines.filter_map(|(line, (style, ink))| Some((shaper.shape(&line?, &style, Some(WORDS)), ink))).collect();
        let wide = words.iter().map(|(line, _)| line.size.width).fold(0.0, f32::max);
        let tall: f32 = words.iter().map(|(line, _)| line.size.height).sum::<f32>()
            + tokens::SPACE_XS * words.len().saturating_sub(1) as f32;
        let image = self.kept.and_then(|kept| kept.image.as_ref());
        let slot = image.map(|_| PICTURE);
        let beside = slot.map_or(0.0, |slot| slot.width + tokens::SPACE_MD);
        // The button stands under the words, as wide as a link preview's control.
        let (across, down) = previews::CONTROL;
        let control = self.offer.as_ref().map(|_| Size { width: across, height: down });
        let (wide, under) = control.map_or((wide, 0.0), |control| (wide.max(control.width), control.height));
        let tall = tall + if under > 0.0 && tall > 0.0 { tokens::SPACE_SM + under } else { under };
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
        if let (Some(slot), Some(asset)) = (slot, image) {
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
        let button = self.offer.as_ref().zip(control).map(|(offer, control)| {
            let top = rect.y + rect.height - PADDING - 1.0 - control.height;
            let at = Rect::new(inner.x + beside, top, control.width, control.height);
            previews::button(offer.says, (offer.waits, offer.aimed), at, shaper, scheme, out);
            at
        });
        (rect, button)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peek(url: &str) -> Option<Peek> {
        Some(Peek { block: BlockId::new("line"), url: url.to_owned(), line: Rect::new(40.0, 100.0, 120.0, 24.0) })
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
    fn a_card_with_a_button_stays_while_the_pointer_is_on_it_or_on_its_way_there() {
        let (card, button) = (Rect::new(40.0, 130.0, 200.0, 60.0), Rect::new(50.0, 156.0, 112.0, 24.0));
        let rested = |drawn| {
            let mut peeks = Peeks::default();
            peeks.point(peek("a"));
            peeks.animate(0);
            peeks.animate(REST_MS);
            peeks.drew(Some(drawn));
            peeks
        };
        // A point of the page area, whether it holds the card that has a button, and whether it is on that.
        let cases = [
            (Point { x: 60.0, y: 127.0 }, true, false),
            (Point { x: 60.0, y: 140.0 }, true, false),
            (Point { x: 60.0, y: 160.0 }, true, true),
            (Point { x: 300.0, y: 160.0 }, false, false),
        ];
        for (at, held, aimed) in cases {
            let mut peeks = rested((card, Some(button)));
            assert_eq!(peeks.held(at), peek("a").as_ref().filter(|_| held), "{at:?}");
            assert_eq!(peeks.aim(Some(at)), aimed, "a frame is owed when the pointer comes onto the button: {at:?}");
            assert_eq!(peeks.aimed(), aimed, "{at:?}");
            assert_eq!(rested((card, None)).held(at), None, "a card with nothing to press goes with the pointer");
        }

        // The card of another link starts with nothing drawn.
        let mut peeks = rested((card, Some(button)));
        peeks.aim(Some(Point { x: 60.0, y: 160.0 }));
        peeks.point(peek("b"));
        assert_eq!((peeks.button(), peeks.aimed()), (None, false));
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
