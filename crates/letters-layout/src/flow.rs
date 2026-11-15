//! Turns the block tree into a flat column of leaves: where each block sits across the page, the space around
//! it and what frames it. No text is shaped here, so this stays cheap for a whole document.

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::sync::Arc;

use letters_model::{Align, Block, BlockId, BlockKind, Body, Document, ListKind, Mark, Run, Style, TextStyle, Tone};
use letters_render::{Color, Point, Rect};
use parley::{Alignment, Language, Layout, LineHeight};

use crate::callout::Head;
use crate::engine::{self, Engine, Spec};
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
    /// How tall a picture may be drawn, so that print shows it whole with a few lines of caption: what is
    /// left of the height of a page's text area.
    pub tallest: f32,
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
        let types: Vec<Type> = types.collect();
        let caption = types[Role::Caption as usize].line;
        let room = (style.page.height - style.page.margin_top - style.page.margin_bottom) * theme::PT;
        let tallest = (room - theme::CAPTION_GAP - theme::CAPTION_LINES * caption).max(caption);
        Self { types, figure: figure_word(style).to_owned(), tallest }
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
        label(&self.figure, number)
    }

    /// What a reference to a figure reads as, from the figure's number. One to nothing has none.
    pub fn reference(&self, number: Option<u32>) -> String {
        match number {
            Some(number) => format!("{} {number}", self.figure),
            None => format!("{} ?", self.figure),
        }
    }
}

/// The label of a numbered figure, from the word it opens with.
pub(crate) fn label(word: &str, number: u32) -> String {
    format!("{word} {number}:")
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
            locale: None,
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
    /// A gallery with nothing in it or a table with no rows: an empty box, so that it can be seen and the
    /// caret has somewhere to stand.
    Empty,
}

/// One of the things in a leaf that a position can be in or on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Part {
    /// The block the leaf stands for, when positions belong to the blocks inside it.
    Whole,
    /// A picture of a gallery.
    Picture(usize),
    /// The leaf's own text: a text block's, a formula's source or a caption.
    Text,
    /// The text of a cell of a table row.
    Cell(usize),
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
    /// Which of its text's languages it is written in, counted from 1, or 0 for the language of the block.
    pub lang: u8,
}

/// A text as the document holds it: the text, how each stretch of it is set, and where its links lead.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Words {
    pub text: String,
    /// Empty when the whole text is unmarked and in the language of its block.
    pub spans: Vec<Span>,
    /// In order: the bytes each link covers and where it leads.
    pub links: Vec<(Range<usize>, String)>,
    /// In order: the bytes of each reference to a figure, and the figure. The page shows the figure's label
    /// in their place.
    pub refs: Vec<(Range<usize>, BlockId)>,
    /// The language the block names for itself. Without one it takes that of the blocks around it.
    pub lang: Option<Language>,
    /// The languages that stretches of the text name, each once.
    pub langs: Vec<Language>,
    /// Whether the text reads from the right, which is then the side it starts at.
    pub rtl: bool,
}

/// The language a tag names, if it is one.
pub(crate) fn language(tag: Option<&str>) -> Option<Language> {
    Language::parse(tag?).ok()
}

/// Whether a block's text reads from the right: its first letter does. The source of a formula is passed
/// over, since the formula stands in its place, and code reads from the left whatever is written in it.
pub(crate) fn rtl(block: &Block) -> bool {
    if matches!(block.kind, BlockKind::Code { .. } | BlockKind::Formula) {
        return false;
    }
    let shown =
        |run: &&Run| !run.marks.contains(&Mark::Math) || run.marks.iter().any(|mark| matches!(mark, Mark::Ref(_)));
    let mut letters = block.body.runs().iter().filter(shown).flat_map(|run| run.text.chars());
    letters.find_map(engine::reads).unwrap_or(false)
}

/// Whether what a quote holds reads from the right: its first block says, a text by its letters and a quote
/// or callout by what it holds in turn.
fn leads(blocks: &[Block]) -> bool {
    blocks.first().is_some_and(|first| match &first.body {
        Body::Blocks(inside) if matches!(first.kind, BlockKind::Quote | BlockKind::Callout { .. }) => leads(inside),
        Body::Text(_) => rtl(first),
        _ => false,
    })
}

impl Words {
    pub fn plain(text: String) -> Self {
        Self { text, ..Self::default() }
    }

    /// Reads a text block's runs.
    pub fn of(block: &Block) -> Self {
        let mut words = Self { lang: language(block.lang.as_deref()), rtl: rtl(block), ..Self::default() };
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
                    Mark::Ref(_) | Mark::Other(_) => 0,
                };
            }
            let target = run.marks.iter().find_map(|mark| match mark {
                Mark::Ref(target) if !run.text.is_empty() => Some(target),
                _ => None,
            });
            if let Some(target) = target {
                // A reference is not also a formula: its text is only what a reader shows that cannot follow it.
                bits &= !marks::MATH;
                words.refs.push((start..words.text.len(), target.clone()));
            }
            // A reference stays a span of its own, and so do two formulas side by side, though marked alike.
            let alone = target.is_some() || words.refs.last().is_some_and(|(before, _)| before.end == start);
            let lang = language(run.lang.as_deref()).filter(|_| !run.text.is_empty()).map_or(0, |lang| {
                let known = words.langs.iter().position(|known| *known == lang);
                let at = known.unwrap_or_else(|| {
                    words.langs.push(lang);
                    words.langs.len() - 1
                });
                // Past the 255th language of one text, a stretch is read in the block's.
                u8::try_from(at + 1).unwrap_or(0)
            });
            let like = |last: &Span| (last.marks, last.lang) == (bits, lang) && bits & marks::MATH == 0 && !alone;
            match words.spans.last_mut() {
                Some(last) if like(last) => last.end = words.text.len(),
                _ if run.text.is_empty() => {}
                _ => words.spans.push(Span { end: words.text.len(), marks: bits, lang }),
            }
        }
        if matches!(words.spans[..], [Span { marks: 0, lang: 0, .. }]) && words.refs.is_empty() {
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

/// A picture of a figure or gallery.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Picture {
    /// The figure block.
    pub id: BlockId,
    /// The entry of the document that holds the image. `None` while it is still a link.
    pub asset: Option<String>,
    /// What it shows, for a reader that cannot see it.
    pub alt: Arc<str>,
    /// The link an image that is not in the document comes from, as the document writes it.
    pub src: Option<String>,
    /// The host that link names.
    pub host: String,
    /// Its stored size, which gives its shape.
    pub size: (f32, f32),
    /// Where it is drawn, from the leaf's left edge and top.
    pub rect: Rect,
    /// The row of its gallery it is in.
    pub row: usize,
}

impl Picture {
    /// Whether `other` shows the same image at the same shape.
    fn same(&self, other: &Self) -> bool {
        (&self.id, &self.asset, &self.src, self.size) == (&other.id, &other.asset, &other.src, other.size)
    }

    fn of(block: &Block) -> Option<Self> {
        let BlockKind::Figure { asset, src, alt, width, height, .. } = &block.kind else { return None };
        // The host is all of a link that the page shows.
        let host = src.as_deref().map_or("", |src| {
            let rest = src.split_once("://").map_or(src, |(_, rest)| rest);
            let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
            host.rsplit_once('@').map_or(host, |(_, host)| host)
        });
        let size = ((width * theme::PT).max(1.0), (height * theme::PT).max(1.0));
        let (id, asset, alt, host) = (block.id.clone(), asset.clone(), alt.as_str().into(), host.to_owned());
        Some(Self { id, asset, alt, src: src.clone(), host, size, rect: Rect::default(), row: 0 })
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
    /// How its column sets its text.
    pub align: Alignment,
    /// The narrowest and the widest its text can be set. Unknown until it is measured.
    pub fit: Option<(f32, f32)>,
}

impl Cell {
    fn of(block: &Block) -> Self {
        let (id, words) = (block.id.clone(), Words::of(block));
        Self { id, words, x: 0.0, width: 1.0, align: Alignment::Start, fit: None }
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
    /// The block the text belongs to when that is not the leaf's own: the caption of a gallery.
    pub caption: Option<BlockId>,
    /// The pictures of a figure or gallery, in order.
    pub pictures: Vec<Picture>,
    /// The number a numbered figure or gallery is given, which its caption opens with.
    pub number: Option<u32>,
    /// Whether a figure or gallery is ready for its caption to be written: the caption keeps its line while
    /// it is empty, and is set in from the end of that line.
    pub open: bool,
    /// How far in from the left and from the right a figure or gallery sets its caption. Nothing at rest.
    pub inset: (f32, f32),
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
    /// Whether its text was last set short of a face it asked for, or of a word list. It is set again when
    /// more fonts or a word list arrive.
    pub short: bool,
    /// The language its texts take from the blocks around them, where one names any.
    pub lang: Option<Language>,
}

impl Leaf {
    /// The leaf's whole vertical extent, with the space above and the padding below.
    pub fn span(&self) -> (f32, f32) {
        (self.y - self.top, self.y + self.height + self.bottom)
    }

    /// Whether the leaf's text has its own box behind it.
    pub fn boxed(&self) -> bool {
        self.role == Role::Code || matches!(self.what, What::Label | What::Empty)
    }

    /// Whether the leaf has no text to lay out, so that its height is known as soon as it is placed.
    pub fn fixed(&self) -> bool {
        matches!(self.what, What::Rule | What::Blank | What::Empty)
    }

    /// A guess at the height from the amount of text, used until the leaf is laid out.
    pub fn estimate(&self, types: &Types) -> f32 {
        let of = types.get(self.role);
        let wrapped = |bytes: usize, width: f32| (bytes as f32 * of.advance / width.max(1.0)).ceil().max(1.0);
        match self.what {
            What::Rule => theme::RULE,
            What::Blank | What::Empty => of.line,
            What::Text | What::Label | What::Figure | What::Gallery => {
                let width = self.width - self.inset.0 - self.inset.1;
                self.tall((self.newlines as f32 + wrapped(self.words.text.len(), width)) * of.line)
            }
            What::Formula => (of.line * 2.5).round(),
            What::Row => {
                let lines = self.cells.iter().map(|cell| wrapped(cell.words.text.len(), cell.width));
                lines.fold(1.0, f32::max) * of.line + 2.0 * theme::CELL_PAD_Y
            }
        }
    }

    /// Whether a figure or gallery has a line for a caption under its pictures.
    pub fn captioned(&self) -> bool {
        !self.words.text.is_empty() || self.number.is_some() || self.open
    }

    /// Makes a figure or gallery ready for its caption to be written, or sets it at rest. Returns whether
    /// that is a change to it.
    ///
    /// A caption being written leaves room at the right of its lines, where a window stands something, or
    /// less where the leaf is too narrow. A centred one leaves as much at the left, to stay in the middle.
    /// One set from the start then stays where it is unless its lines reached that far.
    pub fn ready(&mut self, open: bool, types: &Types) -> bool {
        if !matches!(self.what, What::Figure | What::Gallery) || self.open == open {
            return false;
        }
        let room = if open { theme::CAPTION_INSET.min((self.width - theme::MIN_WIDTH).max(0.0) / 2.0) } else { 0.0 };
        let centred = types.get(self.role).align == Alignment::Center;
        (self.open, self.inset) = (open, (if centred { room } else { 0.0 }, room));
        true
    }

    /// Whether the leaf sets a text of its own, and not only those of its cells.
    pub fn worded(&self) -> bool {
        match self.what {
            What::Text | What::Label | What::Formula => true,
            What::Figure | What::Gallery => self.captioned(),
            What::Rule | What::Blank | What::Row | What::Empty => false,
        }
    }

    /// Whether a position can be in or on the leaf.
    pub fn holds(&self) -> bool {
        self.parts().next().is_some()
    }

    /// Everything in the leaf that a position can be in or on, in reading order, each with its block.
    pub fn parts(&self) -> impl Iterator<Item = (Part, &BlockId)> {
        let own = match self.what {
            What::Text | What::Formula | What::Figure => self.id.as_ref(),
            What::Gallery => self.caption.as_ref(),
            _ => None,
        };
        let pictures = self.pictures.iter().enumerate().filter(|_| self.what == What::Gallery);
        let pictures = pictures.map(|(index, picture)| (Part::Picture(index), &picture.id));
        let cells = self.cells.iter().enumerate().map(|(index, cell)| (Part::Cell(index), &cell.id));
        // An empty box is one place, which is its block's.
        let empty = self.id.as_ref().filter(|_| self.what == What::Empty).map(|id| (Part::Whole, id));
        empty.into_iter().chain(pictures).chain(own.map(|id| (Part::Text, id))).chain(cells)
    }

    /// The block a part's positions belong to.
    pub fn owner(&self, part: Part) -> Option<&BlockId> {
        self.parts().find(|(known, _)| *known == part).map(|(_, id)| id)
    }

    /// The text of a part that holds one.
    pub fn text(&self, part: Part) -> Option<&Words> {
        match part {
            Part::Text => Some(&self.words),
            Part::Cell(index) => self.cells.get(index).map(|cell| &cell.words),
            Part::Whole | Part::Picture(_) => None,
        }
    }

    /// How much text the leaf holds, as a measure of what laying it out costs.
    pub fn bytes(&self) -> usize {
        self.words.text.len() + self.cells.iter().map(|cell| cell.words.text.len()).sum::<usize>()
    }

    /// The line of the leaf, as pagination counts them, that holds line `line` of a part's text.
    pub fn line_of(&self, part: Part, line: usize) -> usize {
        let last = self.lines.max(1) as usize - 1;
        match (self.what, part) {
            (What::Gallery, Part::Picture(index)) => self.pictures.get(index).map_or(0, |picture| picture.row),
            (What::Gallery, _) => last,
            (What::Figure, _) => 0,
            _ => line.min(last),
        }
    }

    /// The formula at `offset` of a part's text: which of the text's formulas it is, and the bytes of its
    /// source. A formula block is all one, and code has none.
    pub fn formula(&self, part: Part, offset: usize) -> Option<(usize, Range<usize>)> {
        match self.what {
            What::Formula => Some((0, 0..self.words.text.len())),
            _ if self.role == Role::Code => None,
            _ => self.text(part)?.formula(offset),
        }
    }

    /// Whether the leaf shows the label of a figure for a reference to it.
    pub fn refers(&self) -> bool {
        !self.words.refs.is_empty() || self.cells.iter().any(|cell| !cell.words.refs.is_empty())
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

    /// Copies a text block's runs into the part that shows them, and hands back the text there was.
    pub fn reword(&mut self, part: Part, block: &Block) -> String {
        if let Part::Cell(index) = part
            && let Some(cell) = self.cells.get_mut(index)
        {
            cell.fit = None;
            return std::mem::replace(&mut cell.words, Words::of(block)).text;
        }
        self.set_content(block)
    }

    /// The cell of a table row under `x` on the page, or the nearest.
    pub fn cell_at(&self, x: f32) -> Option<usize> {
        let right = |cell: &Cell| self.x + cell.x + cell.width + theme::CELL_PAD_X;
        self.cells.iter().position(|cell| x < right(cell)).or(self.cells.len().checked_sub(1))
    }

    /// The picture of a gallery's row `row` under `x` on the page, or the nearest in that row.
    pub fn picture_in(&self, row: usize, x: f32) -> Option<usize> {
        let mut inside = self.pictures.iter().enumerate().filter(|(_, picture)| picture.row == row);
        let right = |picture: &Picture| self.x + picture.rect.x + picture.rect.width + theme::TILE_GAP / 2.0;
        let last = inside.clone().next_back();
        inside.find(|(_, picture)| x < right(picture)).or(last).map(|(index, _)| index)
    }

    /// The part of the leaf at a point on the page, or the nearest that a position can be in or on.
    pub fn part_at(&self, point: Point) -> Option<Part> {
        match self.what {
            What::Text | What::Formula | What::Figure => Some(Part::Text),
            What::Row => self.cell_at(point.x).map(Part::Cell),
            What::Gallery => {
                let under = point.y >= self.y + self.lead - theme::CAPTION_GAP / 2.0;
                if self.pictures.is_empty() || (under && self.caption.is_some()) {
                    return self.caption.as_ref().map(|_| Part::Text);
                }
                let bottom = |picture: &Picture| self.y + picture.rect.y + picture.rect.height + theme::TILE_GAP / 2.0;
                let row = self.pictures.iter().find(|picture| point.y < bottom(picture)).or(self.pictures.last())?.row;
                self.picture_in(row, point.x).map(Part::Picture)
            }
            What::Empty => Some(Part::Whole),
            What::Rule | What::Label | What::Blank => None,
        }
    }

    /// Places a figure's picture, or a gallery's in rows, and makes room for them above the text.
    fn arrange(&mut self, types: &Types) {
        let (width, tallest) = (self.width, types.tallest);
        match &mut self.pictures[..] {
            [] => self.lead = 0.0,
            [picture] if self.what == What::Figure => {
                // At its own size and shape, but no wider than the column and no taller than a page holds.
                let scale = (width / picture.size.0).min(tallest / picture.size.1).min(1.0);
                let wide = (picture.size.0 * scale).round().max(1.0);
                let tall = (picture.size.1 * scale).round().max(1.0);
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
                    let tiles = &mut pictures[at..at + take];
                    let shapes: f32 = tiles.iter().map(|picture| picture.size.0 / picture.size.1).sum();
                    let room = (width - theme::TILE_GAP * (take - 1) as f32).max(take as f32);
                    let full = (room / shapes).round().max(1.0);
                    let tall = full.min(tallest);
                    let scaled = |picture: &Picture| (picture.size.0 / picture.size.1 * tall).round().max(1.0);
                    // A row that a page holds no taller is narrower than the column, and stands in its middle.
                    let spare = if tall < full { room - tiles.iter().map(scaled).sum::<f32>() } else { 0.0 };
                    let mut x = (spare / 2.0).round().max(0.0);
                    for (index, picture) in tiles.iter_mut().enumerate() {
                        // The last one ends at the text edge, whatever the rounding of the others left.
                        let fills = index + 1 == take && tall == full;
                        let wide = if fills { width - x } else { scaled(picture) }.max(1.0);
                        (picture.rect, picture.row) = (Rect::new(x, top, wide, tall), row);
                        x += wide + theme::TILE_GAP;
                    }
                    (at, top, row) = (at + take, top + tall + theme::TILE_GAP, row + 1);
                }
                self.lead = top - theme::TILE_GAP + theme::CAPTION_GAP;
            }
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
    /// A callout's icon, title and colour.
    pub head: Head,
    pub parent: Option<usize>,
    pub first: usize,
    pub last: usize,
    pub x: f32,
    pub width: f32,
    /// How far the frame's top edge sits below the top of its first leaf's span.
    pub lead: f32,
    /// How far the frame reaches below its last leaf's text.
    pub reach: f32,
    /// Whether a quote reads from the right, which is then the side its bar stands at.
    pub rtl: bool,
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

/// A table: columns across a run of row leaves.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Table {
    /// Its rows, as a range of leaves.
    pub rows: Range<usize>,
    /// Whether every cell's text has been measured, so that the columns are as wide as they will stay.
    pub sized: bool,
}

impl Table {
    /// Shares the table's width among its columns by what their cells hold, and places the cells. A cell not
    /// yet measured counts by the amount of its text. A row whose cells move is to be laid out again.
    ///
    /// Every column gets at least its longest word while there is room for that, and the rest goes to the
    /// columns that would otherwise wrap, as a browser shares it.
    pub fn size(&mut self, leaves: &mut [Leaf], types: &Types) {
        let rows = &mut leaves[self.rows.clone()];
        let columns = rows.iter().map(|row| row.cells.len()).max().unwrap_or(0);
        let Some(width) = rows.first().map(|row| row.width).filter(|_| columns > 0) else { return };
        let advance = types.get(rows[0].role).advance;
        let pad = 2.0 * theme::CELL_PAD_X;
        let (mut least, mut most) = (vec![pad; columns], vec![pad; columns]);
        self.sized = true;
        for row in rows.iter() {
            // A header is set bold, which runs a little wider.
            let advance = if row.header { advance * 1.08 } else { advance };
            for (column, cell) in row.cells.iter().enumerate() {
                self.sized &= cell.fit.is_some();
                let (narrow, wide) = cell.fit.unwrap_or_else(|| {
                    let word = cell.words.text.split_whitespace().map(str::len).max().unwrap_or(0);
                    (word as f32 * advance, cell.words.text.len() as f32 * advance)
                });
                least[column] = least[column].max(narrow.ceil() + pad);
                most[column] = most[column].max(wide.ceil() + pad);
            }
        }
        let (narrowest, widest): (f32, f32) = (least.iter().sum(), most.iter().sum());
        let share = |column: usize| match () {
            () if widest <= width => most[column] * width / widest,
            () if narrowest <= width => {
                let spare = (width - narrowest) / (widest - narrowest).max(1.0);
                least[column] + (most[column] - least[column]) * spare
            }
            () => least[column] * width / narrowest,
        };
        // Whole pixels, with the last column ending at the text edge.
        let mut edges = vec![0.0; columns + 1];
        let mut at = 0.0;
        for column in 0..columns {
            at += share(column);
            edges[column + 1] = if column + 1 == columns { width } else { at.round().min(width) };
        }
        for row in rows.iter_mut() {
            let mut moved = false;
            for (column, cell) in row.cells.iter_mut().enumerate() {
                let x = edges[column] + theme::CELL_PAD_X;
                let wide = (edges[column + 1] - edges[column] - pad).max(1.0);
                moved |= (cell.x, cell.width) != (x, wide);
                (cell.x, cell.width) = (x, wide);
            }
            if moved {
                (row.exact, row.laid) = (false, None);
            }
            if !row.exact {
                row.height = row.estimate(types);
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct Flow {
    pub leaves: Vec<Leaf>,
    pub frames: Vec<Frame>,
    pub tables: Vec<Table>,
    /// The blocks whose text was read anew into a leaf that was kept, each with the text it had.
    pub retexted: Vec<(BlockId, String)>,
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

/// Lays the document out as a column `width` wide. `numbers` are those of its numbered figures.
///
/// With `carry`, the leaves of the last column are kept: `old` hands over the leaf a block had, and `carry`
/// names the blocks whose text has changed since, which are read again. A kept leaf brings its height and,
/// when nothing in it changed, its layout.
pub(crate) fn flow(
    doc: &Document,
    types: &Types,
    numbers: &HashMap<BlockId, u32>,
    width: f32,
    carry: Option<&BTreeSet<BlockId>>,
    old: impl FnMut(&BlockId) -> Option<Leaf>,
) -> Flow {
    let out = Flow::default();
    let mut walk = Walk { types, numbers, carry, old, out, pad: 0.0, breaks: false, root: 0, depth: 0, lang: None };
    walk.place(&doc.blocks, 0.0, width, None, false);
    walk.out
}

struct Walk<'a, F> {
    types: &'a Types,
    numbers: &'a HashMap<BlockId, u32>,
    carry: Option<&'a BTreeSet<BlockId>>,
    old: F,
    out: Flow,
    /// Padding of frames opened since the last leaf, which the next leaf carries above it.
    pad: f32,
    /// Whether a block since the last leaf starts a page, which the next leaf then does.
    breaks: bool,
    root: usize,
    /// How far inside the document's own blocks the blocks being placed are.
    depth: usize,
    /// The language the next leaf's texts take from the blocks around them.
    lang: Option<Language>,
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

    /// Places the blocks inside a block, or those a merge left where they have no place.
    fn inside(&mut self, blocks: &[Block], x: f32, width: f32, frame: Option<usize>, quoted: bool) {
        self.depth += 1;
        self.place(blocks, x, width, frame, quoted);
        self.depth -= 1;
    }

    /// Places what a gallery or a table holds that is no part of it, under it, so that nothing a merge
    /// left there is hidden.
    fn strays<'b>(&mut self, blocks: impl Iterator<Item = &'b Block>, x: f32, width: f32, frame: Option<usize>) {
        for block in blocks {
            self.pad += self.types.get(Role::Body).after;
            self.inside(std::slice::from_ref(block), x, width, frame, false);
        }
    }

    /// Places the empty box that stands for a gallery or a table with nothing in it.
    fn empty(&mut self, block: &Block, role: Role, at: (f32, f32, f32, Option<usize>)) {
        let (x, width, gap, frame) = at;
        let (mut leaf, _) = self.leaf(block, What::Empty);
        leaf.role = role;
        self.push(leaf, x, width, gap, frame);
    }

    /// Places the rows of a table, the first `gap` below the row above.
    fn table(
        &mut self,
        rows: &[&Block],
        columns: &[Align],
        header: bool,
        role: Role,
        at: (f32, f32, f32, Option<usize>),
    ) {
        let (x, width, gap, frame) = at;
        let first = self.out.leaves.len();
        // A cell past the last column that says how it is set is set from its start.
        let aligns = || columns.iter().map(|align| alignment(*align)).chain(std::iter::repeat(Alignment::Start));
        let around = self.lang;
        for (index, row) in rows.iter().enumerate() {
            self.lang = language(row.lang.as_deref()).or(around);
            let (mut leaf, _) = self.leaf(row, What::Row);
            let cells = || row.body.blocks().iter().filter(|cell| cell.kind == BlockKind::TableCell);
            let mut reread = leaf.cells.len() != cells().count();
            if reread {
                leaf.cells = cells().map(Cell::of).collect();
            } else {
                for (cell, block) in leaf.cells.iter_mut().zip(cells()) {
                    let known = cell.id == block.id;
                    if known && !self.changed(&block.id) {
                        continue;
                    }
                    let place = Cell { x: cell.x, width: cell.width, ..Cell::of(block) };
                    let before = std::mem::replace(cell, place);
                    if known {
                        self.out.retexted.push((block.id.clone(), before.words.text));
                    }
                    reread = true;
                }
            }
            for (cell, align) in leaf.cells.iter_mut().zip(aligns()) {
                reread |= std::mem::replace(&mut cell.align, align) != align;
            }
            let header = header && index == 0;
            if leaf.header != header || leaf.role != role || leaf.lang != self.lang {
                // Text set another way is another width.
                leaf.cells.iter_mut().for_each(|cell| cell.fit = None);
                reread = true;
            }
            if reread {
                (leaf.exact, leaf.laid) = (false, None);
            }
            (leaf.role, leaf.table, leaf.header) = (role, self.out.tables.len(), header);
            self.push(leaf, x, width, if index == 0 { gap } else { 0.0 }, frame);
        }
        self.lang = around;
        let mut table = Table { rows: first..self.out.leaves.len(), sized: false };
        table.size(&mut self.out.leaves, self.types);
        self.out.tables.push(table);
        let cell = |block: &&Block| block.kind != BlockKind::TableCell;
        self.strays(rows.iter().flat_map(|row| row.body.blocks()).filter(cell), x, width, frame);
    }

    fn place(&mut self, blocks: &[Block], x: f32, width: f32, frame: Option<usize>, quoted: bool) {
        let unit = self.types.get(Role::Body).after;
        let prose = if quoted { Role::Quote } else { Role::Body };
        // The space after the block above, and whether that block was a list item.
        let mut above: Option<(f32, bool)> = None;
        let mut counters: Vec<u32> = Vec::new();
        let around = self.lang;
        for (index, block) in blocks.iter().enumerate() {
            if self.depth == 0 {
                self.root = index;
            }
            // What a block holds takes its language. Its own text reads it from the block.
            let holds = matches!(block.body, Body::Blocks(_));
            self.lang = language(block.lang.as_deref()).filter(|_| holds).or(around);
            self.breaks |= breaks(block);
            let role = match &block.kind {
                BlockKind::Heading { level: 1 } => Role::Heading1,
                BlockKind::Heading { level: 2 } => Role::Heading2,
                BlockKind::Heading { level: 3 } => Role::Heading3,
                BlockKind::Heading { .. } => Role::Heading4,
                BlockKind::Code { .. } => Role::Code,
                BlockKind::Unknown { .. } | BlockKind::Caption => Role::Caption,
                BlockKind::Figure { .. } | BlockKind::Gallery { .. } => Role::Caption,
                _ => prose,
            };
            let of = self.types.get(role);
            let (before, after) = match &block.kind {
                BlockKind::Quote | BlockKind::Formula => (unit, unit),
                BlockKind::Table { .. } | BlockKind::TableRow => (unit, unit),
                BlockKind::Figure { .. } | BlockKind::Gallery { .. } => (unit.max(of.before), unit.max(of.after)),
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
                    let mut head = Head::default();
                    let (look, left, right, pad) = match &block.kind {
                        BlockKind::Callout { tone, .. } => {
                            if let Some(look) = block.kind.callout_look() {
                                let line = if look.title.is_empty() { 0.0 } else { self.types.get(Role::Body).line };
                                let color = look.color.and_then(theme::color);
                                head = Head { icon: look.icon, title: look.title.to_owned(), color, line };
                            }
                            let beside = head.room().1;
                            (Look::Callout(*tone), theme::BOX_PAD_X + beside, theme::BOX_PAD_X, theme::BOX_PAD_Y)
                        }
                        _ => (Look::Quote, theme::QUOTE_INSET, 0.0, 0.0),
                    };
                    // A quote's bar stands at the side its lines start at.
                    let rtl = look == Look::Quote && leads(block.body.blocks());
                    let (left, right) = if rtl { (right, left) } else { (left, right) };
                    let shrink = (left + right).min(width - theme::MIN_WIDTH).max(0.0);
                    let left = left.min(shrink);
                    let this = self.out.frames.len();
                    let first = self.out.leaves.len();
                    let lead = self.pad + gap;
                    let id = block.id.clone();
                    let above = head.room().0;
                    let (last, reach) = (first, 0.0);
                    let framed = Frame { id, look, head, parent: frame, first, last, x, width, lead, reach, rtl };
                    self.out.frames.push(framed);
                    self.pad = lead + pad + above;
                    let quoted = quoted || look == Look::Quote;
                    self.inside(block.body.blocks(), x + left, width - shrink, Some(this), quoted);
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
                BlockKind::Gallery { .. } => {
                    let inside = block.body.blocks();
                    // A gallery has one caption, which is its last. A merge can leave more.
                    let caption = inside.iter().rfind(|part| part.kind == BlockKind::Caption);
                    let pictures: Vec<Picture> = inside.iter().filter_map(Picture::of).collect();
                    if pictures.is_empty() && caption.is_none() {
                        self.empty(block, prose, (x, width, gap, frame));
                        self.strays(inside.iter(), x, width, frame);
                        continue;
                    }
                    let (mut leaf, _) = self.leaf(block, What::Gallery);
                    let same = leaf.pictures.len() == pictures.len()
                        && leaf.pictures.iter().zip(&pictures).all(|(old, new)| old.same(new));
                    let known = leaf.caption.as_ref() == caption.map(|caption| &caption.id);
                    let reread = !known || caption.is_some_and(|caption| self.changed(&caption.id));
                    if reread {
                        let before = match caption {
                            Some(caption) => leaf.set_content(caption),
                            None => std::mem::take(&mut leaf.words).text,
                        };
                        if let (true, Some(caption)) = (known, caption) {
                            self.out.retexted.push((caption.id.clone(), before));
                        }
                        leaf.caption = caption.map(|caption| caption.id.clone());
                    }
                    let number = self.numbers.get(&block.id).copied();
                    if !same || reread || leaf.number != number || leaf.role != role {
                        (leaf.exact, leaf.laid) = (false, None);
                    }
                    (leaf.pictures, leaf.number, leaf.role) = (pictures, number, role);
                    self.push(leaf, x, width, gap, frame);
                    let shown = |part: &&Block| {
                        matches!(part.kind, BlockKind::Figure { .. }) || caption.is_some_and(|it| it.id == part.id)
                    };
                    self.strays(inside.iter().filter(|part| !shown(part)), x, width, frame);
                }
                BlockKind::Table { columns, header } => {
                    let (rows, rest): (Vec<&Block>, Vec<&Block>) =
                        block.body.blocks().iter().partition(|row| row.kind == BlockKind::TableRow);
                    match rows.is_empty() {
                        true => self.empty(block, prose, (x, width, gap, frame)),
                        false => self.table(&rows, columns, *header, prose, (x, width, gap, frame)),
                    }
                    self.strays(rest.into_iter(), x, width, frame);
                }
                // A row that a merge left outside its table is a table of one row.
                BlockKind::TableRow => self.table(&[block], &[], false, prose, (x, width, gap, frame)),
                // A cell outside a row, or a caption outside a gallery, is a line of text.
                _ => {
                    let what = match &block.kind {
                        BlockKind::Formula => What::Formula,
                        BlockKind::Figure { .. } => What::Figure,
                        _ => What::Text,
                    };
                    let (mut leaf, kept) = self.leaf(block, what);
                    if !kept || self.changed(&block.id) {
                        let before = leaf.set_content(block);
                        if kept {
                            self.out.retexted.push((block.id.clone(), before));
                        }
                        (leaf.exact, leaf.laid) = (false, None);
                    }
                    let number = self.numbers.get(&block.id).copied();
                    if leaf.role != role || leaf.number != number {
                        leaf.exact = false;
                    }
                    if leaf.role != role || leaf.marker != marker || leaf.number != number {
                        leaf.laid = None;
                    }
                    leaf.pictures = Picture::of(block).into_iter().collect();
                    (leaf.role, leaf.marker, leaf.number) = (role, marker, number);
                    // A list item is set in from the side it starts at, where its marker stands.
                    let x = if leaf.words.rtl { x } else { x + inset };
                    self.push(leaf, x, width - inset, gap, frame);
                }
            }
        }
        self.lang = around;
    }

    /// Adds a leaf to the column, `gap` below the row above and within the given edges.
    fn push(&mut self, mut leaf: Leaf, x: f32, width: f32, gap: f32, frame: Option<usize>) {
        // The text of a boxed leaf sits inside the box's padding.
        let (pad_x, pad_y) = match leaf.boxed() {
            true => (theme::BOX_PAD_X.min((width - theme::MIN_WIDTH).max(0.0) / 2.0), theme::BOX_PAD_Y),
            false => (0.0, 0.0),
        };
        let (x, width) = (x + pad_x, (width - 2.0 * pad_x).max(1.0));
        // Text is shaped for its language, so in another it is set again.
        if (leaf.x, leaf.width, leaf.lang) != (x, width, self.lang) {
            (leaf.exact, leaf.laid) = (false, None);
        }
        (leaf.x, leaf.width, leaf.pad, leaf.lang) = (x, width, pad_x, self.lang);
        leaf.root = self.root;
        leaf.top = gap + pad_y + std::mem::take(&mut self.pad);
        leaf.bottom = pad_y;
        leaf.frame = frame;
        leaf.breaks = std::mem::take(&mut self.breaks);
        leaf.arrange(self.types);
        if leaf.fixed() {
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
