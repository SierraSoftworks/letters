//! Turns the block tree into a flat column of leaves: where each block sits across the page, the space around
//! it and what frames it. No text is shaped here, so this stays cheap for a whole document.

use std::collections::BTreeSet;
use std::ops::Range;

use letters_model::{Align, Block, BlockId, BlockKind, Body, Document, ListKind, Mark, Style, TextStyle, Tone};
use letters_render::Color;
use parley::{Alignment, Layout, LineHeight};

use crate::engine::{Engine, Spec};
use crate::ink::Brush;
use crate::leaf::Laid;
use crate::theme;

/// The element type whose style a leaf takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Role {
    #[default]
    Body,
    Heading1,
    Heading2,
    Heading3,
    Heading4,
    Quote,
    Code,
    Caption,
}

impl Role {
    pub fn is_heading(self) -> bool {
        self.level().is_some()
    }

    /// A heading's level, from 1.
    pub fn level(self) -> Option<u8> {
        match self {
            Self::Heading1 => Some(1),
            Self::Heading2 => Some(2),
            Self::Heading3 => Some(3),
            Self::Heading4 => Some(4),
            _ => None,
        }
    }
}

const ROLES: [(Role, &str); 8] = [
    (Role::Body, "body"),
    (Role::Heading1, "heading1"),
    (Role::Heading2, "heading2"),
    (Role::Heading3, "heading3"),
    (Role::Heading4, "heading4"),
    (Role::Quote, "quote"),
    (Role::Code, "code"),
    (Role::Caption, "caption"),
];

/// One element type's style, resolved to logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Type {
    pub family: String,
    pub weight: f32,
    pub italic: bool,
    pub size: f32,
    /// `None` when the style's colour cannot be read, which shows as the palette's text colour.
    pub color: Option<Color>,
    pub align: Alignment,
    /// The height of a line. Whole pixels, so baselines stay crisp.
    pub line: f32,
    pub before: f32,
    pub after: f32,
    /// The average advance of a byte of text, for estimating the height of blocks not yet laid out.
    pub advance: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Types(Vec<Type>);

impl Types {
    pub fn resolve(style: &Style, engine: &mut Engine) -> Self {
        let fallback = TextStyle::default();
        let mut scratch = Layout::new();
        let types = ROLES.iter().map(|(role, key)| {
            let from = style.types.get(*key).unwrap_or(&fallback);
            let size = from.size * theme::PT;
            let mut resolved = Type {
                family: from.family.clone(),
                weight: f32::from(from.weight),
                italic: from.italic,
                size,
                color: theme::color(&from.color),
                align: match from.align {
                    Align::Start => Alignment::Start,
                    Align::Center => Alignment::Center,
                    Align::End => Alignment::End,
                    Align::Justify => Alignment::Justify,
                },
                line: (size * from.line_height).round().max(1.0),
                before: (from.space_before * theme::PT).round(),
                after: (from.space_after * theme::PT).round(),
                advance: size * 0.5,
            };
            // A sentence of ordinary prose, measured once, tells how much text fits a line. The margin
            // allows for the ragged ends of wrapped lines.
            const SAMPLE: &str = "The style of a document decides how wide its words run, and so how tall it is.";
            let spec = resolved.spec(*role == Role::Code, Brush::default());
            engine.build(SAMPLE, &[spec], &[(0, 0..SAMPLE.len())], None, &mut scratch);
            scratch.break_all_lines(None);
            if scratch.full_width() > 0.0 {
                resolved.advance = scratch.full_width() / SAMPLE.len() as f32 * 1.04;
            }
            resolved
        });
        Self(types.collect())
    }

    pub fn get(&self, role: Role) -> &Type {
        &self.0[role as usize]
    }

    /// Each role's colour, in the order of [`Role`].
    pub fn colors(&self) -> impl Iterator<Item = Option<Color>> {
        self.0.iter().map(|of| of.color)
    }
}

impl Type {
    /// This type's plain text, before any marks.
    pub fn spec(&self, mono: bool, brush: Brush) -> Spec<'_> {
        Spec {
            family: &self.family,
            mono,
            size: self.size,
            weight: self.weight,
            italic: self.italic,
            line: LineHeight::Absolute(self.line),
            brush,
            underline: false,
            strike: false,
        }
    }
}

/// What a leaf shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum What {
    /// A text block of the document, which can hold the caret.
    #[default]
    Text,
    /// A divider.
    Rule,
    /// A note standing in for a block this build cannot show.
    Label,
    /// An empty line that keeps an empty quote or callout visible.
    Blank,
    /// A formula set as a block. The leaf's text is its source.
    Formula,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Marker {
    #[default]
    None,
    /// A bullet, drawn by nesting depth.
    Bullet(u8),
    Number(u32),
    Task(bool),
}

/// The marks of a stretch of text, as bits.
pub(crate) mod marks {
    pub const BOLD: u8 = 1;
    pub const ITALIC: u8 = 1 << 1;
    pub const UNDERLINE: u8 = 1 << 2;
    pub const STRIKE: u8 = 1 << 3;
    pub const HIGHLIGHT: u8 = 1 << 4;
    pub const CODE: u8 = 1 << 5;
    pub const LINK: u8 = 1 << 6;
    /// The stretch is the source of a formula, which is set in its place.
    pub const MATH: u8 = 1 << 7;
}

/// A stretch of a leaf's text set one way. Spans run in order, each ending at `end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub end: usize,
    pub marks: u8,
}

/// A text as the document holds it: the text, how each stretch of it is set, and where its links lead.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Words {
    pub text: String,
    /// Empty when the whole text is unmarked.
    pub spans: Vec<Span>,
    /// In order: the bytes each link covers and where it leads.
    pub links: Vec<(Range<usize>, String)>,
}

impl Words {
    pub fn plain(text: String) -> Self {
        Self { text, ..Self::default() }
    }

    /// Reads a text block's runs.
    pub fn of(block: &Block) -> Self {
        let mut words = Self::default();
        for run in block.body.runs() {
            let start = words.text.len();
            words.text.push_str(&run.text);
            if let Some(link) = run.link.as_ref().filter(|_| !run.text.is_empty()) {
                match words.links.last_mut() {
                    // One link whose words differ in marks is still one link.
                    Some((range, last)) if range.end == start && last == link => range.end = words.text.len(),
                    _ => words.links.push((start..words.text.len(), link.clone())),
                }
            }
            let mut bits = if run.link.is_some() { marks::LINK } else { 0 };
            for mark in &run.marks {
                bits |= match mark {
                    Mark::Bold => marks::BOLD,
                    Mark::Italic => marks::ITALIC,
                    Mark::Underline => marks::UNDERLINE,
                    Mark::Strike => marks::STRIKE,
                    Mark::Highlight => marks::HIGHLIGHT,
                    Mark::Code => marks::CODE,
                    Mark::Math => marks::MATH,
                    Mark::Other(_) => 0,
                };
            }
            match words.spans.last_mut() {
                // Two formulas side by side stay two, though they are marked alike.
                Some(last) if last.marks == bits && bits & marks::MATH == 0 => last.end = words.text.len(),
                _ if run.text.is_empty() => {}
                _ => words.spans.push(Span { end: words.text.len(), marks: bits }),
            }
        }
        if matches!(words.spans[..], [Span { marks: 0, .. }]) {
            words.spans.clear();
        }
        words
    }

    /// The formula that holds `offset`, or starts or ends there: which of the text's formulas it is, and the
    /// bytes of its source.
    pub fn formula(&self, offset: usize) -> Option<(usize, Range<usize>)> {
        let mut start = 0;
        let mut formulas = 0..;
        for span in &self.spans {
            if span.marks & marks::MATH != 0 {
                let which = formulas.next()?;
                if (start..=span.end).contains(&offset) {
                    return Some((which, start..span.end));
                }
            }
            start = span.end;
        }
        None
    }
}

/// One row of the column: a text block, or something that stands where a block is.
#[derive(Default)]
pub(crate) struct Leaf {
    pub id: Option<BlockId>,
    /// The block's kind when last seen, to tell a retyped block from an edited one.
    pub kind: Option<BlockKind>,
    /// The index of the top-level block this leaf sits under, to find its block again without a search.
    pub root: usize,
    pub what: What,
    pub role: Role,
    pub marker: Marker,
    /// The leaf keeps its own copy of the text, so it can be laid out whenever it scrolls into view.
    pub words: Words,
    pub newlines: usize,

    /// The left edge and width of the text.
    pub x: f32,
    pub width: f32,
    /// How far a boxed leaf's box reaches past the text on each side.
    pub pad: f32,
    /// The space between the row above and this leaf's text, padding included.
    pub top: f32,
    /// Padding below the text that belongs to this leaf or to frames closing after it.
    pub bottom: f32,
    /// The innermost frame around this leaf.
    pub frame: Option<usize>,

    /// Whether a page of print starts with this leaf, come what may.
    pub breaks: bool,

    pub y: f32,
    pub height: f32,
    /// Whether `height` comes from a real layout and not an estimate.
    pub exact: bool,
    /// How many lines the leaf has. Known once `exact`.
    pub lines: u32,
    /// Where each line ends, from the leaf's top, when they are not all as tall as each other.
    pub ends: Vec<f32>,
    /// The lines that carry on a line of code from the line before, in order. No page starts with one.
    pub joined: Vec<u32>,
    pub laid: Option<Box<Laid>>,
    /// When the layout was last used, for dropping the stalest.
    pub used: u64,
}

impl Leaf {
    /// The leaf's whole vertical extent, with the space above and the padding below.
    pub fn span(&self) -> (f32, f32) {
        (self.y - self.top, self.y + self.height + self.bottom)
    }

    /// Whether the leaf's text has its own box behind it.
    pub fn boxed(&self) -> bool {
        self.role == Role::Code || self.what == What::Label
    }

    /// A guess at the height from the amount of text, used until the leaf is laid out.
    pub fn estimate(&self, types: &Types) -> f32 {
        let of = types.get(self.role);
        match self.what {
            What::Rule => theme::RULE,
            What::Blank => of.line,
            What::Text | What::Label => {
                let wrapped = (self.words.text.len() as f32 * of.advance / self.width.max(1.0)).ceil().max(1.0);
                (self.newlines as f32 + wrapped) * of.line
            }
            // A fraction or a sum is about this tall.
            What::Formula => (of.line * 2.5).round(),
        }
    }

    /// Whether a position can be in the leaf.
    pub fn holds(&self) -> bool {
        matches!(self.what, What::Text | What::Formula)
    }

    /// The formula at `offset` of the leaf's text: which of the leaf's formulas it is, and the bytes of its
    /// source. A formula block is all one, and code has none.
    pub fn formula(&self, offset: usize) -> Option<(usize, Range<usize>)> {
        match self.what {
            What::Formula => Some((0, 0..self.words.text.len())),
            _ if self.role == Role::Code => None,
            _ => self.words.formula(offset),
        }
    }

    /// Where a line starts, from the leaf's top.
    pub fn line_top(&self, line: usize) -> f32 {
        match (line, self.ends.get(line.wrapping_sub(1))) {
            (0, _) => 0.0,
            (_, Some(end)) => *end,
            _ => line as f32 * self.height / self.lines.max(1) as f32,
        }
    }

    /// How many lines end within `room` of the leaf's top.
    pub fn lines_in(&self, room: f32) -> usize {
        let count = self.lines.max(1) as usize;
        match self.ends.is_empty() {
            true => (((room * count as f32) / self.height.max(1.0)).floor().max(0.0) as usize).min(count),
            false => self.ends.partition_point(|end| *end <= room).min(count),
        }
    }

    /// Copies a text block's runs, and hands back the text there was.
    pub fn set_content(&mut self, block: &Block) -> String {
        let before = std::mem::replace(&mut self.words, Words::of(block));
        self.newlines = self.words.text.bytes().filter(|byte| *byte == b'\n').count();
        before.text
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Look {
    Quote,
    Callout(Tone),
}

/// A quote or callout: decoration around a run of leaves.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Frame {
    /// The quote or callout block.
    pub id: BlockId,
    pub look: Look,
    pub parent: Option<usize>,
    pub first: usize,
    pub last: usize,
    pub x: f32,
    pub width: f32,
    /// How far the frame's top edge sits below the top of its first leaf's span.
    pub lead: f32,
    /// How far the frame reaches below its last leaf's text.
    pub reach: f32,
}

impl Frame {
    /// The frame's top and bottom edges, from the leaves it encloses.
    pub fn span(&self, leaves: &[Leaf]) -> (f32, f32) {
        let last = &leaves[self.last];
        (leaves[self.first].span().0 + self.lead, last.y + last.height + self.reach)
    }
}

/// Whether a block is marked to start a page of print.
pub(crate) fn breaks(block: &Block) -> bool {
    block.page_break
}

#[derive(Default)]
pub(crate) struct Flow {
    pub leaves: Vec<Leaf>,
    pub frames: Vec<Frame>,
    /// The blocks whose text was read anew into a leaf that was kept, each with the text it had.
    pub retexted: Vec<(BlockId, String)>,
}

/// Lays the document out as a column `width` wide.
///
/// With `carry`, the leaves of the last column are kept: `old` hands over the leaf a block had, and `carry`
/// names the blocks whose text has changed since, which are read again. A kept leaf brings its height and,
/// when nothing in it changed, its layout.
pub(crate) fn flow(
    doc: &Document,
    types: &Types,
    width: f32,
    carry: Option<&BTreeSet<BlockId>>,
    old: impl FnMut(&BlockId) -> Option<Leaf>,
) -> Flow {
    let mut walk = Walk { types, carry, old, out: Flow::default(), pad: 0.0, breaks: false, root: 0 };
    walk.place(&doc.blocks, 0.0, width, None, false);
    walk.out
}

struct Walk<'a, F> {
    types: &'a Types,
    carry: Option<&'a BTreeSet<BlockId>>,
    old: F,
    out: Flow,
    /// Padding of frames opened since the last leaf, which the next leaf carries above it.
    pad: f32,
    /// Whether a block since the last leaf starts a page, which the next leaf then does.
    breaks: bool,
    root: usize,
}

impl<F: FnMut(&BlockId) -> Option<Leaf>> Walk<'_, F> {
    /// The leaf `block` had, when it is still the kind of block it was, and otherwise a new one that shows
    /// `what`. Also says whether the leaf is the old one.
    fn leaf(&mut self, block: &Block, what: What) -> (Leaf, bool) {
        let old = self.carry.and_then(|_| (self.old)(&block.id));
        match old.filter(|old| old.kind.as_ref() == Some(&block.kind) && old.what == what) {
            Some(old) => (old, true),
            None => {
                let (id, kind) = (Some(block.id.clone()), Some(block.kind.clone()));
                (Leaf { id, kind, what, ..Leaf::default() }, false)
            }
        }
    }

    /// Whether a text read from `block` when the column was last built may no longer be its text.
    fn changed(&self, block: &BlockId) -> bool {
        self.carry.is_none_or(|changed| changed.contains(block))
    }

    fn place(&mut self, blocks: &[Block], x: f32, width: f32, frame: Option<usize>, quoted: bool) {
        let unit = self.types.get(Role::Body).after;
        let prose = if quoted { Role::Quote } else { Role::Body };
        // The space after the block above, and whether that block was a list item.
        let mut above: Option<(f32, bool)> = None;
        let mut counters: Vec<u32> = Vec::new();
        for (index, block) in blocks.iter().enumerate() {
            if frame.is_none() {
                self.root = index;
            }
            self.breaks |= breaks(block);
            let role = match &block.kind {
                BlockKind::Heading { level: 1 } => Role::Heading1,
                BlockKind::Heading { level: 2 } => Role::Heading2,
                BlockKind::Heading { level: 3 } => Role::Heading3,
                BlockKind::Heading { .. } => Role::Heading4,
                BlockKind::Code { .. } => Role::Code,
                BlockKind::Unknown { .. } => Role::Caption,
                _ => prose,
            };
            let of = self.types.get(role);
            let (before, after) = match &block.kind {
                BlockKind::Quote | BlockKind::Formula => (unit, unit),
                BlockKind::Callout { .. } => ((unit * 1.5).round(), (unit * 1.5).round()),
                BlockKind::Divider => (unit * 2.0, unit * 2.0),
                BlockKind::Code { .. } | BlockKind::Unknown { .. } => (of.before.max(of.after), of.after),
                _ => (of.before, of.after),
            };
            let item = matches!(block.kind, BlockKind::ListItem { .. });
            let gap = match above {
                None => 0.0,
                // Items of one list sit closer than paragraphs do.
                Some((_, true)) if item => (unit * 0.25).round(),
                Some((space, _)) => space.max(before),
            };
            above = Some((after, item));

            let mut marker = Marker::None;
            let mut inset = 0.0;
            if let BlockKind::ListItem { list, depth } = &block.kind {
                let level = usize::from(*depth);
                counters.resize(level + 1, 0);
                marker = match list {
                    ListKind::Bullet => Marker::Bullet(*depth),
                    ListKind::Task { checked } => Marker::Task(*checked),
                    ListKind::Ordered => {
                        counters[level] += 1;
                        Marker::Number(counters[level])
                    }
                };
                if *list != ListKind::Ordered {
                    counters[level] = 0;
                }
                inset = (theme::LIST_INDENT * (level + 1) as f32).min(width - theme::MIN_WIDTH).max(0.0);
            } else {
                counters.clear();
            }

            match &block.kind {
                BlockKind::Quote | BlockKind::Callout { .. } => {
                    let (look, left, right, pad) = match &block.kind {
                        BlockKind::Callout { tone } => {
                            (Look::Callout(*tone), theme::BOX_PAD_X, theme::BOX_PAD_X, theme::BOX_PAD_Y)
                        }
                        _ => (Look::Quote, theme::QUOTE_INSET, 0.0, 0.0),
                    };
                    let shrink = (left + right).min(width - theme::MIN_WIDTH).max(0.0);
                    let left = left.min(shrink);
                    let this = self.out.frames.len();
                    let first = self.out.leaves.len();
                    let lead = self.pad + gap;
                    let id = block.id.clone();
                    let framed = Frame { id, look, parent: frame, first, last: first, x, width, lead, reach: 0.0 };
                    self.out.frames.push(framed);
                    self.pad = lead + pad;
                    let quoted = quoted || look == Look::Quote;
                    self.place(block.body.blocks(), x + left, width - shrink, Some(this), quoted);
                    if self.out.leaves.len() == first {
                        let blank = Leaf { what: What::Blank, role: prose, ..Leaf::default() };
                        self.push(blank, x + left, width - shrink, 0.0, Some(this));
                    }
                    let last = self.out.leaves.len() - 1;
                    self.out.leaves[last].bottom += pad;
                    (self.out.frames[this].last, self.out.frames[this].reach) = (last, self.out.leaves[last].bottom);
                }
                BlockKind::Divider => {
                    let id = Some(block.id.clone());
                    self.push(Leaf { id, what: What::Rule, ..Leaf::default() }, x, width, gap, frame);
                }
                BlockKind::Unknown { type_name } => {
                    let words = Words::plain(format!("Unsupported block: {type_name}"));
                    let leaf = Leaf { id: Some(block.id.clone()), what: What::Label, role, words, ..Leaf::default() };
                    self.push(leaf, x, width, gap, frame);
                }
                _ => {
                    let what = if block.kind == BlockKind::Formula { What::Formula } else { What::Text };
                    let (mut leaf, kept) = self.leaf(block, what);
                    if !kept || self.changed(&block.id) {
                        let before = leaf.set_content(block);
                        if kept {
                            self.out.retexted.push((block.id.clone(), before));
                        }
                        (leaf.exact, leaf.laid) = (false, None);
                    }
                    if leaf.role != role {
                        leaf.exact = false;
                    }
                    if leaf.role != role || leaf.marker != marker {
                        leaf.laid = None;
                    }
                    (leaf.role, leaf.marker) = (role, marker);
                    self.push(leaf, x + inset, width - inset, gap, frame);
                }
            }
        }
    }

    /// Adds a leaf to the column, `gap` below the row above and within the given edges.
    fn push(&mut self, mut leaf: Leaf, x: f32, width: f32, gap: f32, frame: Option<usize>) {
        // The text of a boxed leaf sits inside the box's padding.
        let (pad_x, pad_y) = match leaf.boxed() {
            true => (theme::BOX_PAD_X.min((width - theme::MIN_WIDTH).max(0.0) / 2.0), theme::BOX_PAD_Y),
            false => (0.0, 0.0),
        };
        let (x, width) = (x + pad_x, (width - 2.0 * pad_x).max(1.0));
        if (leaf.x, leaf.width) != (x, width) {
            (leaf.exact, leaf.laid) = (false, None);
        }
        (leaf.x, leaf.width, leaf.pad) = (x, width, pad_x);
        leaf.root = self.root;
        leaf.top = gap + pad_y + std::mem::take(&mut self.pad);
        leaf.bottom = pad_y;
        leaf.frame = frame;
        leaf.breaks = std::mem::take(&mut self.breaks);
        if matches!(leaf.what, What::Rule | What::Blank) {
            (leaf.height, leaf.exact, leaf.lines) = (leaf.estimate(self.types), true, 1);
        } else if !leaf.exact {
            leaf.height = leaf.estimate(self.types);
        }
        self.out.leaves.push(leaf);
    }
}

/// Finds a block under one top-level block.
pub(crate) fn find<'a>(block: &'a Block, id: &BlockId) -> Option<&'a Block> {
    if &block.id == id {
        return Some(block);
    }
    match &block.body {
        Body::Blocks(children) => children.iter().find_map(|child| find(child, id)),
        _ => None,
    }
}
