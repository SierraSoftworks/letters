//! Turns the block tree into a flat column of leaves: where each block sits across the page, the space around
//! it and what frames it. No text is shaped here, so this stays cheap for a whole document.

use std::ops::Range;

use letters_model::{Align, Block, BlockId, BlockKind, Body, Document, ListKind, Mark, Style, TextStyle, Tone};
use letters_render::{Color, Rect};
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
pub(crate) struct Types {
    types: Vec<Type>,
    /// The word a numbered figure's label opens with.
    pub figure: String,
}

/// The word a style gives its figure labels: the caption type's `label`, or the English one.
pub(crate) fn figure_word(style: &Style) -> &str {
    let label = style.types.get("caption").and_then(|caption| caption.extra.get("label"));
    label.and_then(|label| label.as_str()).filter(|word| !word.trim().is_empty()).unwrap_or("Figure")
}

pub(crate) fn alignment(align: Align) -> Alignment {
    match align {
        Align::Start => Alignment::Start,
        Align::Center => Alignment::Center,
        Align::End => Alignment::End,
        Align::Justify => Alignment::Justify,
    }
}

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
                align: alignment(from.align),
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
        Self { types: types.collect(), figure: figure_word(style).to_owned() }
    }

    pub fn get(&self, role: Role) -> &Type {
        &self.types[role as usize]
    }

    /// Each role's colour, in the order of [`Role`].
    pub fn colors(&self) -> impl Iterator<Item = Option<Color>> {
        self.types.iter().map(|of| of.color)
    }

    /// The label of the `number`th numbered figure, as it stands before a caption.
    pub fn label(&self, number: u32) -> String {
        format!("{} {number}:", self.figure)
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
    /// A picture with its caption, which is the leaf's text, under it.
    Figure,
    /// Pictures in rows, with the caption block's text under them.
    Gallery,
    /// A row of a table. Its cells hold the text.
    Row,
}

impl What {
    /// Whether a caret can sit in or on a leaf of this kind.
    pub fn holds(self) -> bool {
        !matches!(self, Self::Rule | Self::Label | Self::Blank)
    }

    /// Whether the leaf's own text is set as lines of text.
    pub fn worded(self) -> bool {
        matches!(self, Self::Text | Self::Label | Self::Figure | Self::Gallery)
    }
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

    /// The source of the formula that holds `offset` or ends or starts there, when it is in one.
    pub fn formula(&self, offset: usize) -> Option<Range<usize>> {
        let mut start = 0;
        for span in &self.spans {
            if span.marks & marks::MATH != 0 && (start..=span.end).contains(&offset) {
                return Some(start..span.end);
            }
            start = span.end;
        }
        None
    }
}

/// A picture of a figure or gallery.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Picture {
    /// The figure block.
    pub id: BlockId,
    /// The entry of the document that holds the image. `None` while it is still a link.
    pub asset: Option<String>,
    /// Where a linked image comes from.
    pub host: String,
    /// Its stored size, which gives its shape.
    pub size: (f32, f32),
    /// Where it is drawn, from the leaf's left edge and top.
    pub rect: Rect,
    /// The row of its gallery it is in.
    pub row: usize,
}

impl Picture {
    fn of(block: &Block) -> Option<Self> {
        let BlockKind::Figure { asset, src, width, height, .. } = &block.kind else { return None };
        // The host is all of a link that the page shows.
        let host = src.as_deref().map_or("", |src| {
            let rest = src.split_once("://").map_or(src, |(_, rest)| rest);
            let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
            host.rsplit_once('@').map_or(host, |(_, host)| host)
        });
        let size = ((width * theme::PT).max(1.0), (height * theme::PT).max(1.0));
        let (id, asset, host) = (block.id.clone(), asset.clone(), host.to_owned());
        Some(Self { id, asset, host, size, rect: Rect::default(), row: 0 })
    }
}

/// One cell of a table row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Cell {
    pub id: BlockId,
    pub words: Words,
    /// The left edge and width of its text, from the row's left edge.
    pub x: f32,
    pub width: f32,
    /// The narrowest and the widest its text can be set. Unknown until it is measured.
    pub fit: Option<(f32, f32)>,
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
    pub text: String,
    /// Empty when the whole text is unmarked.
    pub spans: Vec<Span>,
    /// The links in the text, in order: the bytes each covers and where it leads.
    pub links: Vec<(Range<usize>, String)>,
    pub newlines: usize,
    /// The block the text belongs to when that is not the leaf's own: the caption of a gallery.
    pub caption: Option<BlockId>,
    /// The pictures of a figure or gallery, in order.
    pub pictures: Vec<Picture>,
    /// The number a numbered figure or gallery is given, which its caption opens with.
    pub number: Option<u32>,
    /// The cells of a table row.
    pub cells: Vec<Cell>,
    /// The table a row is part of, and whether it is its header.
    pub table: usize,
    pub header: bool,

    /// The left edge and width of the text.
    pub x: f32,
    pub width: f32,
    /// How far a boxed leaf's box reaches past the text on each side.
    pub pad: f32,
    /// How far below the leaf's top its own text starts: past the pictures of a figure or gallery.
    pub lead: f32,
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
        let wrapped = |bytes: usize, width: f32| (bytes as f32 * of.advance / width.max(1.0)).ceil().max(1.0);
        match self.what {
            What::Rule => theme::RULE,
            What::Blank => of.line,
            What::Text | What::Label | What::Figure | What::Gallery => {
                self.tall((self.newlines as f32 + wrapped(self.text.len(), self.width)) * of.line)
            }
            What::Formula => (of.line * 2.5).round(),
            What::Row => {
                let lines = self.cells.iter().map(|cell| wrapped(cell.words.text.len(), cell.width));
                lines.fold(1.0, f32::max) * of.line + 2.0 * theme::CELL_PAD_Y
            }
        }
    }

    /// Whether a figure or gallery shows a caption under its pictures.
    pub fn captioned(&self) -> bool {
        !self.text.is_empty() || self.number.is_some()
    }

    /// The leaf's height when its own text is `text` tall.
    pub fn tall(&self, text: f32) -> f32 {
        match self.what {
            What::Figure | What::Gallery if self.captioned() => self.lead + text,
            What::Figure | What::Gallery => (self.lead - theme::CAPTION_GAP).max(1.0),
            _ => text,
        }
    }

    /// Whether the leaf goes to the page of the leaf after it: a heading, or the header of a table.
    pub fn leads(&self) -> bool {
        self.role.is_heading() && self.what == What::Text || self.header
    }

    /// Whether a page may not split the leaf while it fits a page whole, as with a row of a table.
    pub fn whole(&self) -> bool {
        self.what == What::Row
    }

    /// How many lines must stay together at a page's end or start. Rows of pictures stand alone.
    pub fn together(&self) -> usize {
        if self.what == What::Gallery { 1 } else { 2 }
    }

    /// Where a line starts, from the leaf's top.
    pub fn line_top(&self, line: usize) -> f32 {
        match (line, self.ends.get(line.wrapping_sub(1))) {
            (0, _) => 0.0,
            (_, Some(end)) => *end,
            _ => line as f32 * self.height / self.lines.max(1) as f32,
        }
    }

    /// The leaf of a text block, not yet placed.
    pub fn of(block: &Block) -> Self {
        let mut leaf = Self { id: Some(block.id.clone()), kind: Some(block.kind.clone()), ..Self::default() };
        leaf.set_content(block);
        leaf
    }

    /// Copies a text block's runs.
    pub fn set_content(&mut self, block: &Block) {
        let Words { text, spans, links } = Words::of(block);
        (self.text, self.spans, self.links) = (text, spans, links);
        self.newlines = self.text.bytes().filter(|byte| *byte == b'\n').count();
    }

    /// Places a figure's picture, or a gallery's in rows, and makes room for them above the text.
    fn arrange(&mut self, types: &Types) {
        let width = self.width;
        self.ends.clear();
        match &mut self.pictures[..] {
            [] => self.lead = 0.0,
            [picture] if self.what == What::Figure => {
                let wide = picture.size.0.min(width).round().max(1.0);
                let tall = (picture.size.1 * wide / picture.size.0).round().max(1.0);
                // The picture stands where the caption's style puts its lines.
                let x = match types.get(Role::Caption).align {
                    Alignment::Center => ((width - wide) / 2.0).round(),
                    Alignment::End | Alignment::Right => width - wide,
                    _ => 0.0,
                };
                (picture.rect, picture.row) = (Rect::new(x, 0.0, wide, tall), 0);
                self.lead = tall + theme::CAPTION_GAP;
            }
            pictures => {
                let (mut at, mut top, mut row) = (0, 0.0, 0);
                while at < pictures.len() {
                    // Up to three to a row, and the last picture is never left alone.
                    let left = pictures.len() - at;
                    let take = if left == 4 { 2 } else { left.min(3) };
                    let shapes: f32 = pictures[at..at + take].iter().map(|picture| picture.size.0 / picture.size.1).sum();
                    let room = (width - theme::TILE_GAP * (take - 1) as f32).max(take as f32);
                    let tall = (room / shapes).round().max(1.0);
                    let mut x = 0.0;
                    for (index, picture) in pictures[at..at + take].iter_mut().enumerate() {
                        // The last one ends at the text edge, whatever the rounding of the others left.
                        let wide = if index + 1 == take { width - x } else { (picture.size.0 / picture.size.1 * tall).round() };
                        (picture.rect, picture.row) = (Rect::new(x, top, wide.max(1.0), tall), row);
                        x += wide + theme::TILE_GAP;
                    }
                    (at, top, row) = (at + take, top + tall + theme::TILE_GAP, row + 1);
                    // A line of the leaf is a row with the gap under it.
                    self.ends.push(top);
                }
                self.lead = top - theme::TILE_GAP + theme::CAPTION_GAP;
            }
        }
    }

    /// The lines pagination sees in a leaf whose text has `lines` lines and is `text` tall, with where each
    /// ends. `uneven` gives the ends of the text's lines when they differ in height.
    pub fn count(&mut self, lines: u32, uneven: Vec<f32>) {
        let of_text = self.height;
        match self.what {
            // A figure is one line: its picture never parts from its caption.
            What::Figure | What::Formula => (self.lines, self.ends) = (1, Vec::new()),
            What::Gallery => {
                let rows = self.pictures.last().map_or(0, |last| last.row + 1);
                self.ends.truncate(rows);
                if let Some(last) = self.ends.last_mut() {
                    *last = of_text;
                }
                self.lines = rows.max(1) as u32;
            }
            _ => (self.lines, self.ends) = (lines.max(1), uneven),
        }
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
    block.extra.get("break").and_then(|value| value.as_str()) == Some("page")
}

/// A table: columns across a run of row leaves.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Table {
    /// Its rows, as a range of leaves.
    pub rows: Range<usize>,
    /// How each column's text is aligned.
    pub aligns: Vec<Alignment>,
    /// The width of each column, padding included.
    pub widths: Vec<f32>,
    /// Whether the widths come from measured text and not from a guess.
    pub sized: bool,
}

#[derive(Default)]
pub(crate) struct Flow {
    pub leaves: Vec<Leaf>,
    pub frames: Vec<Frame>,
    pub tables: Vec<Table>,
}

/// The number of each numbered figure and gallery, in the order of the document. A figure inside a gallery
/// takes none: the gallery is numbered as one.
pub(crate) fn numbers(blocks: &[Block], next: &mut u32, out: &mut Vec<(BlockId, u32)>) {
    for block in blocks {
        match &block.kind {
            BlockKind::Figure { numbered: true, .. } | BlockKind::Gallery { numbered: true } => {
                *next += 1;
                out.push((block.id.clone(), *next));
            }
            BlockKind::Gallery { .. } | BlockKind::Figure { .. } => {}
            _ => numbers(block.body.blocks(), next, out),
        }
    }
}

/// Lays the document out as a column `width` wide. `reuse` hands back the old leaf of a block whose content
/// has not changed, so its text and layout carry over.
pub(crate) fn flow(doc: &Document, types: &Types, width: f32, reuse: impl FnMut(&Block) -> Option<Leaf>) -> Flow {
    let mut walk = Walk { types, reuse, out: Flow::default(), pad: 0.0, breaks: false, root: 0 };
    walk.place(&doc.blocks, 0.0, width, None, false);
    walk.out
}

struct Walk<'a, F> {
    types: &'a Types,
    reuse: F,
    out: Flow,
    /// Padding of frames opened since the last leaf, which the next leaf carries above it.
    pad: f32,
    /// Whether a block since the last leaf starts a page, which the next leaf then does.
    breaks: bool,
    root: usize,
}

impl<F: FnMut(&Block) -> Option<Leaf>> Walk<'_, F> {
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
                BlockKind::Quote => (unit, unit),
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
                    let (id, text) = (Some(block.id.clone()), format!("Unsupported block: {type_name}"));
                    self.push(Leaf { id, what: What::Label, role, text, ..Leaf::default() }, x, width, gap, frame);
                }
                _ => {
                    let mut leaf = match (self.reuse)(block) {
                        Some(old) if old.kind.as_ref() == Some(&block.kind) => old,
                        _ => Leaf::of(block),
                    };
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
