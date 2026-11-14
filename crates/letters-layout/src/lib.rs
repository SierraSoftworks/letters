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
//!
//! A position is a block and a byte offset in its text, as the model has it. Some text is not drawn as it is
//! written: a formula is typeset in place of its TeX, so every offset inside the source is at the formula's
//! right edge. A picture of a gallery has no text on the page, and a position in it is on the picture.

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

use letters_model::{BlockId, BlockKind, Body, Change, Document, Markers, Page, Pos, Selection, Style};
use letters_render::{DisplayList, Font, Item, Point, Rect};
use parley::{Alignment, Language};

use crate::engine::Engine;
use crate::flow::{Frame, Leaf, Look, Part, Role, Table, Types, What, Words};
use crate::ink::{Brush, Ink, Inks};
use crate::leaf::{Extent, Laid, Over, Painter, Source, Text};
use crate::reveal::{Reveal, Shapes, Shifts};

pub use crate::paginate::{Heading, PageLink, Pages, Sheet};
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

/// Where a figure is drawn. See [`DocLayout::figure`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FigureBox {
    /// The picture. For a gallery, all its rows of pictures.
    pub image: Rect,
    /// The caption under it. While there is none this is where its first line would be, which reaches into
    /// the space below the figure.
    pub caption: Rect,
    /// The cloud on a picture that is still a link, when the picture has room for one.
    pub badge: Option<Rect>,
}

/// The number of each numbered figure and gallery of `doc`, in the order of the document. These are the
/// numbers layout draws, for an export that writes the labels itself.
pub fn figure_numbers(doc: &Document) -> Vec<(BlockId, u32)> {
    let mut numbers = Vec::new();
    flow::numbers(&doc.blocks, &mut 0, &mut numbers);
    numbers
}

/// The label layout draws before the caption of the `number`th numbered figure, such as "Figure 2:". The
/// word is the Caption style's.
pub fn figure_label(style: &Style, number: u32) -> String {
    flow::label(flow::figure_word(style), number)
}

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
            tables: Vec::new(),
            index: HashMap::new(),
            tokens: HashMap::new(),
            spelling: HashMap::new(),
            reveal: Reveal::default(),
            margin: f32::INFINITY,
            shapes: Shapes::default(),
            composition: None,
            stale: 0,
            height: 0.0,
            pending: 0,
            held: 0,
            clock: 0,
            scratch: Laid::default(),
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

    /// How much room lies left of the text edge for a heading's markers to hang in, which is without limit
    /// until set. A marker that does not fit has the heading's first line make room, as in a quote. Nothing
    /// is laid out again.
    pub fn set_margin(&mut self, margin: f32) {
        self.state.get_mut().margin = margin.max(0.0);
    }

    /// Lets formulas link, as `\\href` asks. Off until set, and only a document that is trusted may have it on.
    /// Formulas are set again, since one that links may be drawn another way.
    pub fn set_maths_links(&mut self, links: bool) {
        self.state.get_mut().set_maths_links(links);
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
    /// after it, and a table's header with its first row. A block splits between pages only where two of its
    /// lines stay and two go, and a line of code never splits. A figure stays on one page with its caption,
    /// a gallery breaks only between its rows, and a row of a table stays whole. Only something taller than
    /// a page breaks these rules.
    ///
    /// This is a pass over the heights of lines and shapes nothing, so it is cheap to run again after an
    /// edit. It first lays out whatever is still estimated, which on a long document that was never
    /// settled takes as long as settling it.
    pub fn paginate(&self, page: &Page) -> Pages {
        self.state.borrow_mut().paginate(page)
    }

    /// Emits one sheet of `pages`, with the sheet's top left at the origin: the text in its text area and
    /// the page number beneath. Nothing that belongs to editing is drawn: no markers, no spelling marks and
    /// no composition.
    ///
    /// A sheet is white paper. It is painted with [`Palette::LIGHT`] whatever palette is set, and nothing
    /// is drawn behind the text, so the caller fills the paper with that palette's `page` colour.
    pub fn paint_page(&self, pages: &Pages, index: usize, out: &mut DisplayList) {
        self.state.borrow_mut().paint_page(pages, index, out);
    }

    /// The page of `pages` that shows the line `pos` is on.
    pub fn page_of(&self, pages: &Pages, pos: &Pos) -> Option<usize> {
        self.state.borrow_mut().page_of(pages, pos)
    }

    /// The links on one sheet of `pages`, in reading order. A link that runs over several lines has a
    /// rectangle for each.
    pub fn page_links(&self, pages: &Pages, index: usize) -> Vec<PageLink> {
        self.state.borrow_mut().page_links(pages, index)
    }

    /// The document's headings in order, each with the sheet of `pages` it is on. Headings with no text are
    /// left out.
    pub fn outline(&self, pages: &Pages) -> Vec<Heading> {
        self.state.borrow_mut().outline(pages)
    }

    /// The position nearest to a point: in the nearest text, or on a picture of a gallery. `None` when the
    /// document holds neither.
    pub fn hit(&self, point: Point) -> Option<Pos> {
        self.state.borrow_mut().hit(point)
    }

    /// Where a caret at `pos` is drawn. An offset where a line wraps belongs to the start of the next line.
    pub fn caret(&self, pos: &Pos) -> Option<Rect> {
        self.state.borrow_mut().caret(pos)
    }

    /// Where the formula at `pos` is drawn: the one `pos` is in or beside, in a line of text or as a block of
    /// its own. The sheet its source is written on slides out from under this. `None` when `pos` is at no
    /// formula. A source that cannot be typeset has the place where it shows as text.
    pub fn formula_rect(&self, pos: &Pos) -> Option<Rect> {
        self.state.borrow_mut().formula_rect(pos)
    }

    /// Where a figure or gallery is drawn: its picture and its caption. `None` for any other block.
    ///
    /// A figure inside a gallery has the gallery's caption, since a gallery has one for all its pictures.
    pub fn figure(&self, block: &BlockId) -> Option<FigureBox> {
        self.state.borrow_mut().figure(block)
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

/// Where a block's positions are: a leaf, and the part of it that shows the block.
#[derive(Clone, Copy)]
struct Slot {
    leaf: usize,
    part: Part,
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
    tables: Vec<Table>,
    /// Where each block is that a position can be in or on, and each that a leaf stands for.
    index: HashMap<BlockId, Slot>,
    /// The tokens of code blocks.
    tokens: HashMap<BlockId, Vec<Token>>,
    /// The misspelled words of text blocks.
    spelling: HashMap<BlockId, Vec<Range<usize>>>,
    /// The markers shown around the caret.
    reveal: Reveal,
    /// The room left of the text edge that markers may hang in.
    margin: f32,
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
    /// A layout to measure with, so that measuring allocates nothing.
    scratch: Laid,
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
                    if let Some(&Slot { leaf, .. }) = self.index.get(block) {
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
            let Some(&Slot { leaf: at, part }) = self.index.get(id) else { return false };
            let leaf = &mut self.leaves[at];
            let Some(block) = doc.blocks.get(leaf.root).and_then(|root| flow::find(root, id)) else { return false };
            // A block of another kind changes the column's shape, and so does a picture of another size.
            // Whether a page starts at a block is decided where the column is built.
            let kept = match part {
                Part::Text if leaf.caption.is_none() => {
                    leaf.kind.as_ref() == Some(&block.kind) && flow::breaks(block) == leaf.breaks
                }
                Part::Text => block.kind == BlockKind::Caption,
                Part::Cell(_) => block.kind == BlockKind::TableCell,
                Part::Whole | Part::Picture(_) => false,
            };
            if !kept || !matches!(block.body, Body::Text(_)) {
                return false;
            }
            let before = leaf.reword(part, block);
            let after = leaf.text(part).map_or("", |words| words.text.as_str());
            if let Some(tokens) = self.tokens.get_mut(id) {
                tokens::follow(tokens, &before, after);
            }
            if let Some(spelling) = self.spelling.get_mut(id) {
                tokens::follow_ranges(spelling, &before, after);
            }
            // New text in a cell can change how the table's width is shared.
            if let (Part::Cell(_), Some(table)) = (part, self.tables.get_mut(leaf.table)) {
                table.sized = false;
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
        let flow = flow::flow(doc, &self.types, self.width, carry.map(|change| &change.blocks), |block| {
            // A block inside a leaf, such as a cell, has no leaf to hand over.
            let at = index.get(block)?.leaf;
            (old[at].id.as_ref() == Some(block)).then(|| std::mem::take(&mut old[at]))
        });
        (self.leaves, self.frames, self.tables) = (flow.leaves, flow.frames, flow.tables);
        index.clear();
        for (at, leaf) in self.leaves.iter().enumerate() {
            if let (What::Gallery | What::Row, Some(id)) = (leaf.what, &leaf.id) {
                index.insert(id.clone(), Slot { leaf: at, part: Part::Whole });
            }
            for (part, id) in leaf.parts() {
                index.insert(id.clone(), Slot { leaf: at, part });
            }
        }
        self.index = index;
        for (id, before) in &flow.retexted {
            let Some(after) = self.index.get(id).and_then(|slot| self.leaves[slot.leaf].text(slot.part)) else {
                continue;
            };
            if let Some(tokens) = self.tokens.get_mut(id) {
                tokens::follow(tokens, before, &after.text);
            }
            if let Some(spelling) = self.spelling.get_mut(id) {
                tokens::follow_ranges(spelling, before, &after.text);
            }
        }
        let coded = |id: &BlockId| self.index.get(id).is_some_and(|slot| self.leaves[slot.leaf].role == Role::Code);
        self.tokens.retain(|id, tokens| coded(id) && !tokens.is_empty());
        let worded = |id: &BlockId| self.index.get(id).is_some_and(|slot| slot.part >= Part::Text);
        self.spelling.retain(|id, ranges| worded(id) && !ranges.is_empty());
        self.recount();
        self.stale = 0;
    }

    /// Counts the leaves whose height is a guess and those that hold a layout.
    fn recount(&mut self) {
        self.pending = self.leaves.iter().filter(|leaf| !leaf.exact).count();
        self.held = self.leaves.iter().filter(|leaf| leaf.laid.is_some()).count();
    }

    fn set_spelling(&mut self, block: &BlockId, mut ranges: Vec<Range<usize>>) {
        ranges.retain(|range| !range.is_empty());
        let worded = self.index.get(block).is_some_and(|slot| slot.part >= Part::Text);
        if worded && !ranges.is_empty() {
            ranges.sort_by_key(|range| range.start);
            self.spelling.insert(block.clone(), ranges);
        } else {
            self.spelling.remove(block);
        }
    }

    fn set_maths_links(&mut self, links: bool) {
        if std::mem::replace(&mut self.engine.links, links) != links {
            // A formula may be another size now. Formulas are few, so everything is set again.
            for leaf in &mut self.leaves {
                (leaf.exact, leaf.laid) = (false, None);
            }
            self.recount();
        }
    }

    fn reveal(&mut self, at: Option<(&Pos, &Markers)>) -> bool {
        // Markers are few, so this only guards against a caller that makes up new ones without end.
        if self.shapes.len() > 64 {
            self.shapes.clear();
        }
        self.reveal.set(at)
    }

    /// The markers revealed in one text of a leaf that holds its layout.
    fn shifts(&mut self, at: usize, part: Part) -> Shifts {
        let Self { reveal, shapes, engine, types, leaves, frames, margin, .. } = self;
        let leaf = &leaves[at];
        let (Some(id), Some(text)) = (leaf.owner(part), Text::of(leaf, part)) else { return Shifts::default() };
        // Code shows its text as written.
        if leaf.role == Role::Code {
            return Shifts::default();
        }
        // Inside a quote or callout a marker hangs clear of the bar down its side, never across it.
        let room = leaf.frame.map_or(*margin + leaf.x, |frame| leaf.x - frames[frame].x - theme::BAR - 2.0);
        reveal.over(id, text.set, room, |text| shapes.get(engine, types, leaf.role, false, text))
    }

    fn set_tokens(&mut self, block: &BlockId, mut tokens: Vec<Token>) {
        let coded = self.index.get(block).is_some_and(|slot| self.leaves[slot.leaf].role == Role::Code);
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
        self.size(at);
        if self.leaves[at].laid.is_some() || matches!(self.leaves[at].what, What::Rule | What::Blank) {
            return;
        }
        if self.held >= KEPT {
            self.evict();
        }
        let mut laid = Box::<Laid>::default();
        self.shape(at, &mut laid);
        leaf::adorn(&mut self.engine, &self.types, &self.leaves[at], &mut laid);
        self.leaves[at].laid = Some(laid);
        self.held += 1;
    }

    /// Gives a leaf its exact height without keeping the layout.
    fn measure(&mut self, at: usize) {
        self.size(at);
        if self.leaves[at].exact {
            return;
        }
        let mut scratch = std::mem::take(&mut self.scratch);
        self.shape(at, &mut scratch);
        self.scratch = scratch;
    }

    /// Shapes the texts of a leaf into `laid`, and records what they say of the leaf: its height, and the
    /// lines pagination may split it at.
    fn shape(&mut self, at: usize, laid: &mut Laid) {
        let Self { engine, types, leaves, tables, palette, locale, .. } = self;
        let leaf = &leaves[at];
        let aligns = tables.get(leaf.table).filter(|_| leaf.what == What::Row).map_or(&[][..], |table| &table.aligns);
        let color = ink::of(types, palette, leaf.role);
        leaf::lay(engine, types, leaf, aligns, color, *locale, laid);
        let Extent { height, lines, ends, joined } = leaf::extent(leaf, laid);
        let leaf = &mut self.leaves[at];
        (leaf.lines, leaf.ends, leaf.joined) = (lines, ends, joined);
        if !std::mem::replace(&mut leaf.exact, true) {
            self.pending -= 1;
        }
        if leaf.height != height {
            leaf.height = height;
            self.stale = self.stale.min(at + 1);
        }
    }

    /// Gives the table of a row its columns, when it still has cells that were never measured. This shapes
    /// each of those once, without breaking lines. Rows whose cells move are to be laid out again.
    fn size(&mut self, row: usize) {
        let Self { engine, types, leaves, tables, palette, locale, scratch, .. } = self;
        let leaf = &leaves[row];
        let Some(table) = tables.get_mut(leaf.table).filter(|table| leaf.what == What::Row && !table.sized) else {
            return;
        };
        for at in table.rows.clone() {
            let color = ink::of(types, palette, leaves[at].role);
            for index in 0..leaves[at].cells.len() {
                let leaf = &leaves[at];
                if leaf.cells[index].fit.is_some() {
                    continue;
                }
                let source = Source::cell(leaf, &leaf.cells[index], Alignment::Start, color);
                let fit = leaf::fit(engine, types, &source, *locale, &mut scratch.text);
                leaves[at].cells[index].fit = Some(fit);
            }
        }
        table.size(leaves, types);
        self.stale = self.stale.min(table.rows.start);
        self.recount();
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
                    Painter { engine, types, inks: &inks, band, spelt: false, out }.sign(shape, at, level);
                }
                at.x += width;
            }
        }
        for at in first..end {
            // A viewport taller than the cache holds has dropped its first layouts by now.
            self.lay(at);
            let leaf = &self.leaves[at];
            if leaf.what == What::Rule {
                let rect = Rect::new(leaf.x, leaf.y, leaf.width, leaf.height);
                out.push(Item::Fill { rect, color: palette.line, radius: 0.0 });
                continue;
            }
            // The leaf's own text and what stands around it, then the text of each of its cells.
            let cells = leaf.cells.len();
            for part in [Part::Text].into_iter().chain((0..cells).map(Part::Cell)) {
                let shifts = if print { Shifts::default() } else { self.shifts(at, part) };
                let Self { engine, types, leaves, tokens, spelling, shapes, composition, .. } = self;
                let leaf = &leaves[at];
                let Some(laid) = &leaf.laid else { continue };
                // What belongs to editing is found by the block the text is of.
                let block = leaf.owner(part).filter(|_| !print);
                let own = |id: &BlockId| block == Some(id);
                let composing = composition.as_ref().filter(|(block, _)| own(block)).map(|(_, range)| range.clone());
                let spelling = block.and_then(|id| spelling.get(id)).map_or(&[][..], Vec::as_slice);
                let over = Over { shifts: &shifts, shapes, composing, spelling };
                let mut painter = Painter { engine, types, inks: &inks, band, spelt: print, out };
                match part {
                    Part::Cell(index) => painter.cell(leaf, index, &over),
                    _ if leaf.what == What::Row => painter.rule(leaf),
                    _ => {
                        let tokens = leaf.id.as_ref().and_then(|id| tokens.get(id));
                        painter.leaf(leaf, laid, tokens.map(Vec::as_slice), &over);
                    }
                }
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
        // Paper is white whatever the screen's theme, so a sheet takes the colours the style was written for.
        let screen = std::mem::replace(&mut self.palette, Palette::LIGHT);
        self.sheet(pages, index, out);
        self.palette = screen;
    }

    fn sheet(&mut self, pages: &Pages, index: usize, out: &mut DisplayList) {
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
        let (label, number) = ((index + 1).to_string(), &mut scratch.text.layout);
        let spec = types.get(Role::Caption).spec(false, Brush { ink: Ink::Style(Role::Caption), back: None });
        engine.build(&label, &[spec], &[(0, 0..label.len())], None, number);
        number.break_all_lines(None);
        let (below, foot) = (area.y + area.height, pages.height - area.y - area.height);
        if foot >= number.height() {
            let x = area.x + (area.width - number.full_width()) / 2.0;
            let at = Point { x: x.round(), y: below + ((foot - number.height()) / 2.0).round() };
            let inks = Inks::new(types, *palette);
            Painter { engine, types, inks: &inks, band: (f32::MIN, f32::MAX), spelt: true, out }
                .glyphs(number, &label, at);
        }
    }

    fn page_of(&mut self, pages: &Pages, pos: &Pos) -> Option<usize> {
        let Slot { leaf: at, part } = *self.index.get(&pos.block)?;
        self.clock += 1;
        self.lay(at);
        let leaf = &self.leaves[at];
        let line = Text::of(leaf, part).map_or(0, |text| leaf::line_of(text.set, pos.offset));
        let line = leaf.line_of(part, line);
        pages.sheets.partition_point(|sheet| sheet.first <= (at, line)).checked_sub(1)
    }

    fn page_links(&mut self, pages: &Pages, index: usize) -> Vec<PageLink> {
        let mut links = Vec::new();
        let (Some(sheet), Some(origin)) = (pages.sheets.get(index), pages.origin(index)) else { return links };
        self.clock += 1;
        self.place_all();
        for (block, lines) in &sheet.blocks {
            let Some(at) = self.index.get(block).map(|slot| slot.leaf) else { continue };
            let linked = |words: &Words| !words.links.is_empty();
            if !linked(&self.leaves[at].words) && !self.leaves[at].cells.iter().any(|cell| linked(&cell.words)) {
                continue;
            }
            self.lay(at);
            let leaf = &self.leaves[at];
            for part in [Part::Text].into_iter().chain((0..leaf.cells.len()).map(Part::Cell)) {
                let Some(text) = Text::of(leaf, part) else { continue };
                for (range, target) in &text.source.links {
                    leaf::stretches(text.set, range, &Shifts::default(), |line, part_of| {
                        if lines.contains(&leaf.line_of(part, line)) {
                            let (x, y) = (origin.x + text.at.x + part_of.x, origin.y + text.at.y + part_of.y);
                            links.push(PageLink { rect: Rect { x, y, ..part_of }, target: target.clone() });
                        }
                    });
                }
            }
        }
        links
    }

    fn outline(&mut self, pages: &Pages) -> Vec<Heading> {
        self.place_all();
        let mut headings = Vec::new();
        for (page, sheet) in pages.sheets.iter().enumerate() {
            let Some(origin) = pages.origin(page) else { continue };
            // A heading is listed where it starts.
            for (block, _) in sheet.blocks.iter().filter(|(_, lines)| lines.start == 0) {
                let Some(leaf) = self.index.get(block).map(|slot| &self.leaves[slot.leaf]) else { continue };
                let title = &leaf.words.text;
                let Some(level) = leaf.role.level().filter(|_| !title.trim().is_empty()) else { continue };
                let at = Point { x: origin.x + leaf.x, y: origin.y + leaf.y };
                headings.push(Heading { block: block.clone(), title: title.clone(), level, page, at });
            }
        }
        headings
    }

    fn hit(&mut self, point: Point) -> Option<Pos> {
        self.clock += 1;
        self.place_all();
        let at = self.leaves.partition_point(|leaf| leaf.y + leaf.height < point.y);
        // The nearest leaf that a position can be in, above or below.
        let below = self.leaves.get(at..).and_then(|rest| rest.iter().position(Leaf::holds)).map(|skip| at + skip);
        let above = self.leaves[..at.min(self.leaves.len())].iter().rposition(Leaf::holds);
        let at = match (above, below) {
            (Some(above), Some(below)) => {
                let gap_above = point.y - (self.leaves[above].y + self.leaves[above].height);
                if gap_above < self.leaves[below].y - point.y { above } else { below }
            }
            (above, below) => above.or(below)?,
        };
        self.lay(at);
        let part = self.leaves[at].part_at(point)?;
        self.find(at, part, |text, shifts| leaf::hit(text, shifts, point))
    }

    /// A position in a part of a laid leaf: where `offset` says in its text, or on it when it is a picture.
    fn find(&mut self, at: usize, part: Part, offset: impl FnOnce(&Text<'_>, &Shifts) -> usize) -> Option<Pos> {
        let shifts = self.shifts(at, part);
        let leaf = &self.leaves[at];
        let block = leaf.owner(part)?.clone();
        let offset = Text::of(leaf, part).map_or(0, |text| offset(&text, &shifts));
        Some(Pos { block, offset })
    }

    fn caret(&mut self, pos: &Pos) -> Option<Rect> {
        let Slot { leaf: at, part } = *self.index.get(&pos.block)?;
        self.clock += 1;
        self.lay(at);
        self.place(at);
        let shifts = self.shifts(at, part);
        let leaf = &self.leaves[at];
        if let Part::Picture(index) = part {
            // A picture is one thing to step over: the caret stands at its side.
            let picture = leaf.pictures.get(index)?.rect;
            let x = if pos.offset == 0 { picture.x } else { picture.x + picture.width - theme::CARET };
            return Some(Rect::new(leaf.x + x, leaf.y + picture.y, theme::CARET, picture.height));
        }
        Some(leaf::caret(&Text::of(leaf, part)?, &shifts, pos.offset))
    }

    fn formula_rect(&mut self, pos: &Pos) -> Option<Rect> {
        let Slot { leaf: at, part } = *self.index.get(&pos.block)?;
        self.clock += 1;
        self.lay(at);
        self.place(at);
        let shifts = self.shifts(at, part);
        let leaf = &self.leaves[at];
        let found = leaf::formula_at(&Text::of(leaf, part)?, &shifts, pos.offset);
        // A formula block with nothing typeset in it still has its place.
        let whole = || Rect::new(leaf.x, leaf.y, leaf.width, leaf.height);
        found.or_else(|| (leaf.what == What::Formula).then(whole))
    }

    fn figure(&mut self, block: &BlockId) -> Option<FigureBox> {
        let Slot { leaf: at, part } = *self.index.get(block)?;
        self.clock += 1;
        self.lay(at);
        self.place(at);
        let leaf = &self.leaves[at];
        let on_page = |rect: Rect| Rect { x: leaf.x + rect.x, y: leaf.y + rect.y, ..rect };
        let (image, remote) = match (leaf.what, part) {
            (What::Figure, _) => leaf.pictures.first().map(|picture| (picture.rect, picture.asset.is_none()))?,
            (What::Gallery, Part::Picture(index)) => {
                leaf.pictures.get(index).map(|picture| (picture.rect, picture.asset.is_none()))?
            }
            (What::Gallery, Part::Whole) => {
                (Rect::new(0.0, 0.0, leaf.width, (leaf.lead - theme::CAPTION_GAP).max(0.0)), false)
            }
            _ => return None,
        };
        let image = on_page(image);
        let text = leaf.laid.as_ref().map_or(0.0, |laid| laid.text.layout.height());
        let caption = on_page(Rect::new(0.0, leaf.lead, leaf.width, text));
        Some(FigureBox { image, caption, badge: leaf::badge(image).filter(|_| remote) })
    }

    fn selection(&mut self, selection: &Selection, viewport: Option<Rect>) -> Vec<Rect> {
        let mut rects = Vec::new();
        let end = |pos: &Pos| self.index.get(&pos.block).map(|slot| (slot.leaf, slot.part, pos.offset));
        let (Some(anchor), Some(head)) = (end(&selection.anchor), end(&selection.head)) else { return rects };
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
            if !self.leaves[at].holds() {
                continue;
            }
            self.lay(at);
            let mut next = 0;
            loop {
                let Some(part) = self.leaves[at].parts().nth(next).map(|(part, _)| part) else { break };
                next += 1;
                if (at, part) < (from.0, from.1) || (at, part) > (to.0, to.1) {
                    continue;
                }
                let (starts, ends) = ((at, part) == (from.0, from.1), (at, part) == (to.0, to.1));
                let shifts = self.shifts(at, part);
                let leaf = &self.leaves[at];
                match (part, Text::of(leaf, part)) {
                    (_, Some(text)) => {
                        let len = text.source.text.len();
                        let start = if starts { from.2.min(len) } else { 0 };
                        let end = if ends { to.2.min(len) } else { len };
                        leaf::selection(&text, &shifts, start..end, !ends, &mut rects);
                    }
                    // A picture is selected when the selection runs over it.
                    (Part::Picture(index), _) if !(starts && from.2 > 0) && !(ends && to.2 == 0) => {
                        let picture = leaf.pictures[index].rect;
                        rects.push(Rect { x: leaf.x + picture.x, y: leaf.y + picture.y, ..picture });
                    }
                    _ => {}
                }
            }
        }
        rects
    }

    fn line_step(&mut self, pos: &Pos, x: f32, down: bool) -> Option<Pos> {
        let Slot { leaf: at, part } = *self.index.get(&pos.block)?;
        self.clock += 1;
        self.lay(at);
        if let Some(inside) = self.step_inside(at, part, pos.offset, x, down) {
            return Some(inside);
        }
        // Off this leaf's edge: the nearest line of the next leaf that a position can be in.
        let at = match down {
            true => at + 1 + self.leaves[at + 1..].iter().position(Leaf::holds)?,
            false => self.leaves[..at].iter().rposition(Leaf::holds)?,
        };
        self.lay(at);
        let leaf = &self.leaves[at];
        let rows = leaf.pictures.last().map_or(0, |last| last.row + 1);
        let part = match leaf.what {
            What::Row => leaf.cell_at(x).map(Part::Cell)?,
            // From above a gallery is entered by its first row, and from below by its caption.
            What::Gallery if rows > 0 && (down || leaf.caption.is_none()) => {
                leaf.picture_in(if down { 0 } else { rows - 1 }, x).map(Part::Picture)?
            }
            _ => Part::Text,
        };
        self.find(at, part, |text, shifts| {
            let line = if down { 0 } else { text.set.layout.len().saturating_sub(1) };
            leaf::on_line(text, shifts, line, x)
        })
    }

    /// The position a line above or below one in a part of a leaf, when that is still in the leaf.
    fn step_inside(&mut self, at: usize, part: Part, offset: usize, x: f32, down: bool) -> Option<Pos> {
        let leaf = &self.leaves[at];
        let on = |picture: usize| Some(Pos { block: leaf.pictures.get(picture)?.id.clone(), offset: 0 });
        let rows = leaf.pictures.last().map_or(0, |last| last.row + 1);
        if let Part::Picture(index) = part {
            let row = leaf.pictures.get(index)?.row;
            let next = if down { Some(row + 1).filter(|next| *next < rows) } else { row.checked_sub(1) };
            return match next {
                Some(row) => on(leaf.picture_in(row, x)?),
                // Under the last row is the caption.
                None if down && leaf.caption.is_some() => {
                    self.find(at, Part::Text, |text, shifts| leaf::on_line(text, shifts, 0, x))
                }
                None => None,
            };
        }
        let text = Text::of(leaf, part)?;
        let line = leaf::line_of(text.set, offset);
        let lines = text.set.layout.len();
        match if down { Some(line + 1).filter(|next| *next < lines) } else { line.checked_sub(1) } {
            Some(line) => self.find(at, part, |text, shifts| leaf::on_line(text, shifts, line, x)),
            // Above a gallery's caption is its last row of pictures.
            None if !down && leaf.what == What::Gallery && rows > 0 => on(leaf.picture_in(rows - 1, x)?),
            None => None,
        }
    }

    fn line_edge(&mut self, pos: &Pos, end: bool) -> Option<Pos> {
        let Slot { leaf: at, part } = *self.index.get(&pos.block)?;
        self.clock += 1;
        self.lay(at);
        // A picture is a line of its own.
        let Some(text) = Text::of(&self.leaves[at], part) else { return Some(pos.clone()) };
        // Far off either side, which no marker reaches.
        let x = if end { f32::MAX } else { f32::MIN };
        let offset = leaf::on_line(&text, &Shifts::default(), leaf::line_of(text.set, pos.offset), x);
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
                left = left.saturating_sub(self.leaves[at].bytes() + 64);
                self.measure(at);
            }
            below = next_below.map_or(self.leaves.len(), |at| at + 1);
            above = next_above.unwrap_or(0);
        }
        self.place_all();
        self.pending > 0
    }
}
