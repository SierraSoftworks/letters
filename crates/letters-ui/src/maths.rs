//! The sheet under a formula: its TeX source, out of band, while the caret is in it.
//!
//! The formula stays typeset on the page. Its source is written on a sheet that slides out from under it
//! when the caret reaches it, and the caret shows there, at the place in the source the editor holds it.
//! The sheet is as many rows high as the source has lines, and a line too long for it goes on in a new row.
//! A backslash typed there lists the symbols that match what follows it, each one typeset. The `$` that
//! closes a formula blends the source it was typed as into its glyphs.
//!
//! The source is coloured as it is read: commands, words, comments, and each pair of brackets by how deep
//! it is nested. What is wrong is in red, and what the maths engine says of it is under the source. Neither
//! is said of what is still being written where the caret is.

use std::ops::Range;
use std::rc::Rc;

use accesskit::Role;
use letters_editor::{Editor, LinePos, Lines};
use letters_math::{Command as Symbol, Formula, TEXT_RATIO, commands, typeset};
use letters_model::{BlockId, BlockKind, Pos, Selection};
use letters_render::{Color, DisplayList, Item, Point, Rect, Size};

use crate::color::{fade, resolve};
use crate::element::{Cursor, Element, Paint, Pointer};
use crate::layout::MARGIN;
use crate::motion::{self, FRAME_MS};
use crate::style::{Align, Edges};
use crate::tex::{self, Part};
use crate::text::{Family, Shaped, TextShaper, TextStyle};
use crate::theme::accent;
use crate::tokens::{self, Color as Ink, Scheme};
use crate::widgets::{Choice, Options, Popover, options};
use crate::window::{CARET_WIDTH, Message};

const TEXT: TextStyle = TextStyle::control().family(Family::Mono).size(tokens::TEXT_SM).leading(tokens::LEADING);
/// The room round the source on its sheet: tight above, where the sheet comes out from under the formula.
const PADDING: Edges =
    Edges { top: tokens::SPACE_XS, right: tokens::SPACE_MD, bottom: tokens::SPACE_SM, left: tokens::SPACE_MD };
/// How far the tint behind an open formula reaches past it, which is also how far under it the sheet hangs.
pub(crate) const HALO: f32 = 3.0;
/// How much wider than its formula a sheet is at the least.
const WIDER: f32 = 24.0;
/// The room kept past the end of the source for a caret there, whole however the sheet's edges snap to
/// device pixels.
const CARET_ROOM: f32 = CARET_WIDTH + 1.0;
/// How long the sheet takes to slide out, in milliseconds.
const SLIDE_MS: f32 = 140.0;
/// How far together the source of a closed formula is pressed at the most, as a share of its width.
const SQUEEZE: f32 = 0.5;
/// How many symbols the list shows, and the least room it takes across.
const LISTED: usize = 6;
const LIST_WIDTH: f32 = 300.0;
/// The least room across for what the maths engine says is wrong.
const FAULT_WIDTH: f32 = 240.0;
/// The colours of bracket pairs, by how deep a pair is nested. None is near the red of a mistake.
const PAIRS: [Ink; 3] = [tokens::SYNTAX_NUMBER, tokens::SYNTAX_KEYWORD, tokens::SYNTAX_TYPE];
/// The room a symbol's picture has in its row: at least this wide, and this high.
const PICTURE: Size = Size { width: 24.0, height: 22.0 };

/// What the pointer and the keyboard ask of the sheet.
#[derive(Clone, Debug, PartialEq)]
pub enum Sheet {
    /// A press on the source, at an offset in its block. `extend` keeps the anchor, as Shift does, and
    /// `before_wrap` is for a press past the end of a row that goes on with no space: see [`LinePos`].
    Press {
        offset: usize,
        before_wrap: bool,
        extend: bool,
        clicks: u8,
    },
    /// The pointer moved over the source with the button held, to an offset and its side as for a press.
    Drag(usize, bool),
    Release,
    /// A symbol of the list was pressed, counted from the first.
    Take(usize),
    /// Escape, which puts the list of symbols away, and steps out of the formula when there is none.
    Leave,
}

/// A row of a source as its sheet shows it: a line, or a part of one that is too long for the sheet.
struct Row {
    /// Where it starts in the block.
    start: usize,
    shaped: Shaped,
    /// Whether its line goes on in the next row.
    wrapped: bool,
    /// Whether it goes on with no space to end at, so that where it ends is where the next row starts.
    unspaced: bool,
}

impl Row {
    /// Where the row ends in the block.
    fn end(&self) -> usize {
        self.start + self.shaped.stops.last().map_or(0, |stop| stop.offset)
    }

    /// The last place in the row where the caret is in it, and whether that is where the next row starts
    /// too: see [`LinePos`]. A row that goes on after a space ends before the space.
    fn last(&self) -> (usize, bool) {
        let end = self.shaped.stops.last().map_or(0, |stop| stop.offset);
        let spaced = self.wrapped && !self.unspaced;
        (self.start + if spaced { self.shaped.previous(end) } else { end }, self.unspaced)
    }

    /// The offset in the block nearest a distance from the row's left edge, with its side of the row's end.
    fn nearest(&self, x: f32) -> (usize, bool) {
        let (at, last) = (self.start + self.shaped.offset_at(x), self.last());
        if at < last.0 { (at, false) } else { last }
    }
}

/// Where a line too long for `wide` goes on in a new row, as offsets in the line: after a space where the
/// row has one, else between two characters.
fn breaks(line: &str, shaped: &Shaped, wide: f32) -> Vec<usize> {
    let mut breaks = Vec::new();
    let mut from = 0;
    loop {
        let left = shaped.x_at(from);
        // A row takes what fits, and one character whatever the room.
        let fits = shaped.stops.iter().rev().find(|stop| stop.offset > from && stop.x - left <= wide);
        let limit = fits.map_or_else(|| shaped.next(from), |stop| stop.offset);
        if limit >= line.len() || limit <= from {
            return breaks;
        }
        from = line[from..limit].rfind(' ').map_or(limit, |space| from + space + 1);
        breaks.push(from);
    }
}

/// A formula's source, shaped row by row as the sheet shows it.
pub(crate) struct Source {
    pub block: BlockId,
    /// Where the source sits in its block's text.
    pub range: Range<usize>,
    pub text: String,
    /// Whether the formula is a block of its own.
    display: bool,
    /// The part an input method is still composing, as offsets in the block.
    composing: Option<Range<usize>>,
    rows: Vec<Row>,
}

impl Source {
    /// The source of the formula the editor's caret is in, in rows no more than `wide` across.
    pub fn of(editor: &Editor, shaper: &dyn TextShaper, wide: f32) -> Option<Self> {
        let (block, range) = editor.formula()?;
        let held = editor.doc().block(block)?;
        let (text, display) = (held.plain_text(), held.kind == BlockKind::Formula);
        let composing = editor.composition().filter(|composing| composing.0 == block).map(|composing| composing.1);
        let text = text.get(range.clone())?;
        Some(Self { composing, ..Self::shaped(block.clone(), range.start, text, display, shaper, wide) })
    }

    /// A source that starts `start` bytes into its block, in rows no more than `wide` across.
    fn shaped(block: BlockId, start: usize, text: &str, display: bool, shaper: &dyn TextShaper, wide: f32) -> Self {
        let mut rows = Vec::new();
        let mut at = start;
        for line in text.split('\n') {
            let whole = shaper.shape(line, &TEXT, None);
            let breaks = breaks(line, &whole, wide);
            let mut from = 0;
            for to in breaks.iter().copied() {
                let (part, start) = (&line[from..to], at + from);
                let shaped = shaper.shape(part, &TEXT, None);
                rows.push(Row { start, shaped, wrapped: true, unspaced: !part.ends_with(' ') });
                from = to;
            }
            let rest = if from == 0 { whole } else { shaper.shape(&line[from..], &TEXT, None) };
            rows.push(Row { start: at + from, shaped: rest, wrapped: false, unspaced: false });
            at += line.len() + 1;
        }
        Self { block, range: start..start + text.len(), text: text.to_owned(), display, composing: None, rows }
    }

    /// The stretches of the source that are not in plain ink with the colour of each, and those a line is
    /// drawn under because they are wrong. `refused` is what the maths engine will not have, and `writing`
    /// says that a bracket left open may yet be closed.
    fn marks(&self, refused: Option<&Range<usize>>, writing: bool) -> (Vec<Stretch>, Vec<Stretch>) {
        let within = |range: &Range<usize>| self.range.start + range.start..self.range.start + range.end;
        let (mut inks, mut lined) = (Vec::new(), Vec::new());
        for token in tex::parts(&self.text) {
            let (ink, wrong) = match token.part {
                Part::Command => (tokens::SYNTAX_FUNCTION, false),
                Part::Text => (tokens::SYNTAX_STRING, false),
                Part::Comment => (tokens::SYNTAX_COMMENT, false),
                Part::Paired(depth) => (PAIRS[depth % PAIRS.len()], false),
                Part::Open(depth) if writing => (PAIRS[depth % PAIRS.len()], false),
                Part::Open(_) | Part::Stray => (tokens::DANGER, true),
            };
            let spans = self.spans(&within(&token.range));
            lined.extend(spans.iter().filter(|_| wrong).map(|span| (*span, ink)));
            inks.extend(spans.into_iter().map(|span| (span, ink)));
        }
        // Where something is missing the line is under what stands before it.
        let missing = |range: &Range<usize>| {
            let before = self.text[..range.start].chars().next_back().filter(|_| range.is_empty());
            range.start - before.map_or(0, char::len_utf8)..range.end
        };
        let refused = refused.map(missing);
        lined.extend(refused.iter().flat_map(|range| self.spans(&within(range))).map(|span| (span, tokens::DANGER)));
        (inks, lined)
    }

    /// What the maths engine will not typeset in the source, if anything.
    fn fault(&self) -> Option<Fault> {
        let style =
            letters_math::Style { size: TEXT.size, color: Color::rgb(0, 0, 0), display: self.display, links: false };
        let error = typeset(&self.text, &style).err()?;
        Some(Fault { range: error.span, message: error.message })
    }

    fn line_height() -> f32 {
        TEXT.size * TEXT.leading
    }

    pub fn size(&self) -> Size {
        let width = self.rows.iter().map(|row| row.shaped.size.width).fold(0.0, f32::max);
        Size { width, height: self.rows.len() as f32 * Self::line_height() }
    }

    /// Whether a position is in this source, either end included.
    fn holds(&self, pos: &Pos) -> bool {
        pos.block == self.block && (self.range.start..=self.range.end).contains(&pos.offset)
    }

    /// The row an offset in the block is on. Where a row goes on with no space, its end is the start of
    /// the next row, unless `before_wrap` says the row that ends there is meant.
    fn row(&self, offset: usize, before_wrap: bool) -> usize {
        let at = self.rows.iter().rposition(|row| row.start <= offset).unwrap_or(0);
        let ends = |before: &usize| before_wrap && self.rows[*before].unspaced && self.rows[at].start == offset;
        at.checked_sub(1).filter(ends).unwrap_or(at)
    }

    /// Where the caret is drawn for an offset in the block, from the top left of the source: with
    /// `before_wrap`, at the end of a row that goes on there with no space.
    pub fn caret(&self, offset: usize, before_wrap: bool) -> Rect {
        let at = self.row(offset, before_wrap);
        let row = &self.rows[at];
        let (x, height) = (row.shaped.x_at(offset.saturating_sub(row.start)), Self::line_height());
        Rect::new(x, at as f32 * height, CARET_WIDTH, height)
    }

    /// The offset in the block nearest a point of the source, and whether that is the end of a row that goes
    /// on with no space, as a point past such a row's end is.
    pub fn hit(&self, at: Point) -> (usize, bool) {
        let row = (at.y / Self::line_height()).floor().max(0.0) as usize;
        self.rows[row.min(self.rows.len() - 1)].nearest(at.x)
    }

    /// The offset one row up or down from `offset`, nearest to `x`, with its side. `None` past the first and
    /// last row.
    fn step(&self, offset: usize, before_wrap: bool, x: f32, down: bool) -> Option<(usize, bool)> {
        let row = self.row(offset, before_wrap);
        Some(self.rows.get(if down { row + 1 } else { row.checked_sub(1)? })?.nearest(x))
    }

    /// The command being typed at an offset in the block: where it sits, from its backslash to its last
    /// letter, and the letters before the offset.
    pub fn typed(&self, offset: usize) -> Option<(Range<usize>, &str)> {
        let at = offset.checked_sub(self.range.start)?;
        let before = self.text.get(..at)?;
        let letters = before.bytes().rev().take_while(u8::is_ascii_alphabetic).count();
        let slash = (at - letters).checked_sub(1).filter(|&slash| before.as_bytes()[slash] == b'\\')?;
        // The second backslash of `\\`, which ends a row, begins no command.
        if before[..slash].bytes().rev().take_while(|byte| *byte == b'\\').count() % 2 == 1 {
            return None;
        }
        let after = self.text[at..].bytes().take_while(u8::is_ascii_alphabetic).count();
        Some((self.range.start + slash..self.range.start + at + after, &before[slash + 1..]))
    }

    /// The rectangles under a stretch of the source, one per row it touches.
    fn spans(&self, range: &Range<usize>) -> Vec<Rect> {
        let height = Self::line_height();
        let span = |at: usize| {
            let row = &self.rows[at];
            let (from, to) = (range.start.max(row.start), range.end.min(row.end()));
            let x = |offset: usize| row.shaped.x_at(offset.saturating_sub(row.start));
            (from < to).then(|| Rect::new(x(from), at as f32 * height, x(to) - x(from), height))
        };
        (self.row(range.start, false)..=self.row(range.end, false)).filter_map(span).collect()
    }
}

/// A stretch of the source, from its top left, and the colour it or the line under it is drawn in.
type Stretch = (Rect, Ink);

/// What the maths engine will not typeset in a source.
struct Fault {
    /// Where it is in the source, when the engine can say. Empty where something is missing.
    range: Option<Range<usize>>,
    message: String,
}

impl Fault {
    /// Whether the caret, at `offset` in the source, is where the fault is: just after it or inside it,
    /// which is where it is still being written.
    fn written_at(&self, offset: usize) -> bool {
        self.range.as_ref().is_some_and(|range| match range.is_empty() {
            true => offset == range.start,
            false => range.start < offset && offset <= range.end,
        })
    }
}

/// The lines the editor moves by while the caret is on a sheet: those of the source, and the page's beyond
/// its first and last.
pub(crate) struct SheetLines<L> {
    pub page: L,
    pub source: Option<Source>,
    /// Where the source's left edge is on the page, so that a column kept across lines means the same on
    /// the sheet as beside it.
    pub left: f32,
}

impl<L> SheetLines<L> {
    fn on_sheet(&self, pos: &Pos) -> Option<&Source> {
        self.source.as_ref().filter(|source| source.holds(pos))
    }
}

impl<L: Lines> Lines for SheetLines<L> {
    fn caret_x(&self, at: &LinePos) -> Option<f32> {
        match self.on_sheet(&at.pos) {
            Some(source) => Some(self.left + source.caret(at.pos.offset, at.before_wrap).x),
            None => self.page.caret_x(at),
        }
    }

    fn line_step(&self, at: &LinePos, x: f32, down: bool) -> Option<LinePos> {
        let pos = &at.pos;
        let source = self.on_sheet(pos);
        match source.and_then(|source| source.step(pos.offset, at.before_wrap, x - self.left, down)) {
            Some((offset, before_wrap)) => Some(LinePos { pos: Pos::new(pos.block.clone(), offset), before_wrap }),
            None => self.page.line_step(at, x, down),
        }
    }

    fn line_edge(&self, at: &LinePos, end: bool) -> Option<LinePos> {
        let pos = &at.pos;
        match self.on_sheet(pos) {
            Some(source) => {
                let row = &source.rows[source.row(pos.offset, at.before_wrap)];
                let (offset, before_wrap) = if end { row.last() } else { (row.start, false) };
                Some(LinePos { pos: Pos::new(pos.block.clone(), offset), before_wrap })
            }
            None => self.page.line_edge(at, end),
        }
    }

    fn page_lines(&self) -> usize {
        self.page.page_lines()
    }
}

/// The symbols that match what is typed after a backslash, best first: the command of that name, those
/// that begin with it, then those whose plain name, or another word for them, has a word that does.
pub(crate) fn matching(typed: &str) -> Vec<&'static Symbol> {
    let lower = typed.to_ascii_lowercase();
    let begins = |text: &str| text.to_ascii_lowercase().starts_with(&lower);
    let worded = |text: &str| !typed.is_empty() && text.split(' ').any(begins);
    let rank = |symbol: &Symbol| {
        let command = symbol.command.strip_prefix('\\')?;
        match () {
            () if command == typed => Some(0),
            () if command.starts_with(typed) => Some(1),
            () if begins(command) => Some(2),
            () if worded(symbol.name) || symbol.aliases.iter().any(|alias| worded(alias)) => Some(3),
            () => None,
        }
    };
    let mut found: Vec<(u8, &Symbol)> = commands().iter().filter_map(|symbol| Some((rank(symbol)?, symbol))).collect();
    // Within a rank the table's order stands, which puts the everyday ones first.
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, symbol)| symbol).take(LISTED).collect()
}

/// What a symbol puts into the source: its command, and a pair of braces for each argument it takes.
pub(crate) fn written(symbol: &Symbol) -> String {
    format!("{}{}", symbol.command, "{}".repeat(symbol.arguments.into()))
}

/// A formula that was just closed, while the source it was typed as still blends into its glyphs.
struct Blend {
    /// A place in the formula, by which the layout says where it is drawn.
    at: Pos,
    /// The source as the page showed it while it was typed, with its dollar signs.
    typed: Shaped,
    /// The middle of the left edge of the source where it was typed. `None` where the formula starts.
    from: Option<Point>,
    /// When the blend began, once a frame has said so.
    since: Option<u64>,
    /// How much of the formula shows, from 0 to 1. The source shows as much less.
    shown: f32,
}

/// The sheet's own state: which formula it is out under, and how far out it has slid.
#[derive(Default)]
pub(crate) struct Maths {
    /// The formula the sheet is out under: its block and where its source starts.
    out: Option<(BlockId, usize)>,
    /// When the sheet began to slide out, once a frame has said so.
    since: Option<u64>,
    /// How far out the sheet is, from 0 to 1.
    slid: f32,
    /// Whether the pointer went down on the source and has not come up.
    pub dragging: bool,
    /// Whether the last thing done in the formula was typing, which is when a backslash lists symbols.
    typing: bool,
    /// The symbol picked in the list, counted from the first.
    picked: usize,
    blend: Option<Blend>,
}

impl Maths {
    /// Notes which formula the caret is in. A sheet slides out for one it was not in before, or is out at
    /// once when nothing should move.
    pub fn follow(&mut self, source: Option<&Source>, at_once: bool) {
        let formula = source.map(|source| (source.block.clone(), source.range.start));
        if formula != self.out {
            (self.out, self.since, self.dragging, self.typing) = (formula, None, false, false);
            self.slid = if at_once { 1.0 } else { 0.0 };
        }
    }

    /// Notes whether what was just done in the formula was typing. Anything else puts the list away.
    pub fn typed(&mut self, typing: bool) {
        (self.typing, self.picked) = (typing, 0);
    }

    /// Moves the pick one symbol down or up a list of `listed`, round its ends.
    pub fn pick(&mut self, down: bool, listed: usize) {
        self.picked = Options::<Message>::step(self.picked, down, listed);
    }

    /// The symbol that is picked among `listed`.
    pub fn picked(&self, listed: usize) -> usize {
        self.picked.min(listed.saturating_sub(1))
    }

    /// The symbols on offer at the caret, and the stretch of the block one of them would replace: none
    /// unless a command is being typed there, and the sheet is out.
    pub fn symbols(&self, source: &Source, selection: &Selection) -> Option<(Range<usize>, Vec<&'static Symbol>)> {
        let typing = self.typing && self.slid >= 1.0 && selection.is_caret();
        let (range, typed) = source.typed(selection.head.offset).filter(|_| typing)?;
        Some((range, matching(typed))).filter(|(_, found)| !found.is_empty())
    }

    /// Slides the sheet further out, and blends a closed formula further in. Returns when the next frame is
    /// due, while either still moves.
    pub fn animate(&mut self, now_ms: u64) -> Option<u64> {
        let blending = self.blend.as_mut().map(|blend| {
            let progress = now_ms.saturating_sub(*blend.since.get_or_insert(now_ms)) as f32 / tokens::DURATION_FAST;
            blend.shown = motion::ease(progress);
            progress < 1.0
        });
        if blending == Some(false) {
            self.blend = None;
        }
        let mut sliding = false;
        if self.out.is_some() && self.slid < 1.0 {
            let progress = now_ms.saturating_sub(*self.since.get_or_insert(now_ms)) as f32 / SLIDE_MS;
            self.slid = if progress < 1.0 { motion::ease(progress) } else { 1.0 };
            sliding = progress < 1.0;
        }
        (sliding || blending == Some(true)).then_some(now_ms + FRAME_MS)
    }

    /// Begins to blend the source of a formula that was just closed into its glyphs. `at` is a place in the
    /// formula, and `typed` its source as the page showed it, with its dollar signs. `from` is the middle
    /// of its left edge there, when that is not where the formula now starts.
    pub fn close(&mut self, at: Pos, typed: Shaped, from: Option<Point>) {
        self.blend = Some(Blend { at, typed, from, since: None, shown: 0.0 });
    }

    /// A place in the formula whose source is still blending into its glyphs.
    pub fn closing(&self) -> Option<&Pos> {
        self.blend.as_ref().map(|blend| &blend.at)
    }

    /// Gives up a blend whose formula is no longer there to blend into.
    pub fn settle(&mut self) {
        self.blend = None;
    }

    /// Draws the blend into what the page painted: the formula drawn in `rect` shows as far as the blend has
    /// come, and over it the source goes in `ink`, from where it was typed, its characters pressed together
    /// towards the formula's width.
    pub fn blend(&self, rect: Rect, ink: Color, list: &mut DisplayList) {
        let Some(blend) = &self.blend else { return };
        let faded = |color: Color, share: f32| Color { a: (f32::from(color.a) * share).round() as u8, ..color };
        let around = Rect::new(rect.x - 1.0, rect.y - 1.0, rect.width + 2.0, rect.height + 2.0);
        let inside = |part: &Rect| {
            around.contains(Point { x: part.x, y: part.y })
                && around.contains(Point { x: part.x + part.width, y: part.y + part.height })
        };
        for item in &mut list.items {
            let color = match item {
                Item::Glyphs { origin, color, .. } | Item::Path { origin, color, .. } if around.contains(*origin) => {
                    color
                }
                Item::Fill { rect, color, .. } | Item::Stroke { rect, color, .. } if inside(rect) => color,
                _ => continue,
            };
            *color = faded(*color, blend.shown);
        }
        let typed = &blend.typed;
        let between = |from: f32, to: f32| from + (to - from) * blend.shown;
        let least = (rect.width / typed.size.width.max(1.0)).clamp(SQUEEZE, 1.0);
        let pressed = between(1.0, least);
        // The source ends up in the middle of the formula, as wide as it is when that is not too narrow.
        let middle = rect.y + rect.height / 2.0;
        let from = blend.from.unwrap_or(Point { x: rect.x, y: middle });
        let left = between(from.x, rect.x + (rect.width - typed.size.width * least) / 2.0);
        let top = between(from.y, middle) - typed.size.height / 2.0;
        let first = list.items.len();
        typed.paint(Point { x: 0.0, y: top }, faded(ink, 1.0 - blend.shown), list);
        for item in &mut list.items[first..] {
            if let Item::Glyphs { origin, glyphs, .. } = item {
                origin.x = left + origin.x * pressed;
                glyphs.iter_mut().for_each(|glyph| glyph.x *= pressed);
            }
        }
    }

    /// How wide the source is with a caret past its end.
    fn span(source: &Source) -> f32 {
        source.size().width + CARET_ROOM
    }

    /// The most a row of a source may take across: what a sheet no wider than the text, `column` across in
    /// a page area `room` across, has room for.
    pub fn wide(column: f32, room: f32) -> f32 {
        (column.min(room - 2.0 * MARGIN) - PADDING.left - PADDING.right - CARET_ROOM).max(1.0)
    }

    /// How wide the sheet shows its source: as wide as its longest row or a little wider than its formula,
    /// whichever is more, and no wider than rows may be.
    fn width(source: &Source, formula: f32, wide: f32) -> f32 {
        Self::span(source).max(formula + WIDER).min(wide + CARET_ROOM)
    }

    /// Where the left edge of the source is, for a sheet centred under a formula that starts at `x` and is
    /// `formula` wide.
    pub fn left(source: &Source, x: f32, formula: f32, wide: f32) -> f32 {
        x + (formula - Self::width(source, formula, wide)) / 2.0
    }

    /// The sheet under a formula whose box is `formula` wide, for a source in rows of `wide` at the most:
    /// the source with the selection as the editor holds it, over the symbols on offer when a command is
    /// being typed.
    pub fn sheet(
        &self,
        source: Source,
        (selection, before_wrap): (&Selection, bool),
        formula: f32,
        wide: f32,
        scheme: Scheme,
    ) -> Popover<Message> {
        let symbols = self.symbols(&source, selection);
        // What is still being written is not called wrong: a fault the caret is at, and a bracket left open
        // while the caret is at the end of the source.
        let at = selection.head.offset.saturating_sub(source.range.start);
        let fault = source.fault().filter(|fault| !fault.written_at(at));
        let writing = source.text.get(at..).is_none_or(|rest| rest.trim().is_empty());
        let least = match (&symbols, &fault) {
            (Some(_), _) => LIST_WIDTH,
            (None, Some(_)) => FAULT_WIDTH,
            (None, None) => 0.0,
        };
        let (size, width) = (source.size(), Self::width(&source, formula, wide).max(least));
        let caret = source.caret(selection.head.offset, before_wrap);
        // The sheet comes out from under the formula, its last row first.
        let shift = (1.0 - self.slid) * size.height;
        let (start, end) =
            (selection.anchor.offset.min(selection.head.offset), selection.anchor.offset.max(selection.head.offset));
        let label = source.text.clone();
        let source = Rc::new(source);
        let (mut inks, mut lined) = source.marks(fault.as_ref().and_then(|fault| fault.range.as_ref()), writing);
        // The command being typed is in the accent and underlined, whatever it would be otherwise.
        let typed = symbols.iter().flat_map(|(range, _)| source.spans(range)).map(|span| (span, accent().text));
        inks.extend(typed.clone());
        lined.extend(typed);
        let composing = source.composing.iter().flat_map(|range| source.spans(range)).collect();
        let selected = source.spans(&(start..end));
        let paint = SourcePaint { source: source.clone(), selected, inks, lined, composing, shift };
        let pointed = move |event: Pointer| {
            let hit = |at: Point| source.hit(Point { x: at.x, y: at.y + shift });
            match event {
                Pointer::Down { at, mods, clicks } => {
                    let (offset, before_wrap) = hit(at);
                    Some(Sheet::Press { offset, before_wrap, extend: mods.shift, clicks })
                }
                Pointer::Drag { at } => {
                    let (offset, before_wrap) = hit(at);
                    Some(Sheet::Drag(offset, before_wrap))
                }
                Pointer::Up { .. } => Some(Sheet::Release),
                // The wheel is the page's, and so is a menu.
                Pointer::Menu { .. } | Pointer::Scroll { .. } => None,
            }
        };
        let shown = (self.slid >= 1.0).then_some(caret);
        let text = Element::canvas(Rc::new(paint))
            .width(width)
            .height(size.height * self.slid)
            .role(Role::Code)
            .label("Formula source")
            .description(label)
            .cursor(Cursor::Text)
            .passive()
            .caret(shown)
            .on_pointer(move |event| pointed(event).map(Message::Maths));
        let padding = Edges { top: PADDING.top * self.slid, bottom: PADDING.bottom * self.slid, ..PADDING };
        let mut sheet = Element::column().gap(tokens::SPACE_SM).padding(padding).child(text);
        if let Some(fault) = fault.filter(|_| self.slid >= 1.0) {
            let quiet = TextStyle::body().size(tokens::TEXT_XS);
            let said = Element::paragraph(fault.message, quiet).fg(tokens::DANGER).max_width(width);
            sheet = sheet.child(said.role(Role::Status));
        }
        if let Some((_, symbols)) = &symbols {
            let hint = options::hint("Matches commands and plain names", &["Tab", "Esc"]);
            sheet = sheet.child(self.list(symbols, scheme)).child(hint);
        }
        Popover::new(sheet)
            .flush()
            .flat()
            .max_width(f32::INFINITY)
            .gap(HALO)
            .align(Align::Center)
            .beside()
            .stays()
            .open(true)
            .on_close(Message::Maths(Sheet::Leave))
    }

    /// The symbols on offer, each typeset beside its command and its plain name.
    fn list(&self, symbols: &[&'static Symbol], scheme: Scheme) -> Element<Message> {
        let style = letters_math::Style {
            size: tokens::TEXT_MD * TEXT_RATIO,
            color: resolve(tokens::FG, scheme, 1.0),
            display: false,
            links: false,
        };
        // One too tall for its row is set smaller, so that every row is the same height.
        let picture = |symbol: &&Symbol| {
            let picture = typeset(symbol.sample, &style).ok()?;
            let fit = PICTURE.height / picture.size.height;
            if fit < 1.0 {
                typeset(symbol.sample, &letters_math::Style { size: style.size * fit, ..style }).ok()
            } else {
                Some(picture)
            }
        };
        let pictures: Vec<Option<Formula>> = symbols.iter().map(picture).collect();
        let across = pictures.iter().flatten().map(|picture| picture.size.width).fold(PICTURE.width, f32::max);
        let command = TextStyle::control().family(Family::Mono).size(tokens::TEXT_SM);
        let name = TextStyle::control().size(tokens::TEXT_XS);
        let mut options = Options::new("Symbols").picked(self.picked(symbols.len()));
        for (index, (symbol, picture)) in symbols.iter().zip(pictures).enumerate() {
            let picture = Element::canvas(Rc::new(Picture(picture))).width(across).height(PICTURE.height);
            let choice = Choice::new(symbol.command)
                .style(command)
                .lead(picture.passive().decorative())
                .aside(symbol.name, name)
                .on_take(Message::Maths(Sheet::Take(index)));
            options = options.choice(choice);
        }
        options.into()
    }
}

/// A symbol as its row of the list shows it, in the middle of the room it has.
struct Picture(Option<Formula>);

impl Paint for Picture {
    fn paint(&self, rect: Rect, _visible: Rect, _scheme: Scheme, out: &mut DisplayList) {
        let Some(picture) = &self.0 else { return };
        let (x, y) = ((rect.width - picture.size.width) / 2.0, (rect.height - picture.size.height) / 2.0);
        picture.paint(Point { x: rect.x + x, y: rect.y + y }, out);
    }
}

/// The source as the sheet's canvas draws it.
struct SourcePaint {
    source: Rc<Source>,
    /// What is selected, from the top left of the source.
    selected: Vec<Rect>,
    /// The stretches that are not in plain ink, and the colour of each. A later one counts over an earlier.
    inks: Vec<Stretch>,
    /// The stretches a line is drawn under: the command being typed, and what is wrong.
    lined: Vec<Stretch>,
    /// What an input method is still composing.
    composing: Vec<Rect>,
    /// How far the source is moved up under the canvas, while the sheet slides out.
    shift: f32,
}

impl Paint for SourcePaint {
    fn paint(&self, rect: Rect, _visible: Rect, scheme: Scheme, out: &mut DisplayList) {
        let origin = Point { x: rect.x, y: rect.y - self.shift };
        let placed = |span: &Rect| Rect { x: origin.x + span.x, y: origin.y + span.y, ..*span };
        let selection = resolve(fade(accent().base, 0.3), scheme, 1.0);
        out.items.extend(self.selected.iter().map(|span| Item::Fill {
            rect: placed(span),
            color: selection,
            radius: 0.0,
        }));
        let plain = resolve(tokens::FG, scheme, 1.0);
        let inks: Vec<(Rect, Color)> =
            self.inks.iter().map(|(span, ink)| (*span, resolve(*ink, scheme, 1.0))).collect();
        for (at, row) in self.source.rows.iter().enumerate() {
            let top = at as f32 * Source::line_height();
            // A glyph is in the colour of the stretch it starts in.
            let ink = |x: f32| {
                let holds = |span: &Rect| span.y == top && span.x <= x + 0.5 && x + 0.5 < span.x + span.width;
                inks.iter().rev().find(|(span, _)| holds(span)).map_or(plain, |(_, ink)| *ink)
            };
            row.shaped.paint_inked(Point { x: origin.x, y: origin.y + top }, ink, out);
        }
        let under = |span: Rect| Rect::new(span.x, span.y + span.height - 2.0, span.width, 1.0);
        // Text still being composed is underlined, as on the page.
        let composing = self.composing.iter().map(|span| (under(placed(span)), plain));
        let lined = self.lined.iter().map(|(span, ink)| (under(placed(span)), resolve(*ink, scheme, 1.0)));
        out.items.extend(composing.chain(lined).map(|(rect, color)| Item::Fill { rect, color, radius: 0.0 }));
    }
}

/// The tint behind a formula whose sheet is out, for a formula drawn in `rect`.
pub(crate) fn tint(rect: Rect, scheme: Scheme) -> Item {
    let rect = Rect::new(rect.x - HALO, rect.y - HALO, rect.width + 2.0 * HALO, rect.height + 2.0 * HALO);
    Item::Fill { rect, color: resolve(accent().soft, scheme, 1.0), radius: HALO }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::FixedShaper;

    /// A source in block `f`, which starts ten bytes into its block, in rows of `wide` at the most.
    fn rows(text: &str, wide: f32) -> Source {
        Source::shaped("f".into(), 10, text, true, &FixedShaper, wide)
    }

    /// A source of three lines.
    fn source() -> Source {
        rows("a = b\n+ c\n+ dd", f32::INFINITY)
    }

    /// The page's lines, which answer with a place of their own.
    struct Page;

    impl Lines for Page {
        fn caret_x(&self, _: &LinePos) -> Option<f32> {
            Some(0.0)
        }

        fn line_step(&self, _: &LinePos, _: f32, down: bool) -> Option<LinePos> {
            Some(Pos::new(if down { "below" } else { "above" }, 0).into())
        }

        fn line_edge(&self, _: &LinePos, end: bool) -> Option<LinePos> {
            Some(Pos::new("page", usize::from(end)).into())
        }
    }

    #[test]
    fn a_point_of_the_source_and_the_caret_there_agree() {
        let source = source();
        for offset in [10, 12, 15, 16, 19, 20, 24] {
            let caret = source.caret(offset, false);
            let middle = Point { x: caret.x, y: caret.y + caret.height / 2.0 };
            assert_eq!(source.hit(middle), (offset, false), "{caret:?}");
        }
        assert_eq!(source.hit(Point { x: 500.0, y: 500.0 }), (24, false), "a point past the source is its end");
    }

    #[test]
    fn up_and_down_move_through_the_source_before_they_leave_for_the_page() {
        let lines = SheetLines { page: Page, source: Some(source()), left: 100.0 };
        let at = |offset: usize| LinePos::from(Pos::new("f", offset));
        let x = lines.caret_x(&at(12)).expect("a column");
        let cases = [
            (12, true, at(18)),
            (18, true, at(22)),
            (22, true, Pos::new("below", 0).into()),
            (18, false, at(12)),
            (12, false, Pos::new("above", 0).into()),
        ];
        for (from, down, to) in cases {
            assert_eq!(lines.line_step(&at(from), x, down), Some(to), "from {from}, down: {down}");
        }
    }

    #[test]
    fn home_and_end_stay_on_the_line_of_the_source() {
        let lines = SheetLines { page: Page, source: Some(source()), left: 0.0 };
        let at = |offset: usize| LinePos::from(Pos::new("f", offset));
        for (from, start, end) in [(12, 10, 15), (17, 16, 19), (24, 20, 24)] {
            assert_eq!(lines.line_edge(&at(from), false), Some(at(start)), "home from {from}");
            assert_eq!(lines.line_edge(&at(from), true), Some(at(end)), "end from {from}");
        }
        let page = lines.line_edge(&Pos::new("p", 3).into(), true);
        assert_eq!(page, Some(Pos::new("page", 1).into()), "the page's own lines");
    }

    /// Rows of eight characters at the most.
    fn eight() -> f32 {
        8.0 * FixedShaper::advance(TEXT.size)
    }

    #[test]
    fn a_line_too_long_for_the_sheet_goes_on_in_a_new_row() {
        // A source, and where in it each of its rows starts.
        let cases: [(&str, &[usize]); 5] = [
            ("aaa bbb", &[0]),
            ("aaa bbb ccc ddd", &[0, 8]),
            // A word too long for a row breaks inside it.
            ("aaaaaaaaaaaa", &[0, 8]),
            ("aa aaaaaaaaaaaa", &[0, 3, 11]),
            // Each line breaks for itself.
            ("aaa bbb ccc\nd", &[0, 8, 12]),
        ];
        for (text, starts) in cases {
            let rows: Vec<usize> = rows(text, eight()).rows.iter().map(|row| row.start - 10).collect();
            assert_eq!(rows, starts, "{text:?}");
        }
    }

    #[test]
    fn the_caret_keeps_to_its_row_where_a_line_goes_on() {
        let lines = SheetLines { page: Page, source: Some(rows("aaa bbb ccc ddd", eight())), left: 0.0 };
        let at = |offset: usize| LinePos::from(Pos::new("f", 10 + offset));
        // Where the caret is, and where Home and End take it: End stops before the space the row breaks at.
        for (from, start, end) in [(2, 0, 7), (7, 0, 7), (8, 8, 15), (12, 8, 15)] {
            assert_eq!(lines.line_edge(&at(from), false), Some(at(start)), "home from {from}");
            assert_eq!(lines.line_edge(&at(from), true), Some(at(end)), "end from {from}");
        }
        let x = lines.caret_x(&at(2)).expect("a column");
        assert_eq!(lines.line_step(&at(2), x, true), Some(at(10)), "down keeps the column");
        assert_eq!(lines.line_step(&at(10), x, false), Some(at(2)), "and so does up");
        let source = lines.source.as_ref().expect("a source");
        assert_eq!(source.hit(Point { x: 500.0, y: 1.0 }), (17, false), "a point past the first row is at its end");
        let second = source.caret(18, false);
        assert_eq!((second.x, second.y), (0.0, Source::line_height()), "where it breaks is the start of the next");
    }

    #[test]
    fn a_row_that_goes_on_with_no_space_ends_after_its_last_character() {
        // Twenty characters in rows of eight: one place is the end of a row and the start of the next.
        let lines = SheetLines { page: Page, source: Some(rows("abcdefghijklmnopqrst", eight())), left: 0.0 };
        let at = |offset: usize, before_wrap: bool| LinePos { pos: Pos::new("f", 10 + offset), before_wrap };
        // Where the caret is, and where Home and End take it.
        let cases = [
            (at(3, false), at(0, false), at(8, true)),
            (at(8, true), at(0, false), at(8, true)),
            (at(8, false), at(8, false), at(16, true)),
            (at(18, false), at(16, false), at(20, false)),
        ];
        for (from, start, end) in cases {
            assert_eq!(lines.line_edge(&from, false), Some(start), "home from {from:?}");
            assert_eq!(lines.line_edge(&from, true), Some(end), "end from {from:?}");
        }
        let source = lines.source.as_ref().expect("a source");
        let (first, second) = (source.caret(18, true), source.caret(18, false));
        assert_eq!((first.y, second.y, second.x), (0.0, Source::line_height(), 0.0), "the two sides of the wrap");
        assert!(first.x > second.x, "the end of the first row is at its right");
        assert_eq!(source.hit(Point { x: 500.0, y: 1.0 }), (18, true), "a point past the first row is at its end");
        // Down from the end of a row is the end of the next, and Up is the way back.
        let x = lines.caret_x(&at(8, true)).expect("a column");
        assert_eq!(lines.line_step(&at(8, true), x, true), Some(at(16, true)));
        assert_eq!(lines.line_step(&at(16, true), x, false), Some(at(8, true)));
        let x = lines.caret_x(&at(8, false)).expect("a column");
        assert_eq!(lines.line_step(&at(8, false), x, true), Some(at(16, false)), "from the start of a row");
    }

    #[test]
    fn a_command_is_being_typed_from_its_backslash_to_its_last_letter() {
        // The source, where the caret is in it, and the command found there with what was typed of it.
        let cases = [
            (r"a \ap", 5, Some((2..5, "ap"))),
            (r"a \", 3, Some((2..3, ""))),
            (r"\alpha + b", 3, Some((0..6, "al"))),
            (r"a \ap b", 7, None),
            (r"a + b", 3, None),
            (r"a \\", 4, None),
            (r"a \\ \be", 8, Some((5..8, "be"))),
        ];
        for (text, caret, expected) in cases {
            let source = rows(text, f32::INFINITY);
            let expected =
                expected.map(|(range, typed): (Range<usize>, &str)| (range.start + 10..range.end + 10, typed));
            assert_eq!(source.typed(10 + caret), expected, "{text:?} at {caret}");
        }
    }

    #[test]
    fn symbols_match_by_command_and_by_plain_name() {
        let first = |typed: &str| matching(typed).first().map(|symbol| symbol.command);
        // What is typed after the backslash, and the symbol offered first.
        let cases = [
            ("ap", Some(r"\approx")),
            ("in", Some(r"\in")),
            ("Gam", Some(r"\Gamma")),
            ("gam", Some(r"\gamma")),
            ("fraction", Some(r"\frac")),
            ("", Some(r"\frac")),
            ("zzzz", None),
        ];
        for (typed, expected) in cases {
            assert_eq!(first(typed), expected, "{typed:?}");
        }
        let divide: Vec<&str> = matching("divide").iter().map(|symbol| symbol.command).collect();
        assert!(divide.contains(&r"\frac"), "another word for a symbol finds it: {divide:?}");
        assert!(matching("a").len() <= LISTED, "the list stays short");
    }

    #[test]
    fn a_symbol_is_written_with_braces_for_its_arguments() {
        let written = |command: &str| matching(&command[1..]).first().map(|symbol| written(symbol));
        for (command, expected) in [(r"\approx", r"\approx"), (r"\sqrt", r"\sqrt{}"), (r"\frac", r"\frac{}{}")] {
            assert_eq!(written(command).as_deref(), Some(expected));
        }
    }

    #[test]
    fn a_sheet_slides_out_once_for_each_formula_the_caret_comes_to() {
        let (mut maths, source) = (Maths::default(), source());
        maths.follow(Some(&source), false);
        assert_eq!(maths.slid, 0.0);
        assert!(maths.animate(1000).is_some(), "it has yet to move");
        maths.animate(1070);
        assert!(maths.slid > 0.0 && maths.slid < 1.0, "half way: {}", maths.slid);
        assert_eq!(maths.animate(1140), None, "out, and at rest");
        maths.follow(Some(&source), false);
        assert_eq!(maths.slid, 1.0, "the same formula keeps its sheet out");

        maths.follow(None, false);
        maths.follow(Some(&source), true);
        assert_eq!((maths.slid, maths.animate(2000)), (1.0, None), "nothing moves when motion is reduced");
    }
}
