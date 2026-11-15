//! History: the document as it was, version by version.
//!
//! The window swaps its bar's content for this view's, lists the versions beside the page and shows the
//! picked one where the page was. A version is read there and not written on: what changes the document
//! from here is restoring one, which is an edit that undo takes back.
//!
//! The model's history has a step for each minute of a writer's typing. A version is a run of steps with
//! no long rest in it and no night between: what was written at one sitting. A step that was marked ends
//! its version, so that a named version, an import, a merge or a restore is listed on its own.
//!
//! A version shows with what it changed, or with what has changed since: compared with the version before
//! it or with the document as it reads now. A block that was rewritten shows as it read before and after,
//! the page split there as it is at a conflict. A block that only gained words, or only lost some, shows
//! once with those words marked, and a line over blocks that only one version has says so.
//!
//! Under the page a strip has a cell for each version, to press on or drag along, and the bar names the
//! version that is picked: a name is a mark in the history, which is saved and is no edit.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use accesskit::{Node, NodeId, Role};
use letters_editor::{Key, KeyEvent};
use letters_layout::{Arrangement, DocLayout, Fonts, Palette, Piece, Side, Split};
use letters_model::{
    Block, BlockId, BlockKind, Body, Document, Point as Step, Pos, Revision, Selection, Style, Tag, TagKind,
};
use letters_render::image::Images;
use letters_render::{Color, DisplayList, Item, PathEl, Point, Rect, Size};

use crate::color::resolve;
use crate::describe::{self, Told};
use crate::element::{Cursor, Element, Paint, Pointer};
use crate::highlight;
use crate::icon;
use crate::motion::{self, FRAME_MS};
use crate::notes;
use crate::opening::{MONTHS, WEEKDAYS};
use crate::paint::scrollbar;
use crate::pictures;
use crate::review;
use crate::style::{Align, Edges, Look};
use crate::text::{Family, TextShaper, TextStyle};
use crate::theme::accent;
use crate::threads;
use crate::tokens::{self, Color as Ink, Scheme};
use crate::widgets::{Button, ControlSize, Menu, MenuItem, Segmented, SidePanel, TextField, TopBar};
use crate::window::{GUTTER, Message, PT, TOP};

/// How long writing rests before what comes after it is another version, in seconds.
const PAUSE: u64 = 600;
/// The keys of the bar's two forms, which are none of the editing bar's or Print view's.
const FORMS: (usize, usize) = (12, 13);
/// How far an arrow key scrolls.
const STEP: f32 = 48.0;
/// How much text one idle step lays out, in bytes. Small enough to fit between two frames.
const SETTLE_BYTES: usize = 4096;
/// The rail a day's versions hang on: the room it has across, the size of a version's marker on it, how
/// thick the rail and a marker's outline are, and the room a marker keeps clear of the rail.
const TRACK: f32 = 20.0;
const DOT: f32 = 10.0;
const LINE: f32 = 2.0;
const CLEAR: f32 = 3.0;
/// How tall the line of a version's title is, which its marker stands in the middle of.
const HEAD: f32 = 20.0;
/// The sign of a version that was given a name.
const SIGN: f32 = 12.0;
/// The room over a stretch of blocks that only one version has, for the line that says so. The bar beside
/// the stretch stands this far to the left of its text, and is dashed this long where the stretch was
/// taken out.
const CAPTION: f32 = 22.0;
const BAR_OUT: f32 = 16.0;
const DASH: f32 = 4.0;
/// How many times a version is split again before it is drawn, while laying out what shows changes heights.
const SETTLES: usize = 3;
/// How many versions that were read are kept, so that a step to the next version reads one and not two.
const KEPT: usize = 3;
/// The most characters of the words a block gained or lost that assistive technology is told.
const TOLD: usize = 160;
/// The most room the field takes that a version's name is typed in.
const NAME_WIDTH: f32 = 200.0;
/// The strip of versions under the page: how tall its bar is and how tall its cells are, the narrowest a
/// cell is set apart from the next, the narrowest a version that stands out is drawn, and how wide the
/// head at the picked version is and how far it reaches past the cells.
pub(crate) const STRIP: f32 = 44.0;
const CELL: f32 = 6.0;
const CELL_APART: f32 = 4.0;
const CELL_SEEN: f32 = 2.0;
const SCRUB: (f32, f32) = (3.0, 6.0);

/// What is asked of the history.
#[derive(Clone, Debug, PartialEq)]
pub enum Historying {
    /// Shows the document's versions, the newest picked.
    Show,
    /// Goes back to editing.
    Back,
    /// Picks a version, counted from the oldest.
    Pick(usize),
    /// Picks the version before the one picked, or the one after it.
    Step {
        back: bool,
    },
    /// The wheel turned over the version.
    Scroll(Point),
    /// A press began at a point of the version, or ended there.
    Down(Point),
    Up(Point),
    /// Shows the picked version against another, or as it read.
    Compare(Compare),
    /// The pointer went down on the strip of versions, or moved along it while held: this far across it,
    /// from 0 at the oldest version to 1.
    Scrub(f32),
    /// What is done about the picked version's name.
    Name(Naming),
    /// Makes the document read as the picked version again.
    Restore,
}

/// What the field sends that a version's name is typed in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Naming {
    /// Opens the name to be typed, as the version has it.
    Open,
    /// What the field holds.
    Typed(String),
    /// Puts the field away with the name taken. No name takes off the one the version had.
    Apply,
    /// Puts the field away and drops what was typed.
    Cancel,
}

/// What the picked version is compared with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compare {
    /// The version before it, which shows what the picked one changed.
    #[default]
    Previous,
    /// The document as it reads now, which shows what has changed since.
    Current,
    /// Nothing: the version shows as it read.
    Off,
}

impl Compare {
    const ALL: [(Self, &'static str); 3] =
        [(Self::Previous, "Previous"), (Self::Current, "Current"), (Self::Off, "Off")];
}

/// A version the history lists.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Version {
    /// The revision that shows it.
    pub at: Revision,
    /// When it was last written, in seconds since 1970. 0 when nobody told the model the time.
    time: u64,
    /// Who wrote it, each with how many of its steps are theirs. Whoever wrote most comes first.
    authors: Vec<(String, usize)>,
    /// How many characters of text it put in, and how many it took out.
    added: usize,
    removed: usize,
    /// Why it was marked, when it was.
    tag: Option<Tag>,
}

impl Version {
    fn kind(&self) -> Option<TagKind> {
        self.tag.as_ref().map(|tag| tag.kind)
    }

    /// The name a person gave it.
    fn name(&self) -> Option<&str> {
        let tag = self.tag.as_ref().filter(|tag| tag.kind == TagKind::Named)?;
        tag.name.as_deref().map(str::trim).filter(|name| !name.is_empty())
    }

    /// Whether it came from elsewhere: an import, a merge, or an earlier version restored.
    fn apart(&self) -> bool {
        matches!(self.kind(), Some(TagKind::Import | TagKind::Merge | TagKind::Restore))
    }

    /// The time of day it was last written at, on the person's clock.
    fn clock(&self, zone: i32) -> Option<String> {
        let of_day = local(self.time, zone).rem_euclid(86_400);
        (self.time > 0).then(|| format!("{:02}:{:02}", of_day / 3_600, of_day / 60 % 60))
    }

    /// What it is listed as: its name, what made it a version, or else the time it was written at. `first`
    /// and `newest` say whether it is the oldest and the newest of the history.
    fn title(&self, (first, newest): (bool, bool), zone: i32) -> String {
        let import = |tag: &Tag| match tag.name.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
            Some(file) => format!("Imported from {file}"),
            None => "Imported".to_owned(),
        };
        match (self.name(), &self.tag, self.clock(zone)) {
            (Some(name), ..) => name.to_owned(),
            _ if newest => "Current version".to_owned(),
            (_, Some(tag), _) if tag.kind == TagKind::Import => import(tag),
            (_, Some(tag), _) if tag.kind == TagKind::Merge => "Merged edits".to_owned(),
            (_, Some(tag), _) if tag.kind == TagKind::Restore => "Restored an earlier version".to_owned(),
            (.., Some(clock)) => clock,
            // What a document began with was written before the model was told the time.
            _ if first => "First version".to_owned(),
            _ => "Undated edits".to_owned(),
        }
    }

    /// Who wrote it, in a few words. `None` when nobody put a name to it.
    fn who(&self) -> Option<String> {
        let name = |at: usize| threads::named(&self.authors[at].0);
        match self.authors.len() {
            0 => None,
            1 => Some(name(0).to_owned()),
            2 => Some(format!("{} and {}", name(0), name(1))),
            many => Some(format!("{} and {} others", name(0), many - 1)),
        }
    }

    /// What the day it was written on is called on `today`: "Today", "Yesterday", the weekday for a week
    /// back, and the date before that.
    fn day(&self, today: Option<i64>, zone: i32) -> String {
        if self.time == 0 {
            return "Undated".to_owned();
        }
        let day = day(self.time, zone);
        match today.map(|today| today - day) {
            Some(0) => "Today".to_owned(),
            Some(1) => "Yesterday".to_owned(),
            Some(2..7) => WEEKDAYS[(day + 4).rem_euclid(7) as usize].to_owned(),
            _ => self.date(today, zone),
        }
    }

    /// The date it was written on: "6 Nov", with its year where that is not plain.
    fn date(&self, today: Option<i64>, zone: i32) -> String {
        let day = day(self.time, zone);
        let (year, month, date) = notes::date(u64::try_from(day).unwrap_or_default());
        let month = MONTHS[(month.clamp(1, 12) - 1) as usize];
        match today.map(|today| today - day) {
            // A date without its year is plain for most of a year back.
            Some(0..300) => format!("{date} {month}"),
            _ => format!("{date} {month} {year}"),
        }
    }
}

/// A moment on the person's clock, which is `zone` minutes ahead of UTC.
fn local(seconds: u64, zone: i32) -> i64 {
    i64::try_from(seconds).unwrap_or(i64::MAX).saturating_add(i64::from(zone) * 60)
}

/// The day a moment falls on where the person is, counted from 1970.
fn day(seconds: u64, zone: i32) -> i64 {
    local(seconds, zone).div_euclid(86_400)
}

/// A count of characters as the panel has room for it: in thousands from ten thousand on.
fn count(characters: usize) -> String {
    match characters {
        ..10_000 => characters.to_string(),
        _ => format!("{}k", characters / 1_000),
    }
}

/// How much a version put in and took out, in words. `None` for one that changed no text.
fn changed(added: usize, removed: usize) -> Option<String> {
    let characters = |count: usize| if count == 1 { "1 character".to_owned() } else { format!("{count} characters") };
    match (added, removed) {
        (0, 0) => None,
        (added, 0) => Some(format!("{} added", characters(added))),
        (0, removed) => Some(format!("{} removed", characters(removed))),
        (added, removed) => Some(format!("{} added, {removed} removed", characters(added))),
    }
}

/// The versions a history of `steps` lists, oldest first. Where a day ends is told by `zone`, which is how
/// far the person's clock is ahead of UTC in minutes.
fn versions(steps: &[Step], zone: i32) -> Vec<Version> {
    let mut versions: Vec<Version> = Vec::new();
    let mut open: Option<Version> = None;
    for step in steps {
        // A long rest or a night ends a version, and what was written after it is the next.
        let goes_on =
            |open: &Version| step.time.abs_diff(open.time) <= PAUSE && day(step.time, zone) == day(open.time, zone);
        if !open.as_ref().is_some_and(goes_on) {
            versions.extend(open.take());
        }
        let version = open.get_or_insert_with(|| Version {
            at: step.after.clone(),
            time: step.time,
            authors: Vec::new(),
            added: 0,
            removed: 0,
            tag: None,
        });
        (version.at, version.time) = (step.after.clone(), step.time);
        version.added += step.added;
        version.removed += step.removed;
        for (name, edits) in &step.authors {
            match version.authors.iter_mut().find(|(known, _)| known == name) {
                Some((_, count)) => *count += edits,
                None => version.authors.push((name.clone(), *edits)),
            }
        }
        // A step that was marked ends its version too. Each mark is a version, shown at its own revision,
        // and the first of them stands for what the run wrote.
        if let Some(run) = open.take_if(|_| !step.tags.is_empty()) {
            for (index, tag) in step.tags.iter().enumerate() {
                let (added, removed) = if index == 0 { (run.added, run.removed) } else { (0, 0) };
                let mut version = Version { at: tag.at.clone(), added, removed, tag: Some(tag.clone()), ..run.clone() };
                // What nobody dated or signed, as the steps of an import, is told by its mark.
                if version.time == 0 {
                    version.time = tag.time;
                }
                if version.authors.is_empty() {
                    version.authors.extend(tag.author.clone().map(|author| (author, 0)));
                }
                versions.push(version);
            }
        }
    }
    versions.extend(open);
    for version in &mut versions {
        version.authors.sort_by_key(|(_, edits)| std::cmp::Reverse(*edits));
    }
    versions
}

/// The version that holds what was written up to `revision`: the first that ends there or later. `None`
/// for a revision that no step of the history ends at.
fn holding(steps: &[Step], versions: &[Version], revision: &Revision) -> Option<usize> {
    let ends = |step: &Step, at: &Revision| step.after == *at || step.tags.iter().any(|tag| tag.at == *at);
    let (mut next, mut passed) = (0, false);
    for step in steps {
        passed |= ends(step, revision);
        // The versions are listed in the order of the steps they end at.
        while versions.get(next).is_some_and(|version| ends(step, &version.at)) {
            if passed {
                return Some(next);
            }
            next += 1;
        }
    }
    None
}

/// A stretch of blocks that only one of two versions has.
#[derive(Clone, Debug, PartialEq)]
struct Run {
    first: BlockId,
    last: BlockId,
    /// Whether the later version is the one that has them.
    added: bool,
    /// The first of its blocks that holds text, which the line over the stretch stands right above.
    text: Option<BlockId>,
}

/// How a block of the page differs between two versions.
#[derive(Clone, Debug, PartialEq)]
enum Sort {
    /// Only the later version has it, or only the earlier.
    Added,
    Removed,
    /// It has words that the earlier version does not, and all of that one's. The words are kept to be told.
    Gained(String),
    /// It lost words and gained none, so it reads as the earlier version has it.
    Lost(String),
    /// It reads otherwise in each, and both show.
    Rewritten,
}

/// How a later version of a document differs from an earlier one, as the page shows it.
#[derive(Clone, Debug, Default, PartialEq)]
struct Differences {
    /// The blocks of the page that differ, from its top down.
    sorts: Vec<(BlockId, Sort)>,
    /// The blocks that were rewritten, as the earlier version has them.
    pairs: Vec<Block>,
    /// The words of those that the later version does not have.
    gone: Vec<Selection>,
    /// The words on the page that one version does not have: the later version's own, and under
    /// [`Side::First`] those of a block that only lost words.
    words: Vec<(Selection, Side)>,
    runs: Vec<Run>,
}

/// The page that shows how `later` differs from `earlier`, and what differs on it. The page is the later
/// version with what only the earlier one has put back in: the blocks that were taken out, each behind the
/// block it followed, and a block that only lost words as it read with them. Blocks are compared where
/// they stand at the top of the document, and what lies inside one is part of it.
fn compare(earlier: &Document, later: &Document) -> (Document, Differences) {
    let was: HashMap<&BlockId, &Block> = earlier.blocks.iter().map(|block| (&block.id, block)).collect();
    let stays: HashSet<&BlockId> = later.walk().map(|block| &block.id).collect();
    // What was taken out, under the block it came after: `None` for what the document began with.
    let mut taken: HashMap<Option<BlockId>, Vec<&Block>> = HashMap::new();
    let mut after = None;
    for block in &earlier.blocks {
        match stays.contains(&block.id) {
            true => after = Some(block.id.clone()),
            false => taken.entry(after.clone()).or_default().push(block),
        }
    }

    let mut found = Differences::default();
    let mut page = Vec::with_capacity(earlier.blocks.len().max(later.blocks.len()));
    let mut put_back = |after: Option<&BlockId>, page: &mut Vec<Block>, found: &mut Differences| {
        for block in taken.remove(&after.cloned()).unwrap_or_default() {
            found.sorts.push((block.id.clone(), Sort::Removed));
            page.push(block.clone());
        }
    };
    put_back(None, &mut page, &mut found);
    for block in &later.blocks {
        let mut shows = block;
        match was.get(&block.id) {
            None => found.sorts.push((block.id.clone(), Sort::Added)),
            Some(old) if *old == block => {}
            Some(old) => {
                let [gone, came] = review::differing(old, block);
                let sort = match (gone.is_empty(), came.is_empty()) {
                    (true, false) => Sort::Gained(read(block, &came)),
                    (false, true) => {
                        shows = old;
                        let lost = read(old, &gone);
                        found.words.extend(gone.into_iter().map(|words| (words, Side::First)));
                        Sort::Lost(lost)
                    }
                    // A block that reads the same and looks otherwise has two versions as well.
                    _ => {
                        found.pairs.push((*old).clone());
                        found.gone.extend(gone);
                        Sort::Rewritten
                    }
                };
                found.words.extend(came.into_iter().map(|words| (words, Side::Second)));
                found.sorts.push((block.id.clone(), sort));
            }
        }
        page.push(shows.clone());
        put_back(Some(&block.id), &mut page, &mut found);
    }

    // Blocks that follow one another and were all added, or all taken out, are one stretch.
    let mut sorts = found.sorts.iter().peekable();
    let mut open: Option<Run> = None;
    for block in &page {
        let sort = sorts.next_if(|(id, _)| *id == block.id).map(|(_, sort)| sort);
        let added = match sort {
            Some(Sort::Added) => Some(true),
            Some(Sort::Removed) => Some(false),
            _ => None,
        };
        if open.as_ref().map(|run| run.added) != added {
            found.runs.extend(open.take());
        }
        if let Some(added) = added {
            let run =
                open.get_or_insert_with(|| Run { first: block.id.clone(), last: block.id.clone(), added, text: None });
            run.last = block.id.clone();
            run.text = run.text.take().or_else(|| written(block).map(|text| text.id.clone()));
        }
    }
    found.runs.extend(open);
    (Document { blocks: page, ..later.clone() }, found)
}

/// The first block that holds text in a block or under it.
fn written(block: &Block) -> Option<&Block> {
    match &block.body {
        Body::Text(_) => Some(block),
        _ => block.body.blocks().iter().find_map(written),
    }
}

/// The block of an id in a block or under it.
fn under<'a>(block: &'a Block, id: &BlockId) -> Option<&'a Block> {
    if block.id == *id {
        return Some(block);
    }
    block.body.blocks().iter().find_map(|part| under(part, id))
}

/// What stretches of a block's text read, one after another, for assistive technology: no more than
/// [`TOLD`] characters of them.
fn read(block: &Block, words: &[Selection]) -> String {
    let texts = words.iter().filter_map(|words| {
        let text = under(block, &words.anchor.block)?.plain_text();
        let (from, to) = (words.anchor.offset.min(words.head.offset), words.anchor.offset.max(words.head.offset));
        text.get(from..to).map(str::to_owned)
    });
    let all = texts.collect::<Vec<_>>().join(", ");
    match all.char_indices().nth(TOLD) {
        Some((end, _)) => format!("{}\u{2026}", all[..end].trim_end()),
        None => all,
    }
}

/// Makes room in a split page over the stretch of the page's column that `band` is, for the line that says
/// which version has it. The stretch becomes a piece of its own, under the first of its blocks.
fn part(split: &mut Split, first: &BlockId, band: (f32, f32)) {
    let holds = |piece: &Piece| piece.block.is_none() && piece.band.0 <= band.0 && band.0 < piece.band.1;
    let Some(at) = split.pieces.iter().position(holds) else { return };
    let whole = split.pieces.remove(at);
    let end = band.1.min(whole.band.1);
    let cuts = [
        ((whole.band.0, band.0), None, 0.0),
        ((band.0, end), Some(first.clone()), CAPTION),
        ((end, whole.band.1), None, 0.0),
    ];
    let mut y = whole.rect.y;
    let mut pieces = Vec::with_capacity(cuts.len());
    for (band, block, head) in cuts {
        if band.1 > band.0 {
            let rect = Rect::new(whole.rect.x, y, whole.rect.width, head + band.1 - band.0);
            let origin = Point { x: whole.origin.x, y: y + head - band.0 };
            pieces.push(Piece { side: Side::Second, block, rect, head, origin, band });
            y += rect.height;
        }
    }
    for piece in &mut split.pieces[at..] {
        piece.rect.y += CAPTION;
        piece.origin.y += CAPTION;
    }
    split.pieces.splice(at..at, pieces);
    split.height += CAPTION;
}

/// The part of its version's column that a piece of a split page shows in `view`, a part of what the
/// split counts from.
fn cut(piece: &Piece, view: Rect) -> Option<Rect> {
    let top = (view.y - piece.origin.y).max(piece.band.0);
    let bottom = (view.y + view.height - piece.origin.y).min(piece.band.1);
    (top < bottom).then(|| Rect::new(view.x - piece.origin.x, top, view.width, bottom - top))
}

/// Who wrote one of the two versions that are compared, and when.
#[derive(Clone, Debug, Default)]
struct Head {
    /// The writer whose face shows, and everyone who wrote it in a few words.
    face: Option<String>,
    who: Option<String>,
    when: String,
}

impl Head {
    /// What assistive technology is told of the version, after which of the two it is.
    fn told(&self, label: &str) -> String {
        match &self.who {
            Some(who) => format!("{label}, by {who}, {}", self.when),
            None => format!("{label}, {}", self.when),
        }
    }

    /// Draws the line at the head of one version of a rewritten block, inside the top of its frame: which
    /// of the two it is, who wrote it and when. The time goes first where the frame is narrow.
    fn paint(&self, side: Side, frame: Rect, shaper: &dyn TextShaper, scheme: Scheme, out: &mut DisplayList) {
        let ink = |ink: Ink| resolve(ink, scheme, 1.0);
        let small = TextStyle::control().size(tokens::TEXT_SM);
        let (label, tone) = match side {
            Side::First => ("Before", tokens::WARN),
            Side::Second => ("After", tokens::INFO),
        };
        let (left, right) =
            (frame.x + review::MARGIN - review::INSET, frame.x + frame.width - review::MARGIN + review::INSET);
        let middle = frame.y + review::RULE + ((review::HEAD - review::RULE) / 2.0).round();
        let gap = tokens::SPACE_SM;
        out.push(Item::PushClip { rect: Rect::new(left, frame.y, (right - left).max(0.0), review::HEAD), radius: 0.0 });
        // Says a word at `x`, when it has to be said or there is room for it, and returns where the next goes.
        let say = |text: &str, style: &TextStyle, color: Ink, x: f32, must: bool, out: &mut DisplayList| {
            let word = shaper.shape(text, style, None);
            if must || x + word.size.width <= right {
                word.paint(Point { x, y: middle - (word.size.height / 2.0).round() }, ink(color), out);
            }
            x + word.size.width + gap
        };
        let mut x = say(label, &small.weight(600), tone, left, true, out);
        if let Some(name) = &self.face {
            review::face(name, Point { x, y: middle }, shaper, scheme, out);
            x += review::FACE + gap;
        }
        if let Some(who) = &self.who {
            x = say(who, &small, tokens::FG, x, true, out);
        }
        say(&self.when, &small, tokens::FG_MUTED, x, false, out);
        out.push(Item::PopClip);
    }
}

/// The version that is picked, read and laid out: as it read, or as it differs from another.
struct Leaf {
    /// The revision the page is of and, where that is compared with an earlier one, the earlier one.
    between: (Option<Revision>, Revision),
    /// The page: the later version, with what only the earlier one has put back in.
    doc: Document,
    style: Style,
    layout: DocLayout,
    /// The page with each rewritten block as the earlier version has it, laid out for that side of the
    /// pairs. `None` when no block has two versions.
    earlier: Option<(Document, DocLayout)>,
    found: Differences,
    /// The rewritten blocks, which the page is split at.
    pairs: Vec<BlockId>,
    /// Who wrote the earlier version and the later, and when.
    heads: [Head; 2],
    /// What the line over a stretch says that was taken out, and over one that was added.
    captions: [String; 2],
    /// What assistive technology is told of the blocks that differ.
    said: HashMap<BlockId, String>,
    /// The width it is laid out at, once it is.
    laid: Option<f32>,
    /// Whether some of its heights are still estimates.
    estimated: bool,
}

/// What the history keeps while it shows.
pub(crate) struct History {
    /// Oldest first, and never empty.
    versions: Vec<Version>,
    /// The version picked.
    at: usize,
    /// What it is compared with.
    compare: Compare,
    /// What the field holds that its name is typed in, while that is open.
    naming: Option<String>,
    /// The versions the window read last.
    read: Vec<(Revision, Rc<(Document, Style)>)>,
    /// What the page area shows of it, once the window has read it.
    leaf: Option<Leaf>,
    /// How far down the version is scrolled, which stays as it is from one version to the next: the same
    /// part of the document is looked at in each.
    scroll: f32,
    /// The room the last frame had, how tall what scrolls was, and how many device pixels a pixel is.
    room: Size,
    tall: f32,
    scale: f32,
    /// The time when the history was shown, when the shell has a clock, and how far the person's clock is
    /// ahead of UTC in minutes.
    now: Option<u64>,
    zone: i32,
    /// The version the window is pinned to where it has no room for both of a rewritten block: After,
    /// until the other is brought across. Then how far the pairs have slid there, and when they set off.
    active: Side,
    slide: f32,
    since: Option<u64>,
    /// Whether what moves may be shown moving.
    pub still: bool,
    /// Where the pointer rests in the page area, and the button a press began on.
    pointer: Option<Point>,
    pressed: Option<Side>,
    /// What was last drawn: how the pairs shared the window, where the versions of the rewritten blocks
    /// in view stand in the page area, and the buttons that bring a version across.
    arrangement: Option<Arrangement>,
    frames: Vec<(BlockId, Side, Rect)>,
    hot: Vec<(Rect, Side)>,
}

impl History {
    /// The history of a document that reads as `current`, with its newest version picked.
    pub fn new(steps: &[Step], current: Revision, now: Option<u64>, zone: i32) -> Self {
        let mut history = Self {
            versions: Vec::new(),
            at: 0,
            compare: Compare::default(),
            naming: None,
            read: Vec::new(),
            leaf: None,
            scroll: 0.0,
            room: Size::default(),
            tall: 0.0,
            scale: 1.0,
            now,
            zone,
            active: Side::Second,
            slide: 1.0,
            since: None,
            still: false,
            pointer: None,
            pressed: None,
            arrangement: None,
            frames: Vec::new(),
            hot: Vec::new(),
        };
        history.follow(steps, current);
        history
    }

    /// Lists the versions again after the document changed under the history, as when another writer's
    /// edits came in. The version that was picked stays picked while it is still listed.
    pub fn follow(&mut self, steps: &[Step], current: Revision) {
        let picked = self.versions.get(self.at).map(|version| version.at.clone());
        self.versions = versions(steps, self.zone);
        // The document as it reads now is always the last version, marked or not.
        let newest = steps.last().map_or(&current, |step| &step.after);
        if self.versions.last().is_none_or(|last| last.at != *newest) {
            let time = steps.last().map_or(0, |step| step.time);
            let (authors, tag) = (Vec::new(), None);
            self.versions.push(Version { at: newest.clone(), time, authors, added: 0, removed: 0, tag });
        }
        // A version that is listed no longer, as one whose name was taken off, is part of the one that
        // holds what it wrote.
        let kept = picked.and_then(|picked| {
            let listed = self.versions.iter().position(|version| version.at == picked);
            listed.or_else(|| holding(steps, &self.versions, &picked))
        });
        self.at = kept.unwrap_or(self.versions.len() - 1);
    }

    pub fn picked(&self) -> &Version {
        &self.versions[self.at]
    }

    /// Whether the version picked is the document as it reads now, which there is nothing to restore of.
    pub fn newest(&self) -> bool {
        self.at + 1 == self.versions.len()
    }

    /// The two versions that are compared, the earlier first: the picked one and the one it is compared
    /// with. `None` where the picked version shows as it read, which it also does with nothing before it
    /// and when it is the document as it reads now.
    fn sides(&self) -> Option<(usize, usize)> {
        let newest = self.versions.len() - 1;
        match self.compare {
            Compare::Previous => self.at.checked_sub(1).map(|before| (before, self.at)),
            Compare::Current => (self.at < newest).then_some((self.at, newest)),
            Compare::Off => None,
        }
    }

    /// The revision the page is to be of and, where it is compared with an earlier one, the earlier one.
    fn between(&self) -> (Option<Revision>, Revision) {
        match self.sides() {
            Some((earlier, later)) => (Some(self.versions[earlier].at.clone()), self.versions[later].at.clone()),
            None => (None, self.picked().at.clone()),
        }
    }

    /// The revisions to read before what is picked can show.
    pub fn wanted(&self) -> Vec<Revision> {
        let between = self.between();
        if self.leaf.as_ref().is_some_and(|leaf| leaf.between == between) {
            return Vec::new();
        }
        let known = |revision: &Revision| self.read.iter().any(|(read, _)| read == revision);
        between.0.into_iter().chain([between.1]).filter(|revision| !known(revision)).collect()
    }

    /// Takes a revision as the window read it.
    pub fn take(&mut self, revision: Revision, read: (Document, Style)) {
        self.read.push((revision, Rc::new(read)));
    }

    /// Shows what is picked, from the revisions the window read. Without the revision that the page is of
    /// the page area stays empty, and without the one to compare it with the version shows as it read.
    pub fn show(&mut self) {
        let between = self.between();
        if self.leaf.as_ref().is_some_and(|leaf| leaf.between == between) {
            return;
        }
        let read =
            |revision: &Revision| self.read.iter().find(|(read, _)| read == revision).map(|(_, read)| read.clone());
        let (earlier, later) = (between.0.as_ref().and_then(read), read(&between.1));
        let sides = self.sides().filter(|_| earlier.is_some());
        let heads = sides.map(|(earlier, later)| [earlier, later].map(|at| self.head(at))).unwrap_or_default();
        // What changed from the version before is its writer's. What changed since is many people's.
        let by = sides.and_then(|(_, later)| self.versions[later].who());
        let captions = match (self.compare, by) {
            (Compare::Current, _) => ["Removed later", "Added later"].map(str::to_owned),
            (_, Some(by)) => [format!("Removed by {by}"), format!("Added by {by}")],
            (_, None) => ["Removed", "Added"].map(str::to_owned),
        };
        self.leaf = later.map(|later| Leaf::new(between, earlier.as_deref(), &later, heads, captions));
        // The versions just shown are the ones a step is most likely to want again.
        let spare = self.read.len().saturating_sub(KEPT);
        self.read.drain(..spare);
        (self.frames, self.hot, self.pressed) = (Vec::new(), Vec::new(), None);
    }

    /// Who wrote a version and when, for the line at the head of its side of a rewritten block.
    fn head(&self, at: usize) -> Head {
        let version = &self.versions[at];
        Head {
            face: version.authors.first().map(|(name, _)| name.clone()),
            who: version.who(),
            when: self.when(version),
        }
    }

    pub fn pick(&mut self, at: usize) {
        self.at = at.min(self.versions.len() - 1);
        // After is the version the window shows whole, until the other is asked for.
        (self.active, self.slide, self.since) = (Side::Second, 1.0, None);
    }

    /// Compares the picked version with another, or with none.
    pub fn compare(&mut self, with: Compare) {
        self.compare = with;
        (self.active, self.slide, self.since) = (Side::Second, 1.0, None);
    }

    /// The version that stands so far across the strip, from 0 at the oldest to 1.
    pub fn across(&self, part: f32) -> usize {
        let count = self.versions.len();
        ((part.clamp(0.0, 1.0) * count as f32) as usize).min(count - 1)
    }

    pub fn at(&self) -> usize {
        self.at
    }

    /// Opens the picked version's name to be typed, takes what the field holds, or drops it.
    pub fn name(&mut self, asked: Naming) {
        match asked {
            Naming::Open => self.naming = Some(self.picked().name().unwrap_or_default().to_owned()),
            Naming::Typed(typed) => self.naming = self.naming.as_ref().map(|_| typed),
            Naming::Apply | Naming::Cancel => self.naming = None,
        }
    }

    /// Puts the name field away. Returns the picked version's revision and what to call it, or `None` to
    /// take its name off, when what was typed is not what it is called already. With nothing typed, a
    /// version that has no name of its own stays marked as it was.
    pub fn named(&mut self) -> Option<(Revision, Option<String>)> {
        let typed = self.naming.take()?;
        let (version, name) = (self.picked(), typed.trim());
        let changed = version.name().unwrap_or_default() != name;
        changed.then(|| (version.at.clone(), Some(name.to_owned()).filter(|name| !name.is_empty())))
    }

    /// Picks the version before the one picked, or the one after it, when there is one.
    pub fn step(&mut self, back: bool) {
        self.pick(if back { self.at.saturating_sub(1) } else { self.at + 1 });
    }

    /// Takes faces that arrived late.
    pub fn add_fonts(&mut self, fonts: &Fonts) {
        if let Some(leaf) = &mut self.leaf {
            let earlier = leaf.earlier.as_mut().is_some_and(|(_, layout)| layout.add_fonts(fonts));
            if leaf.layout.add_fonts(fonts) || earlier {
                leaf.estimated = true;
            }
        }
    }

    pub fn scroll(&mut self, delta: Point) {
        self.scroll += delta.y;
        self.keep();
    }

    /// Scrolls for a key that moves through the version, and says whether `event` was one.
    pub fn key(&mut self, event: &KeyEvent) -> bool {
        if event.mods.primary || event.mods.alt {
            return false;
        }
        let screen = 0.9 * self.room.height;
        match event.key {
            Key::Down => self.scroll += STEP,
            Key::Up => self.scroll -= STEP,
            Key::PageDown => self.scroll += screen,
            Key::PageUp => self.scroll -= screen,
            Key::Home => self.scroll = 0.0,
            Key::End => self.scroll = f32::MAX,
            _ => return false,
        }
        self.keep();
        true
    }

    /// Keeps the scroll within what there is to show.
    fn keep(&mut self) {
        self.scroll = self.scroll.clamp(0.0, (self.tall - self.room.height).max(0.0));
    }

    /// How far down the version shows: the nearest whole device pixel to where it was scrolled to, so
    /// that hinted text stays sharp.
    fn scrolled(&self) -> f32 {
        (self.scroll * self.scale).round() / self.scale
    }

    /// The part of the version in view: of what scrolls, counted from under the room at its top.
    fn viewport(&self) -> Rect {
        Rect::new(0.0, self.scrolled() - TOP, self.room.width, self.room.height)
    }

    /// Lays out some of what is out of view, in idle time. The line at the top of the view stays where it
    /// is while what is above it changes height. Returns whether there was something to lay out.
    pub fn settle(&mut self) -> bool {
        let (view, room, pinned) = (self.viewport(), self.room.width, (self.active, self.slide));
        let Some(leaf) = self.leaf.as_mut().filter(|leaf| leaf.estimated && leaf.laid.is_some()) else {
            return false;
        };
        let split = leaf.split(room, pinned);
        let pin = leaf.layout.hit(Point { x: 0.0, y: review::shown(&split, view).0.max(0.0) });
        let top = |leaf: &Leaf, split: &Split| {
            let line = pin.as_ref().and_then(|pos| leaf.layout.caret(pos))?.y;
            Some(line + review::origin(split, line).y)
        };
        let before = top(leaf, &split);
        leaf.settle(&split, view, SETTLE_BYTES);
        if let (Some(before), Some(after)) = (before, top(leaf, &leaf.split(room, pinned))) {
            self.scroll += after - before;
        }
        true
    }

    /// Steps the slide of the pairs to the version that was brought across. Returns when it wants to be
    /// drawn next.
    pub fn animate(&mut self, now_ms: u64) -> Option<u64> {
        if self.slide >= 1.0 {
            return None;
        }
        let since = *self.since.get_or_insert(now_ms);
        let progress = now_ms.saturating_sub(since) as f32 / review::SLIDE_MS;
        self.slide = if progress < 1.0 && !self.still { motion::ease(progress) } else { 1.0 };
        (self.slide < 1.0).then_some(now_ms + FRAME_MS)
    }

    /// Makes `side` the version the window shows whole where it has no room for two, sliding there.
    fn bring(&mut self, side: Side) {
        if self.active != side {
            let slides = self.arrangement == Some(Arrangement::Pinned) && !self.still;
            (self.active, self.since) = (side, None);
            self.slide = if slides { 0.0 } else { 1.0 };
        }
    }

    /// The button at a point of the page area, by the version it brings across.
    fn button(&self, at: Point) -> Option<Side> {
        self.hot.iter().find(|(rect, _)| rect.contains(at)).map(|(_, side)| *side)
    }

    /// Takes where the pointer rests in the page area. Returns whether what shows changed with it.
    pub fn point(&mut self, at: Option<Point>) -> bool {
        let changed = at.and_then(|at| self.button(at)) != self.pointer.and_then(|at| self.button(at));
        self.pointer = at;
        changed
    }

    /// Takes a press at a point of the page area. One on a version of a rewritten block brings that
    /// version across, and one on a button waits to end there.
    pub fn press(&mut self, at: Point) {
        self.pressed = self.button(at);
        let on = self.frames.iter().find(|(_, _, frame)| frame.contains(at)).map(|(_, side, _)| *side);
        if let Some(side) = on.filter(|_| self.pressed.is_none()) {
            self.bring(side);
        }
    }

    /// Lets go at a point of the page area. A button does what it is for when the press ends on it.
    pub fn release(&mut self, at: Point) {
        if let Some(side) = self.pressed.take().filter(|side| self.button(at) == Some(*side)) {
            self.bring(side);
        }
    }

    /// The picked version, for the area where the page was. Also returns the pictures in it that the
    /// shell has not handed over yet, which keep their place.
    pub fn view(
        &mut self,
        room: Size,
        images: &Images,
        shaper: &dyn TextShaper,
        palette: Palette,
        scheme: Scheme,
        scale: f32,
    ) -> (Element<Message>, Vec<String>) {
        (self.room, self.scale) = (room, scale);
        (self.frames, self.hot) = (Vec::new(), Vec::new());
        let mut list = DisplayList::default();
        if let Some(mut leaf) = self.leaf.take() {
            self.draw(&mut leaf, images, shaper, palette, scheme, &mut list);
            self.leaf = Some(leaf);
        }
        let awaited = pictures::awaited(images, scheme, &mut list);
        let paint = Page { list, page: palette.page, content: self.tall, scrolled: self.scrolled() };
        let mut canvas = Element::canvas(Rc::new(paint))
            .grow(1.0)
            .fg(tokens::FG)
            .role(Role::Document)
            .label("History")
            .default_focus()
            .on_pointer(|event| match event {
                Pointer::Scroll { delta } => Some(Message::History(Historying::Scroll(delta))),
                Pointer::Down { at, .. } => Some(Message::History(Historying::Down(at))),
                Pointer::Up { at } => Some(Message::History(Historying::Up(at))),
                _ => None,
            })
            .pointed();
        if self.pointer.is_some_and(|at| self.button(at).is_some()) {
            canvas = canvas.cursor(Cursor::Pointer);
        }
        (canvas, awaited)
    }

    /// Lays out what shows of a version and draws it into `list`, in the page area's coordinates.
    fn draw(
        &mut self,
        leaf: &mut Leaf,
        images: &Images,
        shaper: &dyn TextShaper,
        palette: Palette,
        scheme: Scheme,
        list: &mut DisplayList,
    ) {
        let room = self.room;
        // The version is set as the page is: as wide as its paper leaves between the margins, or as the
        // window has room for.
        let paper = &leaf.style.page;
        let text = (paper.width - paper.margin_left - paper.margin_right) * PT;
        let width = text.min(room.width - 2.0 * GUTTER).max(1.0);
        leaf.lay(width, images, palette);

        // What is about to show is laid out for real, and the page split again while that changes
        // heights. The rest waits for idle time.
        let pinned = (self.active, self.slide);
        let mut split = leaf.split(room.width, pinned);
        for _ in 0..SETTLES {
            self.tall = split.height + 2.0 * TOP;
            self.keep();
            if !leaf.settle(&split, self.viewport(), 0) {
                break;
            }
            split = leaf.split(room.width, pinned);
        }
        self.tall = split.height + 2.0 * TOP;
        self.keep();
        self.arrangement = Some(split.arrangement);

        let view = self.viewport();
        // A split stands the page's column in the very middle, and the page area on a whole pixel beside it.
        let middle = ((room.width - width) / 2.0).max(0.0);
        let shift = Point { x: middle.round() - middle, y: -view.y };
        let ink = |ink: Ink| resolve(ink, scheme, 1.0);
        for piece in &split.pieces {
            let (top, bottom) = (piece.rect.y + shift.y, piece.rect.y + piece.rect.height + shift.y);
            if bottom <= 0.0 || top >= room.height {
                continue;
            }
            let origin = Point { x: piece.origin.x + shift.x, y: piece.origin.y + shift.y };
            let block = piece.block.as_ref();
            if let Some(block) = block.filter(|block| leaf.pairs.contains(block)) {
                // One version of a rewritten block: a frame round it, and a line at its head.
                let frame = review::frame(piece, width, shift, room.width);
                let (tone, head) = match piece.side {
                    Side::First => (tokens::WARN, &leaf.heads[0]),
                    Side::Second => (tokens::INFO, &leaf.heads[1]),
                };
                list.push(Item::Stroke { rect: frame, color: ink(tokens::LINE), width: 1.0, radius: 0.0 });
                list.push(Item::Fill { rect: Rect { height: review::RULE, ..frame }, color: ink(tone), radius: 0.0 });
                head.paint(piece.side, frame, shaper, scheme, list);
                self.frames.push((block.clone(), piece.side, frame));
            } else if let Some(run) = block.and_then(|block| leaf.found.runs.iter().find(|run| run.first == *block)) {
                // A stretch that only one version has: a bar beside it, and a line over its first line.
                let (tone, caption) = match run.added {
                    true => (tokens::INFO, &leaf.captions[1]),
                    false => (tokens::WARN, &leaf.captions[0]),
                };
                let first = run.text.as_ref().and_then(|text| leaf.layout.caret(&Pos::new(text.clone(), 0)));
                let over = origin.y + first.map_or(piece.band.0, |line| line.y) - CAPTION;
                let said = shaper.shape(caption, &TextStyle::control().size(tokens::TEXT_SM).weight(600), None);
                said.paint(
                    Point { x: origin.x, y: over + ((CAPTION - said.size.height) / 2.0).round() },
                    ink(tone),
                    list,
                );
                // What was taken out has a broken bar, so that the two are told apart without their colours.
                let (dash, gap) = if run.added { (bottom - over, 0.0) } else { (DASH, DASH) };
                let mut y = over;
                while y < bottom {
                    let rect = Rect::new(origin.x - BAR_OUT, y, review::RULE, dash.min(bottom - y));
                    list.push(Item::Fill { rect, color: ink(tone), radius: 0.0 });
                    y += dash + gap;
                }
            }
            let Some(cut) = cut(piece, view) else { continue };
            let layout = match (piece.side, &leaf.earlier) {
                (Side::First, Some((_, earlier))) => earlier,
                _ => &leaf.layout,
            };
            list.push(Item::PushClip { rect: review::text(piece, shift), radius: 0.0 });
            let from = list.items.len();
            match piece.side {
                Side::First => {
                    for words in &leaf.found.gone {
                        review::mark(&layout.selection_in(words, cut), Side::First, scheme, list);
                    }
                }
                Side::Second => {
                    for (words, side) in &leaf.found.words {
                        review::mark(&layout.selection_in(words, cut), *side, scheme, list);
                    }
                }
            }
            layout.paint(cut, list);
            list.translate(from, origin);
            list.push(Item::PopClip);
        }

        // Of pinned pairs the window shows one version whole. The other lies under a veil, with a button
        // at the window's edge that brings it across.
        if split.arrangement == Arrangement::Pinned && self.slide >= 1.0 {
            let aside = if self.active == Side::First { Side::Second } else { Side::First };
            let (pointer, down) = (self.pointer, self.pressed);
            let lit =
                |button: Rect| pointer.is_some_and(|at| button.contains(at)) && down.is_none_or(|down| down == aside);
            for (_, _, frame) in self.frames.iter().filter(|(_, side, _)| *side == aside) {
                let button = review::veil(*frame, aside, room, palette.page, &lit, scheme, list);
                self.hot.extend(button.map(|button| (button, aside)));
            }
        }
    }

    /// Fills the bar while the history shows: the way back, what the version is of and when it was
    /// written, what it is compared with, and the button that restores it. A narrow window has the bar
    /// folded.
    pub fn bar(&self, bar: TopBar<Message>, name: &str) -> TopBar<Message> {
        let back = Message::History(Historying::Back);
        let mut compare = Segmented::new("Compare with").small();
        let mut compares = Menu::new().label("Compare with");
        for (with, label) in Compare::ALL {
            let pick = Message::History(Historying::Compare(with));
            compare = compare.segment(label, self.compare == with, pick.clone());
            compares = compares.item(MenuItem::new(label).checked(self.compare == with).on_select(pick));
        }
        let with = Compare::ALL.iter().find(|(with, _)| *with == self.compare).map_or("Off", |(_, label)| label);
        let name = || Element::text(name, TextStyle::control().weight(600)).min_width(tokens::SPACE_3XL).shrink();
        let when = Element::text(self.said(), TextStyle::control().size(tokens::TEXT_SM));
        // The picked version's name is typed where the button stood that asks for it.
        let naming = |button: Button<Message>| -> Element<Message> {
            let send = |asked: Naming| Message::History(Historying::Name(asked));
            let Some(typed) = &self.naming else { return button.small().on_press(send(Naming::Open)).into() };
            let field = TextField::new("Version name", typed)
                .placeholder("Name this version")
                .size(ControlSize::Small)
                .on_input(|typed| Message::History(Historying::Name(Naming::Typed(typed))))
                .on_submit(send(Naming::Apply))
                .on_cancel(send(Naming::Cancel))
                // Leaving the field takes the name, as leaving the field of the document's own name does.
                .on_leave(send(Naming::Apply));
            Element::from(field).key("naming").width(NAME_WIDTH).shrink().takes_focus()
        };
        // The document as it reads now is what a restore would leave: there is nothing to do.
        let restore = |button: Button<Message>| {
            let button = button.primary().small().disabled(self.newest());
            button.on_press(Message::History(Historying::Restore))
        };
        let bar = bar
            .apart()
            .label("History")
            .form(FORMS.0)
            .start(Button::new("Back to editing").icon(icon::ARROW_LEFT).small().on_press(back.clone()))
            .start(name())
            .start(when.fg(tokens::FG_MUTED).shrink())
            .middle(compare)
            .middle(TopBar::rule())
            .middle(naming(Button::new("Name").icon(icon::TAG)))
            .middle(restore(Button::new("Restore this version").icon(icon::ROTATE_CCW)));
        bar.form(FORMS.1)
            .start(Button::icon_only(icon::ARROW_LEFT, "Back to editing").small().on_press(back))
            .start(name())
            .middle(Element::from(Button::new(with).small().menu(compares)).description("Compare with"))
            .middle(TopBar::rule())
            .middle(naming(Button::icon_only(icon::TAG, "Name")))
            .middle(restore(Button::icon_only(icon::ROTATE_CCW, "Restore this version")))
    }

    /// The strip under the page and the panel: a cell for each version, the oldest first, with a head at
    /// the one that is picked. A press on it picks the version there, and a drag goes through them.
    pub fn strip(&self) -> Element<Message> {
        let quiet = TextStyle::control().size(tokens::TEXT_SM);
        let label = |text: String| Element::text(text, quiet).fg(tokens::FG_MUTED);
        // The strip runs from the day of the oldest version that has one to now.
        let first = self.versions.iter().find(|version| version.time > 0);
        let first = first.map(|version| label(version.date(self.today(), self.zone)));
        let out = self.versions.iter().map(|version| version.name().is_some() || version.apart()).collect();
        let track = Element::canvas(Rc::new(Strip { out, at: self.at }))
            .grow(1.0)
            .height(STRIP - 1.0)
            .passive()
            .role(Role::Image)
            .label("Versions, oldest first")
            .description(format!("{} of {}", self.at + 1, self.versions.len()))
            .cursor(Cursor::Pointer)
            .spread()
            .on_pointer(|event| match event {
                Pointer::Down { at, .. } | Pointer::Drag { at } => Some(Message::History(Historying::Scrub(at.x))),
                _ => None,
            });
        // The keys that step through the versions, as a reminder beside them.
        let key = |name: &str| {
            let name = Element::text(name, TextStyle::control().size(tokens::TEXT_XS)).fg(tokens::FG_MUTED);
            Element::row()
                .padding(Edges::xy(tokens::SPACE_XS, 1.0))
                .radius(tokens::RADIUS_SM)
                .bg(tokens::NEUTRAL_SOFT)
                .child(name)
        };
        Element::row()
            .height(STRIP)
            .align(Align::Center)
            .gap(tokens::SPACE_MD)
            .padding(Edges::xy(tokens::SPACE_LG, 0.0))
            .bg(tokens::SURFACE)
            .border(Edges::top(1.0), tokens::LINE)
            .children(first)
            .child(track)
            .child(label("Now".to_owned()))
            .child(Element::row().gap(tokens::SPACE_XS).child(key("Left")).child(key("Right")).decorative())
    }

    /// When the picked version was written and by whom, as the bar says it: "Tuesday 16:42, by Alice
    /// Moreau".
    fn said(&self) -> String {
        let version = self.picked();
        let when = self.when(version);
        match version.who() {
            Some(who) => format!("{when}, by {who}"),
            None => when,
        }
    }

    /// The day and the time a version was last written at, on the person's clock.
    fn when(&self, version: &Version) -> String {
        let day = version.day(self.today(), self.zone);
        version.clock(self.zone).map_or_else(|| day.clone(), |clock| format!("{day} {clock}"))
    }

    fn today(&self) -> Option<i64> {
        self.now.map(|now| day(now, self.zone))
    }

    /// The days the versions were written on, newest first, each with what it is called and its versions.
    fn days(&self) -> Vec<(String, Vec<usize>)> {
        let today = self.today();
        let mut days: Vec<(String, Vec<usize>)> = Vec::new();
        for at in (0..self.versions.len()).rev() {
            let name = self.versions[at].day(today, self.zone);
            match days.last_mut().filter(|(last, _)| *last == name) {
                Some((_, listed)) => listed.push(at),
                None => days.push((name, vec![at])),
            }
        }
        days
    }

    /// The panel beside the version: every version, newest first under the day it was written on. A press
    /// on one picks it.
    pub fn panel(&self) -> Element<Message> {
        let count = match self.versions.len() {
            1 => "1 version".to_owned(),
            count => format!("{count} versions"),
        };
        let mut panel = SidePanel::new("History").aside(count);
        for (name, listed) in self.days() {
            let rows = listed.iter().enumerate().map(|(place, at)| {
                let ends = (place == 0, place + 1 == listed.len());
                self.row(*at, ends)
            });
            let list = Element::column().key(listed[0]).role(Role::ListBox).label(name.as_str()).children(rows);
            panel = panel.child(SidePanel::<Message>::section(&name)).child(list);
        }
        panel.into()
    }

    /// One version in the panel: its marker on the day's rail, what it is listed as, who wrote it and how
    /// much it changed. `ends` says whether it is the first and the last of its day.
    fn row(&self, at: usize, (first, last): (bool, bool)) -> Element<Message> {
        let version = &self.versions[at];
        let (newest, picked) = (at + 1 == self.versions.len(), at == self.at);
        let small = TextStyle::control().size(tokens::TEXT_SM);
        let quiet = TextStyle::control().size(tokens::TEXT_XS);
        let title = version.title((at == 0, newest), self.zone);
        let who = version.who();
        // The time is said once: by the title, or else after who wrote the version.
        let when = version.clock(self.zone).filter(|clock| *clock != title);
        let size = changed(version.added, version.removed);

        let mut head = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .min_height(HEAD)
            .child(Element::text(title.as_str(), small).shrink())
            .child(Element::spacer());
        if version.name().is_some() {
            let named = Element::row()
                .align(Align::Center)
                .gap(tokens::SPACE_2XS)
                .padding(Edges::xy(tokens::SPACE_XS, 1.0))
                .border(Edges::all(1.0), tokens::LINE)
                .radius(tokens::RADIUS_SM)
                .child(Element::icon(icon::TAG, SIGN))
                .child(Element::text("Named", quiet));
            head = head.child(named.fg(tokens::FG_MUTED));
        } else if size.is_some() {
            // What it put in and what it took out, in the colours those have on the page.
            let figures = quiet.family(Family::Mono);
            let put_in = Element::text(format!("+{}", count(version.added)), figures).fg(tokens::INFO);
            let taken_out = Element::text(format!("\u{2212}{}", count(version.removed)), figures).fg(tokens::WARN);
            head = head.child(Element::row().gap(tokens::SPACE_XS).child(put_in).child(taken_out).decorative());
        }
        let mut body =
            Element::column().grow(1.0).gap(tokens::SPACE_2XS).padding(Edges::xy(0.0, tokens::SPACE_SM)).child(head);
        let by: Vec<&str> = [who.as_deref(), when.as_deref()].into_iter().flatten().collect();
        if !by.is_empty() {
            let face = version.authors.first().map(|(name, _)| threads::face(name, false));
            let by = Element::text(by.join(" \u{b7} "), small).fg(tokens::FG_MUTED).shrink();
            body = body.child(Element::row().align(Align::Center).gap(tokens::SPACE_SM).children(face).child(by));
        }

        // What made it a version of its own shows in its marker: the one that is the document now, and
        // one that came from elsewhere.
        let apart = version.apart();
        let ink = match (newest, apart) {
            (true, _) => accent().base,
            (false, true) => tokens::INFO,
            (false, false) => tokens::LINE_STRONG,
        };
        let rail =
            Rail { diamond: apart && !newest, ink, at: tokens::SPACE_SM + HEAD / 2.0, above: !first, below: !last };
        let told: Vec<String> = [who, when, size].into_iter().flatten().collect();
        let row = Element::row()
            .key(version.at.to_string())
            .gap(tokens::SPACE_SM)
            .padding(Edges::xy(tokens::SPACE_XS, 0.0))
            .radius(tokens::RADIUS_CONTROL)
            .hover(Look::bg(tokens::NEUTRAL_SOFT))
            .role(Role::ListBoxOption)
            .label(title)
            .description(told.join(", "))
            .selected(picked)
            .in_view(picked)
            // A press picks the version. The keyboard stays with the page, where the arrows step on.
            .on_click(Message::History(Historying::Pick(at)))
            .child(Element::canvas(Rc::new(rail)).width(TRACK).passive().decorative())
            .child(body);
        if picked { row.bg(accent().soft) } else { row }
    }

    /// Tells assistive technology of the version that shows, under the canvas that shows it: its blocks,
    /// how each differs from the version it is compared with, and what the rewritten ones in view read
    /// before.
    pub fn describe(&self, told: &mut Told, whole: bool, canvas: &mut Node, nodes: &mut Vec<(NodeId, Node)>) {
        canvas.set_description(self.said());
        let Some(leaf) = &self.leaf else { return };
        describe::version(&leaf.doc, &leaf.said, told, whole, canvas, nodes);
        for (block, _, frame) in self.frames.iter().filter(|(_, side, _)| *side == Side::First) {
            // What is told is what is drawn there: the block of the document that the earlier side shows.
            let Some(before) = leaf.earlier.as_ref().and_then(|(earlier, _)| earlier.block(block)) else { continue };
            let label = leaf.heads[0].told("Before");
            describe::held_back(block, &label, &before.plain_text(), *frame, canvas, nodes);
        }
    }
}

impl Leaf {
    /// A version to show: `later`, as it read or as it differs from `earlier`. `heads` say who wrote the
    /// two and when, and `captions` what stands over a stretch that was taken out and over one that was
    /// added.
    fn new(
        between: (Option<Revision>, Revision),
        earlier: Option<&(Document, Style)>,
        later: &(Document, Style),
        heads: [Head; 2],
        captions: [String; 2],
    ) -> Self {
        let (doc, found) = match earlier {
            Some((earlier, _)) => compare(earlier, &later.0),
            None => (later.0.clone(), Differences::default()),
        };
        let fresh = || {
            let mut layout = DocLayout::new(Fonts { faces: letters_fonts::bundled() });
            // Nothing has a caret here, so a formula is typeset and never shows its source.
            layout.set_formula_source(false);
            layout
        };
        let earlier = (!found.pairs.is_empty()).then(|| {
            let mut first = doc.clone();
            for before in &found.pairs {
                review::put(&mut first.blocks, before.clone());
            }
            (first, fresh())
        });
        let said = found.sorts.iter().map(|(block, sort)| {
            let said = match sort {
                Sort::Added => captions[1].clone(),
                Sort::Removed => captions[0].clone(),
                Sort::Gained(words) => format!("Added: {words}"),
                Sort::Lost(words) => format!("Removed: {words}"),
                Sort::Rewritten => heads[1].told("After"),
            };
            (block.clone(), said)
        });
        Self {
            between,
            said: said.collect(),
            pairs: found.pairs.iter().map(|before| before.id.clone()).collect(),
            doc,
            style: later.1.clone(),
            layout: fresh(),
            earlier,
            found,
            heads,
            captions,
            laid: None,
            estimated: true,
        }
    }

    /// Sets the version for a column `width` wide, when it is not.
    fn lay(&mut self, width: f32, images: &Images, palette: Palette) {
        let mut layouts =
            [Some((&self.doc, &mut self.layout)), self.earlier.as_mut().map(|(doc, layout)| (&*doc, layout))];
        for (doc, layout) in layouts.iter_mut().flatten() {
            if self.laid != Some(width) {
                layout.update(doc, &self.style, width, None);
            }
            if self.laid.is_none() {
                dress(doc, layout, images);
            }
            layout.set_palette(palette);
        }
        if self.laid != Some(width) {
            (self.laid, self.estimated) = (Some(width), true);
        }
    }

    /// The page as what differs splits it, in a page area `room` wide: at each block that has two
    /// versions, with room over each stretch that only one version has. `pinned` is as for
    /// [`review::split`].
    fn split(&self, room: f32, pinned: (Side, f32)) -> Split {
        let first = self.earlier.as_ref().map_or(&self.layout, |(_, layout)| layout);
        let mut split = review::split(first, &self.layout, &self.pairs, room, pinned);
        for run in &self.found.runs {
            if let Some((first, last)) = self.layout.band(&run.first).zip(self.layout.band(&run.last)) {
                part(&mut split, &run.first, (first.0, last.1));
            }
        }
        split
    }

    /// Lays out what `view` shows of the two versions, a part of what `split` counts from, and about
    /// `budget` bytes of text round that. Returns whether a height changed, so that the page is to be
    /// split again.
    fn settle(&mut self, split: &Split, view: Rect, budget: usize) -> bool {
        let tall = |leaf: &Self| (leaf.layout.height(), leaf.earlier.as_ref().map(|(_, layout)| layout.height()));
        let before = tall(self);
        let (top, shown) = review::shown(split, view);
        let page = Rect::new(0.0, top, view.width, shown);
        self.estimated = self.layout.settle(page, budget);
        if let Some((_, earlier)) = &mut self.earlier {
            // Where none of the earlier version is in view, it is laid out round what the page shows.
            self.estimated |= earlier.settle(split.shown(Side::First, view).unwrap_or(page), budget);
        }
        tall(self) != before
    }
}

/// Gives a layout what the page's has beside the document: the colours of its code, and the pictures it
/// links to that are there to draw.
fn dress(doc: &Document, layout: &mut DocLayout, images: &Images) {
    for block in doc.walk() {
        if let BlockKind::Code { language } = &block.kind {
            let tokens = highlight::tokens(&block.plain_text(), language.as_deref().unwrap_or_default());
            layout.set_tokens(&block.id, tokens);
        }
        if let Some((src, image)) = pictures::link(block).and_then(|src| Some((src, images.get(src)?))) {
            layout.set_loaded(src, Some((image.width(), image.height())));
        }
    }
}

/// What was drawn of the version in view, in the coordinates of the area where the page was.
struct Page {
    list: DisplayList,
    page: Color,
    /// The height of everything that scrolls, and how far it is scrolled.
    content: f32,
    scrolled: f32,
}

impl Paint for Page {
    fn paint(&self, rect: Rect, _visible: Rect, scheme: Scheme, out: &mut DisplayList) {
        out.push(Item::Fill { rect, color: self.page, radius: 0.0 });
        let first = out.items.len();
        out.items.extend(self.list.items.iter().cloned());
        out.translate(first, Point { x: rect.x, y: rect.y });
        out.items.extend(scrollbar(rect, self.content, self.scrolled, scheme, 1.0));
    }
}

/// The strip of versions as it is drawn: a cell for each, the oldest first.
struct Strip {
    /// Whether each version stands out: one that has a name, or that came from elsewhere.
    out: Vec<bool>,
    /// The version that is picked.
    at: usize,
}

impl Paint for Strip {
    fn paint(&self, rect: Rect, _visible: Rect, scheme: Scheme, out: &mut DisplayList) {
        let ink = |ink: Ink| resolve(ink, scheme, 1.0);
        let cell = rect.width / self.out.len().max(1) as f32;
        let cells = Rect::new(rect.x, rect.y + ((rect.height - CELL) / 2.0).round(), rect.width, CELL);
        // With room for them the cells stand apart. Without, they run into one bar that the versions
        // which stand out are marked on.
        let apart = cell >= CELL_APART;
        out.push(Item::PushClip { rect: cells, radius: tokens::RADIUS_SM });
        if !apart {
            out.push(Item::Fill { rect: cells, color: ink(tokens::NEUTRAL_SOFT), radius: 0.0 });
        }
        for (at, stands_out) in self.out.iter().enumerate().filter(|(_, stands_out)| apart || **stands_out) {
            let left = (cells.x + at as f32 * cell).round();
            let right = (cells.x + (at + 1) as f32 * cell).round() - if apart { 1.0 } else { 0.0 };
            let wide = (right - left).max(if apart { 1.0 } else { CELL_SEEN });
            let color = ink(if *stands_out { tokens::INFO } else { tokens::NEUTRAL_SOFT });
            out.push(Item::Fill { rect: Rect::new(left, cells.y, wide, CELL), color, radius: 0.0 });
        }
        out.push(Item::PopClip);
        let middle = cells.x + (self.at as f32 + 0.5) * cell;
        let head = Rect::new((middle - SCRUB.0 / 2.0).round(), cells.y - SCRUB.1, SCRUB.0, CELL + 2.0 * SCRUB.1);
        // A rim of the bar's own colour keeps the head apart from a cell of its colour.
        let rim = Rect::new(head.x - 1.0, head.y - 1.0, head.width + 2.0, head.height + 2.0);
        out.push(Item::Fill { rect: rim, color: ink(tokens::SURFACE), radius: rim.width / 2.0 });
        out.push(Item::Fill { rect: head, color: ink(accent().text), radius: SCRUB.0 / 2.0 });
    }
}

/// A version's piece of its day's rail, with its marker.
struct Rail {
    /// A version that came from elsewhere is a diamond, and any other a ring.
    diamond: bool,
    ink: Ink,
    /// How far down the piece the middle of the marker is.
    at: f32,
    /// Whether the rail comes in from the version above, and goes on to the one below.
    above: bool,
    below: bool,
}

impl Paint for Rail {
    fn paint(&self, rect: Rect, _visible: Rect, scheme: Scheme, out: &mut DisplayList) {
        let rail = resolve(tokens::LINE_STRONG, scheme, 1.0);
        let x = rect.x + ((rect.width - LINE) / 2.0).round();
        let (top, bottom) = (rect.y + self.at - DOT / 2.0 - CLEAR, rect.y + self.at + DOT / 2.0 + CLEAR);
        if self.above {
            out.push(Item::Fill { rect: Rect::new(x, rect.y, LINE, top - rect.y), color: rail, radius: 0.0 });
        }
        if self.below {
            let rest = Rect::new(x, bottom, LINE, rect.y + rect.height - bottom);
            out.push(Item::Fill { rect: rest, color: rail, radius: 0.0 });
        }
        let color = resolve(self.ink, scheme, 1.0);
        let middle = Point { x: x + LINE / 2.0, y: rect.y + self.at };
        if self.diamond {
            let reach = DOT / 2.0 - LINE / 2.0;
            let corners = [(0.0, -reach), (reach, 0.0), (0.0, reach), (-reach, 0.0)].map(|(x, y)| Point { x, y });
            let mut path = vec![PathEl::MoveTo(corners[0])];
            path.extend(corners[1..].iter().map(|corner| PathEl::LineTo(*corner)));
            path.push(PathEl::Close);
            out.push(Item::Path { origin: middle, scale: 1.0, path: path.into(), color, stroke: Some(LINE) });
        } else {
            let ring = Rect::new(middle.x - DOT / 2.0, middle.y - DOT / 2.0, DOT, DOT);
            out.push(Item::Stroke { rect: ring, color, width: LINE, radius: DOT / 2.0 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Noon on Tuesday 10 November 2026, in UTC.
    const NOON: u64 = 1_794_312_000;
    const HOUR: u64 = 3_600;
    const DAY: u64 = 86_400;

    fn revision(counter: usize) -> Revision {
        format!("{counter}@1").parse().expect("a revision")
    }

    /// When each step of a history was written and by whom, and the steps its versions are expected to
    /// end with.
    type Sittings<'a> = (&'a [(u64, &'a str)], &'a [usize]);

    /// A step of the timeline: its place in it, when it was written and by whom.
    fn step(place: usize, time: u64, author: &str) -> Step {
        Step {
            before: revision(place),
            after: revision(place + 1),
            authors: (!author.is_empty()).then(|| (author.to_owned(), 1)).into_iter().collect(),
            time,
            edits: 1,
            size: 10,
            added: 10,
            removed: 0,
            tags: Vec::new(),
        }
    }

    fn tag(at: usize, kind: TagKind, name: Option<&str>) -> Tag {
        Tag { at: revision(at), kind, name: name.map(str::to_owned), author: None, time: 0, restored: None }
    }

    #[test]
    fn steps_written_at_one_sitting_are_one_version() {
        let cases: [Sittings; 5] = [
            // A minute apart, by one writer and by two.
            (&[(NOON, "Sam"), (NOON + 60, "Sam"), (NOON + 120, "Sam")], &[3]),
            (&[(NOON, "Sam"), (NOON + 60, "Alice"), (NOON + 120, "Sam")], &[3]),
            // A rest longer than ten minutes.
            (&[(NOON, "Sam"), (NOON + 60, "Sam"), (NOON + HOUR, "Sam")], &[2, 3]),
            // A night between two steps a minute apart.
            (&[(NOON + 12 * HOUR - 30, "Sam"), (NOON + 12 * HOUR + 30, "Sam")], &[1, 2]),
            // Steps nobody dated are told apart from those that were.
            (&[(0, ""), (NOON, "Sam"), (NOON + 60, "Sam")], &[1, 3]),
        ];
        for (written, expected) in cases {
            let steps: Vec<Step> =
                written.iter().enumerate().map(|(place, (time, author))| step(place, *time, author)).collect();
            let ends: Vec<Revision> = versions(&steps, 0).into_iter().map(|version| version.at).collect();
            let expected: Vec<Revision> = expected.iter().map(|end| revision(*end)).collect();
            assert_eq!(ends, expected, "{written:?}");
        }
    }

    #[test]
    fn a_version_says_who_wrote_most_of_it_first_and_how_much_it_changed() {
        let steps = [step(0, NOON, "Sam"), step(1, NOON + 60, "Alice"), step(2, NOON + 120, "Alice")];
        let listed = versions(&steps, 0);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].who().as_deref(), Some("Alice and Sam"));
        assert_eq!((listed[0].added, listed[0].removed), (30, 0));
    }

    #[test]
    fn a_marked_step_ends_its_version_at_the_revision_that_was_marked() {
        let mut steps = vec![step(0, NOON, "Sam"), step(1, NOON + 60, "Sam"), step(2, NOON + 120, "Sam")];
        steps[1].tags = vec![tag(2, TagKind::Named, Some("Sent to builder")), tag(1, TagKind::Merge, None)];
        let listed = versions(&steps, 0);
        let read: Vec<(Revision, String)> = listed
            .iter()
            .enumerate()
            .map(|(at, version)| (version.at.clone(), version.title((at == 0, at + 1 == listed.len()), 0)))
            .collect();
        let expected = [(2, "Sent to builder"), (1, "Merged edits"), (3, "Current version")];
        assert_eq!(read, expected.map(|(at, title)| (revision(at), title.to_owned())));
        // What the run wrote is counted once.
        assert_eq!(listed.iter().map(|version| version.added).collect::<Vec<_>>(), [20, 0, 10]);
    }

    #[test]
    fn a_version_is_listed_under_its_day_on_the_persons_clock() {
        let today = Some(day(NOON, 0));
        // When it was written, how far the clock is ahead of UTC, the day and time it is listed at, and
        // the date the strip begins at when it is the oldest.
        let cases = [
            (NOON, 0, "Today", Some("12:00"), "10 Nov"),
            (NOON - DAY, 0, "Yesterday", Some("12:00"), "9 Nov"),
            (NOON - 2 * DAY, 0, "Sunday", Some("12:00"), "8 Nov"),
            (NOON - 6 * DAY, 0, "Wednesday", Some("12:00"), "4 Nov"),
            (NOON - 7 * DAY, 0, "3 Nov", Some("12:00"), "3 Nov"),
            (NOON - 400 * DAY, 0, "6 Oct 2025", Some("12:00"), "6 Oct 2025"),
            // Half past eleven at night in UTC is the next morning two hours east of it.
            (NOON - 12 * HOUR - 1_800, 120, "Today", Some("01:30"), "10 Nov"),
            (NOON - 12 * HOUR - 1_800, 0, "Yesterday", Some("23:30"), "9 Nov"),
            (NOON - 12 * HOUR + 1_800, -300, "Yesterday", Some("19:30"), "9 Nov"),
        ];
        for (time, zone, name, clock, date) in cases {
            let version = Version { time, ..versions(&[step(0, time, "Sam")], zone).remove(0) };
            let today = today.map(|_| day(NOON, zone));
            assert_eq!(version.day(today, zone), name, "{time} at {zone}");
            assert_eq!(version.clock(zone).as_deref(), clock, "{time} at {zone}");
            assert_eq!(version.date(today, zone), date, "{time} at {zone}");
        }
        let undated = versions(&[step(0, 0, "Sam")], 0).remove(0);
        assert_eq!((undated.day(today, 0).as_str(), undated.clock(0)), ("Undated", None));
    }

    #[test]
    fn a_version_that_nobody_dated_is_told_by_when_and_by_whom_it_was_marked() {
        let mut steps = vec![step(0, 0, ""), step(1, NOON + HOUR, "Alice")];
        let import = Tag { author: Some("Sam".to_owned()), time: NOON, ..tag(1, TagKind::Import, Some("Plan.docx")) };
        steps[0].tags = vec![import];
        let listed = versions(&steps, 0);
        assert_eq!(listed[0].title((true, false), 0), "Imported from Plan.docx");
        assert_eq!(listed[0].day(Some(day(NOON, 0)), 0), "Today");
        assert_eq!((listed[0].clock(0).as_deref(), listed[0].who().as_deref()), (Some("12:00"), Some("Sam")));
        // A step that says when it was written and by whom is told by that, marked or not.
        steps[1].tags =
            vec![Tag { author: Some("Sam".to_owned()), time: NOON, ..tag(2, TagKind::Named, Some("Draft")) }];
        let listed = versions(&steps, 0);
        assert_eq!((listed[1].clock(0).as_deref(), listed[1].who().as_deref()), (Some("13:00"), Some("Alice")));
    }

    #[test]
    fn a_version_that_is_listed_no_longer_leaves_the_one_that_holds_it_picked() {
        // A step the day before, three at one sitting with the second of them named, and one an hour later.
        let mut steps = vec![step(0, NOON - DAY, "Sam")];
        steps.extend((1..4).map(|place| step(place, NOON + 60 * place as u64, "Sam")));
        steps.push(step(4, NOON + HOUR, "Sam"));
        steps[2].tags = vec![tag(3, TagKind::Named, Some("Sent"))];
        let newest = steps[4].after.clone();
        let mut history = History::new(&steps, newest.clone(), None, 0);
        history.pick(1);
        assert_eq!(history.picked().name(), Some("Sent"));

        // With its name taken off, what it wrote is part of the sitting.
        steps[2].tags.clear();
        history.follow(&steps, newest);
        assert_eq!(history.picked().at, steps[3].after);
        assert_eq!(history.versions.len(), 3);
    }

    #[test]
    fn the_strip_gives_each_version_an_equal_part_of_its_width() {
        let steps: Vec<Step> = (0..4).map(|place| step(place, NOON + DAY * place as u64, "Sam")).collect();
        let history = History::new(&steps, steps[3].after.clone(), None, 0);
        // How far across the strip, and the version there. Past either end is the version at that end.
        let cases = [(0.0, 0), (0.24, 0), (0.26, 1), (0.6, 2), (0.99, 3), (1.0, 3), (-0.5, 0), (7.0, 3)];
        for (across, version) in cases {
            assert_eq!(history.across(across), version, "{across}");
        }
    }

    #[test]
    fn a_name_is_taken_only_when_it_is_not_what_the_version_is_called() {
        // A version as it is marked, what is typed for it, and the name it is then given: `None` for
        // nothing to do, `Some(None)` to take its name off.
        type Case<'a> = (Option<(TagKind, &'a str)>, &'a str, Option<Option<&'a str>>);
        let cases: [Case; 7] = [
            (None, "Sent", Some(Some("Sent"))),
            (None, "  Sent ", Some(Some("Sent"))),
            (None, "  ", None),
            (Some((TagKind::Named, "Sent")), "Sent", None),
            (Some((TagKind::Named, "Sent")), "Sent again", Some(Some("Sent again"))),
            (Some((TagKind::Named, "Sent")), "", Some(None)),
            (Some((TagKind::Import, "plan.docx")), "", None),
        ];
        for (marked, typed, expected) in cases {
            let mut steps = vec![step(0, NOON, "Sam"), step(1, NOON + DAY, "Sam")];
            steps[0].tags = marked.iter().map(|(kind, name)| tag(1, *kind, Some(name))).collect();
            let mut history = History::new(&steps, steps[1].after.clone(), None, 0);
            history.pick(0);
            history.name(Naming::Open);
            history.name(Naming::Typed(typed.to_owned()));
            let named = history.named();
            let expected = expected.map(|name| (steps[0].after.clone(), name.map(str::to_owned)));
            assert_eq!(named, expected, "{marked:?} with {typed:?} typed");
            assert_eq!(history.named(), None, "the field is put away");
        }
    }

    #[test]
    fn the_history_always_ends_with_the_document_as_it_reads_now() {
        // A new document has no steps yet, and a mark can stand at an older revision than the last step.
        let mut marked = vec![step(0, NOON, "Sam"), step(1, NOON + 60, "Sam")];
        marked[1].tags = vec![tag(1, TagKind::Named, Some("Draft"))];
        for (steps, expected) in [(Vec::new(), 1), (marked, 2)] {
            let history = History::new(&steps, revision(2), Some(NOON), 0);
            assert_eq!(history.versions.len(), expected);
            assert!(history.newest());
            assert_eq!(history.picked().at, revision(2));
        }
    }
}
