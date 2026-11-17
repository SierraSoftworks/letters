//! Notes beside the page: every open comment and import note on a card of its own, in a column to the
//! right of the text.
//!
//! A card stands level with the passage it is about, or under the card before it. The one that is open,
//! because it was pressed or the caret is in its passage, stands exactly at its passage, and the others
//! make room above and below it. How much of the column shows follows the rule for two things side by
//! side ([`fit`]): in a window with no room for it the notes open under their passages instead, as
//! [`crate::threads`] shows them.

use std::collections::HashMap;
use std::rc::Rc;

use accesskit::Role;
use letters_editor::{Key, KeyEvent};
use letters_model::Note;

use crate::element::Element;
use crate::icon;
use crate::motion::{self, FRAME_MS};
use crate::notes::Noting;
use crate::opening;
use crate::style::{Align, Edges, Look};
use crate::text::{TextShaper, TextStyle};
use crate::threads::{self, Kind, Spoken};
use crate::tokens;
use crate::widgets::{Button, ControlSize};
use crate::window::{GUTTER, Message};

/// How wide a card is, the room between the text and the cards, and between the cards and the edge of the
/// page area.
pub(crate) const WIDTH: f32 = 280.0;
pub(crate) const GAP: f32 = tokens::SPACE_LG;
const EDGE: f32 = tokens::SPACE_MD;
/// The room the column takes beside the text.
pub(crate) const COLUMN: f32 = GAP + WIDTH + EDGE;
/// The room between two cards, and inside a card round what it says.
const BETWEEN: f32 = tokens::SPACE_SM;
const PADDING: f32 = tokens::SPACE_MD;
/// How wide what is said runs on a card.
pub(crate) const SAID: f32 = WIDTH - 2.0 * PADDING - 2.0;
/// How tall the line is that says who speaks, with the button beside it.
const HEAD: f32 = tokens::CONTROL_HEIGHT_SM;
/// How many times its own width the page needs for the column to stay beside it, pinned.
const PINNED: f32 = 1.25;
/// How long the cards take to make room for the one that opens, and the page to slide under a pin, in
/// milliseconds.
const SLIDE_MS: f32 = 150.0;

/// How the cards fit beside the page, by the rule for two things side by side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Fit {
    /// No room, or no cards: notes open under their passages.
    #[default]
    Under,
    /// Room for part of the column: the page is pinned to the left edge while it is written on, and the
    /// column to the right edge while a card is.
    Pinned,
    /// Room for both.
    Beside,
}

/// How the column fits beside text `text` wide in a page area `room` wide. Both show whole where there
/// is room for both. From 1.25 times the page's own width they stay side by side and one is pinned. With
/// less the notes go under their passages.
pub(crate) fn fit(text: f32, room: f32) -> Fit {
    let page = text + 2.0 * GUTTER;
    if room >= GUTTER + text + COLUMN {
        Fit::Beside
    } else if room >= PINNED * page {
        Fit::Pinned
    } else {
        Fit::Under
    }
}

/// Where each card stands down the page: `wants` is where each passage is and how tall its card, in the
/// order of the page. A card stands at its passage or under the card before it. The one that is `open`
/// stands at its passage whatever is above it, and those above move up to make room.
pub(crate) fn stack(wants: &[(f32, f32)], open: Option<usize>) -> Vec<f32> {
    let mut tops: Vec<f32> = Vec::with_capacity(wants.len());
    for (at, (passage, _)) in wants.iter().enumerate() {
        let under = at.checked_sub(1).map(|before| tops[before] + wants[before].1 + BETWEEN);
        let pinned = open == Some(at);
        tops.push(if pinned { *passage } else { under.map_or(*passage, |under| under.max(*passage)) });
    }
    for at in (0..open.unwrap_or(0)).rev() {
        tops[at] = tops[at].min(tops[at + 1] - BETWEEN - wants[at].1);
    }
    tops
}

/// Where the cards stand, and how they get there.
#[derive(Default)]
pub(crate) struct Cards {
    fit: Fit,
    /// Where each card last stood down the page, by its note.
    stood: HashMap<String, f32>,
    /// The card that was open when they were last placed.
    open: Option<String>,
    /// How far each card stood from its place when the cards last began to move.
    off: HashMap<String, f32>,
    /// Whether a card holds the pin, how far the page has slid toward that from 0 to 1, and from where it
    /// began to.
    held: bool,
    slid: (f32, f32),
    /// How much of a move is done, while there is one, and when it began, once a frame has said.
    going: Option<f32>,
    since: Option<u64>,
    /// Whether the person asked for less motion.
    pub still: bool,
}

impl Cards {
    /// Whether the cards show beside the page.
    pub fn on(&self) -> bool {
        self.fit != Fit::Under
    }

    /// Takes the cards away, as when the page is.
    pub fn clear(&mut self) {
        *self = Self { still: self.still, ..Self::default() };
    }

    /// How much of the way a move still has to go.
    fn rest(&self) -> f32 {
        self.going.map_or(0.0, |done| 1.0 - motion::ease(done))
    }

    /// Starts a move from where everything stands now.
    fn start(&mut self) {
        let rest = self.rest();
        self.off.values_mut().for_each(|off| *off *= rest);
        self.slid.1 = self.slid.0;
        (self.since, self.going) = (None, (!self.still).then_some(0.0));
    }

    /// Takes how the column fits and whether a card holds the pin. Returns where text `text` wide starts
    /// across a page area `room` wide: `None` where the cards do not show and the page stands as it does
    /// without them.
    pub fn left(&mut self, fit: Fit, held: bool, (text, room): (f32, f32)) -> Option<f32> {
        if fit == Fit::Under {
            self.clear();
            return None;
        }
        if self.fit != fit {
            // Cards that come to show, or a window that changes, move nothing by themselves.
            self.slid = (f32::from(u8::from(held)), f32::from(u8::from(held)));
            (self.fit, self.held) = (fit, held);
        } else if self.held != held {
            self.start();
            self.held = held;
        }
        let goal = f32::from(u8::from(self.held));
        self.slid.0 = goal + (self.slid.1 - goal) * self.rest();
        let whole = room - COLUMN - text;
        Some(match fit {
            // The page keeps to the middle while that leaves the cards their room.
            Fit::Beside => whole.min((room - text) / 2.0),
            _ => GUTTER + (whole - GUTTER) * self.slid.0,
        })
    }

    /// Places the cards: `wants` is each note with where its passage is down the page and how tall its
    /// card, in the order of the page, and `open` the note whose card is open. Returns where each card
    /// stands now, on its way to its place when another card has just opened.
    pub fn place(&mut self, wants: &[(&str, f32, f32)], open: Option<&str>) -> Vec<f32> {
        let at = open.and_then(|open| wants.iter().position(|(note, ..)| *note == open));
        let sizes: Vec<(f32, f32)> = wants.iter().map(|(_, passage, tall)| (*passage, *tall)).collect();
        let tops = stack(&sizes, at);
        if self.open.as_deref() != open {
            self.start();
            self.open = open.map(str::to_owned);
            let off =
                |(note, top): (&(&str, f32, f32), &f32)| Some(((*note.0).to_owned(), self.stood.get(note.0)? - top));
            self.off = wants.iter().zip(&tops).filter_map(off).collect();
        }
        let rest = self.rest();
        let now = |(note, top): (&(&str, f32, f32), &f32)| top + self.off.get(note.0).map_or(0.0, |off| off * rest);
        let shown: Vec<f32> = wants.iter().zip(&tops).map(now).collect();
        self.stood = wants.iter().zip(&shown).map(|(note, top)| (note.0.to_owned(), *top)).collect();
        shown
    }

    /// Steps the move. Returns when it wants to be drawn next.
    pub fn animate(&mut self, now_ms: u64) -> Option<u64> {
        self.going?;
        let since = *self.since.get_or_insert(now_ms);
        let done = now_ms.saturating_sub(since) as f32 / SLIDE_MS;
        self.going = Some(done).filter(|done| *done < 1.0 && !self.still);
        self.going.map(|_| now_ms + FRAME_MS)
    }
}

/// The reply of the card that is open: the field it is written in, with the button that sends it. `blank`
/// is whether nothing is written. With it comes how tall it stands.
pub(crate) fn reply(field: Element<Message>, blank: bool) -> (Element<Message>, f32) {
    let tall = field.style.height.unwrap_or(0.0).max(ControlSize::Small.height());
    let post =
        Button::icon_only(icon::SEND, "Send").primary().small().disabled(blank).on_press(Message::Note(Noting::Answer));
    // A reply of several lines has the button beside its last.
    let row = Element::row().align(Align::End).gap(tokens::SPACE_SM).height(tall).child(field.key("reply")).child(post);
    (row, tall)
}

/// Why nothing can be added to the notes of this document, where the reply would be written, and how tall
/// it stands.
pub(crate) fn refusal(why: &str, shaper: &dyn TextShaper) -> (Element<Message>, f32) {
    let small = TextStyle::control().size(tokens::TEXT_SM);
    let tall = shaper.shape(why, &small, Some(SAID)).size.height.ceil();
    let said = Element::paragraph(why, small).fg(tokens::FG_MUTED).width(SAID).height(tall).role(Role::Status);
    (said, tall)
}

/// How tall a card stands: `said` is what its note and each reply say, and `under` how tall the reply
/// under them is, on the card that is open.
pub(crate) fn tall(said: &[Rc<Spoken>], under: Option<f32>) -> f32 {
    let voices: f32 = said.iter().map(|said| HEAD + tokens::SPACE_XS + said.tall()).sum();
    let between = tokens::SPACE_MD * said.len().saturating_sub(1) as f32;
    let under = under.map_or(0.0, |under| 2.0 * tokens::SPACE_MD + 1.0 + under);
    2.0 * PADDING + 2.0 + voices + between + under
}

/// What a card shows.
pub(crate) struct Card<'a> {
    pub note: &'a Note,
    /// What the note and each reply say, as [`threads::Said::card`] set them.
    pub said: &'a [Rc<Spoken>],
    /// The time that notes are told against.
    pub now: &'a str,
    /// Whether the card is the one that is open.
    pub open: bool,
    /// What stands under what was said on the card that is open, and how tall: see [`reply`], [`refusal`].
    pub under: Option<(Element<Message>, f32)>,
    /// Whether nothing can be added to the notes of this document.
    pub refused: bool,
}

impl Card<'_> {
    /// The card: who said what and when, with the button that resolves the note. An import note is no
    /// one's, so it is amber, names no one and takes no reply. A press anywhere on a card opens it.
    pub fn view(self) -> Element<Message> {
        let Self { note, said, now, open, under, refused } = self;
        let import = Kind::of(note) == Some(Kind::Import);
        let opens = Message::Note(Noting::Open(note.id.clone()));
        let small = TextStyle::control().size(tokens::TEXT_SM);
        let resolve = Button::icon_only(icon::CHECK, "Resolve")
            .subtle()
            .small()
            .disabled(refused)
            .on_press(Message::Note(Noting::ResolveNote(note.id.clone())));
        let by = std::iter::once((note.author.as_str(), note.created.as_str()));
        let by = by.chain(note.replies.iter().map(|reply| (reply.author.as_str(), reply.created.as_str())));
        let mut resolve = Some(resolve);
        let voices = by.zip(said).enumerate().map(|(index, ((who, when), said))| {
            let (name, ago) = (threads::named(who), opening::ago(now, when));
            let head = Element::row().align(Align::Center).gap(tokens::SPACE_SM).height(HEAD);
            let (head, name) = if import && index == 0 {
                let sign = Element::icon(icon::FILE_INPUT, 14.0).fg(tokens::WARN);
                (head.child(sign).child(Element::text("Import note", small.weight(600)).shrink()), "Import note")
            } else {
                let head = head.child(threads::face(name)).child(Element::text(name, small.weight(600)).shrink());
                (head.child(Element::text(ago.as_str(), small).fg(tokens::FG_MUTED)), name)
            };
            let head = head.child(Element::row().grow(1.0)).children(resolve.take());
            Element::column()
                .key(("said", index))
                .gap(tokens::SPACE_XS)
                .height(HEAD + tokens::SPACE_XS + said.tall())
                .role(Role::Comment)
                .label(name)
                .description(if import { String::new() } else { ago })
                .child(head)
                .child(threads::words(said, (!open).then(|| opens.clone())))
        });
        let height = tall(said, under.as_ref().map(|(_, tall)| *tall));
        let under = under.map(|(under, tall)| {
            Element::row()
                .height(tokens::SPACE_MD + 1.0 + tall)
                .padding(Edges::top(tokens::SPACE_MD))
                .border(Edges::top(1.0), tokens::LINE)
                .align(Align::End)
                .child(under)
        });
        // An import note is amber, a comment is a card as any other, and the open one is lit.
        let (ground, line) = match (import, open) {
            (true, false) => threads::amber(),
            (true, true) => (threads::amber().0, tokens::WARN),
            (false, false) => (tokens::SURFACE, tokens::CARD_LINE),
            (false, true) => (tokens::SURFACE_RAISED, tokens::INFO),
        };
        let card = Element::column()
            .key(("card", note.id.as_str()))
            .width(WIDTH)
            .height(height)
            .gap(tokens::SPACE_MD)
            .padding(Edges::all(PADDING))
            .bg(ground)
            .fg(tokens::FG)
            .border(Edges::all(1.0), line)
            .radius(tokens::RADIUS_CARD)
            .shadow(if open { tokens::SHADOW_OVERLAY } else { tokens::SHADOW_CARD })
            .clip()
            .role(Role::Group)
            .label(if import { "Import note" } else { "Comment" })
            .selected(open)
            .children(voices)
            .children(under);
        if open {
            // Escape in the reply or on a button of the card leaves the card.
            let leaves = |event: &KeyEvent| (event.key == Key::Escape).then(|| Message::Note(Noting::Close));
            card.on_key(leaves)
        } else {
            card.focusable().hover(Look::NONE.with_border(tokens::LINE_STRONG)).on_press(opens)
        }
    }
}

/// The column of cards over the page area, `room` in size: `cards` are those in view from the top, each
/// with where its top is down the page area, and `left` is where they start across it. The pointer goes
/// through to the page wherever no card is.
pub(crate) fn column(
    cards: Vec<(f32, f32, Element<Message>)>,
    left: f32,
    room: letters_render::Size,
) -> Element<Message> {
    let first = cards.first().map_or(0.0, |(top, ..)| *top);
    let mut column = Element::column()
        .overlay()
        .hollow()
        .width(room.width)
        .height(room.height)
        .padding(Edges::left(left))
        .align(Align::Start)
        // A card that has scrolled part of the way out starts above the page area.
        .lifted((-first).max(0.0));
    let mut under = first.min(0.0);
    for (top, tall, card) in cards {
        column = column.child(Element::row().height((top - under).max(0.0)).hollow()).child(card);
        under = top + tall;
    }
    column
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_column_follows_the_rule_for_two_things_side_by_side() {
        let text = 600.0;
        let page = text + 2.0 * GUTTER;
        // The width of the page area, and how the column fits.
        let cases = [
            (GUTTER + text + COLUMN + 400.0, Fit::Beside),
            (GUTTER + text + COLUMN, Fit::Beside),
            (GUTTER + text + COLUMN - 1.0, Fit::Pinned),
            (1.25 * page, Fit::Pinned),
            (1.25 * page - 1.0, Fit::Under),
            (page, Fit::Under),
        ];
        for (room, fits) in cases {
            assert_eq!(fit(text, room), fits, "{room}");
        }
    }

    #[test]
    fn the_page_is_pinned_while_it_is_written_on_and_the_cards_while_one_is() {
        let (text, room) = (600.0, 860.0);
        let mut cards = Cards { still: true, ..Cards::default() };
        assert_eq!(cards.left(Fit::Under, false, (text, room)), None);
        // The page shows whole from its left margin, and what the room leaves of the cards.
        assert_eq!(cards.left(Fit::Pinned, false, (text, room)), Some(GUTTER));
        // A card that holds the pin shows whole, to the right edge.
        let left = cards.left(Fit::Pinned, true, (text, room)).expect("the cards show");
        assert_eq!(left + text + COLUMN, room);
        // With room for both, the page keeps to the middle while the cards fit beside it.
        assert_eq!(cards.left(Fit::Beside, true, (text, 1600.0)), Some(500.0));
        assert_eq!(cards.left(Fit::Beside, false, (text, 1000.0)), Some(1000.0 - COLUMN - text));
    }

    #[test]
    fn cards_stand_at_their_passages_or_under_one_another_and_make_room_for_the_open_one() {
        // Where each passage is and how tall its card.
        let wants = [(100.0, 80.0), (120.0, 60.0), (130.0, 40.0), (600.0, 50.0)];
        let closed = stack(&wants, None);
        assert_eq!(closed, [100.0, 100.0 + 80.0 + BETWEEN, 188.0 + 60.0 + BETWEEN, 600.0]);
        for open in 0..wants.len() {
            let tops = stack(&wants, Some(open));
            assert_eq!(tops[open], wants[open].0, "{open}: the open card is at its passage");
            for at in 1..wants.len() {
                let above = tops[at - 1] + wants[at - 1].1;
                assert!(tops[at] >= above + BETWEEN, "{open}: card {at} is clear of the one above: {tops:?}");
            }
        }
    }

    #[test]
    fn cards_slide_to_make_room_when_another_opens() {
        let wants = [("a", 100.0, 80.0), ("b", 120.0, 60.0)];
        let mut cards = Cards::default();
        let closed = cards.place(&wants, None);
        // The frame a card opens in, every card is still where it was.
        assert_eq!(cards.place(&wants, Some("b")), closed);
        _ = cards.animate(0);
        _ = cards.animate(SLIDE_MS as u64 / 2);
        let midway = cards.place(&wants, Some("b"));
        assert!(midway[1] < closed[1] && midway[1] > 120.0, "{midway:?}");
        assert_eq!(cards.animate(SLIDE_MS as u64), None);
        assert_eq!(cards.place(&wants, Some("b")), [120.0 - BETWEEN - 80.0, 120.0]);
        // Someone who asked for less motion has them there at once.
        let mut cards = Cards { still: true, ..Cards::default() };
        cards.place(&wants, None);
        assert_eq!(cards.place(&wants, Some("b")), [120.0 - BETWEEN - 80.0, 120.0]);
    }
}
