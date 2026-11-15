//! Conflict review: the page split at a block that two writers changed apart.
//!
//! A conflict that is open shows both versions of its block as segments of the page, side by side or one
//! above the other as the window has room: the version held back, which is read, and the one on the page,
//! which is written on as ever. A line at the head of each says whose it is, where it stands, and what
//! choosing it does. Only the words in which the two differ are marked.
//!
//! The two trade places when the version held back is used, so that either is written on in the page
//! before one is kept, which settles the conflict.
//!
//! The layout cuts the page ([`Split`]). This module keeps the held-back version laid out, draws what
//! stands round the two, and tells the window where each part of the page shows.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use accesskit::{Node, NodeId};
use letters_layout::{Arrangement, DocLayout, Fonts, Palette, Piece, Side, Split, Uncovered, Window, WordList};
use letters_model::{Block, BlockId, BlockKind, Body, Document, Note, NoteKind, Pos, Selection, Style, Version};
use letters_render::{Color, DisplayList, Item, Point, Rect, Size};

use crate::color::resolve;
use crate::describe;
use crate::element::Element;
use crate::icon::{self, Icon};
use crate::motion::{self, FRAME_MS};
use crate::opening;
use crate::text::{Shaped, TextShaper, TextStyle};
use crate::threads;
use crate::tokens::{self, Color as Ink, Scheme};
use crate::widgets::{Banner, Button, Tone};
use crate::window::Message;
use crate::words::WordLists;

/// The room at the head of a version for the line that says whose it is, the room under its text, and
/// the room between its frame and what the page has above it.
pub(crate) const HEAD: f32 = 44.0;
const FOOT: f32 = tokens::SPACE_MD;
const ABOVE: f32 = tokens::SPACE_MD;
/// The margin on either side of a version's text, and how far inside its segment its frame stands, so
/// that two frames side by side have a gap between them.
pub(crate) const MARGIN: f32 = tokens::SPACE_XL;
pub(crate) const INSET: f32 = tokens::SPACE_XS;
/// How thick the rule at the top of a version is, which tells the two apart with the marks on its words.
pub(crate) const RULE: f32 = 3.0;
/// The face of who wrote a version, the icon on a button, and the room inside a button beside them.
pub(crate) const FACE: f32 = 20.0;
const SIGN: f32 = 14.0;
const PAD: f32 = 10.0;
/// How thick the line under a word that differs is, and how long its dots and gaps are where it is dotted.
const UNDER: f32 = 2.0;
const DOT: f32 = 2.0;
/// The round button that brings a version cut off by the window's edge across.
const BRING: f32 = 32.0;
/// How long a pair takes to slide to its other end, in milliseconds.
pub(crate) const SLIDE_MS: f32 = 150.0;
/// How much of the page's colour lies over the version that is not the active one of a pinned pair.
const ASIDE: f32 = 0.5;
/// The most pairs of words that are compared one by one. Longer texts are compared by their ends alone.
const MOST: usize = 1 << 20;

/// What is asked about the conflict that is open, and about those that wait.
#[derive(Clone, Debug, PartialEq)]
pub enum Reviewing {
    /// Opens the next conflict down the page, or with `back` the one before it. With none open, the
    /// first or the last.
    Step { back: bool },
    /// Keeps the version on the page, which settles the conflict.
    Keep,
    /// Puts the version that was held back on the page and holds back what the page read: the two trade
    /// places, and the conflict stays open. A version that took the block out takes it out, which settles.
    Use,
    /// Puts the open conflict away undecided.
    Close,
}

/// What a press on the page area is on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Press {
    Keep,
    Use,
    /// The version that the window's edge cuts off, to bring it across.
    Bring(Side),
}

/// The conflict that is open, with the version it holds back laid out.
struct Open {
    note: String,
    block: BlockId,
    versions: [Version; 2],
    /// Which of the note's two versions the page stands for. The other is held back.
    page: usize,
    /// The block as the version held back has it: as the note does, or as the page read when the two
    /// last traded places. `None` for a version that took the block out.
    held: Option<Block>,
    /// The document with the held-back version in the block's place, and its layout.
    doc: Document,
    first: DocLayout,
    /// What that layout was last made for.
    laid: Option<(f32, Style, Palette)>,
    /// The version the window is pinned to where it cannot show both whole.
    active: Side,
    /// How far the pair has slid to the active version's end, and when it set off.
    slide: f32,
    since: Option<u64>,
    /// The selection when it was last looked at. A change brings the page's version across.
    seen: Option<Selection>,
    /// The page's block as its words were last compared, and the words that differ in each version.
    compared: Option<Block>,
    words: [Vec<Selection>; 2],
}

/// The conflict that is open in the page, and where its parts were last drawn.
#[derive(Default)]
pub(crate) struct Review {
    open: Option<Open>,
    /// The faces of the system that back the fonts up, for a conflict opened after they came.
    backup: Fonts,
    /// The word lists, for a conflict opened after they came.
    lists: WordLists,
    /// What can be pressed, where it shows in the page area.
    hot: Vec<(Rect, Press)>,
    /// Where the held-back version shows in the page area, which the page has nothing under.
    held: Option<Rect>,
    /// How the two versions shared the window when they were last drawn.
    arrangement: Option<Arrangement>,
    pointer: Option<Point>,
    pressed: Option<Press>,
    /// What the page read of a conflict's block when its versions last traded places, by the note: which
    /// of the note's versions that was, and the block. A conflict that is opened again holds it back so.
    traded: HashMap<String, (usize, Block)>,
    /// Whether what moves may be shown moving.
    pub still: bool,
}

impl Open {
    /// `page` is the block as the page has it, and `traded` what the page last read of another version.
    fn new(
        note: &Note,
        versions: &[Version; 2],
        doc: &Document,
        page: &Block,
        traded: Option<&(usize, Block)>,
    ) -> Self {
        let block = note.anchor.block.clone();
        // The page stands for the version it reads more like: the later one, until the two trade places.
        let far = |version: &Version| version.block.as_ref().map_or(usize::MAX, |block| apart(block, page));
        let on_page = usize::from(far(&versions[0]) >= far(&versions[1]));
        let traded = traded.filter(|(side, _)| *side != on_page);
        let held = traded.map(|(_, block)| block.clone()).or_else(|| versions[1 - on_page].block.clone());
        let mut open = Self {
            note: note.id.clone(),
            block,
            versions: versions.clone(),
            page: on_page,
            held: None,
            doc: doc.clone(),
            first: DocLayout::new(Fonts { faces: letters_fonts::bundled() }),
            laid: None,
            active: Side::Second,
            slide: 1.0,
            since: None,
            seen: None,
            compared: None,
            words: Default::default(),
        };
        open.hold(held);
        open
    }

    /// Whose the version held back is and when it was written, or with `on_page` the page's.
    fn of(&self, on_page: bool) -> &Version {
        &self.versions[if on_page { self.page } else { 1 - self.page }]
    }

    /// Holds a version back: `None` for one that took the block out.
    fn hold(&mut self, held: Option<Block>) {
        // A version that took the block away stands as an empty line, so that the page has a place for it.
        let shown = held.clone().unwrap_or_else(|| Block::new(self.block.clone(), BlockKind::Paragraph));
        put(&mut self.doc.blocks, Block { id: self.block.clone(), ..shown });
        (self.held, self.laid) = (held, None);
    }

    /// Makes `side` the version the window is pinned to, sliding there where the pair is pinned.
    fn turn(&mut self, side: Side, slides: bool) {
        if self.active != side {
            (self.active, self.since) = (side, None);
            self.slide = if slides { 0.0 } else { 1.0 };
        }
    }
}

/// Puts `with` where the block of its id is among `blocks`, however deep.
pub(crate) fn put(blocks: &mut [Block], with: Block) -> Option<Block> {
    let mut with = Some(with);
    for block in blocks {
        let held = with.take()?;
        if block.id == held.id {
            *block = held;
            return None;
        }
        with = match &mut block.body {
            Body::Blocks(parts) => put(parts, held),
            _ => Some(held),
        };
    }
    with
}

impl Review {
    /// Brings what is kept here up to date with the conflict that is open, `note`, and with the
    /// document. With no note, or one whose block the page does not hold, nothing is open.
    pub fn follow(
        &mut self,
        note: Option<&Note>,
        doc: &Document,
        selection: Option<&Selection>,
        (width, style, palette): (f32, &Style, Palette),
    ) {
        let conflict = note.and_then(|note| match &note.kind {
            NoteKind::Conflict { versions } => Some((note, &**versions)),
            _ => None,
        });
        let Some(((note, versions), page)) = conflict.zip(note.and_then(|note| doc.block(&note.anchor.block))) else {
            (self.open, self.held, self.pressed) = (None, None, None);
            return self.hot.clear();
        };
        let same = |open: &Open| open.note == note.id && open.block == note.anchor.block && open.versions == *versions;
        if !self.open.as_ref().is_some_and(same) {
            let mut open = Open::new(note, versions, doc, page, self.traded.get(&note.id));
            open.first.add_fallback_fonts(&self.backup);
            self.lists.hand(&mut open.first);
            open.seen = selection.cloned();
            self.open = Some(open);
        }
        let Some(open) = self.open.as_mut() else { return };
        if open.compared.as_ref() != Some(page) {
            // A page that comes to read as the version held back has traded places with it: that version
            // was used, here or by a writer elsewhere, or a use was taken back. What the page read until
            // then is the version held back now.
            let was = open.compared.replace(page.clone());
            let traded = open.held.as_ref().is_some_and(|held| apart(held, page) == 0);
            if let Some(was) = was.filter(|_| traded) {
                self.traded.insert(open.note.clone(), (open.page, was.clone()));
                open.page = 1 - open.page;
                open.hold(Some(was));
            }
            let held = open.doc.block(&open.block);
            open.words = held.map(|held| differing(held, page)).unwrap_or_default();
        }
        if !open.laid.as_ref().is_some_and(|laid| (laid.0, &laid.1, laid.2) == (width, style, palette)) {
            // A formula is typeset in both versions: only the page's has a caret to show its source for.
            open.first.set_formula_source(false);
            open.first.set_palette(palette);
            open.first.update(&open.doc, style, width, None);
            open.laid = Some((width, style.clone(), palette));
        }
        // The version that holds the caret is the active one, and the caret is only ever in the page's.
        if open.seen.as_ref() != selection {
            open.seen = selection.cloned();
            open.turn(Side::Second, self.arrangement == Some(Arrangement::Pinned) && !self.still);
        }
    }

    /// Takes faces that arrived late, for the version that is held back.
    pub fn add_fonts(&mut self, fonts: &Fonts) {
        if let Some(open) = &mut self.open {
            open.first.add_fonts(fonts);
        }
    }

    /// Takes faces of the system that back the fonts up, for the version that is held back. Returns
    /// whether a character that was shown without a glyph has one now.
    pub fn add_fallback_fonts(&mut self, fonts: &Fonts) -> bool {
        self.backup.faces.extend(fonts.faces.iter().cloned());
        self.open.as_mut().is_some_and(|open| open.first.add_fallback_fonts(fonts))
    }

    /// The characters the version held back showed without a glyph since this was last asked.
    pub fn take_uncovered(&mut self) -> Vec<Uncovered> {
        self.open.as_mut().map(|open| open.first.take_uncovered()).unwrap_or_default()
    }

    /// Takes the word list of a script, for the version that is held back. Returns whether its lines may
    /// now end elsewhere.
    pub fn add_word_list(&mut self, list: WordList, data: &Rc<[u8]>) -> bool {
        let kept = self.lists.keep(list, data);
        self.open.as_mut().filter(|_| kept).is_some_and(|open| open.first.add_word_list(list, data))
    }

    /// The scripts the version held back was set in without their word list since this was last asked.
    pub fn take_unlisted(&mut self) -> Vec<WordList> {
        self.open.as_mut().map(|open| open.first.take_unlisted()).unwrap_or_default()
    }

    /// The page as the open conflict splits it, in a page area `room` wide: counted from the top of what
    /// scrolls. `None` while no conflict is open.
    pub fn split(&self, page: &DocLayout, room: f32) -> Option<Split> {
        let open = self.open.as_ref()?;
        Some(split(&open.first, page, std::slice::from_ref(&open.block), room, (open.active, open.slide)))
    }

    /// Lays out what `view`, a part of what scrolls, shows of the held-back version. Returns whether
    /// that changed what it had estimated, so that the page is to be split again.
    pub fn settle(&mut self, split: &Split, view: Rect) -> bool {
        let shown = split.shown(Side::First, view);
        self.open.as_mut().zip(shown).is_some_and(|(open, shown)| {
            let before = open.first.height();
            open.first.settle(shown, 0);
            open.first.height() != before
        })
    }

    /// The words that differ in the page's version, to be marked under its text.
    pub fn words(&self) -> &[Selection] {
        self.open.as_ref().map_or(&[], |open| &open.words[1])
    }

    /// The note of the conflict that is open.
    pub fn note(&self) -> Option<&str> {
        self.open.as_ref().map(|open| open.note.as_str())
    }

    /// The version a conflict holds back, to put in its block's place: `None` inside for one that took
    /// the block away.
    pub fn held_back(&self) -> Option<(BlockId, Option<Box<Block>>)> {
        let open = self.open.as_ref()?;
        Some((open.block.clone(), open.held.clone().map(Box::new)))
    }

    /// Steps the slide of a pinned pair. Returns when it wants to be drawn next.
    pub fn animate(&mut self, now_ms: u64) -> Option<u64> {
        let open = self.open.as_mut().filter(|open| open.slide < 1.0)?;
        let since = *open.since.get_or_insert(now_ms);
        let progress = now_ms.saturating_sub(since) as f32 / SLIDE_MS;
        open.slide = if progress < 1.0 && !self.still { motion::ease(progress) } else { 1.0 };
        (open.slide < 1.0).then_some(now_ms + FRAME_MS)
    }

    /// Takes where the pointer rests in the page area. Returns whether what shows changed with it.
    pub fn point(&mut self, at: Option<Point>) -> bool {
        let over =
            |pointer: Option<Point>| pointer.and_then(|at| self.hot.iter().position(|(rect, _)| rect.contains(at)));
        let changed = over(self.pointer) != over(at);
        self.pointer = at;
        changed
    }

    /// Whether the pointer rests on something to press.
    pub fn pressable(&self) -> bool {
        self.pointer.is_some_and(|at| self.at(at).is_some())
    }

    /// Whether a point of the page area is on the version that is held back or on one of the buttons,
    /// where the page has nothing of its own.
    pub fn covers(&self, at: Point) -> bool {
        self.at(at).is_some() || self.held.is_some_and(|held| held.contains(at))
    }

    fn at(&self, at: Point) -> Option<Press> {
        self.hot.iter().find(|(rect, _)| rect.contains(at)).map(|(_, press)| *press)
    }

    /// Takes a press at a point of the page area. Returns whether it was on something of the review's
    /// own, which the page then leaves alone. A press on the held-back version brings it across.
    pub fn press(&mut self, at: Point) -> bool {
        self.pressed = self.at(at);
        let held = self.pressed.is_none() && self.held.is_some_and(|held| held.contains(at));
        if held {
            self.bring(Side::First);
        }
        self.pressed.is_some() || held
    }

    /// Whether a press on a button is still down.
    pub fn pressed(&self) -> bool {
        self.pressed.is_some()
    }

    /// Lets go at a point of the page area. A button does what it is for when the press ends on it.
    pub fn release(&mut self, at: Point) -> Option<Reviewing> {
        let pressed = self.pressed.take().filter(|pressed| self.at(at) == Some(*pressed))?;
        match pressed {
            Press::Keep => Some(Reviewing::Keep),
            Press::Use => Some(Reviewing::Use),
            Press::Bring(side) => {
                self.bring(side);
                None
            }
        }
    }

    fn bring(&mut self, side: Side) {
        let slides = self.arrangement == Some(Arrangement::Pinned) && !self.still;
        if let Some(open) = &mut self.open {
            open.turn(side, slides);
        }
    }

    /// Draws what stands round the two versions and the held-back one itself, where `split` has them.
    /// `shift` is where the page area shows the top left of what the split counts from, and `room` the
    /// size of the page area. What lies over the page's own text goes to `over`.
    pub fn paint(&mut self, split: &Split, shown: &Shown<'_>, out: &mut DisplayList, over: &mut DisplayList) {
        self.hot.clear();
        (self.held, self.arrangement) = (None, Some(split.arrangement));
        let Some(open) = self.open.as_ref() else { return };
        let Shown { shift, room, scheme, shaper, .. } = *shown;
        let (pointer, down) = (self.pointer, self.pressed);
        // A button is lit under the pointer, unless a press that began elsewhere is still down.
        let lit = |rect: Rect, press: Press| {
            pointer.is_some_and(|at| rect.contains(at)) && down.is_none_or(|down| down == press)
        };
        let ink = |ink: Ink| resolve(ink, scheme, 1.0);
        let measure = open.laid.as_ref().map_or(0.0, |laid| laid.0);
        let view = Rect::new(-shift.x, -shift.y, room.width, room.height);
        for piece in split.pieces.iter().filter(|piece| piece.block.is_some()) {
            let frame = frame(piece, measure, shift, room.width);
            if frame.y + frame.height < 0.0 || frame.y > room.height {
                continue;
            }
            let (first, tone) = match piece.side {
                Side::First => (true, TONES[0].1),
                Side::Second => (false, TONES[1].1),
            };
            out.push(Item::Stroke { rect: frame, color: ink(tokens::LINE), width: 1.0, radius: 0.0 });
            out.push(fill(Rect { height: RULE, ..frame }, ink(tone)));
            let head = Head {
                version: open.of(!first),
                first,
                gone: open.held.is_none(),
                can: shown.choosing,
                now: shown.now,
            };
            self.hot.extend(head.paint(frame, shaper, scheme, &lit, out));
            if !first {
                continue;
            }
            self.held = Some(frame);
            // The held-back version is painted here. The page paints its own.
            let text = text(piece, shift);
            let origin = Point { x: piece.origin.x + shift.x, y: piece.origin.y + shift.y };
            out.push(Item::PushClip { rect: text, radius: 0.0 });
            let from = out.items.len();
            if open.held.is_none() {
                // Said on the line the block would start on, level with the first line of the other version.
                let gone = shaper.shape("Not in this version", &TextStyle::control().size(tokens::TEXT_SM), None);
                let line = open.first.caret(&Pos::new(open.block.clone(), 0));
                let y = line.map_or(piece.band.0, |line| line.y + ((line.height - gone.size.height) / 2.0).round());
                gone.paint(Point { x: 0.0, y }, ink(tokens::FG_MUTED), out);
            }
            if let Some(cut) = split.shown(Side::First, view) {
                for words in &open.words[0] {
                    mark(&open.first.selection_in(words, cut), Side::First, &TONES, scheme, out);
                }
                open.first.paint(cut, out);
            }
            out.translate(from, origin);
            out.push(Item::PopClip);
        }
        // Of a pinned pair the window shows one version whole. The other lies under a veil, with a
        // button at the window's edge that brings it across.
        if split.arrangement == Arrangement::Pinned && open.slide >= 1.0 {
            let aside = if open.active == Side::First { Side::Second } else { Side::First };
            let piece = split.pieces.iter().find(|piece| piece.block.is_some() && piece.side == aside);
            let frame = piece.map(|piece| frame(piece, measure, shift, room.width));
            let lit = |button: Rect| lit(button, Press::Bring(aside));
            let button = frame.and_then(|frame| veil(frame, aside, room, shown.page, &lit, scheme, over));
            self.hot.extend(button.map(|button| (button, Press::Bring(aside))));
        }
    }

    /// Tells assistive technology of the two choices and of the version that is held back, under the
    /// page's node.
    pub fn describe(&self, page: &mut Node, nodes: &mut Vec<(NodeId, Node)>) {
        let Some(open) = &self.open else { return };
        if let Some(held) = self.held {
            let by = threads::named(&open.of(false).author);
            let text = open.held.as_ref().map(Block::plain_text);
            let text = text.unwrap_or_else(|| "Not in this version".into());
            describe::held_back(&open.block, &format!("Version held back, by {by}"), &text, held, page, nodes);
        }
        for (rect, press) in &self.hot {
            let (keep, label) = match press {
                Press::Keep => (true, "Keep this version"),
                Press::Use => (false, "Use this version"),
                Press::Bring(_) => continue,
            };
            let by = threads::named(&open.of(keep).author);
            let place = if keep { format!("By {by}, on the page now") } else { format!("By {by}, held back") };
            describe::choice(&open.block, keep, label, &place, *rect, page, nodes);
        }
    }

    /// What a press by assistive technology on a choice about `block` asks.
    pub fn asked(&self, block: &BlockId, keep: bool) -> Option<Reviewing> {
        let open = self.open.as_ref().is_some_and(|open| &open.block == block);
        open.then_some(if keep { Reviewing::Keep } else { Reviewing::Use })
    }
}

/// What the versions are drawn with and against.
#[derive(Clone, Copy)]
pub(crate) struct Shown<'a> {
    /// Where the page area shows the top left of what the split counts from.
    pub shift: Point,
    /// The size of the page area.
    pub room: Size,
    /// The time that the versions are told against, as RFC 3339.
    pub now: &'a str,
    /// Whether a version can be chosen: not on a document whose notes cannot be changed.
    pub choosing: bool,
    pub page: Color,
    pub scheme: Scheme,
    pub shaper: &'a dyn TextShaper,
}

/// Where the frame round a version stands in the page area: round its text and the margins beside it,
/// kept inside the window where one version stands alone at the page's width.
pub(crate) fn frame(piece: &Piece, measure: f32, shift: Point, room: f32) -> Rect {
    let left = piece.origin.x - MARGIN + INSET;
    let right = piece.origin.x + measure + MARGIN - INSET;
    let (left, right) = match piece.rect.width >= room {
        true => (left.max(INSET), right.min(room - INSET)),
        false => (left, right),
    };
    let top = piece.rect.y + ABOVE;
    Rect::new((left + shift.x).round(), top + shift.y, (right - left).round(), piece.rect.height - ABOVE)
}

/// Where a part's text shows in the page area: no more of the part than its stretch of the page fills,
/// since one of two versions side by side can be the shorter.
pub(crate) fn text(piece: &Piece, shift: Point) -> Rect {
    let tall = (piece.band.1 - piece.band.0).min(piece.rect.height - piece.head);
    Rect::new(piece.rect.x + shift.x, piece.rect.y + piece.head + shift.y, piece.rect.width, tall.max(0.0))
}

/// Where the split shows the top left of the page's own column, for what is `y` down the column: of the
/// part that holds it, or of the nearest.
pub(crate) fn origin(split: &Split, y: f32) -> Point {
    let own = || split.pieces.iter().filter(|piece| piece.side == Side::Second);
    let within = own().find(|piece| piece.band.0 <= y && y < piece.band.1);
    let nearest = || if y < 0.0 { own().next() } else { own().next_back() };
    within.or_else(nearest).map_or(Point { x: 0.0, y: 0.0 }, |piece| piece.origin)
}

/// A point of what the split counts from as the page's own column counts it: in the part of the page
/// that shows there, or in the nearest above or below. A point beside a part's text, or on the line at the
/// head of a version, is at the nearest text of that part.
pub(crate) fn within(split: &Split, at: Point) -> Point {
    let own = split.pieces.iter().filter(|piece| piece.side == Side::Second);
    let off = |piece: &&Piece| (piece.rect.y - at.y).max(at.y - piece.rect.y - piece.rect.height).max(0.0);
    let Some(piece) = own.min_by(|a, b| off(a).total_cmp(&off(b))) else { return at };
    let last = (piece.band.1 - 0.5).max(piece.band.0);
    Point { x: at.x - piece.origin.x, y: (at.y - piece.origin.y).clamp(piece.band.0, last) }
}

/// The part of the page's own column that a split shows in `view`, a part of what it counts from: where
/// that starts down the column and how tall it is. Where only the held-back version shows, nothing at the
/// top of the page's version.
pub(crate) fn shown(split: &Split, view: Rect) -> (f32, f32) {
    match split.shown(Side::Second, view) {
        Some(shown) => (shown.y, shown.height),
        None => {
            let pair = split.pieces.iter().find(|piece| piece.block.is_some() && piece.side == Side::Second);
            (pair.map_or(0.0, |piece| piece.band.0), 0.0)
        }
    }
}

/// The parts of the page's own column that a split shows, each with the part of the page area it is cut
/// to and where the page area shows the column's top left for it.
pub(crate) fn cuts(split: &Split, shift: Point, room: Size) -> Vec<(Rect, Point)> {
    let own = split.pieces.iter().filter(|piece| piece.side == Side::Second);
    let placed =
        own.map(|piece| (text(piece, shift), Point { x: piece.origin.x + shift.x, y: piece.origin.y + shift.y }));
    placed.filter(|(text, _)| text.y + text.height > 0.0 && text.y < room.height).collect()
}

/// Where the two versions of the open conflict stand down what the split counts from: their top and
/// their bottom.
pub(crate) fn pair(split: &Split) -> Option<(f32, f32)> {
    let parts = split.pieces.iter().filter(|piece| piece.block.is_some());
    parts.map(|piece| (piece.rect.y, piece.rect.y + piece.rect.height)).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
}

/// The line at the head of a version.
struct Head<'a> {
    version: &'a Version,
    /// Whether it is the version held back, and whether that took the block out.
    first: bool,
    gone: bool,
    /// Whether it can be chosen.
    can: bool,
    now: &'a str,
}

impl Head<'_> {
    /// Draws the line inside the top of `frame`: who wrote the version and when, where it stands, and
    /// the button that chooses it. Says less where the frame is narrow. Returns where the button is.
    fn paint(
        &self,
        frame: Rect,
        shaper: &dyn TextShaper,
        scheme: Scheme,
        lit: &dyn Fn(Rect, Press) -> bool,
        out: &mut DisplayList,
    ) -> Option<(Rect, Press)> {
        let ink = |ink: Ink| resolve(ink, scheme, 1.0);
        let small = TextStyle::control().size(tokens::TEXT_SM);
        let name = threads::named(&self.version.author);
        let (stands, label, sign_of, press) = match (self.first, self.gone) {
            (true, true) => ("Took this out", "Use this version", icon::ARROW_LEFT_RIGHT, Press::Use),
            (true, false) => ("Held back", "Use this version", icon::ARROW_LEFT_RIGHT, Press::Use),
            (false, _) => ("On the page now", "Keep this version", icon::CHECK, Press::Keep),
        };
        let shape = |text: &str, style: &TextStyle| shaper.shape(text, style, None);
        let by = shape(name, &small.weight(600));
        let when = shape(&opening::ago(self.now, &self.version.edited), &small);
        let stands = shape(stands, &small);
        let label = shape(label, &small.weight(500));

        let (left, right) = (frame.x + MARGIN - INSET, frame.x + frame.width - MARGIN + INSET);
        let middle = frame.y + RULE + ((HEAD - RULE) / 2.0).round();
        let wide = if self.can { 2.0 * PAD + SIGN + tokens::SPACE_XS + label.size.width } else { 0.0 };
        let gap = tokens::SPACE_SM;
        // The name and the button stay. The time goes first, then where the version stands.
        let needs = |words: &[&Shaped]| {
            FACE + gap + by.size.width + gap + wide + words.iter().map(|word| word.size.width + gap).sum::<f32>()
        };
        let room = right - left;
        let (timed, placed) = (needs(&[&when, &stands]) <= room, needs(&[&stands]) <= room);
        let text = |word: &Shaped, x: f32, color: Ink, out: &mut DisplayList| {
            word.paint(Point { x, y: middle - (word.size.height / 2.0).round() }, ink(color), out);
        };

        out.push(Item::PushClip { rect: Rect::new(left, frame.y, (right - left).max(0.0), HEAD), radius: 0.0 });
        face(name, Point { x: left, y: middle }, shaper, scheme, out);
        let mut x = left + FACE + gap;
        text(&by, x, tokens::FG, out);
        x += by.size.width + gap;
        if timed {
            text(&when, x, tokens::FG_MUTED, out);
        }
        let button = Rect::new(right - wide, middle - tokens::CONTROL_HEIGHT_SM / 2.0, wide, tokens::CONTROL_HEIGHT_SM);
        if placed {
            text(&stands, button.x - gap - stands.size.width, tokens::FG_MUTED, out);
        }
        out.push(Item::PopClip);
        if !self.can || button.x < left {
            return None;
        }
        let radius = tokens::RADIUS_CONTROL;
        out.push(Item::Fill { rect: button, color: ink(tokens::SURFACE), radius });
        if lit(button, press) {
            out.push(Item::Fill { rect: button, color: ink(tokens::NEUTRAL_SOFT), radius });
        }
        out.push(Item::Stroke { rect: button, color: ink(tokens::LINE_STRONG), width: 1.0, radius });
        sign(sign_of, Point { x: button.x + PAD, y: middle }, ink(tokens::FG), out);
        text(&label, button.x + PAD + SIGN + tokens::SPACE_XS, tokens::FG, out);
        Some((button, press))
    }
}

/// Two layouts of a document as one page in a page area `room` wide, cut at each of `blocks` that both
/// hold: with room over each version of a block for the line at its head, and room under its text. `pinned`
/// is the version the window is pinned to where it cannot show both whole, and how far the pair has slid
/// to that version's end.
pub(crate) fn split(
    first: &DocLayout,
    second: &DocLayout,
    blocks: &[BlockId],
    room: f32,
    (active, slide): (Side, f32),
) -> Split {
    let head = ABOVE + HEAD + FOOT;
    let window = Window { width: room, margin: MARGIN, head, active, slide };
    let mut split = Split::new(first, second, blocks, &window);
    // The layout knows of room over a version and of none under it, so some of what was asked for over
    // each goes under it here.
    for piece in split.pieces.iter_mut().filter(|piece| piece.block.is_some()) {
        piece.head -= FOOT;
        piece.origin.y -= FOOT;
    }
    split
}

/// Draws what lies over the version of a pinned pair that the window does not show whole: a veil, and at
/// the window's edge the round button that brings the version across. `frame` is where the version stands
/// in the page area, which is `room` large, and `lit` says whether the button is to show as under the
/// pointer. Returns where the button is. Nothing is drawn for a version out of view.
pub(crate) fn veil(
    frame: Rect,
    aside: Side,
    room: Size,
    page: Color,
    lit: &dyn Fn(Rect) -> bool,
    scheme: Scheme,
    over: &mut DisplayList,
) -> Option<Rect> {
    if frame.y + frame.height <= 0.0 || frame.y >= room.height {
        return None;
    }
    let ink = |ink: Ink| resolve(ink, scheme, 1.0);
    let seen = Rect::new(frame.x.max(0.0), frame.y, frame.width, frame.height);
    let seen = Rect { width: (frame.x + frame.width).min(room.width) - seen.x, ..seen };
    over.push(fill(seen, Color { a: (255.0 * ASIDE) as u8, ..page }));
    let (top, bottom) = (frame.y.max(0.0), (frame.y + frame.height).min(room.height));
    let x = if aside == Side::First { tokens::SPACE_LG } else { room.width - tokens::SPACE_LG - BRING };
    let button = Rect::new(x, ((top + bottom - BRING) / 2.0).round(), BRING, BRING);
    over.push(Item::Fill { rect: button, color: ink(tokens::SURFACE_RAISED), radius: BRING / 2.0 });
    if lit(button) {
        over.push(Item::Fill { rect: button, color: ink(tokens::NEUTRAL_SOFT), radius: BRING / 2.0 });
    }
    over.push(Item::Stroke { rect: button, color: ink(tokens::LINE_STRONG), width: 1.0, radius: BRING / 2.0 });
    let arrow = if aside == Side::First { icon::CHEVRON_LEFT } else { icon::CHEVRON_RIGHT };
    sign(arrow, Point { x: button.x + (BRING - SIGN) / 2.0, y: button.y + BRING / 2.0 }, ink(tokens::FG), over);
    Some(button)
}

/// Draws the face of who wrote a version: a round with the first letters of the name, its left edge at
/// `at` and its middle at that height.
pub(crate) fn face(name: &str, at: Point, shaper: &dyn TextShaper, scheme: Scheme, out: &mut DisplayList) {
    let ink = |ink: Ink| resolve(ink, scheme, 1.0);
    let round = Rect::new(at.x, at.y - FACE / 2.0, FACE, FACE);
    out.push(Item::Fill { rect: round, color: ink(tokens::NEUTRAL_SOFT), radius: FACE / 2.0 });
    let letters = shaper.shape(&threads::initials(name), &TextStyle::control().size(9.0).weight(600), None);
    let x = round.x + ((FACE - letters.size.width) / 2.0).round();
    letters.paint(Point { x, y: at.y - (letters.size.height / 2.0).round() }, ink(tokens::FG_MUTED), out);
}

/// Draws an icon [`SIGN`] large, with its left edge at `at` and its middle at that height.
fn sign(icon: Icon, at: Point, color: Color, out: &mut DisplayList) {
    out.push(Item::Path {
        origin: Point { x: at.x, y: at.y - SIGN / 2.0 },
        scale: SIGN / icon::GRID,
        path: icon.outline(),
        color,
        stroke: (!icon.is_filled()).then_some(tokens::ICON_STROKE),
    });
}

fn fill(rect: Rect, color: Color) -> Item {
    Item::Fill { rect, color, radius: 0.0 }
}

/// The soft and the strong colour of what only the first of two versions has, then of what only the
/// second has.
pub(crate) type Tones = [(Ink, Ink); 2];

/// The two sides of a conflict are amber and blue: colours that put neither before the other.
pub(crate) const TONES: Tones = [(tokens::WARN_SOFT, tokens::WARN), (tokens::INFO_SOFT, tokens::INFO)];

/// Marks the words of one version that the other does not have, under their text and in that version's
/// colours: with a dotted line for the first version and a whole line for the second, so that the two are
/// told apart without their colours.
pub(crate) fn mark(lines: &[Rect], side: Side, tones: &Tones, scheme: Scheme, out: &mut DisplayList) {
    let (soft, strong) = tones[if side == Side::First { 0 } else { 1 }];
    let (soft, strong) = (resolve(soft, scheme, 1.0), resolve(strong, scheme, 1.0));
    for line in lines.iter().filter(|line| line.width > 0.0) {
        out.push(Item::Fill { rect: *line, color: soft, radius: 2.0 });
        let y = line.y + line.height - UNDER;
        let (on, off) = if side == Side::First { (DOT, DOT) } else { (line.width, 0.0) };
        let mut x = line.x;
        while x < line.x + line.width {
            out.push(fill(Rect::new(x, y, on.min(line.x + line.width - x), UNDER), strong));
            x += on + off;
        }
    }
}

/// The banner over the page while conflicts wait: how many there are, which of them is open, and the
/// buttons that step through them.
pub(crate) fn bar(count: usize, open: Option<usize>) -> Element<Message> {
    let said = match count {
        1 => "1 passage changed in two places needs a choice. Both versions stay in history.".to_owned(),
        count => format!("{count} passages changed in two places need a choice. Both versions stay in history."),
    };
    let send = |asked: Reviewing| Message::Review(asked);
    let step = |icon, label: &str, back| {
        Button::icon_only(icon, label).subtle().small().on_press(send(Reviewing::Step { back }))
    };
    let mut bar = Banner::new(said).tone(Tone::Warn).icon(icon::GIT_MERGE);
    if let Some(open) = open {
        let which = format!("{} of {count}", open + 1);
        bar = bar.action(Element::text(which, TextStyle::control().size(tokens::TEXT_SM)).fg(tokens::FG_MUTED));
    }
    bar = bar.action(step(icon::CHEVRON_UP, "Previous conflict", true));
    bar = bar.action(step(icon::CHEVRON_DOWN, "Next conflict", false));
    match open {
        Some(_) => bar.on_dismiss("Close conflict", send(Reviewing::Close)).into(),
        None => bar.into(),
    }
}

/// How far apart two versions of a block read: how much text only one of them has, in bytes. Nothing
/// where they read alike.
fn apart(a: &Block, b: &Block) -> usize {
    let long = |words: &Selection| words.head.offset.abs_diff(words.anchor.offset);
    differing(a, b).iter().flatten().map(long).sum()
}

/// The words in which two versions of a block differ, as selections in each: the held-back version's
/// first. Text that only one of them has differs whole.
pub(crate) fn differing(held: &Block, page: &Block) -> [Vec<Selection>; 2] {
    fn texts<'a>(block: &'a Block, out: &mut Vec<(&'a BlockId, String)>) {
        if matches!(block.body, Body::Text(_)) {
            out.push((&block.id, block.plain_text()));
        }
        block.body.blocks().iter().for_each(|part| texts(part, out));
    }
    let (mut was, mut now) = (Vec::new(), Vec::new());
    texts(held, &mut was);
    texts(page, &mut now);
    let stretch = |id: &BlockId, range: Range<usize>| Selection {
        anchor: Pos::new(id.clone(), range.start),
        head: Pos::new(id.clone(), range.end),
    };
    let mut marks: [Vec<Selection>; 2] = Default::default();
    for (id, text) in &was {
        let other = now.iter().find(|(other, _)| other == id).map_or("", |(_, text)| text.as_str());
        let (gone, came) = words(text, other);
        marks[0].extend(gone.into_iter().map(|range| stretch(id, range)));
        marks[1].extend(came.into_iter().map(|range| stretch(id, range)));
    }
    for (id, text) in now.iter().filter(|(id, _)| !was.iter().any(|(other, _)| other == id)) {
        marks[1].extend(words("", text).1.into_iter().map(|range| stretch(id, range)));
    }
    marks
}

/// A text cut into its words and what stands between them: each run of letters and digits, each run of
/// white space, and every other character by itself.
fn tokens_of(text: &str) -> Vec<Range<usize>> {
    #[derive(PartialEq)]
    enum Sort {
        Word,
        Space,
        Other,
    }
    let sort = |c: char| match c {
        c if c.is_alphanumeric() || c == '\'' || c == '\u{2019}' => Sort::Word,
        c if c.is_whitespace() => Sort::Space,
        _ => Sort::Other,
    };
    let mut tokens: Vec<Range<usize>> = Vec::new();
    let mut last = Sort::Other;
    for (at, c) in text.char_indices() {
        let this = sort(c);
        match tokens.last_mut() {
            Some(token) if this == last && this != Sort::Other => token.end = at + c.len_utf8(),
            _ => tokens.push(at..at + c.len_utf8()),
        }
        last = this;
    }
    tokens
}

/// The stretches of each of two texts that the other does not have, word by word. Stretches that only
/// white space parts are one, and none begins or ends with white space.
fn words(a: &str, b: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let (cut_a, cut_b) = (tokens_of(a), tokens_of(b));
    let (of_a, of_b) = (|i: usize| &a[cut_a[i].clone()], |i: usize| &b[cut_b[i].clone()]);
    let (long_a, long_b) = (cut_a.len(), cut_b.len());
    // What the two start and end with alike is no part of the search.
    let lead = (0..long_a.min(long_b)).take_while(|i| of_a(*i) == of_b(*i)).count();
    let most = long_a.min(long_b) - lead;
    let tail = (1..=most).take_while(|i| of_a(long_a - i) == of_b(long_b - i)).count();
    let (n, m) = (long_a - lead - tail, long_b - lead - tail);
    let (mut same_a, mut same_b) = (vec![false; long_a], vec![false; long_b]);
    same_a[..lead].fill(true);
    same_b[..lead].fill(true);
    same_a[long_a - tail..].fill(true);
    same_b[long_b - tail..].fill(true);
    if n > 0 && m > 0 && n.saturating_mul(m) <= MOST {
        // The longest run of words that both keep in order, by its length from each pair of places on.
        let mut longest = vec![0u32; (n + 1) * (m + 1)];
        let at = |i: usize, j: usize| i * (m + 1) + j;
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                longest[at(i, j)] = match of_a(lead + i) == of_b(lead + j) {
                    true => longest[at(i + 1, j + 1)] + 1,
                    false => longest[at(i + 1, j)].max(longest[at(i, j + 1)]),
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if of_a(lead + i) == of_b(lead + j) {
                (same_a[lead + i], same_b[lead + j]) = (true, true);
                (i, j) = (i + 1, j + 1);
            } else if longest[at(i + 1, j)] >= longest[at(i, j + 1)] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    (stretches(a, &cut_a, &same_a), stretches(b, &cut_b, &same_b))
}

/// The stretches of a text made of the tokens that are not `same`, joined over white space.
fn stretches(text: &str, tokens: &[Range<usize>], same: &[bool]) -> Vec<Range<usize>> {
    let space = |token: &&Range<usize>| text[(*token).clone()].chars().all(char::is_whitespace);
    let mut found: Vec<Range<usize>> = Vec::new();
    // Whether nothing but white space has come since the last stretch, which then goes on.
    let mut open = false;
    for (token, same) in tokens.iter().zip(same).filter(|(token, _)| !space(token)) {
        match (*same, found.last_mut()) {
            (false, Some(last)) if open => last.end = token.end,
            (false, _) => found.push(token.clone()),
            (true, _) => {}
        }
        open = !*same;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_words_that_differ_are_found_in_each_text() {
        let cases = [
            // (one text, the other, what only the first has, what only the second has)
            (
                "start on 10 March.",
                "start once the scaffold is up.",
                vec!["on 10 March"],
                vec!["once the scaffold is up"],
            ),
            ("one two three", "one three", vec!["two"], vec![]),
            ("one three", "one two three", vec![], vec!["two"]),
            (
                "up for three weeks. Gutters",
                "up until repointed. Gutters",
                vec!["for three weeks"],
                vec!["until repointed"],
            ),
            ("a b c d", "a x c y", vec!["b", "d"], vec!["x", "y"]),
            ("same text", "same text", vec![], vec![]),
            ("", "all of it", vec![], vec!["all of it"]),
            ("caf\u{e9} cr\u{e8}me", "caf\u{e9} noir", vec!["cr\u{e8}me"], vec!["noir"]),
            ("it's here", "its here", vec!["it's"], vec!["its"]),
        ];
        for (a, b, only_a, only_b) in cases {
            let (gone, came) = words(a, b);
            let read = |text: &'static str, found: Vec<Range<usize>>| -> Vec<&str> {
                found.into_iter().map(|range| &text[range]).collect()
            };
            assert_eq!((read(a, gone), read(b, came)), (only_a, only_b), "{a:?} against {b:?}");
        }
    }

    #[test]
    fn texts_too_long_to_compare_word_by_word_differ_between_what_they_share_at_their_ends() {
        let middle = |word: &str| vec![word; 1100].join(" ");
        let (a, b) = (format!("first {} last", middle("one")), format!("first {} last", middle("two")));
        let (gone, came) = words(&a, &b);
        let whole = |text: &str| Some(6..text.len() - 5);
        assert_eq!((gone.len(), gone.first().cloned()), (1, whole(&a)));
        assert_eq!((came.len(), came.first().cloned()), (1, whole(&b)));
    }
}
