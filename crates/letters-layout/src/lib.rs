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

mod engine;
mod flow;
mod ink;
mod leaf;
mod palette;
mod shape;
mod theme;
mod tokens;

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

use letters_model::{BlockId, Body, Change, Document, Pos, Selection, Style};
use letters_render::{DisplayList, Font, Item, Point, Rect};
use parley::{Language, Layout};

use crate::engine::Engine;
use crate::flow::{Frame, Leaf, Look, Role, Types, What};
use crate::ink::{Brush, Inks};
use crate::leaf::Painter;

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

    /// The height of the whole document. Partly an estimate until [`DocLayout::settle`] returns `false`.
    pub fn height(&self) -> f32 {
        let state = &mut *self.state.borrow_mut();
        state.place_all();
        state.height
    }

    /// Emits the part of the document that falls inside `viewport`.
    pub fn paint(&self, viewport: Rect, out: &mut DisplayList) {
        self.state.borrow_mut().paint(viewport, out);
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
            let before = self.tokens.get_mut(id).map(|tokens| (tokens, leaf.text.clone()));
            leaf.set_content(block);
            if let Some((tokens, before)) = before {
                tokens::follow(tokens, &before, &leaf.text);
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
        let tokens = &mut self.tokens;
        let flow = flow::flow(doc, &self.types, self.width, |block| {
            let (change, at) = (carry?, *index.get(&block.id)?);
            if change.blocks.contains(&block.id) {
                if let Some(tokens) = tokens.get_mut(&block.id) {
                    tokens::follow(tokens, &old[at].text, &block.plain_text());
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
        self.pending = self.leaves.iter().filter(|leaf| !leaf.exact).count();
        self.held = self.leaves.iter().filter(|leaf| leaf.laid.is_some()).count();
        self.stale = 0;
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
        self.set_height(at, leaf::height(&laid.text));
        self.leaves[at].laid = Some(Box::new(laid));
        self.held += 1;
    }

    /// Gives a leaf its exact height without keeping the layout.
    fn measure(&mut self, at: usize) {
        if self.leaves[at].exact {
            return;
        }
        leaf::shape(&mut self.engine, &self.types, &self.leaves[at], self.locale, &mut self.scratch);
        self.set_height(at, leaf::height(&self.scratch));
    }

    fn set_height(&mut self, at: usize, height: f32) {
        let leaf = &mut self.leaves[at];
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

    fn paint(&mut self, viewport: Rect, out: &mut DisplayList) {
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
            let frame = &self.frames[frame];
            let (top, bottom) = frame.span(&self.leaves);
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
        }
        for at in first..end {
            // A viewport taller than the cache holds has dropped its first layouts by now.
            self.lay(at);
            let Self { engine, types, leaves, tokens, .. } = self;
            let leaf = &leaves[at];
            if leaf.what == What::Rule {
                let rect = Rect::new(leaf.x, leaf.y, leaf.width, leaf.height);
                out.push(Item::Fill { rect, color: palette.line, radius: 0.0 });
            } else if let Some(laid) = &leaf.laid {
                let tokens = leaf.id.as_ref().and_then(|id| tokens.get(id));
                Painter { engine, types, inks: &inks, band, out }.leaf(leaf, laid, tokens.map(Vec::as_slice));
            }
        }
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
        let leaf = &self.leaves[at];
        let offset = leaf::hit(leaf.laid.as_ref()?, point.x - leaf.x, point.y - leaf.y);
        Some(Pos { block: leaf.id.clone()?, offset })
    }

    fn caret(&mut self, pos: &Pos) -> Option<Rect> {
        let at = *self.index.get(&pos.block)?;
        self.clock += 1;
        self.lay(at);
        self.place(at);
        let leaf = &self.leaves[at];
        Some(leaf::caret(leaf, leaf.laid.as_ref()?, pos.offset))
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
            let leaf = &self.leaves[at];
            let Some(laid) = &leaf.laid else { continue };
            let len = leaf.text.len();
            let start = if at == from.0 { from.1.min(len) } else { 0 };
            let end = if at == to.0 { to.1.min(len) } else { len };
            leaf::selection(leaf, laid, start..end, at != to.0, &mut rects);
        }
        rects
    }

    fn line_step(&mut self, pos: &Pos, x: f32, down: bool) -> Option<Pos> {
        let at = *self.index.get(&pos.block)?;
        self.clock += 1;
        self.lay(at);
        let leaf = &self.leaves[at];
        let laid = leaf.laid.as_ref()?;
        let line = leaf::line_of(laid, pos.offset);
        let within = if down { Some(line + 1).filter(|next| *next < laid.text.len()) } else { line.checked_sub(1) };
        if let Some(line) = within {
            return Some(Pos { block: pos.block.clone(), offset: leaf::on_line(laid, line, x - leaf.x) });
        }
        // Off this block's edge: the nearest line of the next block that holds text.
        let text = |leaf: &Leaf| leaf.what == What::Text;
        let at = match down {
            true => at + 1 + self.leaves[at + 1..].iter().position(text)?,
            false => self.leaves[..at].iter().rposition(text)?,
        };
        self.lay(at);
        let leaf = &self.leaves[at];
        let laid = leaf.laid.as_ref()?;
        let line = if down { 0 } else { laid.text.len().saturating_sub(1) };
        Some(Pos { block: leaf.id.clone()?, offset: leaf::on_line(laid, line, x - leaf.x) })
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
