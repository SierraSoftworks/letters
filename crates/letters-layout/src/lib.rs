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
//! right edge. While the caret is in a formula, its source shows instead. See [`DocLayout::reveal`]. A
//! reference to a figure reads as the figure's label, whatever its own text is. A picture of a gallery has
//! no text on the page, and a position in it is on the picture.
//!
//! A table of contents and a link preview are whole blocks, as a divider is: no position is inside one.
//! [`DocLayout::contents`] and [`DocLayout::previews`] say where they are and where they lead.

mod callout;
mod engine;
mod flow;
mod ink;
mod leaf;
mod paginate;
mod palette;
mod reveal;
mod shape;
mod split;
mod theme;
mod tokens;
mod words;

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use letters_model::{BlockId, BlockKind, Body, Change, Document, Markers, Page, Pos, Selection, Style};
use letters_render::{Color, DisplayList, Font, Item, Point, Rect, Role as Tag};
use parley::{Alignment, Language, PositionedLayoutItem};

use crate::engine::Engine;
use crate::flow::{Cards, Frame, Leaf, Look, Part, Picture, Role, Table, Types, What, Words};
use crate::ink::{Brush, Ink, Inks};
use crate::leaf::{Extent, Laid, Lay, Lit, Open, Over, Painter, Source, Swap, Text};
use crate::reveal::{Reveal, Shapes, Shifts};

pub use crate::callout::icon_path;
pub use crate::engine::{Uncovered, reads};
pub use crate::flow::{leads, rtl};
pub use crate::paginate::{Heading, PageBlock, PageLink, Pages, Sheet};
pub use crate::palette::{Callout, Palette, Scheme, Syntax};
pub use crate::shape::{FontSpec, ShapedRun, ShapedText, Shaper};
pub use crate::split::{Arrangement, Piece, Side, Split, Window};
pub use crate::theme::CAPTION_INSET;
pub use crate::tokens::{Token, TokenKind};
pub use crate::words::WordList;

/// The fonts layout may use. The shell supplies them, since the core never reads files.
///
/// Text is set in these faces wherever they have its characters, never in the system's. A family the style
/// names but these lack falls back to the first proportional face here, or the first monospace one for code.
/// Faces of the system only back them up, for the characters they lack: see
/// [`DocLayout::add_fallback_fonts`].
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
    /// The picture. For a gallery, all its rows of pictures. For a link that is loaded, the box its picture
    /// is fitted in.
    pub image: Rect,
    /// The caption under it, the whole width of the figure. While there is none this is where its first
    /// line would be, which reaches into the space below the figure unless
    /// [`DocLayout::set_caption_open`] keeps that line open.
    pub caption: Rect,
    /// The cloud on a picture that is still a link, when the picture has room for one. It keeps its place
    /// when the picture is loaded and drawn, where nothing marks it.
    pub badge: Option<Rect>,
}

/// Where a table is drawn. See [`DocLayout::tables`].
#[derive(Clone, Debug, PartialEq)]
pub struct TableBox {
    /// The whole table.
    pub rect: Rect,
    /// The left edge and the width of each column.
    pub columns: Vec<(f32, f32)>,
    /// Its rows, from the top.
    pub rows: Vec<TableRow>,
}

/// A row of a table, where it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct TableRow {
    pub y: f32,
    pub height: f32,
    /// Its cells, in the order of the columns.
    pub cells: Vec<BlockId>,
}

/// A table of contents, where it is drawn. See [`DocLayout::contents`].
#[derive(Clone, Debug, PartialEq)]
pub struct ContentsBox {
    pub block: BlockId,
    /// All its lines.
    pub rect: Rect,
    /// Its lines from the top: the heading each leads to, and the line across the whole table, with the
    /// room its page number has on a sheet. Empty for a table with no heading to list, which says so in a
    /// line of its own.
    pub lines: Vec<(BlockId, Rect)>,
}

/// A link preview, where it is drawn. See [`DocLayout::previews`].
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewBox {
    pub block: BlockId,
    /// The card, all of which leads to the preview's address.
    pub rect: Rect,
    /// The room a preview that has only its address keeps clear at the end of the card, for a control
    /// that fetches the rest. `None` once anything was fetched, and where the card is too narrow.
    pub control: Option<Rect>,
}

/// The number of each numbered figure and gallery of `doc`, in the order of the document. These are the
/// numbers layout draws, for an export that writes the labels itself.
pub fn figure_numbers(doc: &Document) -> Vec<(BlockId, u32)> {
    let mut numbers = Vec::new();
    flow::numbers(&doc.blocks, &mut 0, &mut numbers);
    numbers
}

/// The word a style gives its figure labels, such as "Figure", for whoever reads a label typed by hand.
pub fn figure_word(style: &Style) -> &str {
    flow::figure_word(style)
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
            open: None,
            in_place: true,
            good: None,
            stands: false,
            term: None,
            numbers: HashMap::new(),
            loaded: HashMap::new(),
            caption: None,
            listing: false,
            cards: Cards::default(),
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
    /// while the caret stays put. Returns whether a marker appears or goes, or a formula opens or closes.
    ///
    /// Markers never change the layout. One inside a line pushes the rest of that line aside, past the text
    /// edge when the line is full, and no other line moves. A heading's hangs left of the text edge, or as
    /// far left as the bar of the quote or callout around it allows, with its first line making room. A
    /// callout's sits in the corner of its box. [`DocLayout::caret`], [`DocLayout::hit`] and the other
    /// queries answer for the page as drawn, so a caret among markers at its offset sits after the first
    /// `before_caret` of them.
    ///
    /// A formula is the one thing that is laid out again. While the caret is in it or at either end of it,
    /// its TeX shows in its place, written as code, with the editor's markers around it like any others.
    /// It is typeset again when the caret leaves, or with `None`. A window that shows the source somewhere
    /// else turns this off with [`DocLayout::set_formula_source`].
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

    /// Adds faces to the fonts given at the start, for a shell whose fonts arrive one after another. Until a
    /// face arrives, text that asks for it is set in what there is: its family's upright face slanted for
    /// an italic, or the first family given for a family that is missing.
    ///
    /// A face takes the place of one given earlier that it is again: the same family, width, style and
    /// weight, with the same axes. So a page can start in a part of a face, as its Latin alone, and take
    /// the whole after its first frame. Text is set in the later face from then on, and what is laid out
    /// keeps the earlier one until it is laid out again, which is right while the two set what both hold
    /// alike: the same glyph numbers and advances.
    ///
    /// Only the blocks with such text are laid out again, as after an edit: each keeps its height as an
    /// estimate until it is. Returns whether there are any, which is whether the page may now look different.
    pub fn add_fonts(&mut self, fonts: &Fonts) -> bool {
        self.state.get_mut().add_fonts(fonts, false)
    }

    /// Adds faces that only back up the fonts: the system's, for emoji and for scripts the fonts lack. Text
    /// is set in one only where none of the fonts has its characters, so what they cover measures as it
    /// did, and a style never gets one by naming its family. A face is taken by its place in its file: of
    /// a file of several families, only the family of the face given is used. A file that holds a family
    /// of the fonts' own is left out.
    ///
    /// The blocks that were short of a character or of a face are laid out again, as [`Self::add_fonts`]
    /// does. Returns whether a character that was shown without a glyph has one now.
    pub fn add_fallback_fonts(&mut self, fonts: &Fonts) -> bool {
        self.state.get_mut().add_fonts(fonts, true)
    }

    /// The characters shown without a glyph since this was last asked, since no face has one: each once,
    /// in the order they were met. Whatever needs no glyph is left out. Text is looked at as it is laid
    /// out, so a long document tells of its characters as they come into view, and asking costs nothing
    /// when there are none. Ask once every font of [`Self::add_fonts`] has come, since a late one may
    /// have them.
    pub fn take_uncovered(&mut self) -> Vec<Uncovered> {
        self.state.get_mut().engine.take_uncovered()
    }

    /// Hands over the word list of a script that is written without spaces, as [`WordList::data`] gives it.
    /// Lines of that script then end between words. Only the blocks set without a list are laid out again,
    /// as [`Self::add_fonts`] does. Returns whether the page may now look different: `false` for a list
    /// that is there already, for bytes that are not that list as this build reads it (a file cut short,
    /// or one kept from another build), and always where the lists are built in.
    pub fn add_word_list(&mut self, list: WordList, data: &[u8]) -> bool {
        self.state.get_mut().add_word_list(list, data)
    }

    /// The scripts met since this was last asked whose lines cannot end between words yet, since their
    /// word list is not here: each once ever. Text is looked at as it is laid out, as for
    /// [`Self::take_uncovered`]. Always empty where the lists are built in.
    pub fn take_unlisted(&mut self) -> Vec<WordList> {
        self.state.get_mut().engine.lists.take()
    }

    /// Says that the picture a link leads to is there to draw, with its size in pixels, or with `None` that
    /// it no longer is. A figure that is still a link shows as a frame with its host and a cloud, since
    /// whether to fetch it is the shell's to decide. Once loaded it is drawn as [`Item::Image`] with the
    /// link for its `asset`, written as the document writes it: the name to keep the picture under among
    /// the images the page is rendered with. A sheet draws it too, and a picture that was never loaded
    /// stays a frame there.
    ///
    /// The size a link's figure stores may be a guess, as it is for one written without a size, and loading
    /// does not change the document. So the picture is drawn at its own shape, as large as fits in the
    /// figure's box and in the middle of it. Nothing is laid out again.
    ///
    /// The picture of a link preview is told the same way, by its name among the document's assets. A
    /// preview shows its picture in a slot of one size, and the document does not store the picture's.
    /// Once told, the slot is filled at the picture's own shape, with what does not fit cut off. Until
    /// then the picture is drawn at the slot's shape.
    ///
    /// Returns whether the document shows the picture, which is whether to paint again: a name that no
    /// figure links to and no preview shows changes nothing. [`FigureBox::image`] stays the figure's box
    /// and [`FigureBox::badge`] answers either way, since a loaded picture is a link until it is embedded.
    pub fn set_loaded(&mut self, src: &str, size: Option<(u32, u32)>) -> bool {
        self.state.get_mut().set_loaded(src, size)
    }

    /// Makes one figure or gallery ready for its caption to be written: the one the selection is at. `block`
    /// is the figure or the gallery, a picture of the gallery or the gallery's caption, and `None` or any
    /// other block names none.
    ///
    /// Its caption keeps a line while it is empty. A figure with no caption and no number is only as tall
    /// as its picture, so that [`FigureBox::caption`] and the caret of an empty caption lie over the block
    /// below. With the line open they have a line of their own, and nothing moves when the first letter is
    /// typed.
    ///
    /// Its caption is also set [`CAPTION_INSET`] in from the right, written or not, which leaves the end of
    /// its lines to what a window stands there. A centred caption is set as far in from the left, so it
    /// stays in the middle, and one set from the start stays where it is unless a line of it reached that
    /// far. [`FigureBox::caption`] stays the whole width.
    ///
    /// Returns whether a figure is laid out anew for it, which can move what is below. Only the screen
    /// shows any of this: a sheet shows the document.
    pub fn set_caption_open(&mut self, block: Option<&BlockId>) -> bool {
        self.state.get_mut().set_caption_open(block)
    }

    /// Lets formulas link, as `\\href` asks. Off until set, and only a document that is trusted may have it on.
    /// Formulas are set again, since one that links may be drawn another way.
    pub fn set_maths_links(&mut self, links: bool) {
        self.state.get_mut().set_maths_links(links);
    }

    /// Sets whether a formula shows its source in its place while the caret is in it, which it does until
    /// set. A window that shows the source on a sheet of its own turns this off: every formula then stays
    /// typeset, and [`DocLayout::formula_rect`] is where the sheet hangs. Either way takes hold at the next
    /// [`DocLayout::reveal`].
    ///
    /// A formula that stays typeset does so while it is being written too. When its source stops being
    /// TeX that can be typeset, as at every half-typed command, it stands as it last was typeset for as
    /// long as the caret is in it, so its line does not jump. [`DocLayout::reveal`] returns `true` when the
    /// caret leaves one that stands so: it then shows as written, in the colour of a warning. Only the
    /// screen keeps what was: a sheet shows the document.
    pub fn set_formula_source(&mut self, in_place: bool) {
        self.state.get_mut().set_formula_source(in_place);
    }

    /// Lights the term a caret is at in a formula that stays typeset while it is written. `at` is the caret
    /// in the formula's source, as [`DocLayout::formula_rect`] takes one, and the colour its term is drawn in.
    /// `None` draws every formula plain again. Nothing is laid out for it: that one formula is drawn another
    /// way from the next [`DocLayout::paint`], so only its block is to be drawn again.
    ///
    /// A term is what [`letters_math::Formula::term`] gives: a symbol, an operator, a run in a font. Nothing
    /// is lit where the caret is at none, in a formula that shows its source in its place, or in one that
    /// stands as it last could be typeset. Only the screen shows it: a sheet shows the document.
    pub fn tint_term(&mut self, at: Option<(&Pos, Color)>) {
        self.state.get_mut().term = at.map(|(caret, color)| (caret.clone(), color));
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

    /// The width the document is laid out at.
    pub(crate) fn width(&self) -> f32 {
        self.state.borrow().width
    }

    /// The stretch of the column that a block takes up with the space above it, from the column's top. A
    /// block inside another, as a cell is in a table, answers for the whole top-level block around it.
    pub fn band(&self, block: &BlockId) -> Option<(f32, f32)> {
        self.state.borrow_mut().band(block)
    }

    /// The position at a point, or the nearest to it, among what a stretch of the column holds.
    pub(crate) fn hit_in(&self, point: Point, band: (f32, f32)) -> Option<Pos> {
        let mut state = self.state.borrow_mut();
        state.hit(point, Some(band)).map(|stop| state.unsided(stop))
    }

    /// Emits the part of the document that falls inside `viewport`.
    pub fn paint(&self, viewport: Rect, out: &mut DisplayList) {
        self.state.borrow_mut().paint(viewport, None, out);
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
    /// Pages hold the document and nothing of the editor, so a formula that shows its source for the caret
    /// is paged, and painted, typeset.
    ///
    /// This is a pass over the heights of lines and shapes nothing, so it is cheap to run again after an
    /// edit. It first lays out whatever is still estimated, which on a long document that was never
    /// settled takes as long as settling it.
    pub fn paginate(&self, page: &Page) -> Pages {
        self.state.borrow_mut().printed(|state| state.paginate(page))
    }

    /// Emits one sheet of `pages`, with the sheet's top left at the origin: the text in its text area and
    /// the page number beneath. Nothing that belongs to editing is drawn: no markers, no spelling marks and
    /// no composition.
    ///
    /// A sheet is white paper. It is painted with [`Palette::LIGHT`] whatever palette is set, and nothing
    /// is drawn behind the text, so the caller fills the paper with that palette's `page` colour.
    ///
    /// A sheet also says what its parts are, for a reader that cannot see it: [`Item::Open`] and
    /// [`Item::Close`] go around what each block draws. A list item's marker is its label, a formula says
    /// how it was written, a picture what it shows, and a figure holds its caption. A quote or callout goes
    /// around the blocks inside it, and a table around its rows of cells. A block that runs on to the next
    /// sheet opens there under the same id. The page number is part of nothing, and nor are the boxes and
    /// rules of the page.
    ///
    /// Each line of a table of contents ends in the number of the sheet its heading starts on, which only
    /// a sheet shows. The line is a paragraph that holds one link, and so is all that a link preview says.
    /// A preview's picture is decoration.
    pub fn paint_page(&self, pages: &Pages, index: usize, out: &mut DisplayList) {
        self.state.borrow_mut().printed(|state| state.paint_page(pages, index, out));
    }

    /// The page of `pages` that shows the line `pos` is on.
    pub fn page_of(&self, pages: &Pages, pos: &Pos) -> Option<usize> {
        self.state.borrow_mut().printed(|state| state.page_of(pages, pos))
    }

    /// The top-level blocks one sheet of `pages` shows, in order, each with where it stands on the sheet.
    /// Print view marks where a page was made to start, and offers to start one, between them. A block
    /// that runs on to the next sheet is on both.
    pub fn page_blocks(&self, pages: &Pages, index: usize) -> Vec<PageBlock> {
        self.state.borrow_mut().printed(|state| state.page_blocks(pages, index))
    }

    /// The links on one sheet of `pages`, in reading order: those that lead out of the document, and each
    /// reference that names a figure, which leads to where its figure is. A link that runs over several
    /// lines has a rectangle for each. A line of a table of contents is one link, which leads to where its
    /// heading starts, and the card of a link preview is one that leads to the preview's address.
    pub fn page_links(&self, pages: &Pages, index: usize) -> Vec<PageLink> {
        self.state.borrow_mut().printed(|state| state.page_links(pages, index))
    }

    /// The document's headings in order, each with the sheet of `pages` it is on. Headings with no text are
    /// left out.
    pub fn outline(&self, pages: &Pages) -> Vec<Heading> {
        self.state.borrow_mut().printed(|state| state.outline(pages))
    }

    /// The position nearest to a point: in the nearest text, on a picture of a gallery, or on the empty box
    /// of a gallery or a table that holds nothing, at offset 0. `None` when the document holds none of these.
    ///
    /// Past the end of a line that wraps, this is the last place that is on that line alone: before the space
    /// it wraps at, or before its last character when it wraps with no space. To put a caret, use
    /// [`DocLayout::hit_sided`].
    pub fn hit(&self, point: Point) -> Option<Pos> {
        let mut state = self.state.borrow_mut();
        state.hit(point, None).map(|stop| state.unsided(stop))
    }

    /// [`DocLayout::hit`] for a caret, which can be at the end of any line.
    ///
    /// Where a line wraps with no space to wrap at, as in a long word, after a hyphen and in the scripts
    /// written without spaces, one place is the end of that line and the start of the next. The flag tells
    /// the two apart: it is set for a position that is the end of the line before the wrap, which is what a
    /// point past that line's end gives. A line that wraps at a space ends before the space, and no flag is
    /// set for it. [`DocLayout::caret_sided`], [`DocLayout::line_step_sided`] and
    /// [`DocLayout::line_edge_sided`] take the flag with the position.
    pub fn hit_sided(&self, point: Point) -> Option<(Pos, bool)> {
        self.state.borrow_mut().hit(point, None)
    }

    /// Where a caret at `pos` is drawn. An offset where a line wraps belongs to the start of the next line.
    pub fn caret(&self, pos: &Pos) -> Option<Rect> {
        self.state.borrow_mut().caret(pos, false)
    }

    /// Where a caret at `pos` is drawn that has the side [`DocLayout::hit_sided`] tells of: with
    /// `before_wrap`, at the end of the line that wraps at `pos`. The flag changes nothing at a place where
    /// no line wraps, or where one wraps at a space, so one left over from before an edit does no harm.
    pub fn caret_sided(&self, pos: &Pos, before_wrap: bool) -> Option<Rect> {
        self.state.borrow_mut().caret(pos, before_wrap)
    }

    /// Where the formula at `pos` is drawn: the one `pos` is in or beside, in a line of text or as a block of
    /// its own. `None` when `pos` is at no formula. One that shows as its source, because the caret is in it
    /// or it cannot be typeset, has the place of that text.
    pub fn formula_rect(&self, pos: &Pos) -> Option<Rect> {
        self.state.borrow_mut().formula_rect(pos)
    }

    /// Where a figure or gallery is drawn: its picture and its caption. `None` for any other block.
    ///
    /// A figure inside a gallery has the gallery's caption, since a gallery has one for all its pictures.
    pub fn figure(&self, block: &BlockId) -> Option<FigureBox> {
        self.state.borrow_mut().figure(block)
    }

    /// The pictures `viewport` shows that are still links, in order, each with where its cloud sits. One
    /// with no room for a cloud is left out.
    pub fn linked(&self, viewport: Rect) -> Vec<(BlockId, Rect)> {
        self.state.borrow_mut().linked(viewport)
    }

    /// The callouts whose tops `viewport` shows, in order, each with the box around its icon and title. One
    /// that shows neither is left out.
    pub fn callouts(&self, viewport: Rect) -> Vec<(BlockId, Rect)> {
        self.state.borrow_mut().callouts(viewport)
    }

    /// The code blocks that `viewport` shows, in order, each with the box drawn behind its text.
    pub fn code_boxes(&self, viewport: Rect) -> Vec<(BlockId, Rect)> {
        self.state.borrow_mut().code_boxes(viewport)
    }

    /// The tables that `viewport` shows any row of, in order, each with all its rows. A row out of view
    /// may still have a height that is an estimate.
    pub fn tables(&self, viewport: Rect) -> Vec<TableBox> {
        self.state.borrow_mut().tables(viewport)
    }

    /// The tables of contents that `viewport` shows, in order, each with all its lines and the heading
    /// each line leads to. The lines are those of [`Document::contents`], set in from the shallowest
    /// level among them, and follow the headings as they are edited.
    pub fn contents(&self, viewport: Rect) -> Vec<ContentsBox> {
        self.state.borrow_mut().contents(viewport)
    }

    /// The link previews that `viewport` shows, in order, each with its card.
    pub fn previews(&self, viewport: Rect) -> Vec<PreviewBox> {
        self.state.borrow_mut().previews(viewport)
    }

    /// Writes a line under the address of a link preview that has only its address: what a window has to
    /// say of it, such as why it could not be fetched. It is set small and muted, on two lines at most, and
    /// the card grows by it. `None` takes it away. The room [`PreviewBox::control`] names stays clear beside
    /// both lines.
    ///
    /// A preview that shows what was fetched ignores its note, and shows it if it loses that again, so a
    /// window takes the note away once it no longer holds. Only the screen shows it: a sheet leaves the
    /// words out, though the card keeps their height there for as long as the note stands.
    ///
    /// Returns whether a card is laid out anew for it, which can move what is below.
    pub fn set_preview_note(&mut self, block: &BlockId, note: Option<&str>) -> bool {
        self.state.get_mut().set_preview_note(block, note)
    }

    /// Sets whether a link preview that has only its address keeps room at its end for a control that
    /// fetches the rest, which it does until set. A window that stands no control there, as in a shell that
    /// cannot fetch, turns this off: the address and its note then have the card's whole width, and
    /// [`PreviewBox::control`] is `None`.
    ///
    /// Returns whether a card is laid out anew for it, which can move what is below.
    pub fn set_preview_control(&mut self, kept: bool) -> bool {
        self.state.get_mut().set_preview_control(kept)
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
    /// the document's first or last line. For a caret, see [`DocLayout::line_step_sided`].
    pub fn line_step(&self, pos: &Pos, x: f32, down: bool) -> Option<Pos> {
        let mut state = self.state.borrow_mut();
        state.line_step(pos, false, x, down).map(|stop| state.unsided(stop))
    }

    /// [`DocLayout::line_step`] for a caret with the side [`DocLayout::hit_sided`] tells of. The line stepped
    /// from is the one that wraps at `pos` if `before_wrap`, and the answer has its side: an `x` past the end
    /// of a line that wraps with no space leads to that line's end.
    pub fn line_step_sided(&self, pos: &Pos, before_wrap: bool, x: f32, down: bool) -> Option<(Pos, bool)> {
        self.state.borrow_mut().line_step(pos, before_wrap, x, down)
    }

    /// The start or the end of the visual line that holds `pos`, which for a line read from the right are at
    /// its right and its left. The end of a wrapped line is the last place on that line alone, as for
    /// [`DocLayout::hit`]. For a caret, see [`DocLayout::line_edge_sided`].
    pub fn line_edge(&self, pos: &Pos, end: bool) -> Option<Pos> {
        let mut state = self.state.borrow_mut();
        state.line_edge(pos, false, end).map(|stop| state.unsided(stop))
    }

    /// [`DocLayout::line_edge`] for a caret with the side [`DocLayout::hit_sided`] tells of. The line is the
    /// one that wraps at `pos` if `before_wrap`, and the answer has its side: the end of a line that wraps
    /// with no space is after its last character, with the flag set. So End pressed twice stays put, and Home
    /// from there goes back to the start of that line.
    pub fn line_edge_sided(&self, pos: &Pos, before_wrap: bool, end: bool) -> Option<(Pos, bool)> {
        self.state.borrow_mut().line_edge(pos, before_wrap, end)
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

/// How a sheet names its parts. A leaf's parts are numbered from its place in the column, with room under it
/// for the parts inside it, and a quote, callout or table from its own place among those.
const PARTS: u32 = 32;
const FRAME: u64 = 1 << 63;
const TABLE: u64 = 1 << 62;

/// The sheet that shows the start of a leaf, counted from 0.
fn sheet_of(pages: &Pages, leaf: usize) -> Option<usize> {
    pages.sheets.partition_point(|sheet| sheet.first <= (leaf, 0)).checked_sub(1)
}

/// A position and whether it is the end of the line that wraps there, where that place also starts the
/// next line.
type Stop = (Pos, bool);

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
    /// The formula the caret is in: its block, and which of the block's formulas it is.
    open: Option<(BlockId, usize)>,
    /// Whether the formula the caret is in shows its source in its place.
    in_place: bool,
    /// The last source of the formula the caret is in that could be typeset.
    good: Option<Arc<str>>,
    /// Whether that formula is set from it now, because its source as written cannot be typeset.
    stands: bool,
    /// A caret in the source of a formula, and the colour the term it is at is lit in.
    term: Option<(Pos, Color)>,
    /// The number of each numbered figure and gallery.
    numbers: HashMap<BlockId, u32>,
    /// The links whose pictures the shell has, which are drawn in place of their frames, each with the
    /// size of its picture.
    loaded: HashMap<String, (f32, f32)>,
    /// The figure or gallery that keeps the line of its caption while it has none.
    caption: Option<BlockId>,
    /// Whether the document has a table of contents, which reads what its headings say.
    listing: bool,
    /// What a window has told of the document's link previews.
    cards: Cards,
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
            // A table of contents says what its headings say, so its lines are read again.
            if self.listing && leaf.role.is_heading() {
                return false;
            }
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
            // A text that turns to read from the other side takes what stands beside it along: its marker
            // and indent, the bar of its quote.
            if part == Part::Text && flow::rtl(block) != leaf.words.rtl {
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
        let numbers: HashMap<BlockId, u32> = figure_numbers(doc).into_iter().collect();
        let renumbered = numbers != self.numbers;
        self.numbers = numbers;
        let changed = carry.map(|change| &change.blocks);
        let flow = flow::flow(doc, &self.types, &self.numbers, &self.cards, self.width, changed, |block| {
            // A block inside a leaf, such as a cell, has no leaf to hand over.
            let at = index.get(block)?.leaf;
            (old[at].id.as_ref() == Some(block)).then(|| std::mem::take(&mut old[at]))
        });
        (self.leaves, self.frames, self.tables) = (flow.leaves, flow.frames, flow.tables);
        // A reference reads as the number of its figure, so it is set again when the numbers change, and a
        // cell that holds one is measured again.
        for leaf in self.leaves.iter_mut().filter(|leaf| renumbered && leaf.refers()) {
            (leaf.exact, leaf.laid) = (false, None);
            for cell in leaf.cells.iter_mut().filter(|cell| !cell.words.refs.is_empty()) {
                cell.fit = None;
                self.tables[leaf.table].sized = false;
            }
        }
        index.clear();
        for (at, leaf) in self.leaves.iter().enumerate() {
            if let (What::Gallery | What::Row | What::Contents | What::Preview, Some(id)) = (leaf.what, &leaf.id) {
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
        self.listing = self.leaves.iter().any(|leaf| leaf.what == What::Contents);
        // A leaf made anew for the figure whose caption is open, as when its picture is resized, opens too.
        self.open_caption(true);
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

    fn add_fonts(&mut self, fonts: &Fonts, backup: bool) -> bool {
        if !(if backup { self.engine.back(fonts) } else { self.engine.add(fonts) }) {
            return false;
        }
        // How wide text of each type runs is an estimate taken from its face, and markers are shaped once.
        if let Some(style) = &self.style {
            self.types = Types::resolve(style, &mut self.engine);
        }
        self.shapes.clear();
        // A character that has its glyph now was met somewhere, if only in a marker.
        self.redo() || backup
    }

    fn add_word_list(&mut self, list: WordList, data: &[u8]) -> bool {
        self.engine.lists.add(list, data) && self.redo()
    }

    /// Has the leaves that were set short of something laid out again, and says whether there are any.
    fn redo(&mut self) -> bool {
        let mut again = false;
        for leaf in self.leaves.iter_mut().filter(|leaf| leaf.short) {
            (leaf.short, leaf.exact, leaf.laid) = (false, false, None);
            // A cell may be another width in its own face, or with its words kept whole.
            for cell in &mut leaf.cells {
                cell.fit = None;
            }
            if let Some(table) = self.tables.get_mut(leaf.table).filter(|_| leaf.what == What::Row) {
                table.sized = false;
            }
            again = true;
        }
        self.recount();
        again
    }

    fn set_loaded(&mut self, src: &str, size: Option<(u32, u32)>) -> bool {
        let changed = match size.map(|(width, height)| (width.max(1) as f32, height.max(1) as f32)) {
            Some(size) => self.loaded.insert(src.to_owned(), size) != Some(size),
            None => self.loaded.remove(src).is_some(),
        };
        let shows = |picture: &Picture| picture.asset.is_none() && picture.src.as_deref() == Some(src);
        // A link preview fits its picture to its slot by the picture's size.
        let slotted = |leaf: &Leaf| leaf.image.as_deref() == Some(src);
        changed && self.leaves.iter().any(|leaf| slotted(leaf) || leaf.pictures.iter().any(shows))
    }

    fn set_preview_note(&mut self, block: &BlockId, note: Option<&str>) -> bool {
        let note = note.map(str::trim).filter(|note| !note.is_empty());
        match note {
            Some(note) => self.cards.notes.insert(block.clone(), note.to_owned()),
            None => self.cards.notes.remove(block),
        };
        let Some(&Slot { leaf: at, .. }) = self.index.get(block) else { return false };
        let leaf = &mut self.leaves[at];
        let card = leaf.kind.as_ref().filter(|_| leaf.what == What::Preview);
        let Some(entries) = card.map(|kind| flow::card(kind, leaf.role, note)).filter(|said| *said != leaf.entries)
        else {
            return false;
        };
        // Its layout stays, and tells which of its lines still stand. It is given its height at once, since
        // the window draws on it.
        leaf.entries = entries;
        if std::mem::take(&mut leaf.exact) {
            self.pending += 1;
        }
        self.lay(at);
        true
    }

    fn set_preview_control(&mut self, kept: bool) -> bool {
        if std::mem::replace(&mut self.cards.bare, !kept) != kept {
            return false;
        }
        // Its lines are another width now, so the layout it held tells nothing of them.
        let unfetched = |leaf: &Leaf| leaf.what == What::Preview && leaf.kind.as_ref().is_some_and(flow::unfetched);
        let cards: Vec<usize> = (0..self.leaves.len()).filter(|at| unfetched(&self.leaves[*at])).collect();
        for at in &cards {
            self.leaves[*at].bare = !kept;
            self.unlay(*at);
        }
        !cards.is_empty()
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
        let formula = |caret: &Pos| {
            let Slot { leaf, part } = *self.index.get(&caret.block)?;
            Some((caret.block.clone(), self.leaves[leaf].formula(part, caret.offset)?.0))
        };
        let open = at.and_then(|(caret, _)| formula(caret));
        // The formula the caret has left is set as the document has it again, where it was not: its source
        // showed, or it stood as it last could be typeset.
        let moved = open != self.open;
        let reset = moved && self.shows_open();
        if reset {
            self.unlay_open();
        }
        if moved {
            (self.open, self.good, self.stands) = (open, None, false);
            self.unlay_open();
            // One that stays typeset is set at once, to know what it is set from before its source changes.
            if let Some(&Slot { leaf, .. }) = self.open_slot().filter(|_| !self.in_place) {
                self.lay(leaf);
            }
        }
        self.reveal.set(at) | reset
    }

    fn set_formula_source(&mut self, in_place: bool) {
        if self.in_place != in_place {
            // Which formula the caret is in is told again by the next reveal.
            self.unlay_open();
            (self.in_place, self.open, self.good, self.stands) = (in_place, None, None, false);
        }
    }

    fn open_slot(&self) -> Option<&Slot> {
        self.open.as_ref().and_then(|(block, _)| self.index.get(block))
    }

    /// Whether the caret's being in a formula changes what the page shows.
    fn shows_open(&self) -> bool {
        self.in_place || self.stands
    }

    /// Drops the layout of the leaf with the formula the caret is in, which is laid out again when the
    /// caret comes or goes. A cell whose formula stands as it was is measured again too: as written, it is
    /// another size.
    fn unlay_open(&mut self) {
        let Some(&Slot { leaf, part }) = self.open_slot() else { return };
        let table = self.tables.get_mut(self.leaves[leaf].table).filter(|_| self.stands);
        if let (Part::Cell(index), Some(table)) = (part, table) {
            (self.leaves[leaf].cells[index].fit, table.sized) = (None, false);
        }
        self.unlay(leaf);
    }

    /// Runs `work` on the document as it prints, where no formula shows for the caret and no caption is
    /// set for being written. The pages of one pagination are then the same wherever the caret goes
    /// meanwhile.
    fn printed<T>(&mut self, work: impl FnOnce(&mut Self) -> T) -> T {
        // The leaf of the formula the caret is in, where the caret changes how that shows.
        let shown = self.open_slot().map(|slot| slot.leaf).filter(|_| self.shows_open());
        if shown.is_some() {
            self.unlay_open();
        }
        let (open, captioned) = (self.open.take(), self.open_caption(false));
        // Every sheet below it is placed by the height it prints at, whichever sheet is asked for first.
        if let Some(leaf) = shown {
            self.measure(leaf);
        }
        let done = work(self);
        self.open = open;
        if shown.is_some() {
            self.unlay_open();
        }
        if captioned {
            self.open_caption(true);
        }
        done
    }

    fn set_caption_open(&mut self, block: Option<&BlockId>) -> bool {
        // A picture of a gallery and the gallery's caption both name the gallery.
        let leaf = block.and_then(|block| self.index.get(block)).map(|slot| &self.leaves[slot.leaf]);
        let named =
            leaf.filter(|leaf| matches!(leaf.what, What::Figure | What::Gallery)).and_then(|leaf| leaf.id.clone());
        if named == self.caption {
            return false;
        }
        let closed = self.open_caption(false);
        self.caption = named;
        self.open_caption(true) | closed
    }

    /// Makes the leaf of the figure whose caption is being written ready for that, or sets it at rest.
    /// Returns whether the leaf is laid out anew for it.
    fn open_caption(&mut self, open: bool) -> bool {
        let Some(&Slot { leaf: at, .. }) = self.caption.as_ref().and_then(|block| self.index.get(block)) else {
            return false;
        };
        if !self.leaves[at].ready(open, &self.types) {
            return false;
        }
        // It is what the selection is at, so it is given its height at once.
        self.unlay(at);
        self.lay(at);
        true
    }

    /// Drops a leaf's layout, so that it is laid out again when next needed. Its height stays as the
    /// estimate until then, so nothing jumps.
    fn unlay(&mut self, at: usize) {
        let leaf = &mut self.leaves[at];
        if leaf.laid.take().is_some() {
            self.held -= 1;
        }
        if std::mem::take(&mut leaf.exact) {
            self.pending += 1;
        }
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
        let room = leaf.frame.map_or(*margin + leaf.x, |frame| {
            let bar = if frames[frame].rtl { 0.0 } else { theme::BAR + 2.0 };
            leaf.x - frames[frame].x - bar
        });
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

    fn band(&mut self, block: &BlockId) -> Option<(f32, f32)> {
        let at = self.index.get(block)?.leaf;
        let under = |leaf: &Leaf| leaf.root == self.leaves[at].root;
        let first = self.leaves[..at].iter().rposition(|leaf| !under(leaf)).map_or(0, |above| above + 1);
        let last = self.leaves[at..].iter().position(|leaf| !under(leaf)).map_or(self.leaves.len(), |end| at + end) - 1;
        self.place(last);
        let top = first.checked_sub(1).map_or(0.0, |above| self.leaves[above].span().1);
        Some((top, self.leaves[last].span().1))
    }

    fn place_all(&mut self) {
        self.place(usize::MAX);
        self.height = self.leaves.last().map_or(0.0, |last| last.span().1);
    }

    /// Makes sure a leaf holds its layout, which also makes its height exact.
    fn lay(&mut self, at: usize) {
        self.leaves[at].used = self.clock;
        self.size(at);
        let leaf = &mut self.leaves[at];
        if leaf.laid.is_some() && leaf.exact || leaf.fixed() {
            return;
        }
        // A table of contents whose lines have changed keeps its layout, which is brought up to date.
        let stale = leaf.laid.take();
        self.held -= usize::from(stale.is_some());
        if self.held >= KEPT {
            self.evict();
        }
        let mut laid = stale.unwrap_or_default();
        self.shape(at, &mut laid);
        leaf::adorn(&mut self.engine, &self.types, &self.leaves[at], &mut laid);
        self.leaves[at].short |= self.engine.short;
        self.leaves[at].laid = Some(laid);
        self.held += 1;
    }

    /// Gives a leaf its exact height without keeping the layout.
    fn measure(&mut self, at: usize) {
        self.size(at);
        if self.leaves[at].exact {
            return;
        }
        // An edit to a heading changes the table of contents, wherever that is. So it keeps its layout:
        // bringing that up to date sets one line, and not all of them.
        if self.leaves[at].laid.is_some() || self.leaves[at].what == What::Contents {
            return self.lay(at);
        }
        let mut scratch = std::mem::take(&mut self.scratch);
        // What it last measured was another leaf's.
        scratch.set_from.clear();
        self.shape(at, &mut scratch);
        self.scratch = scratch;
    }

    /// Shapes the texts of a leaf into `laid`, and records what they say of the leaf: its height, and the
    /// lines pagination may split it at.
    fn shape(&mut self, at: usize, laid: &mut Laid) {
        let Self { engine, types, leaves, index, palette, locale, open, in_place, good, numbers, .. } = self;
        let leaf = &leaves[at];
        // Whether the leaf has the formula the caret is in, and that stays typeset.
        let slot = open.as_ref().filter(|_| !*in_place).and_then(|(block, _)| index.get(block));
        let here = slot.is_some_and(|slot| slot.leaf == at);
        let page = leaf::Page { color: ink::of(types, palette, leaf.role), numbers };
        let open =
            open.as_ref().map(|(block, which)| Open { block, which: *which, in_place: *in_place, good: good.as_ref() });
        engine.short = false;
        leaf::lay(engine, types, leaf, &Lay { open, page, locale: *locale }, laid);
        let short = engine.short;
        if here {
            // What the caret's formula is set from is kept, to stand while its source cannot be typeset.
            let set = leaf::opened(leaf, laid);
            self.stands = set.is_some_and(|(_, swap)| swap.kept.is_some());
            if let Some((part, Swap { source, kept: None, .. })) = set {
                self.good = leaf.text(part).and_then(|words| words.text.get(source.clone())).map(Arc::from);
            }
        }
        let Extent { height, lines, ends, joined } = leaf::extent(leaf, laid);
        let leaf = &mut self.leaves[at];
        (leaf.lines, leaf.ends, leaf.joined) = (lines, ends, joined);
        leaf.short |= short;
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
        let Self { engine, types, leaves, tables, palette, locale, numbers, scratch, open, in_place, good, .. } = self;
        let leaf = &leaves[row];
        let Some(table) = tables.get_mut(leaf.table).filter(|table| leaf.what == What::Row && !table.sized) else {
            return;
        };
        // A cell is as wide as its formulas are typeset, also while one shows its source for the caret.
        let open = open.as_ref().filter(|_| !*in_place);
        let open = open.map(|(block, which)| Open { block, which: *which, in_place: false, good: good.as_ref() });
        for at in table.rows.clone() {
            let page = leaf::Page { color: ink::of(types, palette, leaves[at].role), numbers };
            for index in 0..leaves[at].cells.len() {
                let leaf = &leaves[at];
                if leaf.cells[index].fit.is_some() {
                    continue;
                }
                let open = open.and_then(|open| open.of(leaf, Part::Cell(index)));
                let source = Source::cell(leaf, &leaf.cells[index], Alignment::Start, open, page);
                engine.short = false;
                let fit = leaf::fit(engine, types, &source, *locale, &mut scratch.text);
                leaves[at].cells[index].fit = Some(fit);
                leaves[at].short |= engine.short;
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

    /// Paints a band of the column. With `pages` it is a band of one of their sheets, which leaves out what
    /// belongs to editing and not to the document.
    fn paint(&mut self, viewport: Rect, pages: Option<&Pages>, out: &mut DisplayList) {
        let print = pages.is_some();
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
            let Self { engine, types, leaves, frames, reveal, shapes, numbers, loaded, .. } = self;
            let frame = &frames[frame];
            let (top, bottom) = frame.span(leaves);
            let rect = Rect::new(frame.x, top, frame.width, bottom - top);
            // The bar stands at the side the lines start at.
            let aside = if frame.rtl { rect.width - theme::BAR } else { 0.0 };
            let bar = Rect { x: rect.x + aside, width: theme::BAR, ..rect };
            match frame.look {
                Look::Quote => out.push(Item::Fill { rect: bar, color: palette.line_strong, radius: theme::BAR / 2.0 }),
                Look::Callout(tone) => {
                    let Callout { accent, fill } = frame.head.colors(palette, tone);
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
                    Painter { engine, types, inks: &inks, band, spelt: false, part: None, numbers, loaded, out }
                        .sign(shape, at, level);
                }
                at.x += width;
            }
        }
        // The boxes and rules of the leaves are decoration too, and go behind all their text.
        // An empty gallery or table is boxed only for whoever edits it.
        let backed = self.leaves[first..end].iter().filter(|leaf| !(print && leaf.what == What::Empty));
        out.items.extend(backed.filter_map(|leaf| leaf::backing(leaf, palette)));
        // A sheet says what its parts are. `around` is what is open around the leaf being painted, outermost
        // first: its quotes and callouts, and its table.
        let mut around: Vec<u64> = Vec::new();
        for at in first..end {
            // A viewport taller than the cache holds has dropped its first layouts by now.
            self.lay(at);
            let leaf = &self.leaves[at];
            let mut inside = Vec::new();
            if print {
                inside.extend((leaf.what == What::Row).then_some((TABLE | leaf.table as u64, Tag::Table)));
                let mut next = leaf.frame;
                while let Some(frame) = next {
                    let role = match self.frames[frame].look {
                        Look::Quote => Tag::Quote,
                        Look::Callout(_) => Tag::Note,
                    };
                    inside.push((FRAME | frame as u64, role));
                    next = self.frames[frame].parent;
                }
                inside.reverse();
            }
            let shared = around.iter().zip(&inside).take_while(|(open, (id, _))| *open == id).count();
            out.items.extend(around.drain(shared..).map(|_| Item::Close));
            for (id, role) in inside.drain(shared..) {
                out.push(Item::Open { role, id });
                around.push(id);
            }
            // A callout's icon and title stand above its first leaf, as part of the callout.
            let mut next = leaf.frame;
            while let Some(frame) = next {
                let (engine, types, leaves, shapes) = (&mut self.engine, &self.types, &self.leaves, &mut self.shapes);
                let frame = &self.frames[frame];
                next = frame.parent;
                let Look::Callout(tone) = frame.look else { continue };
                if frame.first != at {
                    continue;
                }
                let color = frame.head.colors(palette, tone).accent;
                let (x, width) = (frame.x + theme::BOX_PAD_X, (frame.width - theme::BOX_PAD_X * 2.0).max(0.0));
                let head = Rect::new(x, frame.span(leaves).0 + theme::BOX_PAD_Y, width, 0.0);
                let line = if frame.head.line > 0.0 { frame.head.line } else { types.get(leaves[at].role).line };
                // The leaf's later lines can be all that shows of it, as on the sheet after its first.
                if head.y >= band.1 || head.y + line <= band.0 {
                    continue;
                }
                // The icon leads and the title follows it, from the side the callout's lines start at.
                let mut after = 0.0;
                if let Some((path, scale, stroke)) = frame.head.icon.and_then(callout::outline) {
                    let x = frame.headed(after, callout::ICON);
                    let origin = Point { x, y: head.y + ((line - callout::ICON) / 2.0).round() };
                    out.push(Item::Path { origin, scale, path, color, stroke: Some(stroke) });
                    after += callout::ICON + callout::ICON_GAP;
                }
                if frame.head.title.is_empty() {
                    continue;
                }
                let (shape, wide) = shapes.title(engine, types, &frame.head.title);
                let x = frame.headed(after, wide);
                let Some(line) = shapes.layout(shape).and_then(|shape| shape.get(0)) else { continue };
                // A title longer than the callout is wide is cut at its edge.
                out.push(Item::PushClip { rect: Rect { height: frame.head.line, ..head }, radius: 0.0 });
                let mut clusters = line.runs().flat_map(|run| run.visual_clusters()).peekable();
                for item in line.items() {
                    let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                    let mut glyphs = engine.glyphs(&run, Point { x, y: head.y }, color);
                    // A sheet says what the title spells, as it does of the text under it.
                    if let (Some(item), true) = (&mut glyphs, print) {
                        leaf::spell(item, &mut clusters, &frame.head.title);
                    }
                    out.items.extend(glyphs);
                }
                out.push(Item::PopClip);
            }
            // A row holds its cells.
            let name = print.then_some((at as u64 + 1) << PARTS);
            let row = name.filter(|_| leaf.what == What::Row);
            out.items.extend(row.map(|id| Item::Open { role: Tag::Row, id }));
            // The leaf's own text and what stands around it, then the text of each of its cells.
            let cells = leaf.cells.len();
            for part in [Part::Text].into_iter().chain((0..cells).map(Part::Cell)) {
                let shifts = if print { Shifts::default() } else { self.shifts(at, part) };
                let lit = if print { None } else { self.lit(at, part) };
                let Self {
                    engine, types, leaves, index, tokens, spelling, shapes, composition, numbers, loaded, ..
                } = self;
                let leaf = &leaves[at];
                let Some(laid) = &leaf.laid else { continue };
                // What belongs to editing is found by the block the text is of.
                let block = leaf.owner(part).filter(|_| !print);
                let own = |id: &BlockId| block == Some(id);
                let composing = composition.as_ref().filter(|(block, _)| own(block)).map(|(_, range)| range.clone());
                let spelling = block.and_then(|id| spelling.get(id)).map_or(&[][..], Vec::as_slice);
                let sheet_of = &|block: &BlockId| sheet_of(pages?, index.get(block)?.leaf);
                let over = Over { shifts: &shifts, shapes, composing, spelling, sheet_of, lit };
                let mut painter =
                    Painter { engine, types, inks: &inks, band, spelt: print, part: name, numbers, loaded, out };
                match part {
                    Part::Cell(index) => painter.cell(leaf, index, &over),
                    _ => {
                        let tokens = leaf.id.as_ref().and_then(|id| tokens.get(id));
                        painter.leaf(leaf, laid, tokens.map(Vec::as_slice), &over);
                    }
                }
            }
            out.items.extend(row.map(|_| Item::Close));
        }
        out.items.extend(around.drain(..).map(|_| Item::Close));
    }

    /// The term to light in a text of a leaf: the one at the caret [`DocLayout::tint_term`] was told, where
    /// that is in a formula of this text.
    fn lit(&self, at: usize, part: Part) -> Option<Lit> {
        let (caret, color) = self.term.as_ref()?;
        let leaf = &self.leaves[at];
        let (_, source) = leaf.formula(part, caret.offset).filter(|_| leaf.owner(part) == Some(&caret.block))?;
        Some(Lit { caret: caret.offset - source.start, source, color: *color })
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
            self.paint(Rect::new(0.0, top + 0.25, self.width, bottom - top - 0.5), Some(pages), out);
            out.push(Item::PopClip);
            out.translate(from, origin);
        }
        // The page number sits in the middle of the margin beneath the text, if the margin has room for it.
        let Self { engine, types, scratch, palette, numbers, loaded, .. } = self;
        let (label, number) = ((index + 1).to_string(), &mut scratch.text.layout);
        let spec = types.get(Role::Caption).spec(false, Brush { ink: Ink::Style(Role::Caption), back: None });
        engine.build(&label, &[spec], &[(0, 0..label.len())], None, number);
        number.break_all_lines(None);
        let (below, foot) = (area.y + area.height, pages.height - area.y - area.height);
        if foot >= number.height() {
            let x = area.x + (area.width - number.full_width()) / 2.0;
            let at = Point { x: x.round(), y: below + ((foot - number.height()) / 2.0).round() };
            let inks = Inks::new(types, *palette);
            // The number is of the sheet and not of the document, so it is part of nothing.
            let band = (f32::MIN, f32::MAX);
            Painter { engine, types, inks: &inks, band, spelt: true, part: None, numbers, loaded, out }
                .glyphs(number, &label, at);
        }
    }

    fn page_of(&mut self, pages: &Pages, pos: &Pos) -> Option<usize> {
        let Slot { leaf: at, part } = *self.index.get(&pos.block)?;
        self.clock += 1;
        self.lay(at);
        let leaf = &self.leaves[at];
        let line = Text::of(leaf, part).map_or(0, |text| leaf::line_of(text.set, pos.offset, false));
        let line = leaf.line_of(part, line);
        pages.sheets.partition_point(|sheet| sheet.first <= (at, line)).checked_sub(1)
    }

    fn page_blocks(&mut self, pages: &Pages, index: usize) -> Vec<PageBlock> {
        self.place_all();
        paginate::blocks(&self.leaves, &self.frames, pages, index)
    }

    fn page_links(&mut self, pages: &Pages, index: usize) -> Vec<PageLink> {
        let mut links = Vec::new();
        let (Some(sheet), Some(origin)) = (pages.sheets.get(index), pages.origin(index)) else { return links };
        self.clock += 1;
        self.place_all();
        for (block, lines) in &sheet.blocks {
            let Some(at) = self.index.get(block).map(|slot| slot.leaf) else { continue };
            if self.leaves[at].stacks() {
                self.lay(at);
                self.entry_links(pages, at, lines, origin, &mut links);
                continue;
            }
            let linked = |words: &Words| !words.links.is_empty() || !words.refs.is_empty();
            if !linked(&self.leaves[at].words) && !self.leaves[at].cells.iter().any(|cell| linked(&cell.words)) {
                continue;
            }
            self.lay(at);
            let leaf = &self.leaves[at];
            for part in [Part::Text].into_iter().chain((0..leaf.cells.len()).map(Part::Cell)) {
                let Some(text) = Text::of(leaf, part) else { continue };
                let out = text.source.links.iter().map(|(range, target)| (range, target.clone(), None));
                // A reference leads to its figure when it names one. Each link and reference is numbered as
                // painting numbers them, which names its text on the sheet.
                let places =
                    text.source.refs.iter().map(|(range, target)| (range, String::new(), self.leads_to(pages, target)));
                let led = |(_, target, place): &(_, String, Option<_>)| !target.is_empty() || place.is_some();
                let mut found: Vec<_> = out.chain(places).enumerate().filter(|(_, link)| led(link)).collect();
                found.sort_by_key(|(_, (range, ..))| range.start);
                for (which, (range, target, place)) in found {
                    let name = ((at as u64 + 1) << PARTS) | leaf::link(part, which);
                    let named = (leaf.role != Role::Code).then_some(name);
                    leaf::stretches(text.set, range, &Shifts::default(), |line, part_of| {
                        if lines.contains(&leaf.line_of(part, line)) {
                            let (x, y) = (origin.x + text.at.x + part_of.x, origin.y + text.at.y + part_of.y);
                            let (rect, target) = (Rect { x, y, ..part_of }, target.clone());
                            links.push(PageLink { rect, target, place, part: named });
                        }
                    });
                }
            }
        }
        links
    }

    /// The links of the lines of a table of contents that a sheet holds, or the one link that is the card
    /// of a link preview. Each is named as painting names its part.
    fn entry_links(&self, pages: &Pages, at: usize, lines: &Range<usize>, origin: Point, links: &mut Vec<PageLink>) {
        let leaf = &self.leaves[at];
        let on_sheet = |rect: Rect| Rect { x: origin.x + rect.x, y: origin.y + rect.y, ..rect };
        let named = |part: Part| Some(((at as u64 + 1) << PARTS) | leaf::link(part, 0));
        if let Some(BlockKind::LinkPreview { url, .. }) = &leaf.kind {
            let (rect, part) = (on_sheet(leaf::boxed(leaf)), named(Part::Text));
            links.extend(lines.contains(&0).then(|| PageLink { rect, target: url.clone(), place: None, part }));
            return;
        }
        let Some(laid) = &leaf.laid else { return };
        for (index, (entry, _, top, height)) in leaf::stack(leaf, laid, &self.types).enumerate() {
            let place = entry.target.as_ref().and_then(|target| self.shown_at(pages, target));
            if let (true, Some(place)) = (lines.contains(&index), place) {
                let rect = on_sheet(Rect::new(leaf.x, leaf.y + top.y, leaf.width, height));
                links.push(PageLink {
                    rect,
                    target: String::new(),
                    place: Some(place),
                    part: named(Part::Cell(index)),
                });
            }
        }
    }

    /// Where a sheet shows the figure that a reference to `target` reads as: the sheet, and the top left of
    /// the figure's picture on it. `None` for a block with no number, which no reference names.
    fn leads_to(&self, pages: &Pages, target: &BlockId) -> Option<(usize, Point)> {
        self.shown_at(pages, target).filter(|_| self.numbers.contains_key(target))
    }

    /// Where a sheet shows the start of a block: the sheet, and the top left of the block on it.
    fn shown_at(&self, pages: &Pages, target: &BlockId) -> Option<(usize, Point)> {
        let at = self.index.get(target)?.leaf;
        let leaf = &self.leaves[at];
        let page = sheet_of(pages, at)?;
        let origin = pages.origin(page)?;
        // A figure's picture stands where its caption's lines do. A gallery starts at the column's edge.
        let across = leaf.pictures.first().filter(|_| leaf.what == What::Figure).map_or(0.0, |picture| picture.rect.x);
        Some((page, Point { x: origin.x + leaf.x + across, y: origin.y + leaf.y }))
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

    /// The position at a point and whether it is the end of the line that wraps there, among the leaves of a
    /// stretch of the column if `band` gives one.
    fn hit(&mut self, point: Point, band: Option<(f32, f32)>) -> Option<Stop> {
        self.clock += 1;
        self.place_all();
        let (first, end) = match band {
            Some((top, bottom)) => {
                let first = self.leaves.partition_point(|leaf| leaf.span().1 <= top);
                (first, self.leaves.partition_point(|leaf| leaf.span().0 < bottom).max(first))
            }
            None => (0, self.leaves.len()),
        };
        let at = first + self.leaves[first..end].partition_point(|leaf| leaf.y + leaf.height < point.y);
        // The nearest leaf that a position can be in, above or below.
        let below = self.leaves[at..end].iter().position(Leaf::holds).map(|skip| at + skip);
        let above = self.leaves[first..at].iter().rposition(Leaf::holds).map(|back| first + back);
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

    /// A position in a part of a laid leaf: where `stop` says in its text, with the side it says, or on it
    /// when it is a picture.
    fn find(&mut self, at: usize, part: Part, stop: impl FnOnce(&Text<'_>, &Shifts) -> (usize, bool)) -> Option<Stop> {
        let shifts = self.shifts(at, part);
        let leaf = &self.leaves[at];
        let block = leaf.owner(part)?.clone();
        let (offset, before_wrap) = Text::of(leaf, part).map_or((0, false), |text| stop(&text, &shifts));
        Some((Pos { block, offset }, before_wrap))
    }

    /// A stop for a caller that takes a position alone. One that ends a line where the next begins goes back
    /// before that line's last character, which is the last place on the line that needs no side.
    fn unsided(&self, (pos, before_wrap): Stop) -> Pos {
        let text = self.index.get(&pos.block).filter(|_| before_wrap);
        let text = text.and_then(|slot| Text::of(&self.leaves[slot.leaf], slot.part));
        let offset = text.map_or(pos.offset, |text| leaf::back(text.set, pos.offset));
        Pos { offset, ..pos }
    }

    fn caret(&mut self, pos: &Pos, before_wrap: bool) -> Option<Rect> {
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
        if leaf.what == What::Empty {
            return Some(Rect::new(leaf.x, leaf.y, theme::CARET, leaf.height));
        }
        Some(leaf::caret(&Text::of(leaf, part)?, &shifts, pos.offset, before_wrap))
    }

    fn formula_rect(&mut self, pos: &Pos) -> Option<Rect> {
        let Slot { leaf: at, part } = *self.index.get(&pos.block)?;
        let (_, source) = self.leaves[at].formula(part, pos.offset)?;
        self.clock += 1;
        self.lay(at);
        self.place(at);
        let shifts = self.shifts(at, part);
        let leaf = &self.leaves[at];
        // A formula block with nothing written in it still has its place.
        let whole = || Rect::new(leaf.x, leaf.y, leaf.width, leaf.height);
        let found = leaf::formula_at(&Text::of(leaf, part)?, &shifts, &source);
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

    fn linked(&mut self, viewport: Rect) -> Vec<(BlockId, Rect)> {
        let (first, end) = self.show(viewport.y, viewport.y + viewport.height);
        let mut found = Vec::new();
        for leaf in &self.leaves[first..end] {
            let links = leaf.pictures.iter().filter(|picture| picture.asset.is_none());
            found.extend(links.filter_map(|picture| {
                let image = Rect { x: leaf.x + picture.rect.x, y: leaf.y + picture.rect.y, ..picture.rect };
                Some((picture.id.clone(), leaf::badge(image)?))
            }));
        }
        found
    }

    fn callouts(&mut self, viewport: Rect) -> Vec<(BlockId, Rect)> {
        let (first, end) = self.show(viewport.y, viewport.y + viewport.height);
        let mut found = Vec::new();
        for at in 0..self.frames.len() {
            let Self { engine, types, leaves, frames, shapes, .. } = self;
            let frame = &frames[at];
            if !(first..end).contains(&frame.first) || !matches!(frame.look, Look::Callout(_)) {
                continue;
            }
            let icon = frame.head.icon.map_or(0.0, |_| callout::ICON);
            let title = match frame.head.title.as_str() {
                "" => 0.0,
                title => shapes.title(engine, types, title).1 + if icon > 0.0 { callout::ICON_GAP } else { 0.0 },
            };
            let line = if frame.head.line > 0.0 { frame.head.line } else { types.get(leaves[frame.first].role).line };
            let width = (icon + title).min((frame.width - theme::BOX_PAD_X * 2.0).max(0.0));
            let rect = Rect::new(frame.headed(0.0, width), frame.span(leaves).0 + theme::BOX_PAD_Y, width, line);
            if rect.width > 0.0 {
                found.push((frame.id.clone(), rect));
            }
        }
        found
    }

    fn code_boxes(&mut self, viewport: Rect) -> Vec<(BlockId, Rect)> {
        let (first, end) = self.show(viewport.y, viewport.y + viewport.height);
        let coded = self.leaves[first..end].iter().filter(|leaf| matches!(leaf.kind, Some(BlockKind::Code { .. })));
        coded.filter_map(|leaf| Some((leaf.id.clone()?, leaf::boxed(leaf)))).collect()
    }

    fn tables(&mut self, viewport: Rect) -> Vec<TableBox> {
        let (first, end) = self.show(viewport.y, viewport.y + viewport.height);
        let mut shown: Vec<usize> =
            self.leaves[first..end].iter().filter(|leaf| leaf.what == What::Row).map(|leaf| leaf.table).collect();
        shown.dedup();
        let boxed = |table: usize| {
            let rows = self.leaves.get(self.tables.get(table)?.rows.clone())?;
            let (top, last) = (rows.first()?, rows.last()?);
            let rect = Rect::new(top.x, top.y, top.width, last.y + last.height - top.y);
            // Every row has its cells at the same places, and the longest has them all.
            let widest = rows.iter().max_by_key(|row| row.cells.len())?;
            let pad = theme::CELL_PAD_X;
            let columns = widest.cells.iter().map(|cell| (top.x + cell.x - pad, cell.width + 2.0 * pad)).collect();
            let cells = |row: &Leaf| row.cells.iter().map(|cell| cell.id.clone()).collect();
            let rows = rows.iter().map(|row| TableRow { y: row.y, height: row.height, cells: cells(row) }).collect();
            Some(TableBox { rect, columns, rows })
        };
        shown.into_iter().filter_map(boxed).collect()
    }

    fn contents(&mut self, viewport: Rect) -> Vec<ContentsBox> {
        let (first, end) = self.show(viewport.y, viewport.y + viewport.height);
        let mut found = Vec::new();
        for at in first..end {
            if self.leaves[at].what != What::Contents {
                continue;
            }
            // A viewport taller than the cache holds has dropped its first layouts by now.
            self.lay(at);
            let leaf = &self.leaves[at];
            let (Some(block), Some(laid)) = (leaf.id.clone(), &leaf.laid) else { continue };
            let across = |top: f32, height: f32| Rect::new(leaf.x, leaf.y + top, leaf.width, height);
            let lines = leaf::stack(leaf, laid, &self.types)
                .filter_map(|(entry, _, at, height)| Some((entry.target.clone()?, across(at.y, height))));
            found.push(ContentsBox { block, rect: across(0.0, leaf.height), lines: lines.collect() });
        }
        found
    }

    fn previews(&mut self, viewport: Rect) -> Vec<PreviewBox> {
        let (first, end) = self.show(viewport.y, viewport.y + viewport.height);
        let cards = self.leaves[first..end].iter().filter(|leaf| leaf.what == What::Preview);
        let boxed = |leaf: &Leaf| {
            let spare = leaf.spare(&self.types);
            let control = Rect::new(leaf.x + leaf.width - spare, leaf.y, spare, leaf.height);
            Some(PreviewBox {
                block: leaf.id.clone()?,
                rect: leaf::boxed(leaf),
                control: (spare > 0.0).then_some(control),
            })
        };
        cards.filter_map(boxed).collect()
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

    fn line_step(&mut self, pos: &Pos, before_wrap: bool, x: f32, down: bool) -> Option<Stop> {
        let Slot { leaf: at, part } = *self.index.get(&pos.block)?;
        self.clock += 1;
        self.lay(at);
        if let Some(inside) = self.step_inside(at, part, (pos.offset, before_wrap), x, down) {
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
            What::Empty => Part::Whole,
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
    fn step_inside(&mut self, at: usize, part: Part, from: (usize, bool), x: f32, down: bool) -> Option<Stop> {
        let leaf = &self.leaves[at];
        let on = |picture: usize| Some((Pos { block: leaf.pictures.get(picture)?.id.clone(), offset: 0 }, false));
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
        let line = leaf::line_of(text.set, from.0, from.1);
        let lines = text.set.layout.len();
        match if down { Some(line + 1).filter(|next| *next < lines) } else { line.checked_sub(1) } {
            Some(line) => self.find(at, part, |text, shifts| leaf::on_line(text, shifts, line, x)),
            // Above a gallery's caption is its last row of pictures.
            None if !down && leaf.what == What::Gallery && rows > 0 => on(leaf.picture_in(rows - 1, x)?),
            None => None,
        }
    }

    fn line_edge(&mut self, pos: &Pos, before_wrap: bool, end: bool) -> Option<Stop> {
        let Slot { leaf: at, part } = *self.index.get(&pos.block)?;
        self.clock += 1;
        self.lay(at);
        // A picture is a line of its own.
        let Some(text) = Text::of(&self.leaves[at], part) else { return Some((pos.clone(), false)) };
        // Far off either side, which no marker reaches. A line read from the right ends at its left.
        let x = if end != text.source.rtl { f32::MAX } else { f32::MIN };
        let line = leaf::line_of(text.set, pos.offset, before_wrap);
        let (offset, before_wrap) = leaf::on_line(&text, &Shifts::default(), line, x);
        Some((Pos { block: pos.block.clone(), offset }, before_wrap))
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
