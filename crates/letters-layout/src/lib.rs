//! Lays a document out and answers questions about where things are.
//!
//! [`DocLayout`] is the whole interface for documents: update it after each edit, then paint a viewport,
//! hit-test a point or ask where a caret sits. [`Shaper`] does the same job for the plain strings of widgets.
//!
//! Layout is incremental and viewport-first. Opening a document shapes no text: every block starts with a
//! height estimated from the amount of text it holds, and is laid out with Parley the first time something
//! asks about it. An edit redoes only the blocks it touched. [`DocLayout::settle`] replaces the remaining
//! estimates a little at a time while the shell is idle.
//!
//! Text takes its colours from the document's style and everything else from a [`Palette`]. Layout has no
//! theme of its own.
//!
//! The Markdown markers around the caret are drawn over the layout and are no part of it. See
//! [`DocLayout::reveal`].
//!
//! Pages exist only on request: [`DocLayout::paginate`] splits the same column into sheets for Print view
//! and export, and typing never pays for it.

mod engine;
mod flow;
mod ink;
mod leaf;
mod paginate;
mod palette;
mod reveal;
mod shape;
mod theme;
mod tokens;

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::ops::Range;

use letters_model::{BlockId, Body, Change, Document, Markers, Page, Pos, Selection, Style};
use letters_render::{DisplayList, Font, Item, Point, Rect};
use parley::{Language, Layout};

use crate::engine::Engine;
use crate::flow::{Frame, Leaf, Look, Role, Types, What};
use crate::ink::{Brush, Ink, Inks};
use crate::leaf::{Over, Painter};
use crate::reveal::{Reveal, Shapes, Shifts};

pub use crate::paginate::{Pages, Sheet};
pub use crate::palette::{Callout, Palette, Scheme, Syntax};
pub use crate::shape::{FontSpec, ShapedRun, ShapedText, Shaper};
pub use crate::tokens::{Token, TokenKind};

/// The fonts layout may use. The shell supplies them, since the core never reads files.
///
/// Only these faces are used, never the system's. A family the style names but these lack falls back to the
/// first proportional face here, or the first monospace one for code.
#[derive(Clone, Debug, Default)]
pub struct Fonts {
    pub faces: Vec<Font>,
}

/// How many leaves keep their layout. Enough for several screens, so scrolling back costs nothing, while a
/// long document's memory stays flat.
const KEPT: usize = 512;

/// How many edited blocks are laid out again at once. An edit beyond that settles like unseen text.
const EAGER: usize = 32;

/// A laid-out document. Coordinates are logical pixels from the top left of the page column.
///
/// Queries take `&self` but lay blocks out on demand, so [`DocLayout::height`] and the position of blocks
/// further down can shift as estimates above them are replaced.
pub struct DocLayout {
    state: RefCell<State>,
}

impl Default for DocLayout {
    fn default() -> Self {
        Self::new(Fonts::default())
    }
}

impl fmt::Debug for DocLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.borrow();
        f.debug_struct("DocLayout")
            .field("width", &state.width)
            .field("leaves", &state.leaves.len())
            .field("estimated", &state.pending)
            .finish_non_exhaustive()
    }
}

impl DocLayout {
    pub fn new(fonts: Fonts) -> Self {
        let state = State {
            engine: Engine::new(&fonts),
            types: Types::default(),
            palette: Palette::default(),
            style: None,
            locale: None,
            width: 0.0,
            leaves: Vec::new(),
            frames: Vec::new(),
            index: HashMap::new(),
            tokens: HashMap::new(),
            spelling: HashMap::new(),
            reveal: Reveal::default(),
            shapes: Shapes::default(),
            composition: None,
            stale: 0,
            height: 0.0,
            pending: 0,
            held: 0,
            clock: 0,
            scratch: Layout::new(),
        };
        Self { state: RefCell::new(state) }
    }

    /// Brings the layout up to date with `doc`. `change` is `None` for a first layout or a change of width or
    /// style, and otherwise names what an edit touched.
    pub fn update(&mut self, doc: &Document, style: &Style, width: f32, change: Option<&Change>) {
        self.state.get_mut().update(doc, style, width, change);
    }

    /// Sets the colours to paint with, [`Palette::LIGHT`] until then. Nothing is laid out again, so a change
    /// of theme costs one paint.
    pub fn set_palette(&mut self, palette: Palette) {
        self.state.get_mut().palette = palette;
    }

    /// Colours the text of a code block: each token in the palette's colour for its kind, and the rest in the
    /// style's. Tokens are in order and do not overlap. Nothing is laid out again, so this costs one paint.
    ///
    /// They follow the text through edits until the block is lexed again and they are set anew. Tokens for
    /// a block that is not code, or not in the document, are dropped.
    pub fn set_tokens(&mut self, block: &BlockId, tokens: Vec<Token>) {
        self.state.get_mut().set_tokens(block, tokens);
    }

    /// Shows the Markdown markers around the caret: `at` is the caret and what the editor's `markers()`
    /// gives for it, or `None` to show none. Call it after every call to the editor, since markers change
    /// while the caret stays put. Returns whether a marker appears or goes.
    ///
    /// Markers never change the layout. One inside a line pushes the rest of that line aside, past the text
    /// edge when the line is full, and no other line moves. A heading's hangs left of the text edge, or as
    /// far left as the bar of the quote or callout around it allows, with its first line making room. A
    /// callout's sits in the corner of its box. [`DocLayout::caret`], [`DocLayout::hit`] and the other
    /// queries answer for the page as drawn, so a caret among markers at its offset sits after the first
    /// `before_caret` of them.
    ///
    /// A change is shown at once. To animate it, see [`DocLayout::set_reveal_progress`].
    pub fn reveal(&mut self, at: Option<(&Pos, &Markers)>) -> bool {
        self.state.get_mut().reveal(at)
    }

    /// How far the markers are through their last change, from 0 for the page as it was to 1 for the page as
    /// it will be. Markers fade and the text beside them slides, with nothing shaped or laid out again.
    ///
    /// [`DocLayout::reveal`] leaves this at 1 when it returns `true`. To animate, set 0 straight after and
    /// step to 1 over the time the fade takes.
    pub fn set_reveal_progress(&mut self, progress: f32) {
        self.state.get_mut().reveal.set_progress(progress);
    }

    /// Underlines the text an input method is composing: a block and a byte range in it, as the editor's
    /// `composition()` gives them.
    pub fn set_composition(&mut self, composing: Option<(&BlockId, Range<usize>)>) {
        self.state.get_mut().composition = composing.map(|(block, range)| (block.clone(), range));
    }

    /// Marks the misspelled words of a block with a wavy line in the palette's `spelling` colour. `ranges` are
    /// byte ranges of the block's text. Nothing is laid out again, so this costs one paint.
    ///
    /// The marks follow the text through edits until the block is checked again and they are set anew. A
    /// word the input method is still composing shows none.
    pub fn set_spelling(&mut self, block: &BlockId, ranges: Vec<Range<usize>>) {
        self.state.get_mut().set_spelling(block, ranges);
    }

    /// The misspelled word at a position, as set with [`DocLayout::set_spelling`] and carried through edits
    /// since. Either end of the word counts, so [`DocLayout::hit`] of a point on its first or last letter
    /// finds it.
    pub fn spelling_at(&self, pos: &Pos) -> Option<Range<usize>> {
        let state = self.state.borrow();
        let found =
            state.spelling.get(&pos.block)?.iter().find(|range| (range.start..=range.end).contains(&pos.offset));
        found.cloned()
    }

    /// The height of the whole document. Partly an estimate until [`DocLayout::settle`] returns `false`.
    pub fn height(&self) -> f32 {
        let state = &mut *self.state.borrow_mut();
        state.place_all();
        state.height
    }

    /// Emits the part of the document that falls inside `viewport`.
    pub fn paint(&self, viewport: Rect, out: &mut DisplayList) {
        self.state.borrow_mut().paint(viewport, false, out);
    }

    /// Splits the document into pages of `page`'s size, for Print view and export. Lay the document out at
    /// the page's text width first, which is `(width - margin_left - margin_right) * 96 / 72` logical pixels.
    ///
    /// A block with `"break": "page"` among its properties starts a page. A heading stays with the block
    /// after it. A block splits between pages only where two of its lines stay and two go, and a line of
    /// code never splits. Only something taller than a page breaks these rules.
    ///
    /// This is a pass over the heights of lines and shapes nothing, so it is cheap to run again after an
    /// edit. It first lays out whatever is still estimated, which on a long document that was never
    /// settled takes as long as settling it.
    pub fn paginate(&self, page: &Page) -> Pages {
        self.state.borrow_mut().paginate(page)
    }

    /// Emits one sheet of `pages`, with the sheet's top left at the origin: the text in its text area, as
    /// the document's style has it, and the page number beneath. The paper itself is the caller's to draw.
    /// Nothing that belongs to editing is drawn: no markers, no spelling marks and no composition.
    pub fn paint_page(&self, pages: &Pages, index: usize, out: &mut DisplayList) {
        self.state.borrow_mut().paint_page(pages, index, out);
    }

    /// The page of `pages` that shows the line `pos` is on.
    pub fn page_of(&self, pages: &Pages, pos: &Pos) -> Option<usize> {
        self.state.borrow_mut().page_of(pages, pos)
    }

    /// The text position nearest to a point. `None` when the document holds no text.
    pub fn hit(&self, point: Point) -> Option<Pos> {
        self.state.borrow_mut().hit(point)
    }

    /// Where a caret at `pos` is drawn. An offset where a line wraps belongs to the start of the next line.
    pub fn caret(&self, pos: &Pos) -> Option<Rect> {
        self.state.borrow_mut().caret(pos)
    }

    /// The rectangles that highlight a selection, one per line it touches.
    ///
    /// This lays out every block the selection covers. To paint, use [`DocLayout::selection_in`].
    pub fn selection(&self, selection: &Selection) -> Vec<Rect> {
        self.state.borrow_mut().selection(selection, None)
    }

    /// The rectangles that highlight the part of a selection that `viewport` shows, so that a selection of the
    /// whole document costs no more than one screen of it.
    pub fn selection_in(&self, selection: &Selection, viewport: Rect) -> Vec<Rect> {
        self.state.borrow_mut().selection(selection, Some(viewport))
    }

    /// The position one visual line above or below `pos`, keeping to the horizontal position `x`. `None` on
    /// the document's first or last line.
    pub fn line_step(&self, pos: &Pos, x: f32, down: bool) -> Option<Pos> {
        self.state.borrow_mut().line_step(pos, x, down)
    }

    /// The start or the end of the visual line that holds `pos`. The end of a wrapped line is before the space
    /// it wraps at, which is where a caret still draws on that line.
    pub fn line_edge(&self, pos: &Pos, end: bool) -> Option<Pos> {
        self.state.borrow_mut().line_edge(pos, end)
    }

    /// Replaces height estimates with real layouts: first for the blocks `viewport` shows, then outwards from
    /// it for about `budget` bytes of text. Returns whether any estimates remain.
    ///
    /// Call it with a budget of 0 before painting and with a few thousand bytes whenever the shell is idle.
    /// Blocks above the viewport can change height. To hold the page still, take [`DocLayout::caret`] of a
    /// visible position before and after, and scroll by the difference.
    pub fn settle(&mut self, viewport: Rect, budget: usize) -> bool {
        self.state.get_mut().settle(viewport, budget)
    }
}

struct State {
    engine: Engine,
    types: Types,
    palette: Palette,
    /// The style `types` was resolved from.
    style: Option<Style>,
    locale: Option<Language>,
    width: f32,
    leaves: Vec<Leaf>,
    frames: Vec<Frame>,
    /// Text leaves by block.
    index: HashMap<BlockId, usize>,
    /// The tokens of code blocks.
    tokens: HashMap<BlockId, Vec<Token>>,
    /// The misspelled words of text blocks.
    spelling: HashMap<BlockId, Vec<Range<usize>>>,
    /// The markers shown around the caret.
    reveal: Reveal,
    shapes: Shapes,
    /// The text an input method is composing.
    composition: Option<(BlockId, Range<usize>)>,
    /// The first leaf whose `y` is out of date.
    stale: usize,
    height: f32,
    /// How many leaves have an estimated height.
    pending: usize,
    /// How many leaves hold a layout.
    held: usize,
    /// Counts queries, to tell which layouts were used last.
    clock: u64,
    scratch: Layout<Brush>,
}

impl State {
    fn update(&mut self, doc: &Document, style: &Style, width: f32, change: Option<&Change>) {
        let width = if width.is_finite() { width.max(1.0) } else { 1.0 };
        let locale = Language::parse(&doc.language).ok();
        let restyled = self.style.as_ref() != Some(style);
        if restyled {
            self.types = Types::resolve(style, &mut self.engine);
            self.style = Some(style.clone());
            self.shapes.clear();
        }
        if let Some(change) = change {
            self.reveal.edit(&change.blocks);
        }
        let same = !restyled && width == self.width && locale == self.locale;
        (self.width, self.locale) = (width, locale);
        match change {
            Some(change) if same => {
                if change.structure || !self.edit(doc, change) {
                    self.reflow(doc, Some(change));
                }
                // What was just edited is what is on screen, so it gets its real height at once.
                for block in change.blocks.iter().take(EAGER) {
                    if let Some(&leaf) = self.index.get(block) {
                        self.lay(leaf);
                    }
                }
            }
            _ => self.reflow(doc, None),
        }
    }

    /// Takes in edits that leave the column's shape alone: new text or marks in blocks that keep their kind.
    /// Returns `false` when the change needs a reflow.
    fn edit(&mut self, doc: &Document, change: &Change) -> bool {
        for id in &change.blocks {
            let Some(&at) = self.index.get(id) else { return false };
            let leaf = &mut self.leaves[at];
            let Some(block) = doc.blocks.get(leaf.root).and_then(|root| flow::find(root, id)) else { return false };
            if leaf.kind.as_ref() != Some(&block.kind) || !matches!(block.body, Body::Text(_)) {
                return false;
            }
            // Whether a page starts here is decided where the column is built.
            if flow::breaks(block) != leaf.breaks {
                return false;
            }
            let (tokens, spelling) = (self.tokens.get_mut(id), self.spelling.get_mut(id));
            // Only what follows the text needs to know what the text was.
            let before = if tokens.is_some() || spelling.is_some() { leaf.text.clone() } else { String::new() };
            leaf.set_content(block);
            if let Some(tokens) = tokens {
                tokens::follow(tokens, &before, &leaf.text);
            }
            if let Some(spelling) = spelling {
                tokens::follow_ranges(spelling, &before, &leaf.text);
            }
            if leaf.laid.take().is_some() {
                self.held -= 1;
            }
            // The old height stays as the estimate, so nothing jumps before the leaf is laid out again.
            if std::mem::take(&mut leaf.exact) {
                self.pending += 1;
            }
        }
        true
    }

    /// Rebuilds the column. With `carry`, blocks the change did not touch keep their text and layout.
    fn reflow(&mut self, doc: &Document, carry: Option<&Change>) {
        let mut old = std::mem::take(&mut self.leaves);
        let mut index = std::mem::take(&mut self.index);
        let (tokens, spelling) = (&mut self.tokens, &mut self.spelling);
        let flow = flow::flow(doc, &self.types, self.width, |block| {
            let (change, at) = (carry?, *index.get(&block.id)?);
            if change.blocks.contains(&block.id) {
                if let Some(tokens) = tokens.get_mut(&block.id) {
                    tokens::follow(tokens, &old[at].text, &block.plain_text());
                }
                if let Some(spelling) = spelling.get_mut(&block.id) {
                    tokens::follow_ranges(spelling, &old[at].text, &block.plain_text());
                }
                return None;
            }
            Some(std::mem::take(&mut old[at]))
        });
        (self.leaves, self.frames) = (flow.leaves, flow.frames);
        index.clear();
        for (at, leaf) in self.leaves.iter().enumerate() {
            if let (What::Text, Some(id)) = (leaf.what, &leaf.id) {
                index.insert(id.clone(), at);
            }
        }
        self.index = index;
        let coded = |id: &BlockId| self.index.get(id).is_some_and(|at| self.leaves[*at].role == Role::Code);
        self.tokens.retain(|id, tokens| coded(id) && !tokens.is_empty());
        self.spelling.retain(|id, ranges| self.index.contains_key(id) && !ranges.is_empty());
        self.pending = self.leaves.iter().filter(|leaf| !leaf.exact).count();
        self.held = self.leaves.iter().filter(|leaf| leaf.laid.is_some()).count();
        self.stale = 0;
    }

    fn set_spelling(&mut self, block: &BlockId, mut ranges: Vec<Range<usize>>) {
        ranges.retain(|range| !range.is_empty());
        if self.index.contains_key(block) && !ranges.is_empty() {
            ranges.sort_by_key(|range| range.start);
            self.spelling.insert(block.clone(), ranges);
        } else {
            self.spelling.remove(block);
        }
    }

    fn reveal(&mut self, at: Option<(&Pos, &Markers)>) -> bool {
        // Markers are few, so this only guards against a caller that makes up new ones without end.
        if self.shapes.len() > 64 {
            self.shapes.clear();
        }
        self.reveal.set(at)
    }

    /// The markers revealed in a leaf that holds its layout.
    fn shifts(&mut self, at: usize) -> Shifts {
        let Self { reveal, shapes, engine, types, leaves, frames, .. } = self;
        let leaf = &leaves[at];
        let (Some(id), Some(laid)) = (&leaf.id, &leaf.laid) else { return Shifts::default() };
        // Code shows its text as written.
        if leaf.what != What::Text || leaf.role == Role::Code {
            return Shifts::default();
        }
        // Inside a quote or callout a marker hangs clear of the bar down its side, never across it.
        let room = leaf.frame.map_or(f32::INFINITY, |frame| leaf.x - frames[frame].x - theme::BAR - 2.0);
        reveal.over(id, &laid.text, room, |text| shapes.get(engine, types, leaf.role, false, text))
    }

    fn set_tokens(&mut self, block: &BlockId, mut tokens: Vec<Token>) {
        let coded = self.index.get(block).is_some_and(|at| self.leaves[*at].role == Role::Code);
        if coded && !tokens.is_empty() {
            tokens.sort_by_key(|token| token.range.start);
            self.tokens.insert(block.clone(), tokens);
        } else {
            self.tokens.remove(block);
        }
    }

    /// Brings `y` up to date for every leaf up to and including `upto`.
    fn place(&mut self, upto: usize) {
        while self.stale <= upto && self.stale < self.leaves.len() {
            let above = self.stale.checked_sub(1).map_or(0.0, |above| self.leaves[above].span().1);
            let leaf = &mut self.leaves[self.stale];
            leaf.y = above + leaf.top;
            self.stale += 1;
        }
    }

    fn place_all(&mut self) {
        self.place(usize::MAX);
        self.height = self.leaves.last().map_or(0.0, |last| last.span().1);
    }

    /// Makes sure a leaf holds its layout, which also makes its height exact.
    fn lay(&mut self, at: usize) {
        self.leaves[at].used = self.clock;
        if self.leaves[at].laid.is_some() || matches!(self.leaves[at].what, What::Rule | What::Blank) {
            return;
        }
        if self.held >= KEPT {
            self.evict();
        }
        let laid = leaf::lay(&mut self.engine, &self.types, &self.leaves[at], self.locale);
        self.set_extent(at, leaf::height(&laid.text), leaf::lines(&laid.text, self.leaves[at].role == Role::Code));
        self.leaves[at].laid = Some(Box::new(laid));
        self.held += 1;
    }

    /// Gives a leaf its exact height without keeping the layout.
    fn measure(&mut self, at: usize) {
        if self.leaves[at].exact {
            return;
        }
        leaf::shape(&mut self.engine, &self.types, &self.leaves[at], self.locale, &mut self.scratch);
        let lines = leaf::lines(&self.scratch, self.leaves[at].role == Role::Code);
        self.set_extent(at, leaf::height(&self.scratch), lines);
    }

    /// Records what a layout says of a leaf: its height, and the lines pagination may split it at.
    fn set_extent(&mut self, at: usize, height: f32, (lines, joined): (u32, Vec<u32>)) {
        let leaf = &mut self.leaves[at];
        (leaf.lines, leaf.joined) = (lines, joined);
        if !std::mem::replace(&mut leaf.exact, true) {
            self.pending -= 1;
        }
        if leaf.height != height {
            leaf.height = height;
            self.stale = self.stale.min(at + 1);
        }
    }

    /// Drops the half of the kept layouts that were used longest ago.
    fn evict(&mut self) {
        let mut kept: Vec<(u64, usize)> = self
            .leaves
            .iter()
            .enumerate()
            .filter(|(_, leaf)| leaf.laid.is_some())
            .map(|(at, leaf)| (leaf.used, at))
            .collect();
        kept.sort_unstable();
        for (_, at) in &kept[..kept.len() / 2] {
            self.leaves[*at].laid = None;
        }
        self.held = kept.len() - kept.len() / 2;
    }

    /// Lays out the leaves a band of the page shows and returns their range.
    fn show(&mut self, top: f32, bottom: f32) -> (usize, usize) {
        self.clock += 1;
        self.place_all();
        let first = self.leaves.partition_point(|leaf| leaf.span().1 <= top);
        let mut end = first;
        while end < self.leaves.len() {
            self.place(end);
            if self.leaves[end].span().0 >= bottom {
                break;
            }
            self.lay(end);
            end += 1;
        }
        self.place_all();
        (first, end)
    }

    /// Paints a band of the column. `print` leaves out what belongs to editing and not to the document.
    fn paint(&mut self, viewport: Rect, print: bool, out: &mut DisplayList) {
        let band = (viewport.y, viewport.y + viewport.height);
        let (first, end) = self.show(band.0, band.1);
        let inks = Inks::new(&self.types, self.palette);
        let palette = &inks.palette;
        // Frames go behind what they hold, outermost first. They are numbered in that order.
        let mut frames: Vec<usize> = Vec::new();
        for leaf in &self.leaves[first..end] {
            let mut next = leaf.frame;
            while let Some(frame) = next.filter(|frame| !frames.contains(frame)) {
                frames.push(frame);
                next = self.frames[frame].parent;
            }
        }
        frames.sort_unstable();
        for frame in frames {
            let Self { engine, types, leaves, frames, reveal, shapes, .. } = self;
            let frame = &frames[frame];
            let (top, bottom) = frame.span(leaves);
            let rect = Rect::new(frame.x, top, frame.width, bottom - top);
            let bar = Rect { width: theme::BAR, ..rect };
            match frame.look {
                Look::Quote => out.push(Item::Fill { rect: bar, color: palette.line_strong, radius: theme::BAR / 2.0 }),
                Look::Callout(tone) => {
                    let Callout { accent, fill } = palette.callout(tone);
                    out.push(Item::Fill { rect, color: fill, radius: theme::BOX_RADIUS });
                    // The accent follows the rounded corners.
                    out.push(Item::PushClip { rect, radius: theme::BOX_RADIUS });
                    out.push(Item::Fill { rect: bar, color: accent, radius: 0.0 });
                    out.push(Item::PopClip);
                }
            }
            // A callout's tag sits in the padding above its text, where it moves nothing.
            let mut at = Point { x: frame.x + theme::BOX_PAD_X, y: top + theme::BOX_PAD_Y - 2.0 };
            let seen = !print && top < band.1 && at.y > band.0;
            for (tag, level) in reveal.beside(&frame.id).filter(|_| seen) {
                let (shape, width) = shapes.get(engine, types, Role::Body, true, tag);
                if let Some(shape) = shapes.layout(shape) {
                    Painter { engine, types, inks: &inks, band, out }.sign(shape, at, level);
                }
                at.x += width;
            }
        }
        for at in first..end {
            // A viewport taller than the cache holds has dropped its first layouts by now.
            self.lay(at);
            let shifts = if print { Shifts::default() } else { self.shifts(at) };
            let Self { engine, types, leaves, tokens, spelling, shapes, composition, .. } = self;
            let leaf = &leaves[at];
            if leaf.what == What::Rule {
                let rect = Rect::new(leaf.x, leaf.y, leaf.width, leaf.height);
                out.push(Item::Fill { rect, color: palette.line, radius: 0.0 });
            } else if let Some(laid) = &leaf.laid {
                let tokens = leaf.id.as_ref().and_then(|id| tokens.get(id));
                let own = |id: &&BlockId| !print && leaf.id.as_ref() == Some(*id);
                let composing = composition.as_ref().filter(|(block, _)| own(&block)).map(|(_, range)| range.clone());
                let spelling = leaf.id.as_ref().filter(own).and_then(|id| spelling.get(id));
                let spelling = spelling.map_or(&[][..], Vec::as_slice);
                let over = Over { shifts: &shifts, shapes, composing, spelling };
                Painter { engine, types, inks: &inks, band, out }.leaf(leaf, laid, tokens.map(Vec::as_slice), &over);
            }
        }
    }

    fn paginate(&mut self, page: &Page) -> Pages {
        for at in 0..self.leaves.len() {
            self.measure(at);
        }
        self.place_all();
        let px = |points: f32| points * theme::PT;
        let area = Rect::new(
            px(page.margin_left),
            px(page.margin_top),
            px(page.width - page.margin_left - page.margin_right).max(1.0),
            px(page.height - page.margin_top - page.margin_bottom).max(1.0),
        );
        let sheets = paginate::sheets(&self.leaves, &self.frames, area.height);
        Pages { width: px(page.width), height: px(page.height), area, sheets }
    }

    fn paint_page(&mut self, pages: &Pages, index: usize, out: &mut DisplayList) {
        let (Some(sheet), Some(origin)) = (pages.sheets.get(index), pages.origin(index)) else { return };
        let (area, (top, bottom)) = (pages.area, sheet.band);
        if bottom > top {
            // The page shows a band of the column. A box that runs on to the next page is cut at its edge.
            let from = out.items.len();
            out.push(Item::PushClip { rect: Rect::new(-area.x, top, pages.width, bottom - top), radius: 0.0 });
            // A little inside the band, so that a line ending at its edge is not taken for one in it.
            self.paint(Rect::new(0.0, top + 0.25, self.width, bottom - top - 0.5), true, out);
            out.push(Item::PopClip);
            out.translate(from, origin);
        }
        // The page number sits in the middle of the margin beneath the text, if the margin has room for it.
        let Self { engine, types, scratch, palette, .. } = self;
        let label = (index + 1).to_string();
        let spec = types.get(Role::Caption).spec(false, Brush { ink: Ink::Style(Role::Caption), back: None });
        engine.build(&label, &[spec], &[(0, 0..label.len())], None, scratch);
        scratch.break_all_lines(None);
        let (below, foot) = (area.y + area.height, pages.height - area.y - area.height);
        if foot >= scratch.height() {
            let x = area.x + (area.width - scratch.full_width()) / 2.0;
            let at = Point { x: x.round(), y: below + ((foot - scratch.height()) / 2.0).round() };
            let inks = Inks::new(types, *palette);
            Painter { engine, types, inks: &inks, band: (f32::MIN, f32::MAX), out }.glyphs(scratch, at);
        }
    }

    fn page_of(&mut self, pages: &Pages, pos: &Pos) -> Option<usize> {
        let at = *self.index.get(&pos.block)?;
        self.clock += 1;
        self.lay(at);
        let line = leaf::line_of(self.leaves[at].laid.as_ref()?, pos.offset);
        pages.sheets.partition_point(|sheet| sheet.first <= (at, line)).checked_sub(1)
    }

    fn hit(&mut self, point: Point) -> Option<Pos> {
        self.clock += 1;
        self.place_all();
        let at = self.leaves.partition_point(|leaf| leaf.y + leaf.height < point.y);
        // The nearest leaf that holds text, above or below.
        let text = |leaf: &Leaf| leaf.what == What::Text;
        let below = self.leaves.get(at..).and_then(|rest| rest.iter().position(text)).map(|skip| at + skip);
        let above = self.leaves[..at.min(self.leaves.len())].iter().rposition(text);
        let at = match (above, below) {
            (Some(above), Some(below)) => {
                let gap_above = point.y - (self.leaves[above].y + self.leaves[above].height);
                if gap_above < self.leaves[below].y - point.y { above } else { below }
            }
            (above, below) => above.or(below)?,
        };
        self.lay(at);
        let shifts = self.shifts(at);
        let leaf = &self.leaves[at];
        let offset = leaf::hit(leaf.laid.as_ref()?, &shifts, point.x - leaf.x, point.y - leaf.y);
        Some(Pos { block: leaf.id.clone()?, offset })
    }

    fn caret(&mut self, pos: &Pos) -> Option<Rect> {
        let at = *self.index.get(&pos.block)?;
        self.clock += 1;
        self.lay(at);
        self.place(at);
        let shifts = self.shifts(at);
        let leaf = &self.leaves[at];
        Some(leaf::caret(leaf, leaf.laid.as_ref()?, &shifts, pos.offset))
    }

    fn selection(&mut self, selection: &Selection, viewport: Option<Rect>) -> Vec<Rect> {
        let mut rects = Vec::new();
        let ends = [&selection.anchor, &selection.head].map(|pos| Some((*self.index.get(&pos.block)?, pos.offset)));
        let [Some(anchor), Some(head)] = ends else { return rects };
        let (from, to) = (anchor.min(head), anchor.max(head));
        if from == to {
            return rects;
        }
        self.clock += 1;
        self.place_all();
        let band = viewport.map_or((f32::MIN, f32::MAX), |view| (view.y, view.y + view.height));
        let first = from.0.max(self.leaves.partition_point(|leaf| leaf.span().1 <= band.0));
        for at in first..=to.0 {
            self.place(at);
            if self.leaves[at].span().0 >= band.1 {
                break;
            }
            if self.leaves[at].what != What::Text {
                continue;
            }
            self.lay(at);
            let shifts = self.shifts(at);
            let leaf = &self.leaves[at];
            let Some(laid) = &leaf.laid else { continue };
            let len = leaf.text.len();
            let start = if at == from.0 { from.1.min(len) } else { 0 };
            let end = if at == to.0 { to.1.min(len) } else { len };
            leaf::selection(leaf, laid, &shifts, start..end, at != to.0, &mut rects);
        }
        rects
    }

    fn line_step(&mut self, pos: &Pos, x: f32, down: bool) -> Option<Pos> {
        let at = *self.index.get(&pos.block)?;
        self.clock += 1;
        self.lay(at);
        let shifts = self.shifts(at);
        let leaf = &self.leaves[at];
        let laid = leaf.laid.as_ref()?;
        let line = leaf::line_of(laid, pos.offset);
        let within = if down { Some(line + 1).filter(|next| *next < laid.text.len()) } else { line.checked_sub(1) };
        if let Some(line) = within {
            return Some(Pos { block: pos.block.clone(), offset: leaf::on_line(laid, &shifts, line, x - leaf.x) });
        }
        // Off this block's edge: the nearest line of the next block that holds text.
        let text = |leaf: &Leaf| leaf.what == What::Text;
        let at = match down {
            true => at + 1 + self.leaves[at + 1..].iter().position(text)?,
            false => self.leaves[..at].iter().rposition(text)?,
        };
        self.lay(at);
        let shifts = self.shifts(at);
        let leaf = &self.leaves[at];
        let laid = leaf.laid.as_ref()?;
        let line = if down { 0 } else { laid.text.len().saturating_sub(1) };
        Some(Pos { block: leaf.id.clone()?, offset: leaf::on_line(laid, &shifts, line, x - leaf.x) })
    }

    fn line_edge(&mut self, pos: &Pos, end: bool) -> Option<Pos> {
        let at = *self.index.get(&pos.block)?;
        self.clock += 1;
        self.lay(at);
        let laid = self.leaves[at].laid.as_ref()?;
        // Far off either side, which no marker reaches.
        let x = if end { f32::MAX } else { f32::MIN };
        let offset = leaf::on_line(laid, &Shifts::default(), leaf::line_of(laid, pos.offset), x);
        Some(Pos { block: pos.block.clone(), offset })
    }

    fn settle(&mut self, viewport: Rect, budget: usize) -> bool {
        let (mut above, mut below) = self.show(viewport.y, viewport.y + viewport.height);
        let mut left = budget;
        while left > 0 && self.pending > 0 {
            // The nearest estimate on each side of the viewport, turn about.
            let next_below = (below..self.leaves.len()).find(|at| !self.leaves[*at].exact);
            let next_above = (0..above).rev().find(|at| !self.leaves[*at].exact);
            if next_below.is_none() && next_above.is_none() {
                break;
            }
            for at in [next_below, next_above].into_iter().flatten() {
                // A short block still costs something to set up.
                left = left.saturating_sub(self.leaves[at].text.len() + 64);
                self.measure(at);
            }
            below = next_below.map_or(self.leaves.len(), |at| at + 1);
            above = next_above.unwrap_or(0);
        }
        self.place_all();
        self.pending > 0
    }
}
