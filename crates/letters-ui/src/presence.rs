//! Who else has the document open: their faces on the bar, and their carets and selections on the page.
//!
//! The window shows what a shell tells it and keeps none of it. A person has one colour wherever they
//! show, which the server chose for their session: the window only says which colours there are.

use accesskit::Role;
use letters_layout::DocLayout;
use letters_model::{Pos, Selection};
use letters_render::{DisplayList, Item, Point, Rect};

use crate::color::{fade, mix, resolve};
use crate::element::Element;
use crate::style::{Align, Edges, Justify};
use crate::text::{TextShaper, TextStyle};
use crate::threads::{initials, named};
use crate::tokens::{self, Color as Ink, Scheme};
use crate::widgets::Side;
use crate::window::Message;

/// The presence colours, in the order a server counts them. Blue is left out: it is the colour of comments,
/// and of a person's own selection until they choose another.
const HUES: [Ink; 5] = [tokens::SYNTAX_KEYWORD, tokens::SYNTAX_TYPE, tokens::WARN, tokens::OK, tokens::SYNTAX_PROPERTY];
/// How much of a person's colour is in their face, and how much of it lies over what they have selected.
const TINT: (f32, f32) = (0.3, 0.22);
/// The most faces the bar shows. With more people than that, the last place counts the rest.
const FACES: usize = 4;
/// A window narrower than this has the one place, so that its bar keeps its tools.
const NARROW: f32 = 720.0;
/// A face with the ring that parts it from the one it lies over, and how far it lies over that one.
const FACE: (f32, f32, f32) = (28.0, 2.0, 6.0);
/// How wide a caret is, and the mark on top of one that has no name over it.
const CARET: f32 = 2.0;
const CAP: f32 = 6.0;
/// The room round a name over its caret, and how round the name's corners are.
const FLAG: (f32, f32, f32) = (4.0, 1.0, 3.0);
/// How long a name stays over a caret that came or moved, in milliseconds.
const NAMED_MS: u64 = 3000;
/// How near a caret the pointer brings its name out.
const REACH: f32 = 4.0;

/// Someone else who has the document open, as a shell tells the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// Tells their session from every other for as long as it lasts. One person at two devices is two.
    pub session: u64,
    /// The name the others see. A person with none is shown as "Someone".
    pub name: String,
    /// Which of the window's presence colours is theirs, counted round. The server hands them out, so
    /// that everyone sees a person in the same colour.
    pub colour: u8,
    /// Where their caret or selection is, in the document as the window holds it now. `None` while they
    /// have none in the text.
    pub selection: Option<Selection>,
}

fn hue(colour: u8) -> Ink {
    HUES[usize::from(colour) % HUES.len()]
}

/// Whether a caret has its name over it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Named {
    /// It came or moved, and no frame has been timed since.
    Fresh,
    Until(u64),
    No,
}

#[derive(Default)]
pub(crate) struct Presence {
    peers: Vec<(Peer, Named)>,
    /// The session whose caret the pointer is at.
    over: Option<u64>,
    /// Whether the window is too narrow for more than one face.
    narrow: bool,
}

impl Presence {
    /// Whether these are the people the window was last told of, each where they were.
    pub fn is(&self, peers: &[Peer]) -> bool {
        self.peers.iter().map(|(peer, _)| peer).eq(peers)
    }

    /// Takes who is here, in place of whoever was. A caret that is new, or that moved, has its name over
    /// it for a while.
    pub fn set(&mut self, peers: Vec<Peer>) {
        let were = std::mem::take(&mut self.peers);
        let named = |peer: Peer| {
            let rests = were.iter().find(|(was, _)| was.session == peer.session && was.selection == peer.selection);
            let shows = rests.map_or(Named::Fresh, |(_, shows)| *shows);
            (peer, shows)
        };
        self.peers = peers.into_iter().map(named).collect();
        self.over = self.over.filter(|over| self.peers.iter().any(|(peer, _)| peer.session == *over));
    }

    /// Told how wide the window is, which decides how many faces its bar has room for.
    pub fn fit(&mut self, width: f32) {
        self.narrow = width < NARROW;
    }

    /// How many faces the bar shows, which decides how much else fits on it.
    pub fn shown(&self) -> usize {
        let most = if self.narrow { 1 } else { FACES };
        if self.peers.len() > most { most - 1 } else { self.peers.len() }
    }

    /// Where someone's caret is, and has their name over it again, as when the person goes to look.
    pub fn visit(&mut self, session: u64) -> Option<Pos> {
        let (peer, named) = self.peers.iter_mut().find(|(peer, _)| peer.session == session)?;
        let head = peer.selection.as_ref()?.head.clone();
        *named = Named::Fresh;
        Some(head)
    }

    /// Takes the names off the carets that have rested long enough. Returns when the next is due to go.
    pub fn animate(&mut self, now_ms: u64) -> Option<u64> {
        let mut next: Option<u64> = None;
        for (peer, named) in &mut self.peers {
            let until = match *named {
                Named::Fresh if peer.selection.is_some() => now_ms + NAMED_MS,
                Named::Until(until) if now_ms < until => until,
                _ => {
                    *named = Named::No;
                    continue;
                }
            };
            *named = Named::Until(until);
            next = Some(next.map_or(until, |next| next.min(until)));
        }
        next
    }

    /// The pointer came to rest at someone's caret, or left it. Returns whether what shows changed.
    pub fn point(&mut self, over: Option<u64>) -> bool {
        std::mem::replace(&mut self.over, over) != over
    }

    /// Whose caret is at a point of the layout, among those `viewport` shows.
    pub fn at(&self, layout: &DocLayout, at: Point, viewport: Rect) -> Option<u64> {
        self.peers.iter().find_map(|(peer, _)| {
            let caret = caret(peer, layout, viewport)?;
            let near =
                Rect::new(caret.x - REACH, caret.y - CAP, CARET + 2.0 * REACH + CAP, caret.height + CAP).contains(at);
            near.then_some(peer.session)
        })
    }

    /// The faces of who is here, for the bar: a press on one brings the page to where that person is.
    pub fn faces(&self) -> Option<Element<Message>> {
        if self.peers.is_empty() {
            return None;
        }
        let (shown, rest) = self.peers.split_at(self.shown());
        let faces = shown.iter().map(|(peer, _)| {
            let name = named(&peer.name);
            let face = round(mix(tokens::SURFACE, hue(peer.colour), TINT.0), tokens::FG, initials(name));
            face.role(Role::Button)
                .label(name)
                .description("Has this document open")
                .tooltip(name, Side::Bottom)
                .on_press(Message::Visit(peer.session))
        });
        let more = (!rest.is_empty()).then(|| {
            let names: Vec<&str> = rest.iter().map(|(peer, _)| named(&peer.name)).collect();
            let names = names.join(", ");
            // With no face before it, it counts everyone.
            let (says, label) = match shown {
                [] => (rest.len().to_string(), format!("{} people here", rest.len())),
                _ => (format!("+{}", rest.len()), format!("{} more", rest.len())),
            };
            // It lies over a face, so its ground lets nothing through.
            round(mix(tokens::SURFACE, tokens::FG_MUTED, 0.16), tokens::FG_MUTED, says)
                .role(Role::Label)
                .label(label)
                .description(names.clone())
                .tooltip(names, Side::Bottom)
        });
        let row = Element::row().align(Align::Center).gap(-FACE.2).role(Role::Group).label("Also here");
        Some(row.children(faces).children(more))
    }

    /// What the others have selected, to lie under the page's text: each in a wash of its owner's colour.
    pub fn wash(&self, layout: &DocLayout, viewport: Rect, scheme: Scheme, out: &mut DisplayList) {
        for (peer, _) in &self.peers {
            let Some(selection) = peer.selection.as_ref().filter(|selection| !selection.is_caret()) else { continue };
            let color = resolve(fade(hue(peer.colour), TINT.1), scheme, 1.0);
            let spans = layout.selection_in(selection, viewport);
            out.items.extend(spans.into_iter().map(|rect| Item::Fill { rect, color, radius: 0.0 }));
        }
    }

    /// The others' carets, to lie over the page's text. One that is new where it is, or has the pointer at
    /// it, has its owner's name over it, kept inside `reach`, the furthest right a name may end.
    pub fn carets(
        &self,
        layout: &DocLayout,
        (viewport, reach): (Rect, f32),
        shaper: &dyn TextShaper,
        scheme: Scheme,
        out: &mut DisplayList,
    ) {
        for (peer, shows) in &self.peers {
            let Some(caret) = caret(peer, layout, viewport) else { continue };
            let color = resolve(hue(peer.colour), scheme, 1.0);
            out.items.push(Item::Fill { rect: Rect { width: CARET, ..caret }, color, radius: 0.0 });
            if *shows == Named::No && self.over != Some(peer.session) {
                // The caret alone would be taken for the person's own.
                let cap = Rect::new(caret.x - (CAP - CARET) / 2.0, caret.y - CAP / 2.0, CAP, CAP);
                out.items.push(Item::Fill { rect: cap, color, radius: 1.0 });
                continue;
            }
            let name = shaper.shape(named(&peer.name), &TextStyle::control().size(tokens::TEXT_XS).weight(600), None);
            let (wide, tall) = (name.size.width + 2.0 * FLAG.0, name.size.height + 2.0 * FLAG.1);
            let x = caret.x.min(reach - wide).round();
            let flag = Rect::new(x, (caret.y - tall).round(), wide.round(), tall.round());
            out.items.push(Item::Fill { rect: flag, color, radius: FLAG.2 });
            // What is written on a colour is the surface's, which is light where the colour is dark.
            name.paint(Point { x: flag.x + FLAG.0, y: flag.y + FLAG.1 }, resolve(tokens::SURFACE, scheme, 1.0), out);
        }
    }
}

/// Where someone's caret is, when `viewport` shows it.
fn caret(peer: &Peer, layout: &DocLayout, viewport: Rect) -> Option<Rect> {
    let head = &peer.selection.as_ref()?.head;
    let shows = |(top, bottom): (f32, f32)| bottom > viewport.y && top < viewport.y + viewport.height;
    // A block far from what shows is not asked about: the layout would set its lines to answer.
    layout.band(&head.block).filter(|band| shows(*band))?;
    layout.caret(head).filter(|caret| shows((caret.y, caret.y + caret.height)))
}

/// A round face that says a few letters.
fn round(ground: Ink, ink: Ink, says: String) -> Element<Message> {
    Element::row()
        .width(FACE.0)
        .height(FACE.0)
        .radius(FACE.0 / 2.0)
        .bg(ground)
        .fg(ink)
        .border(Edges::all(FACE.1), tokens::SURFACE)
        .align(Align::Center)
        .justify(Justify::Center)
        .child(Element::text(says, TextStyle::control().size(10.0).weight(600)).decorative())
}
