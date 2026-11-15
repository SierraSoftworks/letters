//! The sheet under a formula: its TeX source, out of band, while the caret is in it.
//!
//! The formula stays typeset on the page. Its source is written on a sheet that slides out from under it
//! when the caret reaches it, and the caret shows there, at the place in the source the editor holds it.
//! A backslash typed there lists the symbols that match what follows it, each one typeset.

use std::ops::Range;
use std::rc::Rc;

use accesskit::Role;
use letters_editor::{Editor, Lines};
use letters_math::{Command as Symbol, Formula, TEXT_RATIO, commands, typeset};
use letters_model::{BlockId, Pos, Selection};
use letters_render::{DisplayList, Item, Point, Rect, Size};

use crate::color::{fade, resolve};
use crate::element::{Cursor, Element, Paint, Pointer};
use crate::layout::MARGIN;
use crate::motion::{self, FRAME_MS};
use crate::style::{Align, Edges, Look};
use crate::text::{Family, Shaped, TextShaper, TextStyle};
use crate::tokens::{self, Scheme};
use crate::widgets::Popover;
use crate::window::{CARET_WIDTH, Message};

const TEXT: TextStyle = TextStyle::control().family(Family::Mono).size(tokens::TEXT_SM).leading(tokens::LEADING);
/// The room round the source on its sheet: tight above, where the sheet comes out from under the formula.
const PADDING: Edges =
    Edges { top: tokens::SPACE_XS, right: tokens::SPACE_MD, bottom: tokens::SPACE_SM, left: tokens::SPACE_MD };
/// How far the tint behind an open formula reaches past it, which is also how far under it the sheet hangs.
pub(crate) const HALO: f32 = 3.0;
/// How much wider than its formula a sheet is at the least.
const WIDER: f32 = 24.0;
/// How long the sheet takes to slide out, in milliseconds.
const SLIDE_MS: f32 = 140.0;
/// How many symbols the list shows, and the least room it takes across.
const LISTED: usize = 6;
const LIST_WIDTH: f32 = 300.0;
/// The room a symbol's picture has in its row: at least this wide, and this high.
const PICTURE: Size = Size { width: 24.0, height: 22.0 };

/// What the pointer and the keyboard ask of the sheet.
#[derive(Clone, Debug, PartialEq)]
pub enum Sheet {
    /// A press on the source, at an offset in its block. `extend` keeps the anchor, as Shift does.
    Press {
        offset: usize,
        extend: bool,
        clicks: u8,
    },
    /// The pointer moved over the source with the button held.
    Drag(usize),
    Release,
    /// A symbol of the list was pressed, counted from the first.
    Take(usize),
    /// Escape, which puts the list of symbols away, and steps out of the formula when there is none.
    Leave,
}

/// A formula's source, shaped line by line as the sheet shows it.
pub(crate) struct Source {
    pub block: BlockId,
    /// Where the source sits in its block's text.
    pub range: Range<usize>,
    pub text: String,
    /// The part an input method is still composing, as offsets in the block.
    composing: Option<Range<usize>>,
    /// Each line: where it starts in the block, and its text shaped.
    lines: Vec<(usize, Shaped)>,
}

impl Source {
    /// The source of the formula the editor's caret is in.
    pub fn of(editor: &Editor, shaper: &dyn TextShaper) -> Option<Self> {
        let (block, range) = editor.formula()?;
        let text = editor.doc().block(block)?.plain_text().get(range.clone())?.to_owned();
        let mut start = range.start;
        let mut lines = Vec::new();
        for line in text.split('\n') {
            lines.push((start, shaper.shape(line, &TEXT, None)));
            start += line.len() + 1;
        }
        let composing = editor.composition().filter(|composing| composing.0 == block).map(|composing| composing.1);
        Some(Self { block: block.clone(), range, text, composing, lines })
    }

    fn line_height() -> f32 {
        TEXT.size * TEXT.leading
    }

    pub fn size(&self) -> Size {
        let width = self.lines.iter().map(|(_, line)| line.size.width).fold(0.0, f32::max);
        Size { width, height: self.lines.len() as f32 * Self::line_height() }
    }

    /// Whether a position is in this source, either end included.
    fn holds(&self, pos: &Pos) -> bool {
        pos.block == self.block && (self.range.start..=self.range.end).contains(&pos.offset)
    }

    /// The line an offset in the block is on.
    fn line(&self, offset: usize) -> usize {
        self.lines.iter().rposition(|(start, _)| *start <= offset).unwrap_or(0)
    }

    /// The offsets in the block a line runs between.
    fn ends(&self, line: usize) -> (usize, usize) {
        let (start, shaped) = &self.lines[line];
        (*start, start + shaped.stops.last().map_or(0, |stop| stop.offset))
    }

    /// Where the caret is drawn for an offset in the block, from the top left of the source.
    pub fn caret(&self, offset: usize) -> Rect {
        let line = self.line(offset);
        let (start, shaped) = &self.lines[line];
        let (x, height) = (shaped.x_at(offset.saturating_sub(*start)), Self::line_height());
        Rect::new(x, line as f32 * height, CARET_WIDTH, height)
    }

    /// The offset in the block nearest a point of the source.
    pub fn hit(&self, at: Point) -> usize {
        let line = (at.y / Self::line_height()).floor().max(0.0) as usize;
        let (start, shaped) = &self.lines[line.min(self.lines.len() - 1)];
        start + shaped.offset_at(at.x)
    }

    /// The offset one line up or down from `offset`, nearest to `x`. `None` past the first and last line.
    fn step(&self, offset: usize, x: f32, down: bool) -> Option<usize> {
        let line = self.line(offset);
        let (start, shaped) = self.lines.get(if down { line + 1 } else { line.checked_sub(1)? })?;
        Some(start + shaped.offset_at(x))
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

    /// The rectangles under a stretch of the source, one per line it touches.
    fn spans(&self, range: &Range<usize>) -> Vec<Rect> {
        let height = Self::line_height();
        let span = |line: usize| {
            let (start, end) = self.ends(line);
            let (from, to) = (range.start.max(start), range.end.min(end));
            let shaped = &self.lines[line].1;
            let (left, right) = (shaped.x_at(from.saturating_sub(start)), shaped.x_at(to.saturating_sub(start)));
            (from < to).then(|| Rect::new(left, line as f32 * height, right - left, height))
        };
        (self.line(range.start)..=self.line(range.end)).filter_map(span).collect()
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
    fn caret_x(&self, pos: &Pos) -> Option<f32> {
        match self.on_sheet(pos) {
            Some(source) => Some(self.left + source.caret(pos.offset).x),
            None => self.page.caret_x(pos),
        }
    }

    fn line_step(&self, pos: &Pos, x: f32, down: bool) -> Option<Pos> {
        let within = self.on_sheet(pos).and_then(|source| source.step(pos.offset, x - self.left, down));
        match within {
            Some(offset) => Some(Pos::new(pos.block.clone(), offset)),
            None => self.page.line_step(pos, x, down),
        }
    }

    fn line_edge(&self, pos: &Pos, end: bool) -> Option<Pos> {
        match self.on_sheet(pos) {
            Some(source) => {
                let (start, last) = source.ends(source.line(pos.offset));
                Some(Pos::new(pos.block.clone(), if end { last } else { start }))
            }
            None => self.page.line_edge(pos, end),
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
        let step = if down { 1 } else { listed.saturating_sub(1) };
        self.picked = (self.picked.min(listed.saturating_sub(1)) + step) % listed.max(1);
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

    /// Slides the sheet further out. Returns when the next frame is due, while it still moves.
    pub fn animate(&mut self, now_ms: u64) -> Option<u64> {
        if self.out.is_none() || self.slid >= 1.0 {
            return None;
        }
        let progress = now_ms.saturating_sub(*self.since.get_or_insert(now_ms)) as f32 / SLIDE_MS;
        self.slid = if progress < 1.0 { motion::ease(progress) } else { 1.0 };
        (progress < 1.0).then_some(now_ms + FRAME_MS)
    }

    /// How wide the sheet shows its source: as wide as the source or a little wider than its formula,
    /// whichever is more, and no wider than the page area has room for.
    fn width(source: &Source, formula: f32, room: f32) -> f32 {
        let most = (room - 2.0 * MARGIN - PADDING.left - PADDING.right).max(1.0);
        source.size().width.max(formula + WIDER).min(most)
    }

    /// Where the left edge of the source is, for a sheet centred under a formula that starts at `x` and is
    /// `formula` wide.
    pub fn left(source: &Source, x: f32, formula: f32, room: f32) -> f32 {
        x + (formula - Self::width(source, formula, room)) / 2.0
    }

    /// The sheet under a formula whose box is `formula` wide, in a page area `room` wide: the source with
    /// the selection as the editor holds it, over the symbols on offer when a command is being typed.
    pub fn sheet(
        &self,
        source: Source,
        selection: &Selection,
        formula: f32,
        room: f32,
        scheme: Scheme,
    ) -> Popover<Message> {
        let symbols = self.symbols(&source, selection);
        let least = if symbols.is_some() { LIST_WIDTH } else { 0.0 };
        let (size, width) = (source.size(), Self::width(&source, formula, room).max(least));
        let caret = source.caret(selection.head.offset);
        // A source too long for the window shows the part around the caret.
        let across = (caret.x - width / 2.0).clamp(0.0, (size.width - width).max(0.0));
        // The sheet comes out from under the formula, its last line first.
        let shift = Point { x: across, y: (1.0 - self.slid) * size.height };
        let (start, end) =
            (selection.anchor.offset.min(selection.head.offset), selection.anchor.offset.max(selection.head.offset));
        let label = source.text.clone();
        let source = Rc::new(source);
        let typed = symbols.iter().flat_map(|(range, _)| source.spans(range)).collect();
        let composing = source.composing.iter().flat_map(|range| source.spans(range)).collect();
        let selected = source.spans(&(start..end));
        let paint = SourcePaint { source: source.clone(), selected, typed, composing, shift };
        let pointed = move |event: Pointer| {
            let offset = |at: Point| source.hit(Point { x: at.x + shift.x, y: at.y + shift.y });
            match event {
                Pointer::Down { at, mods, clicks } => {
                    Some(Sheet::Press { offset: offset(at), extend: mods.shift, clicks })
                }
                Pointer::Drag { at } => Some(Sheet::Drag(offset(at))),
                Pointer::Up { .. } => Some(Sheet::Release),
                // The wheel is the page's, and so is a menu.
                Pointer::Menu { .. } | Pointer::Scroll { .. } => None,
            }
        };
        let shown = (self.slid >= 1.0).then_some(Rect { x: caret.x - across, ..caret });
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
        if let Some((_, symbols)) = &symbols {
            sheet = sheet.child(self.list(symbols, scheme)).child(hint());
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
        let picked = self.picked(symbols.len());
        let row = |(index, (symbol, picture)): (usize, (&&'static Symbol, Option<Formula>))| {
            let picture = Element::canvas(Rc::new(Picture(picture))).width(across).height(PICTURE.height);
            let picture = picture.passive().decorative();
            let command = TextStyle::control().family(Family::Mono).size(tokens::TEXT_SM);
            let name = Element::text(symbol.name, TextStyle::control().size(tokens::TEXT_XS)).fg(tokens::FG_MUTED);
            let mut row = Element::row()
                .key(symbol.command)
                .align(Align::Center)
                .gap(tokens::SPACE_SM)
                .padding(Edges::xy(tokens::SPACE_SM, 3.0))
                .radius(tokens::RADIUS_CONTROL)
                .hover(Look::bg(tokens::NEUTRAL_SOFT))
                .role(Role::ListBoxOption)
                .label(symbol.command)
                .description(symbol.name)
                .selected(index == picked)
                .on_press(Message::Maths(Sheet::Take(index)))
                .child(picture)
                .child(Element::text(symbol.command, command))
                .child(Element::row().grow(1.0).min_width(tokens::SPACE_SM))
                .child(name);
            if index == picked {
                row = row.bg(tokens::ACCENT_SOFT);
            }
            row
        };
        let rows = symbols.iter().zip(pictures).enumerate().map(row);
        Element::column().role(Role::ListBox).label("Symbols").children(rows)
    }
}

/// The line under the list that says what it matches, and the keys that take a symbol or put the list away.
fn hint() -> Element<Message> {
    let quiet = TextStyle::control().size(tokens::TEXT_XS);
    let key = |name: &str| {
        Element::row()
            .padding(Edges::xy(5.0, 1.0))
            .radius(tokens::RADIUS_SM)
            .bg(tokens::NEUTRAL_SOFT)
            .child(Element::text(name, quiet).fg(tokens::FG_MUTED))
    };
    Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_XS)
        .padding(Edges::xy(tokens::SPACE_XS, 0.0))
        .child(Element::text("Matches commands and plain names", quiet).fg(tokens::FG_MUTED))
        .child(Element::row().grow(1.0).min_width(tokens::SPACE_SM))
        .child(key("Tab"))
        .child(key("Esc"))
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
    /// The command the list of symbols is for.
    typed: Vec<Rect>,
    /// What an input method is still composing.
    composing: Vec<Rect>,
    /// How far the source is moved left and up under the canvas.
    shift: Point,
}

impl Paint for SourcePaint {
    fn paint(&self, rect: Rect, _visible: Rect, scheme: Scheme, out: &mut DisplayList) {
        let origin = Point { x: rect.x - self.shift.x, y: rect.y - self.shift.y };
        let placed = |span: &Rect| Rect { x: origin.x + span.x, y: origin.y + span.y, ..*span };
        let selection = resolve(fade(tokens::ACCENT, 0.3), scheme, 1.0);
        out.items.extend(self.selected.iter().map(|span| Item::Fill {
            rect: placed(span),
            color: selection,
            radius: 0.0,
        }));
        let lines = |ink, out: &mut DisplayList| {
            for (line, (_, shaped)) in self.source.lines.iter().enumerate() {
                shaped.paint(Point { x: origin.x, y: origin.y + line as f32 * Source::line_height() }, ink, out);
            }
        };
        let ink = resolve(tokens::FG, scheme, 1.0);
        lines(ink, out);
        let under = |span: Rect| Rect::new(span.x, span.y + span.height - 2.0, span.width, 1.0);
        // Text still being composed is underlined, as on the page.
        let composing = self.composing.iter().map(|span| under(placed(span)));
        out.items.extend(composing.map(|rect| Item::Fill { rect, color: ink, radius: 0.0 }));
        // The command being typed is drawn again in the accent, over itself, and underlined.
        let accent = resolve(tokens::ACCENT_TEXT, scheme, 1.0);
        for span in self.typed.iter().map(placed) {
            out.push(Item::PushClip { rect: span, radius: 0.0 });
            out.push(Item::Fill { rect: span, color: resolve(tokens::SURFACE_RAISED, scheme, 1.0), radius: 0.0 });
            lines(accent, out);
            out.push(Item::PopClip);
            out.push(Item::Fill { rect: under(span), color: accent, radius: 0.0 });
        }
    }
}

/// The tint behind a formula whose sheet is out, for a formula drawn in `rect`.
pub(crate) fn tint(rect: Rect, scheme: Scheme) -> Item {
    let rect = Rect::new(rect.x - HALO, rect.y - HALO, rect.width + 2.0 * HALO, rect.height + 2.0 * HALO);
    Item::Fill { rect, color: resolve(tokens::ACCENT_SOFT, scheme, 1.0), radius: HALO }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::FixedShaper;

    /// A source of three lines in block `f`, which starts ten bytes into its block.
    fn source() -> Source {
        let text = "a = b\n+ c\n+ dd";
        let mut start = 10;
        let mut lines = Vec::new();
        for line in text.split('\n') {
            lines.push((start, FixedShaper.shape(line, &TEXT, None)));
            start += line.len() + 1;
        }
        Source { block: "f".into(), range: 10..10 + text.len(), text: text.into(), composing: None, lines }
    }

    /// The page's lines, which answer with a place of their own.
    struct Page;

    impl Lines for Page {
        fn caret_x(&self, _: &Pos) -> Option<f32> {
            Some(0.0)
        }

        fn line_step(&self, _: &Pos, _: f32, down: bool) -> Option<Pos> {
            Some(Pos::new(if down { "below" } else { "above" }, 0))
        }

        fn line_edge(&self, _: &Pos, end: bool) -> Option<Pos> {
            Some(Pos::new("page", usize::from(end)))
        }
    }

    #[test]
    fn a_point_of_the_source_and_the_caret_there_agree() {
        let source = source();
        for offset in [10, 12, 15, 16, 19, 20, 24] {
            let caret = source.caret(offset);
            let middle = Point { x: caret.x, y: caret.y + caret.height / 2.0 };
            assert_eq!(source.hit(middle), offset, "{caret:?}");
        }
        assert_eq!(source.hit(Point { x: 500.0, y: 500.0 }), 24, "a point past the source is its end");
    }

    #[test]
    fn up_and_down_move_through_the_source_before_they_leave_for_the_page() {
        let lines = SheetLines { page: Page, source: Some(source()), left: 100.0 };
        let at = |offset: usize| Pos::new("f", offset);
        let x = lines.caret_x(&at(12)).expect("a column");
        let cases = [
            (12, true, at(18)),
            (18, true, at(22)),
            (22, true, Pos::new("below", 0)),
            (18, false, at(12)),
            (12, false, Pos::new("above", 0)),
        ];
        for (from, down, to) in cases {
            assert_eq!(lines.line_step(&at(from), x, down), Some(to), "from {from}, down: {down}");
        }
    }

    #[test]
    fn home_and_end_stay_on_the_line_of_the_source() {
        let lines = SheetLines { page: Page, source: Some(source()), left: 0.0 };
        let at = |offset: usize| Pos::new("f", offset);
        for (from, start, end) in [(12, 10, 15), (17, 16, 19), (24, 20, 24)] {
            assert_eq!(lines.line_edge(&at(from), false), Some(at(start)), "home from {from}");
            assert_eq!(lines.line_edge(&at(from), true), Some(at(end)), "end from {from}");
        }
        assert_eq!(lines.line_edge(&Pos::new("p", 3), true), Some(Pos::new("page", 1)), "the page's own lines");
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
            let lines = vec![(10, FixedShaper.shape(text, &TEXT, None))];
            let range = 10..10 + text.len();
            let source = Source { block: "f".into(), range, text: text.into(), composing: None, lines };
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
