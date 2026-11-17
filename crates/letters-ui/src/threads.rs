//! Notes in the margin: the marker beside a block that has open notes, the thread a note opens to under
//! its passage, and the list of them all in the Notes panel.
//!
//! Comments, import notes and conflicts are three kinds of one thing. Each kind has its own sign and
//! tone on the same marker, opens in place and is listed in the panel. The page is drawn on a canvas, so
//! the markers are drawn there too and the window asks this module where they stand.

use std::iter;
use std::rc::Rc;

use accesskit::Role;
use letters_layout::{DocLayout, Fonts, Named, Palette, Uncovered, WordList};
use letters_model::{
    Block, BlockId, BlockKind, Body, Document, Note, NoteKind, Pos, Run, Selection, Style, TextStyle as Type, Version,
};
use letters_render::{DisplayList, Item, Point, Rect, Size};

use crate::color::resolve;
use crate::element::{Element, Paint, Pointer};
use crate::icon::{self, Icon};
use crate::notes::{Margin, Noting};
use crate::opening;
use crate::review;
use crate::safe::Safely;
use crate::style::{Align, Dir, Edges, Justify, Look};
use crate::text::{TextShaper, TextStyle};
use crate::tokens::{self, Color as Ink, Scheme};
use crate::widgets::{Button, Popover};
use crate::window::{Message, PT};
use crate::words::WordLists;

/// How far a marker stands from the text, how tall it is, and the room it keeps from the edge of the page
/// area.
const GAP: f32 = tokens::SPACE_LG;
const HEIGHT: f32 = 22.0;
const EDGE: f32 = tokens::SPACE_XS;
/// The sign on a marker, the room between it and the count, and the room either side of the two.
const SIGN: f32 = 14.0;
const SIGN_GAP: f32 = 4.0;
const PADDING: f32 = 6.0;
/// How far round a marker a press still counts as on it.
const REACH: f32 = 3.0;

/// How wide a note that is open is, and the round face of who speaks in it.
pub(crate) const WIDTH: f32 = 344.0;
pub(crate) const FACE: f32 = 24.0;
/// How wide what is said runs, beside the face of who says it.
pub(crate) const SAID: f32 = WIDTH - FACE - tokens::SPACE_SM;
/// How tall what is said in a note gets before it scrolls.
const TALL: f32 = 320.0;

/// How much of the text a note is about the panel quotes, and how much of what the note says, in characters.
const QUOTED: usize = 60;
/// How much a conflict tells of the version that is no longer on the page.
const KEPT: usize = 280;
const TOLD: usize = 160;

/// Who a note is by when its writer gave no name.
const UNNAMED: &str = "Someone";

/// The kinds of note this build shows, the most pressing last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Import,
    Comment,
    Conflict,
}

impl Kind {
    pub(crate) fn of(note: &Note) -> Option<Self> {
        match &note.kind {
            NoteKind::Comment => Some(Self::Comment),
            NoteKind::Import => Some(Self::Import),
            NoteKind::Conflict { .. } => Some(Self::Conflict),
            NoteKind::Unknown { .. } => None,
        }
    }

    fn sign(self) -> Icon {
        match self {
            Self::Comment => icon::MESSAGE_SQUARE,
            Self::Import => icon::FILE_INPUT,
            Self::Conflict => icon::GIT_MERGE,
        }
    }

    /// The colour that sets the kind apart where its notes are listed.
    fn tone(self) -> Ink {
        match self {
            Self::Comment => tokens::INFO,
            Self::Import => tokens::FG_MUTED,
            Self::Conflict => tokens::WARN,
        }
    }

    /// So many notes of the kind, in words: "1 comment", "3 import notes".
    fn counted(self, count: usize) -> String {
        let one = match self {
            Self::Comment => "comment",
            Self::Import => "import note",
            Self::Conflict => "conflict",
        };
        format!("{count} {one}{}", if count == 1 { "" } else { "s" })
    }

    /// What the panel calls the kind where it offers to list it alone.
    fn title(self) -> &'static str {
        match self {
            Self::Comment => "Comments",
            Self::Import => "Import",
            Self::Conflict => "Conflicts",
        }
    }
}

/// The marker beside a block of the document's own: what its open notes are, and how many.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Marker {
    pub block: BlockId,
    /// The first text in the block that has any words, and how long it is: where the block is taken to
    /// start, since text without words cannot be asked whether it shows.
    pub first: Option<(BlockId, usize)>,
    /// The most pressing kind among the notes, which the marker shows.
    kind: Kind,
    /// Whether the notes are of more than one kind.
    mixed: bool,
    /// Every note and every reply to one counts.
    count: usize,
    /// The notes, in the order a press on the marker opens them.
    pub notes: Vec<String>,
}

impl Marker {
    /// A marker beside `block` that counts nothing yet.
    pub fn beside(block: &Block, kind: Kind) -> Self {
        fn texts<'a>(block: &'a Block, out: &mut Vec<(&'a BlockId, usize)>) {
            if matches!(block.body, Body::Text(_)) {
                out.push((&block.id, block.plain_text().len()));
            }
            block.body.blocks().iter().for_each(|part| texts(part, out));
        }
        let mut all = Vec::new();
        texts(block, &mut all);
        let first = all.iter().find(|(_, long)| *long > 0).or(all.first());
        let first = first.map(|(text, long)| ((*text).clone(), *long));
        Self { block: block.id.clone(), first, kind, mixed: false, count: 0, notes: Vec::new() }
    }

    pub fn add(&mut self, note: &Note, kind: Kind) {
        self.mixed |= kind != self.kind;
        self.kind = self.kind.max(kind);
        self.count += 1 + note.replies.len();
        self.notes.push(note.id.clone());
    }

    /// What the marker is called: "2 comments", or "3 notes" when they are of several kinds.
    pub fn label(&self) -> String {
        if self.mixed { format!("{} notes", self.count) } else { self.kind.counted(self.count) }
    }
}

/// The style a marker's count is set in.
fn digits() -> TextStyle {
    TextStyle::control().size(tokens::TEXT_XS)
}

/// Where a marker stands: in the margin beside the text column, level with `line`, the first line of its
/// block. The column is `column` wide and has `room` beside it. A margin too narrow for the count has
/// the sign alone, on a round marker.
pub(crate) fn place(marker: &Marker, line: Rect, column: f32, room: f32, shaper: &dyn TextShaper) -> Rect {
    let count = shaper.shape(&marker.count.to_string(), &digits(), None).size.width;
    let whole = (2.0 * (PADDING + 1.0) + SIGN + SIGN_GAP + count).round();
    let wide = if room >= GAP + whole + EDGE { whole } else { HEIGHT };
    let gap = GAP.min(((room - wide) / 2.0).max(1.0)).round();
    Rect::new(column + gap, (line.y + (line.height - HEIGHT) / 2.0).round(), wide, HEIGHT)
}

/// Whether a press at `at` is on the marker at `marker`: on it, or as near as a finger comes.
pub(crate) fn pressed(marker: Rect, at: Point) -> bool {
    let reach = Rect::new(marker.x - REACH, marker.y - REACH, marker.width + 2.0 * REACH, marker.height + 2.0 * REACH);
    reach.contains(at)
}

/// Draws a marker at the place [`place`] gave it. `open` is whether one of its notes is open, and `over`
/// whether the pointer rests on it.
pub(crate) fn mark(
    marker: &Marker,
    at: Rect,
    (open, over): (bool, bool),
    shaper: &dyn TextShaper,
    scheme: Scheme,
    out: &mut DisplayList,
) {
    let (line, ink) = match (marker.kind, open) {
        (Kind::Conflict, _) => (tokens::WARN, tokens::WARN),
        (_, true) => (tokens::INFO, tokens::INFO),
        (Kind::Import, false) => (tokens::LINE_STRONG, tokens::FG_MUTED),
        (Kind::Comment, false) => (tokens::CARD_LINE, tokens::FG_MUTED),
    };
    // A conflict that is open is the one marker that is filled.
    let (ground, ink) = match (marker.kind == Kind::Conflict && open, over) {
        (true, _) => (tokens::WARN, tokens::SURFACE),
        (false, true) => (tokens::TONE_1, ink),
        (false, false) => (tokens::SURFACE, ink),
    };
    let radius = at.height / 2.0;
    out.push(Item::Fill { rect: at, color: resolve(ground, scheme, 1.0), radius });
    out.push(Item::Stroke { rect: at, color: resolve(line, scheme, 1.0), width: 1.0, radius });
    let counted = at.width > HEIGHT;
    let left = if counted { at.x + PADDING + 1.0 } else { at.x + (at.width - SIGN) / 2.0 };
    out.push(Item::Path {
        origin: Point { x: left, y: at.y + (at.height - SIGN) / 2.0 },
        scale: SIGN / icon::GRID,
        path: marker.kind.sign().outline(),
        color: resolve(ink, scheme, 1.0),
        stroke: Some(tokens::ICON_STROKE),
    });
    if counted {
        let count = shaper.shape(&marker.count.to_string(), &digits(), None);
        let origin = Point { x: left + SIGN + SIGN_GAP, y: at.y + (at.height - count.size.height) / 2.0 };
        count.paint(origin, resolve(ink, scheme, 1.0), out);
    }
}

/// One comment or reply as it is drawn.
pub(crate) struct Spoken {
    list: DisplayList,
    size: Size,
    /// What it says without its marks, as assistive technology reads it.
    plain: String,
    /// Where its links are, each with its address.
    links: Vec<(Rect, String)>,
}

impl Paint for Spoken {
    fn paint(&self, rect: Rect, _visible: Rect, _scheme: Scheme, out: &mut DisplayList) {
        let first = out.items.len();
        out.items.extend(self.list.items.iter().cloned());
        out.translate(first, Point { x: rect.x, y: rect.y });
    }
}

/// What the note that is open says. The layout of the page sets it, in the type of the interface, so that
/// a comment's bold, italic, code and links look as they do on the page.
#[derive(Default)]
pub(crate) struct Said {
    /// Made when the first note is opened.
    layout: Option<DocLayout>,
    /// The faces of the system that back the fonts up, for a layout made after they came.
    backup: Fonts,
    /// The word lists, for a layout made after they came.
    lists: WordLists,
    /// What was last set: the note, how many replies it had, how wide and in which colours.
    of: Option<(String, usize, f32, Palette)>,
    spoken: Vec<Rc<Spoken>>,
}

impl Said {
    /// Has what is said set again when it is next asked for, as when more fonts have arrived.
    pub fn again(&mut self) {
        self.of = None;
    }

    /// Takes faces of the system that back the fonts up. Returns whether a character that was shown
    /// without a glyph has one now.
    pub fn add_fallback_fonts(&mut self, fonts: &Fonts) -> bool {
        self.backup.faces.extend(fonts.faces.iter().cloned());
        self.again();
        self.layout.as_mut().is_some_and(|layout| layout.add_fallback_fonts(fonts))
    }

    /// The faces that notes asked for since this was last asked.
    pub fn take_named(&mut self) -> Vec<Named> {
        self.layout.as_mut().map(DocLayout::take_named).unwrap_or_default()
    }

    /// The characters that notes showed without a glyph since this was last asked.
    pub fn take_uncovered(&mut self) -> Vec<Uncovered> {
        self.layout.as_mut().map(DocLayout::take_uncovered).unwrap_or_default()
    }

    /// Takes the word list of a script. Returns whether what a note says may now be set otherwise: the
    /// layout holds only what it set last, so whatever was set counts.
    pub fn add_word_list(&mut self, list: WordList, data: &Rc<[u8]>) -> bool {
        if !self.lists.keep(list, data) {
            return false;
        }
        let Some(layout) = &mut self.layout else { return false };
        layout.add_word_list(list, data);
        self.of.take().is_some()
    }

    /// The scripts that notes set without their word list since this was last asked.
    pub fn take_unlisted(&mut self) -> Vec<WordList> {
        self.layout.as_mut().map(DocLayout::take_unlisted).unwrap_or_default()
    }

    /// The layout that sets what notes say, in these colours. The field a note is written in is set in
    /// it too ([`crate::comment`]), so that what is written looks as it will once it is kept.
    pub fn layout(&mut self, palette: Palette) -> &mut DocLayout {
        let fonts = crate::families::fonts();
        let layout = self.layout.get_or_insert_with(|| {
            let mut layout = DocLayout::new(fonts.clone());
            layout.add_fallback_fonts(&self.backup);
            self.lists.hand(&mut layout);
            layout
        });
        // A shell whose fonts come one after another may hold more by now.
        layout.add_fonts(&fonts);
        layout.set_palette(palette);
        layout
    }

    /// What a note says and each reply to it, set `width` wide. They are set again only when they change.
    pub fn of(&mut self, note: &Note, width: f32, palette: Palette) -> &[Rc<Spoken>] {
        let of = (note.id.clone(), note.replies.len(), width, palette);
        if self.of.as_ref() != Some(&of) {
            let layout = self.layout(palette);
            let said = iter::once(&note.content).chain(note.replies.iter().map(|reply| &reply.content));
            let spoken = said.map(|runs| Rc::new(spoken(layout, runs, width))).collect();
            (self.spoken, self.of) = (spoken, Some(of));
        }
        &self.spoken
    }
}

/// The interface's small text as a document's body type, with no room between paragraphs.
pub(crate) fn small() -> Style {
    let mut style = Style::default();
    let body = Type { size: tokens::TEXT_SM / PT, line_height: 1.4, space_after: 0.0, ..Type::default() };
    style.types.insert("body".into(), body);
    style
}

fn spoken(layout: &mut DocLayout, runs: &[Run], width: f32) -> Spoken {
    const ID: &str = "said";
    let block = Block { body: Body::Text(runs.to_vec()), ..Block::new(ID, BlockKind::Paragraph) };
    let doc = Document { blocks: vec![block], ..Document::default() };
    layout.update(&doc, &small(), width, None);
    layout.settle(Rect::new(0.0, 0.0, width, f32::MAX), 0);
    let height = layout.height().ceil();
    let mut list = DisplayList::default();
    layout.paint(Rect::new(0.0, 0.0, width, height), &mut list);
    let mut links = Vec::new();
    let mut at = 0;
    for run in runs {
        let end = at + run.text.len();
        if let Some(href) = &run.link {
            let linked = Selection { anchor: Pos::new(ID, at), head: Pos::new(ID, end) };
            links.extend(layout.selection(&linked).into_iter().map(|line| (line, href.clone())));
        }
        at = end;
    }
    Spoken { list, size: Size { width, height }, plain: plain(runs), links }
}

fn plain(runs: &[Run]) -> String {
    runs.iter().map(|run| run.text.as_str()).collect()
}

pub(crate) fn named(author: &str) -> &str {
    if author.trim().is_empty() { UNNAMED } else { author }
}

/// The first letters of a name's first and last words, as a face shows them.
pub(crate) fn initials(name: &str) -> String {
    let mut letters = name.split_whitespace().filter_map(|word| word.chars().find(|c| c.is_alphanumeric()));
    let first = letters.next();
    first.into_iter().chain(letters.next_back()).flat_map(char::to_uppercase).collect()
}

/// `text` cut to `most` characters, with a sign that there was more.
fn cut(text: &str, most: usize) -> String {
    if text.chars().count() <= most {
        return text.to_owned();
    }
    let kept: String = text.chars().take(most).collect();
    format!("{}\u{2026}", kept.trim_end())
}

/// What a conflict says of itself, where its two versions are not looked at side by side: in the panel,
/// and once the page no longer has its block. `page` is the block as the page has it.
fn conflict(versions: &[Version; 2], page: Option<&Block>) -> String {
    let changed = match (versions[0].author.trim(), versions[1].author.trim()) {
        ("", _) | (_, "") => "This was changed in two places".to_owned(),
        (one, other) if one == other => format!("{one} changed this in two places"),
        (one, other) => format!("{one} and {other} both changed this"),
    };
    // Using the version held back trades the two, so the page can come to show the earlier one.
    match page.map(|page| review::stands_for(versions, page)) {
        Some(0) => format!("{changed}. The page shows the earlier version."),
        Some(_) => format!("{changed}. The page shows the later version."),
        None => format!("{changed}. It is no longer on the page."),
    }
}

fn send(asked: Noting) -> Message {
    Message::Note(asked)
}

/// The face of who speaks: the first letters of their name, or the sign of an import for what it noted.
pub(crate) fn face(name: &str, import: bool) -> Element<Message> {
    match import {
        true => round(Element::icon(icon::FILE_INPUT, SIGN)),
        false => lettered(&initials(name)),
    }
}

/// A face that says a few letters: the first of someone's name, or how many more people there are.
pub(crate) fn lettered(letters: &str) -> Element<Message> {
    round(Element::text(letters, TextStyle::control().size(10.0).weight(600)))
}

fn round(inside: Element<Message>) -> Element<Message> {
    Element::row()
        .width(FACE)
        .height(FACE)
        .radius(FACE / 2.0)
        .bg(tokens::NEUTRAL_SOFT)
        .fg(tokens::FG_MUTED)
        .align(Align::Center)
        .justify(Justify::Center)
        .decorative()
        .child(inside)
}

/// One comment or reply in a note: who said it and when, over what was said. A press on a link in it asks
/// for the link to be opened.
fn voice(index: usize, (who, when): (&str, &str), said: &Rc<Spoken>, import: bool, now: &str) -> Element<Message> {
    let small = TextStyle::control().size(tokens::TEXT_SM);
    let (name, ago) = (named(who), opening::ago(now, when));
    let links = said.links.clone();
    let follow = move |event: Pointer| match event {
        Pointer::Down { at, .. } => {
            let link = links.iter().find(|(line, _)| line.contains(at));
            link.map(|(_, href)| Message::Safe(Safely::Follow(href.clone())))
        }
        _ => None,
    };
    let paint: Rc<dyn Paint> = said.clone();
    let words = Element::canvas(paint)
        .width(said.size.width)
        .height(said.size.height)
        .role(Role::Paragraph)
        .label(said.plain.as_str())
        .on_pointer(follow)
        // The reply keeps the keyboard while what was said is read and its links are pressed.
        .passive();
    let by = Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_SM)
        .child(Element::text(name, small.weight(600)).shrink())
        .child(Element::text(ago.as_str(), small).fg(tokens::FG_MUTED));
    Element::row()
        .key(("said", index))
        .align(Align::Start)
        .gap(tokens::SPACE_SM)
        .role(Role::Comment)
        .label(name)
        .description(ago)
        .child(face(name, import))
        .child(Element::column().gap(tokens::SPACE_2XS).width(SAID).child(by).child(words))
}

/// A note that is open, to hang under its passage: what was said, by whom and when, with Resolve at its
/// top right and a reply under it. `reply` is the field the reply is written in, and whether nothing is
/// written in it. `said` is what the note and each reply say, as [`Said::of`] set them,
/// and `refused` why nothing can be added to the notes of this document. `follows` is for a note the caret
/// opened by coming into its passage, and `back` for a conflict whose block can be put back on the page.
pub(crate) fn thread(
    note: &Note,
    (field, blank): (Element<Message>, bool),
    said: &[Rc<Spoken>],
    now: &str,
    refused: Option<&str>,
    (follows, back): (bool, bool),
) -> Popover<Message> {
    let small = TextStyle::control().size(tokens::TEXT_SM);
    let kind = Kind::of(note).unwrap_or(Kind::Comment);
    let (title, head) = match kind {
        Kind::Comment => ("Comments", kind.counted(1 + note.replies.len())),
        Kind::Import => ("Import note", "Import note".to_owned()),
        Kind::Conflict => ("Conflict", "Conflict".to_owned()),
    };
    let mut top = Element::row()
        .align(Align::Center)
        .justify(Justify::Between)
        .min_height(tokens::CONTROL_HEIGHT_SM)
        .child(Element::text(head, small).fg(tokens::FG_MUTED));
    let mut all = Element::column().gap(tokens::SPACE_MD).width(WIDTH);
    let resolve = Button::new("Resolve").icon(icon::CHECK).subtle().small();
    top = top.child(resolve.disabled(refused.is_some()).on_press(send(Noting::Resolve)));
    if let NoteKind::Conflict { versions } = &note.kind {
        // Only a conflict about a block that is gone is read here: there is no place to show its versions.
        let says = Element::paragraph(conflict(versions, None), small.leading(1.4)).role(Role::Paragraph);
        all = all.child(top).child(says);
        // The version that still has the block can go back where the block stood, to be read on the page
        // beside the one that took it out.
        let kept = versions.iter().find(|version| version.block.is_some()).filter(|_| back);
        all = all.children(kept.map(|version| self::kept(version, now)));
    } else {
        let by = iter::once((note.author.as_str(), note.created.as_str()));
        let by = by.chain(note.replies.iter().map(|reply| (reply.author.as_str(), reply.created.as_str())));
        let voices = by.zip(said).enumerate();
        let voices = voices.map(|(index, (by, said))| voice(index, by, said, kind == Kind::Import && index == 0, now));
        let mut voices = Element::column().gap(tokens::SPACE_MD).children(voices);
        // A long exchange scrolls, so that its reply stays within reach.
        let tall: f32 = said.iter().map(|said| said.size.height + FACE + tokens::SPACE_MD).sum();
        if tall > TALL {
            voices = voices.height(TALL).scroll();
        }
        // A reply of several lines has the button beside its last.
        let under = Element::row()
            .align(Align::End)
            .gap(tokens::SPACE_SM)
            .padding(Edges::top(tokens::SPACE_MD))
            .border(Edges::top(1.0), tokens::LINE);
        let under = match refused {
            Some(why) => under.child(Element::paragraph(why, small).fg(tokens::FG_MUTED).grow(1.0).role(Role::Status)),
            None => {
                let post = Button::icon_only(icon::SEND, "Send")
                    .primary()
                    .small()
                    .disabled(blank)
                    .on_press(send(Noting::Answer));
                // A note the caret opened leaves the keyboard with the page, until its reply is pressed.
                let field = field.key("reply");
                under.child(if follows { field } else { field.takes_focus() }).child(post)
            }
        };
        all = all.child(top).child(voices).child(under);
    }
    let thread = Popover::new(all)
        .label(title)
        .max_width(WIDTH + 2.0 * tokens::SPACE_MD + 2.0)
        .open(true)
        .on_close(send(Noting::Close));
    // A note the caret opened is beside the page, and stays for as long as the caret is in its passage.
    if follows {
        return thread.beside().stays();
    }
    // A stray press on the page does not cost what was typed: Escape drops it.
    if blank { thread } else { thread.stays() }
}

/// The version of a conflict that still has the block the page lost: who wrote it and when, how it reads,
/// and the button that puts it back on the page.
fn kept(version: &Version, now: &str) -> Element<Message> {
    let small = TextStyle::control().size(tokens::TEXT_SM);
    let (name, ago) = (named(&version.author), opening::ago(now, &version.edited));
    let reads = version.block.as_ref().map(Block::plain_text).unwrap_or_default();
    let by = Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_SM)
        .child(Element::text(name, small.weight(600)).shrink())
        .child(Element::text(ago.as_str(), small).fg(tokens::FG_MUTED));
    let reads = Element::paragraph(cut(reads.trim(), KEPT), small.leading(1.4)).role(Role::Paragraph);
    let back = Button::new("Use this version").icon(icon::ARROW_LEFT_RIGHT).small().on_press(send(Noting::PutBack));
    let told = Element::column().gap(tokens::SPACE_XS).grow(1.0).child(by).child(reads);
    let told = told.child(Element::row().padding(Edges::top(tokens::SPACE_XS)).child(back));
    Element::row()
        .align(Align::Start)
        .gap(tokens::SPACE_SM)
        .padding(Edges::top(tokens::SPACE_MD))
        .border(Edges::top(1.0), tokens::LINE)
        .child(face(name, false))
        .child(told)
}

/// The text a note is about, cut short: its passage, or the start of its block.
fn about(doc: &Document, note: &Note) -> Option<String> {
    let text = doc.block(&note.anchor.block)?.plain_text();
    let passage = note.anchor.range.clone().and_then(|range| text.get(range)).unwrap_or(&text).trim();
    (!passage.is_empty()).then(|| cut(passage, QUOTED))
}

/// A note in the panel: its kind, who wrote it and when, the text it is about and what it says. A press
/// opens it in place.
fn row(note: &Note, kind: Kind, doc: &Document, now: &str, open: bool) -> Element<Message> {
    let small = TextStyle::control().size(tokens::TEXT_SM);
    let (who, says) = match &note.kind {
        NoteKind::Conflict { versions } => ("Conflict", conflict(versions, doc.block(&note.anchor.block))),
        _ => (named(&note.author), plain(&note.content)),
    };
    let (when, about) = (opening::ago(now, &note.created), about(doc, note));
    let mut head = Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_SM)
        .child(Element::icon(kind.sign(), SIGN).fg(kind.tone()))
        .child(Element::text(who, small.weight(600)).shrink())
        .child(Element::text(when.as_str(), small).fg(tokens::FG_MUTED));
    let replies = match note.replies.len() {
        0 => None,
        1 => Some("1 reply".to_owned()),
        many => Some(format!("{many} replies")),
    };
    if let Some(replies) = &replies {
        let count = Element::text(replies.as_str(), TextStyle::control().size(tokens::TEXT_XS));
        head = head.child(Element::row().grow(1.0)).child(count.fg(tokens::FG_MUTED));
    }
    let told = [about.as_ref().map(|about| format!("On \u{201c}{about}\u{201d}")), Some(when), replies];
    let told: Vec<String> = told.into_iter().flatten().filter(|part| !part.is_empty()).collect();
    let mut row = Element::column()
        .key(note.id.as_str())
        .gap(tokens::SPACE_XS)
        .padding(Edges::all(tokens::SPACE_SM))
        .radius(tokens::RADIUS_CONTROL)
        .hover(Look::bg(tokens::NEUTRAL_SOFT))
        .focused(Look::bg(tokens::NEUTRAL_SOFT))
        .role(Role::ListBoxOption)
        .label(format!("{who}: {says}"))
        .description(told.join(", "))
        .selected(open)
        .focusable()
        .on_press(send(Noting::Open(note.id.clone())))
        .child(head);
    if open {
        row = row.bg(tokens::NEUTRAL_SOFT);
    }
    if let Some(about) = about {
        // The quoted text keeps to one line, behind a rule as a quotation has.
        let quoted = Element::text(about, small).fg(tokens::FG_MUTED).shrink();
        let quoted = Element::row().padding(Edges::left(tokens::SPACE_SM)).child(quoted);
        row = row.child(quoted.border(Edges::left(2.0), tokens::LINE_STRONG));
    }
    row.child(Element::paragraph(cut(&says, TOLD), small.leading(1.4)))
}

/// What the Notes panel holds: every open note, or those of the one kind it was asked to list, each to
/// be opened in place. `None` when no note is open.
pub(crate) fn listed(margin: &Margin, doc: &Document) -> Option<Element<Message>> {
    let small = TextStyle::control().size(tokens::TEXT_SM);
    let open: Vec<(&Note, Kind)> = margin.open().collect();
    let kinds: Vec<Kind> = [Kind::Comment, Kind::Import, Kind::Conflict]
        .into_iter()
        .filter(|kind| open.iter().any(|open| open.1 == *kind))
        .collect();
    let only = margin.filter.filter(|kind| kinds.contains(kind));
    let shown: Vec<&(&Note, Kind)> = open.iter().filter(|(_, kind)| only.is_none_or(|only| only == *kind)).collect();
    let first = shown.first()?;
    let mut all = Element::column().gap(tokens::SPACE_SM).padding(Edges::top(tokens::SPACE_MD));
    if kinds.len() > 1 {
        let choice = |label: &str, kind: Option<Kind>| {
            Button::new(label).subtle().small().pressed(only == kind).on_press(send(Noting::Show(kind)))
        };
        let kinds = kinds.iter().map(|kind| choice(kind.title(), Some(*kind)));
        let choices = Element::row().gap(tokens::SPACE_2XS).role(Role::Group).label("Show");
        all = all.child(choices.child(choice("All", None)).children(kinds));
    }
    // What is listed, in a word, and the one thing that is done to all of it at once.
    let alike = shown.iter().all(|(_, kind)| *kind == first.1);
    let count = if alike { first.1.counted(shown.len()) } else { format!("{} open notes", shown.len()) };
    let mut count = Element::row()
        .align(Align::Center)
        .justify(Justify::Between)
        .min_height(tokens::CONTROL_HEIGHT_SM)
        .padding(Edges::xy(tokens::SPACE_SM, 0.0))
        .child(Element::text(count, small).fg(tokens::FG_MUTED));
    if alike && first.1 == Kind::Import {
        let resolve = Button::new("Resolve all").icon(icon::CHECK).subtle().small();
        count = count.child(resolve.disabled(margin.refusal().is_some()).on_press(send(Noting::ResolveImports)));
    }
    // The row of the note that is open is the one picked, wherever that note shows.
    let read = margin.reading().map(|(note, _)| note).or_else(|| margin.reviewing()).map(|note| note.id.as_str());
    let rows = shown.iter().map(|(note, kind)| row(note, *kind, doc, &margin.now, read == Some(note.id.as_str())));
    let list = Element::column()
        .gap(tokens::SPACE_2XS)
        .role(Role::ListBox)
        .label("Open notes")
        .arrows(Dir::Column)
        .one_stop()
        .children(rows);
    Some(all.child(count).child(list))
}

#[cfg(test)]
mod tests {
    use letters_model::Anchor;

    use super::*;
    use crate::text::FixedShaper;

    fn note(id: &str, kind: NoteKind, replies: usize) -> Note {
        let reply = letters_model::Reply {
            author: "Alice Moreau".into(),
            created: String::new(),
            content: vec![Run::plain("Yes")],
            extra: Default::default(),
        };
        Note {
            id: id.into(),
            kind,
            anchor: Anchor::block("a"),
            author: "Sam Okafor".into(),
            created: String::new(),
            resolved: false,
            content: vec![Run::plain("Has anyone chased it?")],
            replies: vec![reply; replies],
            extra: Default::default(),
        }
    }

    fn marker(notes: &[(NoteKind, usize)]) -> Marker {
        let block = Block::text("a", BlockKind::Paragraph, "Their written quote covers the slates.");
        let notes: Vec<Note> = notes.iter().map(|(kind, replies)| note("n", kind.clone(), *replies)).collect();
        let mut marker = Marker::beside(&block, Kind::of(&notes[0]).expect("a kind that shows"));
        notes.iter().for_each(|note| marker.add(note, Kind::of(note).expect("a kind that shows")));
        marker
    }

    #[test]
    fn a_marker_counts_its_notes_and_their_replies_and_names_their_kind() {
        let conflict = || {
            let version =
                Version { author: String::new(), edited: String::new(), block: None, extra: Default::default() };
            NoteKind::Conflict { versions: Box::new([version.clone(), version]) }
        };
        // The notes on a block, each with its replies, and what the marker is called.
        let cases = [
            (vec![(NoteKind::Comment, 0)], "1 comment"),
            (vec![(NoteKind::Comment, 1)], "2 comments"),
            (vec![(NoteKind::Comment, 1), (NoteKind::Comment, 0)], "3 comments"),
            (vec![(NoteKind::Import, 0)], "1 import note"),
            (vec![(conflict(), 0)], "1 conflict"),
            (vec![(NoteKind::Import, 0), (NoteKind::Comment, 2)], "4 notes"),
        ];
        for (notes, label) in cases {
            assert_eq!(marker(&notes).label(), label);
        }
        // The most pressing kind is the one a marker shows.
        let kinds = [NoteKind::Import, NoteKind::Comment, conflict()];
        let shown = marker(&kinds.map(|kind| (kind, 0))).kind;
        assert_eq!(shown, Kind::Conflict);
    }

    #[test]
    fn a_marker_stands_in_the_margin_beside_its_line_and_keeps_within_it() {
        let line = Rect::new(0.0, 100.0, 600.0, 24.0);
        // How much room the margin has: plenty, and only the least the window keeps.
        for room in [200.0, 24.0] {
            let at = place(&marker(&[(NoteKind::Comment, 1)]), line, 600.0, room, &FixedShaper);
            assert!(at.x > 600.0 && at.x + at.width <= 600.0 + room, "{room}: beside the text: {at:?}");
            assert_eq!(at.y + at.height / 2.0, 112.0, "{room}: level with the line");
            assert!(pressed(at, Point { x: at.x + 1.0, y: at.y + 1.0 }) && !pressed(at, Point { x: 590.0, y: 112.0 }));
        }
    }

    #[test]
    fn a_face_shows_the_first_letters_of_a_name() {
        let cases = [
            ("Sam Okafor", "SO"),
            ("alice", "A"),
            ("Anna Maria de la Cruz", "AC"),
            ("  ", ""),
            ("\u{201c}Q\u{201d} Branch", "QB"),
        ];
        for (name, letters) in cases {
            assert_eq!(initials(name), letters, "{name:?}");
        }
    }

    #[test]
    fn a_conflict_says_who_changed_what_the_page_shows() {
        let read = |text: &str| Block::text("b", BlockKind::Paragraph, text);
        let by = |author: &str, text: &str| Version {
            author: author.into(),
            edited: String::new(),
            block: Some(read(text)),
            extra: Default::default(),
        };
        // Who wrote the earlier version and the later, what the page reads of their block while it has
        // it, and what is said.
        let cases = [
            (("Sam", "Alice"), Some("later"), "Sam and Alice both changed this. The page shows the later version."),
            (("Sam", "Sam"), Some("later"), "Sam changed this in two places. The page shows the later version."),
            (
                ("", "Alice"),
                Some("later, and more"),
                "This was changed in two places. The page shows the later version.",
            ),
            (("Sam", "Alice"), Some("earlier"), "Sam and Alice both changed this. The page shows the earlier version."),
            (("Sam", "Alice"), None, "Sam and Alice both changed this. It is no longer on the page."),
        ];
        for ((one, other), page, says) in cases {
            let page = page.map(read);
            assert_eq!(conflict(&[by(one, "earlier"), by(other, "later")], page.as_ref()), says);
        }
    }
}
