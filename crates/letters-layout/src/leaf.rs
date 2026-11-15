//! The texts of a leaf: shaping them with Parley, painting them and answering where things are inside them.

use std::collections::HashMap;
use std::iter::Peekable;
use std::ops::Range;
use std::sync::{Arc, LazyLock};

use letters_math::{Formula, Style as Maths, TEXT_RATIO};
use letters_model::{BlockId, BlockKind, ListKind};
use letters_render::{Color, DisplayList, Glyph, Item, PathEl, Point, Rect, Role as Tag, RunText};
use parley::{
    Affinity, Alignment, AlignmentOptions, BreakReason, Cluster, Cursor, GlyphRun, InlineBox, InlineBoxKind, Language,
    Layout, Line, PositionedLayoutItem, Selection as Span, VerticalAlign,
};

use crate::engine::{Engine, Spec};
use crate::flow::{self, Cell, Leaf, Marker, Part, Picture, Role, Type, Types, What, Words, marks};
use crate::ink::{Back, Brush, Ink, Inks};
use crate::palette::Palette;
use crate::reveal::{Shapes, Shifts};
use crate::theme;
use crate::tokens::{self, Token, TokenKind};

/// What stands in a layout's text for a formula. Parley sets a box wherever it meets one.
const OBJECT: char = '\u{FFFC}';

/// Something a text's layout shows in place of a stretch of its source: a formula for its TeX, or the label
/// of a figure for a reference to it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Swap {
    /// The bytes of the source it stands for. Empty for what only the layout has, as with a figure's label.
    pub source: Range<usize>,
    /// The bytes of the layout's text that stand for them.
    pub shown: Range<usize>,
    /// The em of the formula set there, and whether it is set as a block. `None` for text.
    pub maths: Option<(f32, bool)>,
    /// What the formula is set from, when its source as written cannot be typeset.
    pub kept: Option<Arc<str>>,
}

/// A text as laid out. Its layout may hold other text than its source: a formula is one box in place of its
/// TeX, and a figure's label comes before the caption. Offsets are of the source unless they say otherwise.
#[derive(Default)]
pub(crate) struct Set {
    pub layout: Layout<Brush>,
    /// The text the layout holds, where that is not the source as it stands.
    pub shown: Option<String>,
    /// Where the layout's text departs from the source, in order.
    pub swaps: Vec<Swap>,
    /// Which of them is the formula the caret is in, when that is typeset and not shown as its source.
    pub open: Option<usize>,
}

impl Set {
    /// The layout's text, given the source it was set from.
    pub fn shown<'a>(&'a self, source: &'a str) -> &'a str {
        self.shown.as_deref().unwrap_or(source)
    }

    /// Where a source offset is in the layout. One inside a swap is at its end.
    pub fn inward(&self, offset: usize) -> usize {
        let mut at = offset;
        for swap in &self.swaps {
            // An offset where a swap starts is before it. A swap that stands for nothing is the exception:
            // it is something that comes before the text there.
            if offset < swap.source.start || (offset == swap.source.start && !swap.source.is_empty()) {
                break;
            }
            at = swap.shown.end + offset.saturating_sub(swap.source.end);
        }
        at
    }

    /// The source offset of a place in the layout.
    pub fn outward(&self, index: usize) -> usize {
        let mut at = index;
        for swap in &self.swaps {
            if index < swap.shown.start {
                break;
            }
            if index < swap.shown.end {
                return if index == swap.shown.start { swap.source.start } else { swap.source.end };
            }
            at = swap.source.end + (index - swap.shown.end);
        }
        at
    }

    fn boxed(&self) -> bool {
        self.swaps.iter().any(|swap| swap.maths.is_some())
    }
}

/// A leaf's layout, kept while the leaf is near the viewport.
#[derive(Default)]
pub(crate) struct Laid {
    /// The leaf's own text.
    pub text: Set,
    /// The texts of a table row's cells.
    pub cells: Vec<Set>,
    /// The number of an ordered item or the tick of a done task, set beside the first line.
    pub sign: Option<Layout<Brush>>,
    /// The host name on each picture that is still a link, with the picture's place among the leaf's.
    pub names: Vec<(usize, Layout<Brush>)>,
}

/// What a text is set from, and how.
pub(crate) struct Source<'a> {
    pub words: &'a Words,
    pub role: Role,
    /// What the text is drawn in where no mark says otherwise.
    pub ink: Ink,
    pub align: Alignment,
    pub width: f32,
    /// What stands before the text, as a figure's label does.
    pub label: Option<String>,
    /// Sets all of it bold, as in the header of a table.
    pub bold: bool,
    /// The whole text is the source of one formula, set as a block.
    pub formula: bool,
    /// The formula of this text that the caret is in.
    pub open: Option<Open<'a>>,
    pub page: Page<'a>,
}

/// The formula the caret is in.
#[derive(Clone, Copy)]
pub(crate) struct Open<'a> {
    pub block: &'a BlockId,
    /// Which of its text's formulas it is, counted from the first.
    pub which: usize,
    /// Whether its source shows in its place. Otherwise it stays typeset.
    pub in_place: bool,
    /// The last of its sources that could be typeset, which it is set from while its source cannot be. So
    /// the line does not jump at every half-written command.
    pub good: Option<&'a Arc<str>>,
}

impl<'a> Open<'a> {
    /// The formula, if it is in this text of a leaf.
    pub fn of(self, leaf: &Leaf, part: Part) -> Option<Self> {
        (leaf.owner(part) == Some(self.block)).then_some(self)
    }
}

/// What a text takes from the page it is on.
#[derive(Clone, Copy)]
pub(crate) struct Page<'a> {
    /// The colour formulas are set in. They hold their colours, so this is the colour of the next paint.
    pub color: Color,
    /// The number of each numbered figure, which a reference to it reads as.
    pub numbers: &'a HashMap<BlockId, u32>,
}

impl<'a> Source<'a> {
    /// A leaf's own text.
    pub fn own(leaf: &'a Leaf, types: &Types, open: Option<Open<'a>>, page: Page<'a>) -> Self {
        // The source of a formula block reads from the left, as code does.
        let plain = leaf.boxed() || leaf.what == What::Formula;
        Self {
            words: &leaf.words,
            role: leaf.role,
            ink: if leaf.what == What::Label { Ink::Marker } else { Ink::Style(leaf.role) },
            align: if plain { Alignment::Start } else { types.get(leaf.role).align },
            width: leaf.width,
            label: leaf.number.map(|number| types.label(number)),
            bold: false,
            formula: leaf.what == What::Formula,
            open,
            page,
        }
    }

    /// The text of a cell of a table row.
    pub fn cell(leaf: &'a Leaf, cell: &'a Cell, align: Alignment, open: Option<Open<'a>>, page: Page<'a>) -> Self {
        Self {
            words: &cell.words,
            role: leaf.role,
            ink: Ink::Style(leaf.role),
            align,
            width: cell.width,
            label: None,
            bold: leaf.header,
            formula: false,
            open,
            page,
        }
    }
}

/// Typesets a formula for a place `room` wide, and says at what em. One too wide is set smaller to fit.
fn typeset(engine: &mut Engine, source: &str, like: Maths, room: f32) -> Option<(Arc<Formula>, f32)> {
    let set = engine.maths.typeset(source, &like).ok()?;
    if set.size.width <= room {
        return Some((set, like.size));
    }
    let size = like.size * (room / set.size.width).max(theme::FORMULA_SHRINK);
    Some((engine.maths.typeset(source, &Maths { size, ..like }).ok()?, size))
}

/// Shapes a text. The caller still breaks the lines. `by_words` keeps a word whole where a line would
/// otherwise break inside it, for measuring how narrow the text can be set.
fn build(
    engine: &mut Engine,
    types: &Types,
    source: &Source<'_>,
    by_words: bool,
    locale: Option<Language>,
    set: &mut Set,
) {
    let (of, code) = (types.get(source.role), types.get(Role::Code));
    let (text, spans) = (source.words.text.as_str(), &source.words.spans[..]);
    let always = if source.bold { marks::BOLD } else { 0 };
    let mut specs: Vec<Spec<'_>> = Vec::with_capacity(spans.len().max(1));
    let mut runs = Vec::with_capacity(spans.len().max(1));
    // One spec per distinct set of marks, however many spans share it.
    let mut slots = [u16::MAX; 256];
    let mut slot = |bits: u8| {
        let bits = bits | always;
        let slot = &mut slots[usize::from(bits)];
        if *slot == u16::MAX {
            *slot = specs.len() as u16;
            specs.push(marked(of, code, source.role == Role::Code, source.ink, bits));
        }
        *slot
    };
    set.swaps.clear();
    set.open = None;
    // Code shows its text as written, whatever is marked in it.
    let swapped = source.role != Role::Code
        && (!source.words.refs.is_empty() || spans.iter().any(|span| span.marks & marks::MATH != 0));
    if !swapped && !source.formula && source.label.is_none() {
        let mut start = 0;
        for span in spans {
            runs.push((slot(span.marks & !marks::MATH), start..span.end));
            start = span.end;
        }
        if spans.is_empty() {
            let plain = slot(0);
            runs.extend((!text.is_empty()).then_some((plain, 0..text.len())));
        }
        set.shown = None;
        engine.build_with(text, &specs, &runs, &[], by_words, locale, &mut set.layout);
        return;
    }

    let mut shown = set.shown.take().unwrap_or_default();
    shown.clear();
    let mut boxes = Vec::new();
    if let Some(label) = &source.label {
        shown.push_str(label);
        shown.push(' ');
        set.swaps.push(Swap { source: 0..0, shown: 0..shown.len(), maths: None, kept: None });
        runs.push((slot(0), 0..shown.len()));
    }
    let whole = [flow::Span { end: text.len(), marks: if source.formula { marks::MATH } else { 0 } }];
    let color = source.page.color;
    let like = Maths { size: of.size * TEXT_RATIO, color, display: source.formula, links: engine.links };
    let (mut start, mut formulas) = (0, 0..);
    let mut refs = source.words.refs.iter().filter(|_| source.role != Role::Code).peekable();
    for span in if spans.is_empty() || source.formula { &whole[..] } else { spans } {
        let (piece, from, mut bits) = (&text[start..span.end], shown.len(), span.marks & !marks::MATH);
        if let Some((_, target)) = refs.next_if(|(range, _)| range.start == start) {
            // A reference reads as the label of its figure, whatever its own text is. One whose figure is
            // gone, or is no longer numbered, says so in the colour of a warning.
            let number = source.page.numbers.get(target).copied();
            shown.push_str(&types.reference(number));
            bits |= if number.is_none() { marks::MATH } else { 0 };
            set.swaps.push(Swap { source: start..span.end, shown: from..shown.len(), maths: None, kept: None });
        } else if span.marks & marks::MATH == 0 {
            shown.push_str(piece);
        } else {
            let which = formulas.next();
            let open = source.open.filter(|open| Some(open.which) == which);
            let good = open.and_then(|open| open.good);
            if open.is_some_and(|open| open.in_place) || piece.trim().is_empty() {
                // The formula the caret is in shows its source in its place, and so does one still to be
                // written.
                bits |= marks::CODE;
                shown.push_str(piece);
            } else if let Some(((formula, size), kept)) = typeset(engine, piece, like, source.width)
                .map(|set| (set, None))
                // While the caret is in it, the last of it that could be typeset stands in its place.
                .or_else(|| Some((typeset(engine, good?, like, source.width)?, good.cloned())))
            {
                shown.push(OBJECT);
                boxes.push(InlineBox {
                    id: set.swaps.len() as u64,
                    kind: InlineBoxKind::InFlow,
                    index: from,
                    width: formula.size.width,
                    height: formula.size.height,
                    baseline: Some(formula.baseline),
                    vertical_align: VerticalAlign::default(),
                });
                let maths = Some((size, source.formula));
                set.open = open.map(|_| set.swaps.len()).or(set.open);
                set.swaps.push(Swap { source: start..span.end, shown: from..shown.len(), maths, kept });
            } else {
                // A source that cannot be typeset shows as it is written, in the colour of a warning.
                bits |= marks::CODE | marks::MATH;
                shown.push_str(piece);
            }
        }
        if shown.len() > from {
            runs.push((slot(bits), from..shown.len()));
        }
        start = span.end;
    }
    if runs.is_empty() {
        slot(0);
    }
    engine.build_with(&shown, &specs, &runs, &boxes, by_words, locale, &mut set.layout);
    set.shown = Some(shown);
}

/// Shapes a text and breaks it to its width.
pub(crate) fn shape(engine: &mut Engine, types: &Types, source: &Source<'_>, locale: Option<Language>, set: &mut Set) {
    build(engine, types, source, false, locale, set);
    // A formula block stands in the middle of the column.
    let align = if source.formula && set.boxed() { Alignment::Center } else { source.align };
    set.layout.set_alignment(align, AlignmentOptions::default());
    set.layout.break_all_lines(Some(source.width));
}

/// The narrowest a text can be set without breaking a word, and the widest it runs when nothing breaks.
pub(crate) fn fit(
    engine: &mut Engine,
    types: &Types,
    source: &Source<'_>,
    locale: Option<Language>,
    set: &mut Set,
) -> (f32, f32) {
    build(engine, types, source, true, locale, set);
    let widths = set.layout.calculate_content_widths();
    (widths.min, widths.max)
}

/// How text of type `of` looks under a set of marks. The mark of a formula here means something is wrong
/// with the text as it stands: a source that could not be typeset, or a reference to nothing.
fn marked<'a>(of: &'a Type, code: &'a Type, mono: bool, ink: Ink, bits: u8) -> Spec<'a> {
    let mut spec = of.spec(mono, Brush { ink, back: None });
    if bits & marks::BOLD != 0 {
        // Bold must still stand out in a type that is already heavy.
        spec.weight = if of.weight >= 600.0 { (of.weight + 200.0).min(900.0) } else { 700.0 };
    }
    if bits & marks::ITALIC != 0 {
        // Emphasis inside italic text turns upright, as in print.
        spec.italic = !spec.italic;
    }
    if bits & marks::CODE != 0 {
        (spec.family, spec.mono) = (&code.family, true);
        spec.size = of.size * theme::INLINE_CODE_SCALE;
        spec.brush.back = Some(Back::Code);
    }
    if bits & marks::HIGHLIGHT != 0 {
        spec.brush.back = Some(Back::Highlight);
    }
    if bits & marks::LINK != 0 {
        spec.brush.ink = Ink::Link;
    }
    if bits & marks::MATH != 0 {
        spec.brush.ink = Ink::Warning;
    }
    spec.underline = bits & (marks::UNDERLINE | marks::LINK) != 0;
    spec.strike = bits & marks::STRIKE != 0;
    spec
}

/// The height a text takes, in whole pixels so that the rows below stay on the pixel grid.
fn tall(layout: &Layout<Brush>) -> f32 {
    layout.height().round().max(1.0)
}

/// What a layout says of a leaf: how tall it is, and the lines a page may split it at.
pub(crate) struct Extent {
    pub height: f32,
    pub lines: u32,
    /// Where each line ends, from the leaf's top, when they are not all as tall as each other.
    pub ends: Vec<f32>,
    /// The lines that carry on from the line before with no line break between. Only a line of code holds
    /// together like that: prose may go to the next page at any line.
    pub joined: Vec<u32>,
}

/// Where each line of a text ends, when the text starts `lead` below the leaf's top.
fn bottoms(set: &Set, lead: f32) -> impl Iterator<Item = f32> + '_ {
    set.layout.lines().map(move |line| lead + line.metrics().block_max_coord)
}

pub(crate) fn extent(leaf: &Leaf, laid: &Laid) -> Extent {
    let text = tall(&laid.text.layout);
    let (mut ends, mut joined) = (Vec::new(), Vec::new());
    let (height, lines) = match leaf.what {
        What::Row => {
            // The row is as tall as its tallest cell, and splits where that cell's lines do.
            let tallest = laid.cells.iter().max_by(|a, b| a.layout.height().total_cmp(&b.layout.height()));
            ends.extend(tallest.into_iter().flat_map(|set| bottoms(set, theme::CELL_PAD_Y)));
            (tallest.map_or(1.0, |set| tall(&set.layout)) + 2.0 * theme::CELL_PAD_Y, ends.len())
        }
        What::Gallery => {
            // A line is a row of pictures with the gap under it. The caption stays with the last.
            ends.resize(leaf.pictures.last().map_or(0, |last| last.row + 1), 0.0);
            for picture in &leaf.pictures {
                ends[picture.row] = picture.rect.y + picture.rect.height + theme::TILE_GAP;
            }
            (leaf.tall(text), ends.len())
        }
        // A picture never parts from its caption.
        What::Figure => (leaf.tall(text), 1),
        _ => {
            if laid.text.boxed() {
                ends.extend(bottoms(&laid.text, 0.0));
            }
            let mut wrapped = false;
            for (index, line) in laid.text.layout.lines().enumerate().filter(|_| leaf.role == Role::Code) {
                if wrapped {
                    joined.push(index as u32);
                }
                wrapped = line.break_reason() != BreakReason::Explicit;
            }
            (text, laid.text.layout.len())
        }
    };
    if let Some(last) = ends.last_mut() {
        *last = height;
    }
    Extent { height, lines: lines.max(1) as u32, ends, joined }
}

/// The tick in the box of a done task.
const TICK: &str = "\u{2713}";

/// What stands before an ordered item.
fn numbered(number: u32) -> String {
    format!("{number}.")
}

/// How a leaf's texts are to be laid out, beyond what the leaf says itself.
pub(crate) struct Lay<'a> {
    /// The formula the caret is in.
    pub open: Option<Open<'a>>,
    pub page: Page<'a>,
    pub locale: Option<Language>,
}

/// Lays out the texts of a leaf: its own, and those of its cells.
pub(crate) fn lay(engine: &mut Engine, types: &Types, leaf: &Leaf, how: &Lay<'_>, laid: &mut Laid) {
    let open = |part: Part| how.open.and_then(|open| open.of(leaf, part));
    if leaf.what != What::Row {
        let source = Source::own(leaf, types, open(Part::Text), how.page);
        shape(engine, types, &source, how.locale, &mut laid.text);
    }
    laid.cells.resize_with(leaf.cells.len(), Set::default);
    for (index, (cell, set)) in leaf.cells.iter().zip(&mut laid.cells).enumerate() {
        let source = Source::cell(leaf, cell, cell.align, open(Part::Cell(index)), how.page);
        shape(engine, types, &source, how.locale, set);
    }
}

/// The formula the caret is in, where a leaf's layout has it typeset: the text it is in, and its swap.
pub(crate) fn opened<'a>(leaf: &Leaf, laid: &'a Laid) -> Option<(Part, &'a Swap)> {
    let cells = laid.cells.iter().enumerate().map(|(index, set)| (Part::Cell(index), set));
    let own = (leaf.what != What::Row).then_some((Part::Text, &laid.text));
    own.into_iter().chain(cells).find_map(|(part, set)| Some((part, set.swaps.get(set.open?)?)))
}

/// Adds to a leaf's layout what is drawn beside its text: the sign its marker needs, and the names on
/// pictures that are still links.
pub(crate) fn adorn(engine: &mut Engine, types: &Types, leaf: &Leaf, laid: &mut Laid) {
    let of = types.get(leaf.role);
    let mut small = |text: &str, ink: Ink, size: f32, width: Option<f32>| {
        let mut spec = of.spec(false, Brush { ink, back: None });
        (spec.size, spec.italic) = (size, false);
        let mut layout = Layout::new();
        engine.build(text, &[spec], &[(0, 0..text.len())], None, &mut layout);
        if width.is_some() {
            layout.set_alignment(Alignment::Center, AlignmentOptions::default());
        }
        layout.break_all_lines(width);
        layout
    };
    laid.sign = match leaf.marker {
        Marker::Number(number) => Some(small(&numbered(number), Ink::Marker, of.size, None)),
        Marker::Task(true) => Some(small(TICK, Ink::OnAccent, theme::CHECKBOX * 0.8, None)),
        _ => None,
    };
    laid.names.clear();
    for (index, picture) in leaf.pictures.iter().enumerate() {
        let room = picture.rect.width - 2.0 * theme::BADGE_INSET;
        if picture.asset.is_none() && !picture.host.is_empty() && room > 0.0 {
            laid.names.push((index, small(&picture.host, Ink::Marker, of.size, Some(room))));
        }
    }
}

/// Where the cloud sits on a picture that is still a link, when the picture has room for it.
pub(crate) fn badge(frame: Rect) -> Option<Rect> {
    let (side, inset) = (theme::BADGE, theme::BADGE_INSET);
    let fits = frame.width >= side + 2.0 * inset && frame.height >= side + 2.0 * inset;
    fits.then(|| Rect::new(frame.x + frame.width - inset - side, frame.y + inset, side, side))
}

/// The outline of a cloud, in a box of 16 units.
static CLOUD: LazyLock<Arc<[PathEl]>> = LazyLock::new(|| {
    let at = |(x, y): (f32, f32)| Point { x, y };
    let curve = |a, b, end| PathEl::CurveTo(at(a), at(b), at(end));
    Arc::new([
        PathEl::MoveTo(at((4.5, 12.5))),
        curve((2.8, 12.5), (1.5, 11.2), (1.5, 9.6)),
        curve((1.5, 8.1), (2.6, 6.9), (4.1, 6.8)),
        curve((4.6, 4.9), (6.3, 3.5), (8.3, 3.5)),
        curve((10.6, 3.5), (12.4, 5.1), (12.7, 7.3)),
        curve((13.8, 7.6), (14.5, 8.7), (14.5, 9.9)),
        curve((14.5, 11.4), (13.4, 12.5), (12.0, 12.5)),
        PathEl::Close,
    ])
});

/// Glyphs of one style and one kind of token, gathered along a line.
struct Stretch {
    style: u16,
    kind: Option<TokenKind>,
    ink: Ink,
    /// Where the first glyph starts, from the line's left edge.
    start: f32,
    glyphs: Vec<Glyph>,
    spelt: Spelt,
}

/// What a run of glyphs spells, gathered a cluster at a time in the order the glyphs are drawn.
#[derive(Default)]
struct Spelt {
    /// Where in the text each glyph's cluster starts.
    starts: Vec<u32>,
    /// The text of the clusters so far.
    range: Option<Range<usize>>,
}

impl Spelt {
    /// Takes in a cluster that draws `glyphs` glyphs. One that draws none, as the rest of a ligature does,
    /// belongs to the glyphs before it.
    fn add(&mut self, cluster: Range<usize>, glyphs: usize) {
        match &mut self.range {
            Some(range) => *range = range.start.min(cluster.start)..range.end.max(cluster.end),
            None if glyphs > 0 => self.range = Some(cluster.clone()),
            None => return,
        }
        self.starts.extend(std::iter::repeat_n(cluster.start as u32, glyphs));
    }

    /// The text of `source` that `glyphs` glyphs were gathered for.
    fn text(self, source: &str, glyphs: usize) -> Option<Arc<RunText>> {
        let range = self.range.filter(|_| self.starts.len() == glyphs)?;
        let text = source.get(range.clone())?.into();
        let clusters = self.starts.iter().map(|start| start - range.start as u32).collect();
        Some(Arc::new(RunText { text, clusters }))
    }
}

/// Says what the glyphs of `item` spell, and where in `source` that starts. `clusters` are those of its line
/// that no run before it has taken.
fn spell<'a>(
    item: &mut Item,
    clusters: &mut Peekable<impl Iterator<Item = Cluster<'a, Brush>>>,
    source: &str,
) -> Option<usize> {
    let Item::Glyphs { glyphs, text, .. } = item else { return None };
    let mut spelt = Spelt::default();
    while spelt.starts.len() < glyphs.len() {
        let cluster = clusters.next()?;
        spelt.add(cluster.text_range(), cluster.glyphs().count());
    }
    while let Some(rest) = clusters.next_if(Cluster::is_ligature_continuation) {
        spelt.add(rest.text_range(), 0);
    }
    let start = spelt.range.as_ref().map(|range| range.start);
    *text = spelt.text(source, glyphs.len());
    start.filter(|_| text.is_some())
}

/// The stretches of a text that lead somewhere, as its layout has them and in order: its links, and its
/// references that name a figure. Each comes with its number among the text's links and references, which
/// names its part on a sheet. Code has none.
fn leads(text: &Text<'_>, numbers: &HashMap<BlockId, u32>) -> Vec<(Range<usize>, u64)> {
    let Words { links, refs, .. } = text.source;
    let links = links.iter().map(|(range, _)| range);
    let refs = refs.iter().map(|(range, target)| numbers.contains_key(target).then_some(range));
    let all = links.map(Some).chain(refs).enumerate().filter(|_| text.role != Role::Code);
    let inward = |range: &Range<usize>| text.set.inward(range.start)..text.set.inward(range.end);
    let mut leads: Vec<_> = all.filter_map(|(which, range)| Some((inward(range?), which as u64))).collect();
    leads.sort_by_key(|(range, _)| range.start);
    leads
}

/// The part of a text that is `which` of its links and references, to add to the text's own name.
pub(crate) fn link(part: Part, which: usize) -> u64 {
    let cell = if let Part::Cell(index) = part { (index as u64 + 1) << CELLS } else { 0 };
    cell | LINKS | which as u64
}

/// Some of the glyphs of a run that says what it spells, as a run of their own that says what they spell.
fn piece(run: &Item, glyphs: Range<usize>) -> Item {
    let mut piece = run.clone();
    if let Item::Glyphs { glyphs: all, text: Some(spelt), .. } = &mut piece {
        // Their text runs from the first of their clusters to the next cluster that is not theirs.
        let own = &spelt.clusters[glyphs.clone()];
        let (first, last) = (own.iter().min().copied().unwrap_or(0), own.iter().max().copied().unwrap_or(0));
        let after = spelt
            .clusters
            .iter()
            .filter(|start| **start > last)
            .min()
            .map_or(spelt.text.len(), |start| *start as usize);
        let text = spelt.text[first as usize..after].into();
        let clusters = own.iter().map(|start| start - first).collect();
        *spelt = Arc::new(RunText { text, clusters });
        *all = all[glyphs].to_vec();
    }
    piece
}

/// What is drawn over a text without being part of it.
pub(crate) struct Over<'a> {
    /// The markers revealed in the text.
    pub shifts: &'a Shifts,
    pub shapes: &'a Shapes,
    /// The text an input method is composing, which is underlined.
    pub composing: Option<Range<usize>>,
    /// The misspelled words, in order.
    pub spelling: &'a [Range<usize>],
}

/// One laid text of a leaf, and where it is.
#[derive(Clone, Copy)]
pub(crate) struct Text<'a> {
    pub set: &'a Set,
    pub source: &'a Words,
    /// The text's top left on the page.
    pub at: Point,
    pub role: Role,
}

impl<'a> Text<'a> {
    /// The text of a part of a leaf, when the part has one and the leaf holds its layout.
    pub fn of(leaf: &'a Leaf, part: Part) -> Option<Self> {
        let laid = leaf.laid.as_deref()?;
        let (set, source, x, y) = match part {
            Part::Text if leaf.what != What::Row => (&laid.text, &leaf.words, 0.0, leaf.lead),
            Part::Cell(index) => {
                let cell = leaf.cells.get(index)?;
                (laid.cells.get(index)?, &cell.words, cell.x, theme::CELL_PAD_Y)
            }
            _ => return None,
        };
        Some(Self { set, source, at: Point { x: leaf.x + x, y: leaf.y + y }, role: leaf.role })
    }
}

/// What painting draws with and into.
pub(crate) struct Painter<'a> {
    pub engine: &'a mut Engine,
    pub types: &'a Types,
    pub inks: &'a Inks,
    /// The top and bottom of what is being painted. Lines outside it are skipped.
    pub band: (f32, f32),
    /// Whether each run of glyphs says what it spells. Only an export needs that, and it costs the screen.
    pub spelt: bool,
    /// What names the text being painted where a sheet says what its parts are. `None` on screen.
    pub part: Option<u64>,
    /// The number of each numbered figure, which says whether a reference leads anywhere.
    pub numbers: &'a HashMap<BlockId, u32>,
    pub out: &'a mut DisplayList,
}

/// Which part of a text an [`Item::Open`] names, added to the text's own name: the text itself is 0, then
/// its marker, then its formulas.
const LABEL: u64 = 1;
const FORMULAS: u64 = 2;
/// The links of a text come after its formulas.
const LINKS: u64 = 1 << 15;
/// The pictures of a leaf are named after its own text and what is in that.
const PICTURES: u64 = 1 << 31;
/// The cells of a row are named from the row, each with room for what is in its text.
const CELLS: u32 = 16;

/// What is drawn behind a leaf, or in the place of its text: the box of code or of a note, a divider, or
/// the line under a row of a table, which is heavier under the header.
pub(crate) fn backing(leaf: &Leaf, palette: &Palette) -> Option<Item> {
    let boxed = Rect::new(
        leaf.x - leaf.pad,
        leaf.y - theme::BOX_PAD_Y,
        leaf.width + 2.0 * leaf.pad,
        leaf.height + 2.0 * theme::BOX_PAD_Y,
    );
    let line = |y: f32, color| Item::Fill { rect: Rect::new(leaf.x, y, leaf.width, theme::RULE), color, radius: 0.0 };
    match leaf.what {
        What::Rule => Some(line(leaf.y, palette.line)),
        What::Row => {
            let color = if leaf.header { palette.line_strong } else { palette.line };
            Some(line(leaf.y + leaf.height - theme::RULE, color))
        }
        What::Label | What::Empty => {
            Some(Item::Stroke { rect: boxed, color: palette.line_strong, width: 1.0, radius: theme::BOX_RADIUS })
        }
        _ if leaf.boxed() => Some(Item::Fill { rect: boxed, color: palette.code, radius: theme::BOX_RADIUS }),
        _ => None,
    }
}

/// What a leaf's own text is to a reader that cannot see the page.
fn tag(leaf: &Leaf) -> Tag {
    match (&leaf.kind, leaf.what, leaf.role) {
        (_, What::Formula, _) => Tag::Formula { alt: leaf.words.text.as_str().into() },
        (_, What::Figure | What::Gallery, _) => Tag::Caption,
        (Some(BlockKind::ListItem { list, depth }), ..) => Tag::ListItem {
            depth: *depth,
            numbered: *list == ListKind::Ordered,
            checked: match list {
                ListKind::Task { checked } => Some(*checked),
                _ => None,
            },
        },
        (.., Role::Code) => Tag::Code,
        (.., role) => role.level().map_or(Tag::Paragraph, Tag::Heading),
    }
}

impl Painter<'_> {
    fn outside(&self, top: f32, bottom: f32) -> bool {
        bottom <= self.band.0 || top >= self.band.1
    }

    /// Whether any line of a text falls inside the band. A text with none is no part of what is painted.
    fn shows(&self, text: &Text<'_>) -> bool {
        text.set.layout.lines().any(|line| {
            let metrics = line.metrics();
            !self.outside(text.at.y + metrics.block_min_coord, text.at.y + metrics.block_max_coord)
        })
    }

    /// Starts a part of the text, where parts are told: what is drawn up to [`Painter::close`] is of it.
    fn open(&mut self, which: u64, role: impl FnOnce() -> Tag) {
        if let Some(id) = self.part {
            self.out.push(Item::Open { role: role(), id: id | which });
        }
    }

    fn close(&mut self) {
        if self.part.is_some() {
            self.out.push(Item::Close);
        }
    }

    /// Emits what a leaf shows apart from the text of its cells: its marker and pictures, and the lines of
    /// its own text that fall inside the band. `tokens` colour the text of a code block.
    pub fn leaf(&mut self, leaf: &Leaf, laid: &Laid, tokens: Option<&[Token]>, over: &Over<'_>) {
        // A figure holds its caption. The pictures of a gallery are figures side by side, and its caption
        // comes after them.
        let shown = leaf.what == What::Figure && !self.outside(leaf.y, leaf.y + leaf.height);
        let figure = leaf.pictures.first().filter(|_| shown);
        if let Some(picture) = figure {
            self.open(PICTURES, || Tag::Figure { alt: picture.alt.clone() });
        }
        self.pictures(leaf, laid);
        self.own(leaf, laid, tokens, over);
        if figure.is_some() {
            self.close();
        }
    }

    /// Emits the lines of a leaf's own text that fall inside the band, with its marker.
    fn own(&mut self, leaf: &Leaf, laid: &Laid, tokens: Option<&[Token]>, over: &Over<'_>) {
        let (true, Some(text)) = (leaf.worded(), Text::of(leaf, Part::Text)) else { return };
        if !self.shows(&text) {
            return;
        }
        self.open(0, || tag(leaf));
        if let Some(first) = laid.text.layout.get(0)
            && !self.outside(text.at.y + first.metrics().block_min_coord, text.at.y + first.metrics().block_max_coord)
        {
            let baseline = first.metrics().baseline;
            self.marker(leaf, laid, baseline);
            // A heading's marker hangs in the margin. Where the margin is too narrow, the line makes room.
            let mut right = leaf.x - theme::MARKER_GAP + over.shifts.indent;
            for (shape, level) in over.shifts.hung.iter().rev() {
                let Some(shape) = over.shapes.layout(*shape) else { continue };
                right -= shape.full_width();
                self.sign(shape, Point { x: right, y: leaf.y + baseline }, *level);
            }
        }
        self.text(&text, tokens, over);
        self.close();
    }

    /// Emits the lines of one cell of a table row that fall inside the band.
    pub fn cell(&mut self, leaf: &Leaf, index: usize, over: &Over<'_>) {
        let Some(text) = Text::of(leaf, Part::Cell(index)).filter(|text| self.shows(text)) else { return };
        // A cell is named from its row, with room under its name for what is in its text.
        self.part = self.part.map(|row| row | (index as u64 + 1) << CELLS);
        self.open(0, || Tag::Cell { header: leaf.header });
        self.text(&text, None, over);
        self.close();
    }

    /// Draws the pictures of a figure or gallery.
    fn pictures(&mut self, leaf: &Leaf, laid: &Laid) {
        for (index, picture) in leaf.pictures.iter().enumerate() {
            let at = picture.rect;
            let rect = Rect::new(leaf.x + at.x, leaf.y + at.y, at.width, at.height);
            if self.outside(rect.y, rect.y + rect.height) {
                continue;
            }
            let alone = leaf.what == What::Gallery;
            if alone {
                self.open(PICTURES + index as u64, || Tag::Figure { alt: picture.alt.clone() });
            }
            if let Some(asset) = &picture.asset {
                self.out.push(Item::Image { rect, asset: asset.clone() });
            } else {
                self.remote(picture, rect, laid.names.iter().find(|(at, _)| *at == index).map(|(_, name)| name));
            }
            if alone {
                self.close();
            }
        }
    }

    /// Draws a picture that is still a link: a frame with the name of its host and a cloud in the corner.
    fn remote(&mut self, picture: &Picture, rect: Rect, name: Option<&Layout<Brush>>) {
        let palette = self.inks.palette;
        self.out.push(Item::Fill { rect, color: palette.code, radius: theme::BOX_RADIUS });
        self.out.push(Item::Stroke { rect, color: palette.line_strong, width: 1.0, radius: theme::BOX_RADIUS });
        if let Some(name) = name {
            let top = rect.y + ((rect.height - name.height()) / 2.0).round();
            self.out.push(Item::PushClip { rect, radius: theme::BOX_RADIUS });
            self.glyphs(name, &picture.host, Point { x: rect.x + theme::BADGE_INSET, y: top });
            self.out.push(Item::PopClip);
        }
        if let Some(badge) = badge(rect) {
            let (origin, scale) = (Point { x: badge.x, y: badge.y }, badge.width / 16.0);
            let (path, color) = (CLOUD.clone(), palette.marker);
            self.out.push(Item::Path { origin, scale, path, color, stroke: Some(1.25) });
        }
    }

    /// Emits the lines of a text that fall inside the band, and what is drawn over them.
    fn text(&mut self, text: &Text<'_>, tokens: Option<&[Token]>, over: &Over<'_>) {
        let Text { set, at, .. } = *text;
        let shown = set.shown(&text.source.text);
        // A sheet says which of its text leads somewhere.
        let leads = if self.part.is_some() { leads(text, self.numbers) } else { Vec::new() };
        for (index, line) in set.layout.lines().enumerate() {
            let metrics = line.metrics();
            if self.outside(at.y + metrics.block_min_coord, at.y + metrics.block_max_coord) {
                continue;
            }
            let pushed = over.shifts.touches(index);
            // Fills go under every glyph of the line, so one run's fill never covers its neighbour's overhang.
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let Some(back) = run.style().brush.back else { continue };
                let font = run.run().font_metrics();
                let (lead, wider) = over.shifts.spread(index, run.offset(), run.offset() + run.advance());
                let rect = Rect::new(
                    at.x + run.offset() + lead - 2.0,
                    at.y + run.baseline() - font.ascent,
                    run.advance() + wider + 4.0,
                    font.ascent + font.descent,
                );
                self.out.push(Item::Fill { rect, color: self.inks.back(back), radius: theme::INLINE_RADIUS });
            }
            if let Some(tokens) = tokens {
                self.tokens(&line, at, tokens, shown);
            }
            let mut clusters = line.runs().flat_map(|run| run.visual_clusters()).peekable();
            for item in line.items() {
                let run = match item {
                    PositionedLayoutItem::GlyphRun(run) => run,
                    PositionedLayoutItem::InlineBox(inline) => {
                        let push = over.shifts.push(index, inline.x);
                        let origin = Point { x: at.x + inline.x + push, y: at.y + inline.y };
                        self.formula(text, inline.id as usize, origin);
                        continue;
                    }
                };
                let style = run.style();
                let color = self.inks.ink(style.brush.ink);
                match (tokens, pushed) {
                    (Some(_), _) => {}
                    (None, true) => self.pushed(&run, at, color, index, over.shifts),
                    (None, false) => {
                        let mut glyphs = self.engine.glyphs(&run, at, color);
                        let from =
                            glyphs.as_mut().filter(|_| self.spelt).and_then(|item| spell(item, &mut clusters, shown));
                        match (glyphs, from) {
                            (Some(item), Some(from)) if !leads.is_empty() => self.leading(item, from, &leads),
                            (glyphs, _) => self.out.items.extend(glyphs),
                        }
                    }
                }
                let font = run.run().font_metrics();
                let (lead, wider) = over.shifts.spread(index, run.offset(), run.offset() + run.advance());
                let strokes = [
                    (&style.underline, font.underline_offset, font.underline_size),
                    (&style.strikethrough, font.strikethrough_offset, font.strikethrough_size),
                ];
                for (stroke, offset, size) in strokes {
                    let Some(stroke) = stroke else { continue };
                    let size = stroke.size.unwrap_or(size).max(1.0);
                    let y = at.y + run.baseline() - stroke.offset.unwrap_or(offset);
                    let rect = Rect::new(at.x + run.offset() + lead, y, run.advance() + wider, size);
                    self.out.push(Item::Fill { rect, color: self.inks.ink(stroke.brush.ink), radius: 0.0 });
                }
            }
            // Each marker goes in the room pushed open for it.
            let mut room = 0.0;
            for shown in over.shifts.on(index) {
                if let Some(shape) = over.shapes.layout(shown.shape) {
                    self.sign(shape, Point { x: at.x + shown.x + room, y: at.y + metrics.baseline }, shown.level);
                }
                room += shown.width;
            }
        }
        // A word still being composed is not yet a word to judge.
        let composed = |range: &&Range<usize>| {
            over.composing
                .as_ref()
                .is_some_and(|composing| range.start <= composing.end && composing.start <= range.end)
        };
        for range in over.spelling.iter().filter(|range| !composed(range)) {
            self.misspelled(text, range, over.shifts);
        }
        if let Some(range) = &over.composing {
            self.composing(text, range, over.shifts);
        }
    }

    /// Emits a run of glyphs whose text starts at byte `from` of its layout's, with each stretch of it that
    /// leads somewhere as a part of its own. A run can hold more than one, and text that leads nowhere.
    fn leading(&mut self, item: Item, from: usize, leads: &[(Range<usize>, u64)]) {
        let Item::Glyphs { glyphs, text: Some(spelt), .. } = &item else { return self.out.push(item) };
        let lead = |glyph: usize| {
            let at = from + spelt.clusters[glyph] as usize;
            leads.iter().find(|(range, _)| range.contains(&at)).map(|(_, which)| *which)
        };
        let mut start = 0;
        while start < glyphs.len() {
            let which = lead(start);
            let end = (start + 1..glyphs.len()).find(|glyph| lead(*glyph) != which).unwrap_or(glyphs.len());
            let piece = if (start, end) == (0, glyphs.len()) { item.clone() } else { piece(&item, start..end) };
            if let Some(which) = which {
                self.open(LINKS | which, || Tag::Link);
            }
            self.out.push(piece);
            if which.is_some() {
                self.close();
            }
            start = end;
        }
    }

    /// Draws the formula of one of a text's swaps with the top left of its box at `origin`. The cache holds
    /// it from when the text was shaped, unless the colours have changed since.
    fn formula(&mut self, text: &Text<'_>, swap: usize, origin: Point) {
        let Some(Swap { source, maths: Some((size, display)), kept, .. }) = text.set.swaps.get(swap) else { return };
        let Some(source) = kept.as_deref().or_else(|| text.source.text.get(source.clone())) else { return };
        let color = self.inks.ink(Ink::Style(text.role));
        let style = Maths { size: *size, color, display: *display, links: self.engine.links };
        if let Ok(formula) = self.engine.maths.typeset(source, &style) {
            // A formula block is a part already. One in a line is a part of its own inside the text's, and
            // says how it was written, since its glyphs spell nothing.
            let inline = !*display;
            if inline {
                self.open(FORMULAS + swap as u64, || Tag::Formula { alt: source.into() });
            }
            formula.paint(origin, self.out);
            if inline {
                self.close();
            }
        }
    }

    /// Draws a wavy line under a misspelled word.
    fn misspelled(&mut self, text: &Text<'_>, range: &Range<usize>, shifts: &Shifts) {
        let (of, color) = (self.types.get(text.role), self.inks.palette.spelling);
        stretches(text.set, range, shifts, |line, part| {
            let Some(baseline) = text.set.layout.get(line).map(|line| line.metrics().baseline) else { return };
            let y = (text.at.y + baseline + of.size * 0.16).round() + 0.5;
            if self.outside(y - theme::WAVE_SWING, y + theme::WAVE_SWING) || part.width <= 0.0 {
                return;
            }
            // Whole waves, stretched a little to end where the word does.
            let waves = (part.width / theme::WAVE).round().max(2.0);
            let step = part.width / waves;
            let mut path = vec![PathEl::MoveTo(Point { x: 0.0, y: 0.0 })];
            for wave in 0..waves as usize {
                let swing = if wave % 2 == 0 { -2.0 * theme::WAVE_SWING } else { 2.0 * theme::WAVE_SWING };
                let from = wave as f32 * step;
                path.push(PathEl::QuadTo(Point { x: from + step / 2.0, y: swing }, Point { x: from + step, y: 0.0 }));
            }
            let origin = Point { x: text.at.x + part.x, y };
            self.out.push(Item::Path { origin, scale: 1.0, path: path.into(), color, stroke: Some(1.0) });
        });
    }

    /// Emits a run's glyphs on a line that holds markers, each pushed by the markers before it.
    fn pushed(&mut self, run: &GlyphRun<'_, Brush>, at: Point, color: Color, line: usize, shifts: &Shifts) {
        // Along the run, as when nothing is pushed, so that glyphs before the first marker land where they were.
        let mut pen = 0.0;
        let mut open: Option<(f32, f32, Vec<Glyph>)> = None;
        let mut close = |open: Option<(f32, f32, Vec<Glyph>)>| {
            let Some((push, start, glyphs)) = open else { return };
            let origin = Point { x: at.x + run.offset() + start + push, y: at.y + run.baseline() };
            self.out.items.extend(self.engine.item(run.run(), origin, color, false, glyphs));
        };
        for glyph in run.glyphs() {
            let push = shifts.push(line, run.offset() + pen);
            if open.as_ref().is_none_or(|(was, ..)| *was != push) {
                close(open.take());
                open = Some((push, pen, Vec::new()));
            }
            if let Some((_, start, glyphs)) = &mut open {
                glyphs.push(Glyph { id: glyph.id, x: pen - *start + glyph.x, y: glyph.y });
            }
            pen += glyph.advance;
        }
        close(open);
    }

    /// Draws a revealed marker with the left end of its baseline at `at`. `level` is how much of it shows.
    pub fn sign(&mut self, shape: &Layout<Brush>, at: Point, level: f32) {
        let Some(line) = shape.get(0) else { return };
        let Color { r, g, b, a } = self.inks.palette.marker;
        let color = Color::rgba(r, g, b, (f32::from(a) * level.clamp(0.0, 1.0)).round() as u8);
        let at = Point { x: at.x, y: at.y - line.metrics().baseline };
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else { continue };
            self.out.items.extend(self.engine.glyphs(&run, at, color));
        }
    }

    /// Underlines the text an input method is composing.
    fn composing(&mut self, text: &Text<'_>, range: &Range<usize>, shifts: &Shifts) {
        let of = self.types.get(text.role);
        let (size, color) = ((of.size / 12.0).round().max(1.0), self.inks.ink(Ink::Style(text.role)));
        stretches(text.set, range, shifts, |line, part| {
            let Some(baseline) = text.set.layout.get(line).map(|line| line.metrics().baseline) else { return };
            let y = (text.at.y + baseline + of.size * 0.12).round();
            if self.outside(y, y + size) {
                return;
            }
            self.out.push(Item::Fill { rect: Rect::new(text.at.x + part.x, y, part.width, size), color, radius: 0.0 });
        });
    }

    /// Emits a line's glyphs with each token in its colour. A token can start anywhere inside one of Parley's
    /// glyph runs, so this walks the clusters and starts an item wherever the style or the token changes.
    fn tokens(&mut self, line: &Line<'_, Brush>, at: Point, tokens: &[Token], source: &str) {
        let metrics = line.metrics();
        let mut pen = metrics.inline_min_coord + metrics.offset;
        for run in line.runs() {
            let mut open: Option<Stretch> = None;
            let mut close = |open: Option<Stretch>| {
                let Some(Stretch { kind, ink, start, glyphs, spelt, .. }) = open else { return };
                let origin = Point { x: at.x + start, y: at.y + metrics.baseline };
                let color = kind.map_or(self.inks.ink(ink), |kind| self.inks.palette.syntax.color(kind));
                let slant = kind == Some(TokenKind::Comment);
                let spelt = if self.spelt { spelt.text(source, glyphs.len()) } else { None };
                let mut item = self.engine.item(&run, origin, color, slant, glyphs);
                if let Some(Item::Glyphs { text, .. }) = &mut item {
                    *text = spelt;
                }
                self.out.items.extend(item);
            };
            for cluster in run.visual_clusters() {
                let (style, kind) = (cluster.style_index(), tokens::at(tokens, cluster.text_range().start));
                let mut drawn = 0;
                for glyph in cluster.glyphs() {
                    if open.as_ref().is_none_or(|open| (open.style, open.kind) != (style, kind)) {
                        close(open.take());
                        let (ink, spelt) = (cluster.style().brush.ink, Spelt::default());
                        open = Some(Stretch { style, kind, ink, start: pen, glyphs: Vec::new(), spelt });
                    }
                    if let Some(open) = &mut open {
                        open.glyphs.push(Glyph { id: glyph.id, x: pen - open.start + glyph.x, y: glyph.y });
                    }
                    pen += glyph.advance;
                    drawn += 1;
                }
                if let Some(open) = &mut open {
                    open.spelt.add(cluster.text_range(), drawn);
                }
            }
            close(open.take());
        }
    }

    /// Draws a list item's marker in the indent, level with the first line. A task's box is the item's label
    /// whether it is ticked or not.
    fn marker(&mut self, leaf: &Leaf, laid: &Laid, baseline: f32) {
        if leaf.marker != Marker::None {
            self.open(LABEL, || Tag::Label);
            self.mark(leaf, laid, baseline);
            self.close();
        }
    }

    fn mark(&mut self, leaf: &Leaf, laid: &Laid, baseline: f32) {
        let (of, palette) = (self.types.get(leaf.role), &self.inks.palette);
        let right = leaf.x - theme::MARKER_GAP;
        // The middle of a lower-case letter, which is where a bullet or checkbox looks centred.
        let middle = leaf.y + baseline - of.size * 0.34;
        match leaf.marker {
            Marker::None => {}
            Marker::Bullet(depth) => {
                let side = (of.size * 0.36).round().max(3.0);
                let rect = Rect::new(right - side, (middle - side / 2.0).round(), side, side);
                self.out.push(match depth % 3 {
                    0 => Item::Fill { rect, color: palette.marker, radius: side / 2.0 },
                    1 => Item::Stroke { rect, color: palette.marker, width: 1.0, radius: side / 2.0 },
                    _ => Item::Fill { rect, color: palette.marker, radius: 0.0 },
                });
            }
            Marker::Number(number) => {
                let Some(sign) = &laid.sign else { return };
                self.glyphs(sign, &numbered(number), Point { x: right - sign.full_width(), y: leaf.y });
            }
            Marker::Task(checked) => {
                let side = theme::CHECKBOX;
                let rect = Rect::new(right - side, (middle - side / 2.0).round(), side, side);
                if !checked {
                    self.out.push(Item::Stroke { rect, color: palette.line_strong, width: 1.5, radius: 3.0 });
                    return;
                }
                self.out.push(Item::Fill { rect, color: palette.accent, radius: 3.0 });
                let Some(sign) = &laid.sign else { return };
                let at = Point {
                    x: rect.x + (side - sign.full_width()) / 2.0,
                    y: rect.y + ((side - sign.height()) / 2.0).round(),
                };
                self.glyphs(sign, TICK, at);
            }
        }
    }

    /// Emits a whole small layout of `source` with its top left at `at`.
    pub fn glyphs(&mut self, layout: &Layout<Brush>, source: &str, at: Point) {
        for line in layout.lines() {
            let mut clusters = line.runs().flat_map(|run| run.visual_clusters()).peekable();
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let mut glyphs = self.engine.glyphs(&run, at, self.inks.ink(run.style().brush.ink));
                if let (Some(item), true) = (&mut glyphs, self.spelt) {
                    spell(item, &mut clusters, source);
                }
                self.out.items.extend(glyphs);
            }
        }
    }
}

/// The part of each line that a byte range covers: the line, and the part's box from the text's top left.
/// Markers on the line have pushed and widened it.
pub(crate) fn stretches(set: &Set, range: &Range<usize>, shifts: &Shifts, mut each: impl FnMut(usize, Rect)) {
    let layout = &set.layout;
    let start = Cursor::from_byte_index(layout, set.inward(range.start), Affinity::Downstream);
    let end = Cursor::from_byte_index(layout, set.inward(range.end), Affinity::Upstream);
    Span::new(start, end).geometry_with(layout, |area, line| {
        let (lead, wider) = shifts.spread(line, area.x0 as f32, area.x1 as f32);
        let (width, height) = (area.width() as f32 + wider, area.height() as f32);
        each(line, Rect::new(area.x0 as f32 + lead, area.y0 as f32, width, height));
    });
}

/// The byte offset nearest to a point on the page.
///
/// A [`letters_model::Pos`] has no affinity, and an offset at a wrap draws at the start of the next line. So a
/// point past the end of a wrapped line lands before the space the line wraps at, never after it.
pub(crate) fn hit(text: &Text<'_>, shifts: &Shifts, point: Point) -> usize {
    let (layout, x, y) = (&text.set.layout, point.x - text.at.x, point.y - text.at.y);
    let x = if shifts.is_empty() { x } else { shifts.pull(line_at(layout, y), x) };
    let cursor = Cursor::from_point(layout, x, y);
    if cursor.affinity() == Affinity::Upstream
        && cursor.index() < layout.text_len()
        && let Some(last) = cursor.upstream_cluster(layout)
        && last.is_soft_line_break()
    {
        return text.set.outward(last.text_range().start);
    }
    text.set.outward(cursor.index())
}

/// The line at a height given from the text's top, or the nearest one.
fn line_at(layout: &Layout<Brush>, y: f32) -> usize {
    let line = layout.lines().position(|line| y < line.metrics().block_max_coord);
    line.unwrap_or(layout.len().saturating_sub(1))
}

/// The line a byte offset is on and where along it, from the text's left edge. The affinity decides which
/// line an offset at a wrap belongs to.
pub(crate) fn spot(set: &Set, offset: usize, affinity: Affinity) -> (usize, f32) {
    let layout = &set.layout;
    let at = Cursor::from_byte_index(layout, set.inward(offset), affinity).geometry(layout, 0.0);
    let middle = ((at.y0 + at.y1) / 2.0) as f32;
    let line = layout.lines().position(|line| middle < line.metrics().block_max_coord);
    (line.unwrap_or(layout.len().saturating_sub(1)), at.x0 as f32)
}

/// The line holding a byte offset.
pub(crate) fn line_of(set: &Set, offset: usize) -> usize {
    spot(set, offset, Affinity::Downstream).0
}

/// The byte offset on a line nearest to `x` on the page.
pub(crate) fn on_line(text: &Text<'_>, shifts: &Shifts, line: usize, x: f32) -> usize {
    let Some(line) = text.set.layout.get(line) else { return 0 };
    let metrics = line.metrics();
    let y = text.at.y + (metrics.block_min_coord + metrics.block_max_coord) / 2.0;
    hit(text, shifts, Point { x, y })
}

/// Where a caret at a byte offset is drawn on the page. It spans the line's text, not its leading.
pub(crate) fn caret(text: &Text<'_>, shifts: &Shifts, offset: usize) -> Rect {
    let layout = &text.set.layout;
    let at = Cursor::from_byte_index(layout, text.set.inward(offset), Affinity::Downstream);
    let at = at.geometry(layout, theme::CARET);
    let (mut top, mut bottom) = (at.y0 as f32, at.y1 as f32);
    let line = line_of(text.set, offset);
    if let Some(line) = layout.get(line) {
        let metrics = line.metrics();
        (top, bottom) = (metrics.content_block_min_coord, metrics.content_block_max_coord);
    }
    let x = text.at.x + at.x0 as f32 + shifts.caret(line, offset);
    Rect::new(x, text.at.y + top, theme::CARET, bottom - top)
}

/// Adds the rectangles that highlight a byte range, on the page. `newline` also shows the text's end as
/// selected, for a selection that runs on past it.
pub(crate) fn selection(text: &Text<'_>, shifts: &Shifts, range: Range<usize>, newline: bool, out: &mut Vec<Rect>) {
    let (layout, origin) = (&text.set.layout, text.at);
    let first = out.len();
    stretches(text.set, &range, shifts, |_, part| {
        out.push(Rect { x: origin.x + part.x, y: origin.y + part.y, ..part });
    });
    if !newline {
        return;
    }
    let Some(line) = layout.lines().next_back() else { return };
    let metrics = line.metrics();
    let end = Cursor::from_byte_index(layout, layout.text_len(), Affinity::Downstream).geometry(layout, 0.0);
    let push = shifts.push(layout.len().saturating_sub(1), end.x0 as f32);
    let (x, y) = (origin.x + end.x0 as f32 + push, origin.y + metrics.block_min_coord);
    let width = (metrics.line_height * theme::NEWLINE).round();
    match out[first..].last_mut() {
        Some(last) if last.y == y && (last.x + last.width - x).abs() < 0.5 => last.width += width,
        _ => out.push(Rect::new(x, y, width, metrics.block_max_coord - metrics.block_min_coord)),
    }
}

/// Where the formula with the given source is drawn on the page. One that shows as its source has the place
/// of that text.
pub(crate) fn formula_at(text: &Text<'_>, shifts: &Shifts, source: &Range<usize>) -> Option<Rect> {
    let (set, origin) = (text.set, text.at);
    if let Some(swap) = set.swaps.iter().position(|swap| swap.maths.is_some() && swap.source == *source) {
        for (index, line) in set.layout.lines().enumerate() {
            for item in line.items() {
                let PositionedLayoutItem::InlineBox(inline) = item else { continue };
                if inline.id as usize == swap {
                    let x = origin.x + inline.x + shifts.push(index, inline.x);
                    return Some(Rect::new(x, origin.y + inline.y, inline.width, inline.height));
                }
            }
        }
    }
    let mut all: Option<Rect> = None;
    stretches(set, source, shifts, |_, part| {
        let (x, y) = (origin.x + part.x, origin.y + part.y);
        let (left, top) = all.map_or((x, y), |all| (all.x.min(x), all.y.min(y)));
        let right = all.map_or(x + part.width, |all| (all.x + all.width).max(x + part.width));
        let bottom = all.map_or(y + part.height, |all| (all.y + all.height).max(y + part.height));
        all = Some(Rect::new(left, top, right - left, bottom - top));
    });
    all
}
