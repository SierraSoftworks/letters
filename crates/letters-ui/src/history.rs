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
//! it or with the document as it reads now. A block that only gained words, or only lost some, shows once
//! with those words marked. Blocks that only one version has lie in a band right across the page, as if
//! the paper were cut there and pulled apart over them, under a line that says whose they are. What a
//! version put in is green and lies over the page, and what it took out is red and lies under it: the
//! colours of a diff, and not those of a conflict, whose two sides neither came first. A block that was
//! rewritten is both: as it read before in a band of what was taken out, and right under that as it reads
//! after in a band of what was put in. Nothing is chosen between the two, so they do not stand side by side
//! as the versions of a conflict do.
//!
//! Under the page a strip has a cell for each version, to press on or drag along, and the bar names the
//! version that is picked: a name is a mark in the history, which is saved and is no edit. The bar's menu
//! asks for a copy of the picked version, through the card that saves a copy of the document.
//!
//! A passage is copied out of a version as it is out of the page: the pointer selects it, and a tool over
//! what is selected copies it, as the key for a copy does. There is no caret, since nothing is written.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use accesskit::{Node, NodeId, Role};
use letters_editor::{Command, Editor, Key, KeyEvent, Platform};
use letters_layout::{Arrangement, DocLayout, Fonts, Palette, Piece, Side, Split, Uncovered, WordList};
use letters_model::{
    Block, BlockId, BlockKind, Body, Change, Document, Point as Step, Pos, ReplicaId, Revision, Selection, Style, Tag,
    TagKind,
};
use letters_render::image::Images;
use letters_render::{Color, DisplayList, Item, PathEl, Point, Rect, Size};

use crate::color::resolve;
use crate::copying::{CopyFormat, Copying, Earlier};
use crate::describe::{self, Told};
use crate::element::{Cursor, Element, Paint, Pointer};
use crate::highlight;
use crate::icon;
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
use crate::widgets::{self, Button, ControlSize, Menu, MenuItem, Popover, Segmented, SidePanel, TextField, TopBar};
use crate::window::{GUTTER, Message, PT, TOP};
use crate::words::WordLists;

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
/// What a version took out is red and what it put in is green, as a diff has them. Amber and blue are kept
/// for a conflict, so that nobody takes one side of that for the earlier or the lesser.
const TONES: review::Tones = [(tokens::DANGER_SOFT, tokens::DANGER), (tokens::OK_SOFT, tokens::OK)];
/// The band that a stretch of blocks lies in that only one version has, or one version of a rewritten
/// block: the room it keeps clear of what stands over it, the room in it over the stretch for the line
/// that says whose the stretch is, and the room in it under the stretch. Where the stretch was taken
/// out, the cut in the page is a line this thick along each edge, broken into dashes this long. What
/// casts a shadow reaches this far past a band.
const ABOVE: f32 = tokens::SPACE_MD;
const CAPTION: f32 = 22.0;
const UNDER: f32 = tokens::SPACE_MD;
const EDGE: f32 = 1.0;
const DASH: f32 = 4.0;
const CAST: f32 = 48.0;
/// How many times a version is split again before it is drawn, while laying out what shows changes heights.
const SETTLES: usize = 3;
/// How many versions that were read are kept, so that a step to the next version reads one and not two.
const KEPT: usize = 3;
/// The most blocks a version differs in from the last one shown for its layout to go on from that one's.
const NEAR: usize = 8;
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
    /// A press began at a point of the version: the first, second or third in quick succession, and
    /// with Shift to select as far as there. Then the pointer moved with the button held, and the press
    /// ended.
    Down {
        at: Point,
        clicks: u8,
        extend: bool,
    },
    Drag(Point),
    Up,
    /// Selects all of the version that shows, copies what is selected of it, or selects nothing.
    All,
    Copy,
    Unselect,
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

    /// What it is listed as, and the time it was written at where that is not what it is listed as: the
    /// time is said once. `ends` is as for [`Version::title`].
    fn aside(&self, ends: (bool, bool), zone: i32) -> (String, Option<String>) {
        let title = self.title(ends, zone);
        let when = self.clock(zone).filter(|clock| *clock != title);
        (title, when)
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

/// Draws the band of a stretch that only one version has, in that version's colour. What was put in is a
/// sheet laid over the page, which casts a shadow on it. What was taken out lies under the page and shows
/// through the cut, with the shadow of the page on it and the cut a broken line along each edge: the two
/// are told apart without their colours.
fn band(band: Rect, added: bool, page: Color, scheme: Scheme, list: &mut DisplayList) {
    let ink = |ink: Ink| resolve(ink, scheme, 1.0);
    let (soft, tone) = TONES[usize::from(added)];
    let cast = |of: Rect, list: &mut DisplayList| {
        for shadow in tokens::SHADOW_OVERLAY {
            let offset = Point { x: shadow.x, y: shadow.y };
            // Wider than the band, so that the shadow is as dark at the ends of the band as in its middle.
            let rect = Rect::new(of.x - CAST, of.y, of.width + 2.0 * CAST, of.height);
            list.push(Item::Shadow { rect, color: ink(shadow.color), blur: shadow.blur, offset, radius: 0.0 });
        }
    };
    if added {
        cast(band, list);
        list.push(Item::Fill { rect: band, color: page, radius: 0.0 });
        list.push(Item::Fill { rect: band, color: ink(soft), radius: 0.0 });
        return;
    }
    list.push(Item::Fill { rect: band, color: ink(soft), radius: 0.0 });
    // The page over the cut and the page under it both stand over what shows through it.
    list.push(Item::PushClip { rect: band, radius: 0.0 });
    cast(Rect::new(band.x, band.y - CAST, band.width, CAST), list);
    cast(Rect::new(band.x, band.y + band.height, band.width, CAST), list);
    list.push(Item::PopClip);
    for y in [band.y, band.y + band.height - EDGE] {
        let mut x = band.x;
        while x < band.x + band.width {
            let rect = Rect::new(x, y, DASH.min(band.x + band.width - x), EDGE);
            list.push(Item::Fill { rect, color: ink(tone), radius: 0.0 });
            x += 2.0 * DASH;
        }
    }
}

/// The part of its version's column that a piece of a split page shows in `view`, a part of what the
/// split counts from.
fn cut(piece: &Piece, view: Rect) -> Option<Rect> {
    let top = (view.y - piece.origin.y).max(piece.band.0);
    let bottom = (view.y + view.height - piece.origin.y).min(piece.band.1);
    (top < bottom).then(|| Rect::new(view.x - piece.origin.x, top, view.width, bottom - top))
}

/// The version that is picked, read and laid out: as it read, or as it differs from another.
struct Leaf {
    /// The revision the page is of and, where that is compared with an earlier one, the earlier one.
    between: (Option<Revision>, Revision),
    /// The page: the later version, with what only the earlier one has put back in.
    doc: Document,
    style: Style,
    layout: DocLayout,
    /// The page with each rewritten block as the earlier version has it, laid out for the bands that
    /// show those as they read before. `None` when no block has two versions.
    earlier: Option<(Document, DocLayout)>,
    found: Differences,
    /// The rewritten blocks, each with the first block that holds text in it as the earlier version has
    /// it and as the later one does.
    pairs: Vec<(BlockId, [Option<BlockId>; 2])>,
    /// What the line in a band says: over a stretch that was taken out and over one that was added, then
    /// over a rewritten block as it read before and as it reads after.
    captions: [[String; 2]; 2],
    /// What assistive technology is told of the blocks that differ.
    said: HashMap<BlockId, String>,
    /// The width it is laid out at, once it is.
    laid: Option<f32>,
    /// What the page's layout and the earlier side's have still to take in: `None` for one that was never
    /// laid out, and otherwise what differs from the version it was last laid out for.
    behind: [Option<Change>; 2],
    /// The layout of an earlier side that the last version had and this one has no use for, kept for
    /// the next that has.
    spare: Option<Sheet>,
    /// Whether some of its heights are still estimates.
    estimated: bool,
}

/// A document, its layout, and what the layout has still to take in of it.
type Sheet = (Document, DocLayout, Option<Change>);

/// What the history keeps while it shows.
pub(crate) struct History {
    /// Oldest first, and never empty. Shared with the rows of the panel, which are built as they come
    /// near what shows.
    versions: Rc<Vec<Version>>,
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
    /// The passage selected in the version that shows, to be copied out of it: from where a press began
    /// to where it got to, in the earlier version of a rewritten block or in the page. It goes when
    /// another version shows.
    selected: Option<(Side, Selection)>,
    /// Whether the press that selects is still held, and whether what is selected was copied.
    selecting: bool,
    copied: bool,
    /// What was last drawn: where the rewritten blocks in view stand in the page area as they read
    /// before. Then how the page was split and where the page area showed the split's top left, and the
    /// first line and the last of what is selected that showed, with whether the line in a band stands
    /// over the first.
    before: Vec<(BlockId, Rect)>,
    split: Option<(Split, Point)>,
    lines: Option<(Rect, Rect, bool)>,
    /// The faces of the system that back the fonts up, for the layout of a version shown after they came.
    backup: Fonts,
    /// The word lists, for the same.
    lists: WordLists,
}

impl History {
    /// The history of a document that reads as `current`, with its newest version picked.
    pub fn new(steps: &[Step], current: Revision, now: Option<u64>, zone: i32) -> Self {
        let mut history = Self {
            versions: Rc::default(),
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
            selected: None,
            selecting: false,
            copied: false,
            before: Vec::new(),
            split: None,
            lines: None,
            backup: Fonts::default(),
            lists: WordLists::default(),
        };
        history.follow(steps, current);
        history
    }

    /// Lists the versions again after the document changed under the history, as when another writer's
    /// edits came in. The version that was picked stays picked while it is still listed.
    pub fn follow(&mut self, steps: &[Step], current: Revision) {
        let picked = self.versions.get(self.at).map(|version| version.at.clone());
        let mut listed = versions(steps, self.zone);
        // The document as it reads now is always the last version, marked or not.
        let newest = steps.last().map_or(&current, |step| &step.after);
        if listed.last().is_none_or(|last| last.at != *newest) {
            let time = steps.last().map_or(0, |step| step.time);
            let (authors, tag) = (Vec::new(), None);
            listed.push(Version { at: newest.clone(), time, authors, added: 0, removed: 0, tag });
        }
        self.versions = Rc::new(listed);
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
        // What changed from the version before is its writer's. What changed since is many people's.
        let by = match (self.compare, sides.and_then(|(_, later)| self.versions[later].who())) {
            (Compare::Current, _) => " later".to_owned(),
            (_, Some(by)) => format!(" by {by}"),
            (_, None) => String::new(),
        };
        let captions = [[format!("Removed{by}"), format!("Added{by}")], ["Before".to_owned(), format!("Changed{by}")]];
        let last = self.leaf.take();
        let late = (&self.backup, &self.lists);
        self.leaf = later.map(|later| Leaf::new(between, earlier.as_deref(), &later, captions, last, late));
        // The versions just shown are the ones a step is most likely to want again.
        let spare = self.read.len().saturating_sub(KEPT);
        self.read.drain(..spare);
        self.before = Vec::new();
        // A passage is of the version it was selected in.
        (self.split, self.lines) = (None, None);
        self.unselect();
    }

    pub fn pick(&mut self, at: usize) {
        self.at = at.min(self.versions.len() - 1);
    }

    /// Compares the picked version with another, or with none.
    pub fn compare(&mut self, with: Compare) {
        self.compare = with;
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

    /// Takes faces of the system that back the fonts up, for the versions shown from now on. Returns
    /// whether a character that was shown without a glyph has one now.
    pub fn add_fallback_fonts(&mut self, fonts: &Fonts) -> bool {
        self.backup.faces.extend(fonts.faces.iter().cloned());
        let Some(leaf) = &mut self.leaf else { return false };
        let earlier = leaf.earlier.as_mut().is_some_and(|(_, layout)| layout.add_fallback_fonts(fonts));
        let shown = leaf.layout.add_fallback_fonts(fonts) || earlier;
        leaf.estimated |= shown;
        if let Some((_, spare, _)) = &mut leaf.spare {
            spare.add_fallback_fonts(fonts);
        }
        shown
    }

    /// The characters the versions showed without a glyph since this was last asked.
    pub fn take_uncovered(&mut self) -> Vec<Uncovered> {
        let Some(leaf) = &mut self.leaf else { return Vec::new() };
        let mut found = leaf.layout.take_uncovered();
        found.extend(leaf.earlier.iter_mut().flat_map(|(_, layout)| layout.take_uncovered()));
        found
    }

    /// Takes the word list of a script, for the versions shown from now on. Returns whether lines of the
    /// one that shows may now end elsewhere.
    pub fn add_word_list(&mut self, list: WordList, data: &Rc<[u8]>) -> bool {
        if !self.lists.keep(list, data) {
            return false;
        }
        let Some(leaf) = &mut self.leaf else { return false };
        let earlier = leaf.earlier.as_mut().is_some_and(|(_, layout)| layout.add_word_list(list, data));
        let shown = leaf.layout.add_word_list(list, data) || earlier;
        leaf.estimated |= shown;
        if let Some((_, spare, _)) = &mut leaf.spare {
            spare.add_word_list(list, data);
        }
        shown
    }

    /// The scripts the versions were set in without their word list since this was last asked.
    pub fn take_unlisted(&mut self) -> Vec<WordList> {
        let Some(leaf) = &mut self.leaf else { return Vec::new() };
        let mut found = leaf.layout.take_unlisted();
        found.extend(leaf.earlier.iter_mut().flat_map(|(_, layout)| layout.take_unlisted()));
        found
    }

    /// Takes faces that arrived late.
    pub fn add_fonts(&mut self, fonts: &Fonts) {
        if let Some(leaf) = &mut self.leaf {
            let earlier = leaf.earlier.as_mut().is_some_and(|(_, layout)| layout.add_fonts(fonts));
            if leaf.layout.add_fonts(fonts) || earlier {
                leaf.estimated = true;
            }
            if let Some((_, spare, _)) = &mut leaf.spare {
                spare.add_fonts(fonts);
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
        let (view, room) = (self.viewport(), self.room.width);
        let Some(leaf) = self.leaf.as_mut().filter(|leaf| leaf.estimated && leaf.laid.is_some()) else {
            return false;
        };
        let split = leaf.split(room);
        let pin = leaf.layout.hit(Point { x: 0.0, y: review::shown(&split, view).0.max(0.0) });
        let top = |leaf: &Leaf, split: &Split| {
            let line = pin.as_ref().and_then(|pos| leaf.layout.caret(pos))?.y;
            Some(line + review::origin(split, line).y)
        };
        let before = top(leaf, &split);
        leaf.settle(&split, view, SETTLE_BYTES);
        if let (Some(before), Some(after)) = (before, top(leaf, &leaf.split(room))) {
            self.scroll += after - before;
        }
        true
    }

    /// The position in the version nearest to a point of the page area, and the side it is on. With a
    /// passage `held`, the nearest that the passage can reach to: on its side, and for the earlier version
    /// of a rewritten block in that block, which is all that shows of that version.
    fn spot(&self, at: Point, held: Option<&(Side, Selection)>) -> Option<(Side, Pos)> {
        let (leaf, (split, shift)) = (self.leaf.as_ref()?, self.split.as_ref()?);
        let mut at = Point { x: at.x - shift.x, y: at.y - shift.y };
        let earlier = leaf.earlier.as_ref().map_or(&leaf.layout, |(_, layout)| layout);
        if let Some((side, selection)) = held {
            let layout = if *side == Side::First { earlier } else { &leaf.layout };
            let block = layout.band(&selection.anchor.block).filter(|_| *side == Side::First);
            let reached = |piece: &&Piece| {
                piece.side == *side && block.is_none_or(|block| piece.band.0 < block.1 && block.0 < piece.band.1)
            };
            let off = |piece: &&Piece| (piece.rect.y - at.y).max(at.y - piece.rect.y - piece.rect.height).max(0.0);
            let piece = split.pieces.iter().filter(reached).min_by(|a, b| off(a).total_cmp(&off(b)))?.rect;
            // Kept inside that part of the split, the point is in its version whatever stands beside it.
            let (right, bottom) =
                ((piece.x + piece.width - 0.5).max(piece.x), (piece.y + piece.height - 0.5).max(piece.y));
            at = Point { x: at.x.clamp(piece.x, right), y: at.y.clamp(piece.y, bottom) };
        }
        split.hit(earlier, &leaf.layout, at).filter(|(side, _)| held.is_none_or(|(held, _)| held == side))
    }

    /// Takes a press at a point of the page area, which starts a passage where it is, or with `extend`
    /// takes the one there is as far as there.
    pub fn press(&mut self, at: Point, extend: bool) {
        let held = self.selected.clone().filter(|_| extend);
        let Some((side, head)) = self.spot(at, held.as_ref()) else { return };
        let anchor = held.map_or_else(|| head.clone(), |(_, selection)| selection.anchor);
        self.selected = Some((side, Selection { anchor, head }));
        (self.selecting, self.copied) = (true, false);
    }

    /// Takes the passage as far as the pointer went with the button held. Past the top or the bottom of
    /// the page area the version scrolls after it.
    pub fn drag(&mut self, at: Point) {
        let Some(held) = self.selected.clone().filter(|_| self.selecting) else { return };
        if let Some((_, head)) = self.spot(at, Some(&held)) {
            self.selected = Some((held.0, Selection { anchor: held.1.anchor, head }));
        }
        let past = at.y.min(0.0) + (at.y - self.room.height).max(0.0);
        self.scroll += past.clamp(-STEP, STEP);
        self.keep();
    }

    /// Lets go of the press that selects.
    pub fn release(&mut self) {
        self.selecting = false;
    }

    /// An editor that holds one side of the version as its document. It selects and copies as the page's
    /// own does, and nothing is written with it.
    fn reader(&self, side: Side) -> Option<Editor> {
        let leaf = self.leaf.as_ref()?;
        let doc = match (side, &leaf.earlier) {
            (Side::First, Some((earlier, _))) => earlier,
            _ => &leaf.doc,
        };
        Some(Editor::new(doc.clone(), ReplicaId::from(1)))
    }

    /// Has an editor of the side the passage is on select from where the passage ends, as `command` does,
    /// and takes what it selected.
    fn reselect(&mut self, side: Side, command: Command, platform: &mut dyn Platform) {
        let Some(mut reader) = self.reader(side) else { return };
        if let Some((_, selection)) = &self.selected {
            reader.select(Selection::caret(selection.head.clone()));
        }
        reader.run(command, platform);
        self.selected = reader.selection().cloned().map(|selection| (side, selection));
        (self.selecting, self.copied) = (false, false);
    }

    /// Takes the second press of a double click, which selects the word the first was on, or a later one,
    /// which selects its whole block.
    pub fn widen(&mut self, clicks: u8, platform: &mut dyn Platform) {
        let Some(side) = self.selected.as_ref().map(|(side, _)| *side) else { return };
        let command = if clicks == 2 { Command::SelectWord } else { Command::SelectBlock };
        self.reselect(side, command, platform);
    }

    /// Selects all of the version that shows: the page, with what stands in it of the later version.
    pub fn select_all(&mut self, platform: &mut dyn Platform) {
        self.selected = None;
        self.reselect(Side::Second, Command::SelectAll, platform);
    }

    pub fn unselect(&mut self) {
        (self.selected, self.selecting, self.copied) = (None, false, false);
    }

    /// The passage that is selected, when it holds anything.
    fn passage(&self) -> Option<&(Side, Selection)> {
        self.selected.as_ref().filter(|(_, selection)| !selection.is_caret())
    }

    /// Copies the passage to the clipboard as the page's editor copies: with its marks and its blocks, for
    /// Letters and for other applications.
    pub fn copy(&mut self, platform: &mut dyn Platform) {
        let Some((side, selection)) = self.passage().cloned() else { return };
        let Some(mut reader) = self.reader(side) else { return };
        reader.select(selection);
        reader.run(Command::Copy, platform);
        self.copied = true;
    }

    /// The tool over a passage once it is selected, which copies it and then says so: over the first line
    /// of the passage, or under its last where there is no room above or the room is the line that says
    /// whose version it is.
    pub fn tools(&self) -> Option<Popover<Message>> {
        let (first, last, headed) = self.lines.filter(|_| !self.selecting)?;
        let above = !headed && first.y >= notes::TOOLS_ROOM;
        let (sign, label) = if self.copied { (icon::CHECK, "Copied") } else { (icon::COPY, "Copy") };
        let copy = Button::new(label).icon(sign).subtle().small().on_press(Message::History(Historying::Copy));
        let tools =
            Element::row().padding(Edges::all(tokens::SPACE_XS)).role(Role::Toolbar).label("Selection").child(copy);
        let side = if above { widgets::Side::Top } else { widgets::Side::Bottom };
        let tools = Popover::new(tools).flush().side(side).gap(notes::TOOLS_GAP).beside().stays().open(true);
        Some(tools.on_close(Message::History(Historying::Unselect)).at(if above { first } else { last }))
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
        (self.before, self.lines) = (Vec::new(), None);
        let mut list = DisplayList::default();
        if let Some(mut leaf) = self.leaf.take() {
            self.draw(&mut leaf, images, shaper, palette, scheme, &mut list);
            self.leaf = Some(leaf);
        }
        let awaited = pictures::awaited(images, scheme, &mut list);
        let paint = Page { list, page: palette.page, content: self.tall, scrolled: self.scrolled() };
        let canvas = Element::canvas(Rc::new(paint))
            .grow(1.0)
            .fg(tokens::FG)
            .role(Role::Document)
            .label("History")
            .default_focus()
            .on_pointer(|event| match event {
                Pointer::Scroll { delta } => Some(Message::History(Historying::Scroll(delta))),
                Pointer::Down { at, mods, clicks } => {
                    Some(Message::History(Historying::Down { at, clicks, extend: mods.shift }))
                }
                Pointer::Drag { at } => Some(Message::History(Historying::Drag(at))),
                Pointer::Up { .. } => Some(Message::History(Historying::Up)),
                _ => None,
            })
            .pointed();
        // The version is text to select, as the page is.
        (canvas.cursor(Cursor::Text), awaited)
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
        let mut split = leaf.split(room.width);
        for _ in 0..SETTLES {
            self.tall = split.height + 2.0 * TOP;
            self.keep();
            if !leaf.settle(&split, self.viewport(), 0) {
                break;
            }
            split = leaf.split(room.width);
        }
        self.tall = split.height + 2.0 * TOP;
        self.keep();

        let view = self.viewport();
        // A split stands the page's column in the very middle, and the page area on a whole pixel beside it.
        let middle = ((room.width - width) / 2.0).max(0.0);
        let shift = Point { x: middle.round() - middle, y: -view.y };
        let passage = self.passage().cloned();
        let ink = |ink: Ink| resolve(ink, scheme, 1.0);
        // What lies in a band from one edge of the page area to the other: a stretch that only one version
        // has, and each version of a rewritten block. With whether it is of what was put in, what the line
        // in the band says, and the block whose first line that stands over.
        let banded = |piece: &Piece| -> Option<(bool, &str, Option<&BlockId>)> {
            let block = piece.block.as_ref()?;
            if let Some((_, texts)) = leaf.pairs.iter().find(|(pair, _)| pair == block) {
                let after = usize::from(piece.side == Side::Second);
                return Some((after == 1, &leaf.captions[1][after], texts[after].as_ref()));
            }
            let run = leaf.found.runs.iter().find(|run| run.first == *block)?;
            Some((run.added, &leaf.captions[0][usize::from(run.added)], run.text.as_ref()))
        };
        // Where the band of a piece is in the page area: under the room the piece keeps clear over it.
        let place = |piece: &Piece| {
            let over = (piece.rect.y + shift.y + piece.head - CAPTION).round();
            Rect::new(0.0, over, room.width, (piece.rect.y + piece.rect.height + shift.y).round() - over)
        };
        // The bands lie under all the text: those of what was taken out first, which the page is over,
        // then those of what was put in, which are over the page and over what was taken out.
        for added in [false, true] {
            for piece in split.pieces.iter().filter(|piece| banded(piece).is_some_and(|(of, ..)| of == added)) {
                let rect = place(piece);
                if rect.y + rect.height > -CAST && rect.y < room.height + CAST {
                    band(rect, added, palette.page, scheme, list);
                }
            }
        }
        for piece in &split.pieces {
            let (top, bottom) = (piece.rect.y + shift.y, piece.rect.y + piece.rect.height + shift.y);
            if bottom <= 0.0 || top >= room.height {
                continue;
            }
            let origin = Point { x: piece.origin.x + shift.x, y: piece.origin.y + shift.y };
            let layout = match (piece.side, &leaf.earlier) {
                (Side::First, Some((_, earlier))) => earlier,
                _ => &leaf.layout,
            };
            if let Some((added, caption, text)) = banded(piece) {
                // A line over the first line of what is in the band says whose that is.
                let first = text.and_then(|text| layout.caret(&Pos::new(text.clone(), 0)));
                let over = origin.y + first.map_or(piece.band.0, |line| line.y) - CAPTION;
                let said = shaper.shape(caption, &TextStyle::control().size(tokens::TEXT_SM).weight(600), None);
                said.paint(
                    Point { x: origin.x, y: over + ((CAPTION - said.size.height) / 2.0).round() },
                    ink(TONES[usize::from(added)].1),
                    list,
                );
                if let Some(block) = piece.block.as_ref().filter(|_| piece.side == Side::First) {
                    self.before.push((block.clone(), place(piece)));
                }
            }
            let Some(cut) = cut(piece, view) else { continue };
            list.push(Item::PushClip { rect: review::text(piece, shift), radius: 0.0 });
            let from = list.items.len();
            match piece.side {
                Side::First => {
                    for words in &leaf.found.gone {
                        review::mark(&layout.selection_in(words, cut), Side::First, &TONES, scheme, list);
                    }
                }
                Side::Second => {
                    for (words, side) in &leaf.found.words {
                        review::mark(&layout.selection_in(words, cut), *side, &TONES, scheme, list);
                    }
                }
            }
            // What is selected of this version, under its text as on the page.
            let selected = passage.as_ref().filter(|(side, _)| *side == piece.side);
            for line in selected.into_iter().flat_map(|(_, selection)| layout.selection_in(selection, cut)) {
                list.push(Item::Fill { rect: line, color: palette.selection, radius: 0.0 });
                let shown = Rect { x: line.x + origin.x, y: line.y + origin.y, ..line };
                let headed = piece.head > 0.0 && line.y - piece.band.0 < notes::TOOLS_ROOM;
                let (first, headed) = self.lines.map_or((shown, headed), |(first, _, headed)| (first, headed));
                self.lines = Some((first, shown, headed));
            }
            layout.paint(cut, list);
            list.translate(from, origin);
            list.push(Item::PopClip);
        }

        self.split = Some((split, shift));
    }

    /// The picked version as a copy of it is asked for: what the copy's name says it is of is the
    /// version's name, or the day it was written. `None` for the document as it reads now, which is
    /// copied as the document is.
    pub fn copied(&self) -> Option<Earlier> {
        let version = self.picked();
        let called = match version.name() {
            Some(name) => name.to_owned(),
            None if version.time == 0 => "earlier version".to_owned(),
            None => version.date(self.today(), self.zone),
        };
        (!self.newest()).then(|| Earlier { revision: version.at.clone(), called })
    }

    /// Fills the bar while the history shows: the way back, what the version is of and when it was
    /// written, what it is compared with, the button that restores it, and a menu that saves a copy of
    /// it. A narrow window has the bar folded. `told` is what there is to say of a copy that was just
    /// written, which the menu's button says, and `keys` the keys that ask for a copy.
    pub fn bar(&self, bar: TopBar<Message>, name: &str, told: Option<&str>, keys: &str) -> TopBar<Message> {
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
        // A copy of the version is asked for where the bar has its other commands while the page is edited.
        let copy = |format: CopyFormat| Message::Copy(Copying::Open(format));
        let menu = Menu::new()
            .label("More")
            .item(
                MenuItem::new("Save a copy of this version")
                    .icon(icon::COPY)
                    .aside(keys)
                    .on_select(copy(CopyFormat::Letters)),
            )
            .item(MenuItem::new("Export this version").icon(icon::DOWNLOAD).on_select(copy(CopyFormat::Pdf)));
        let more = Button::icon_only(icon::ELLIPSIS, "More").subtle().small();
        let more = Element::from(more.popover(menu.popover().align(Align::End)));
        let more = match told {
            Some(told) => more.tooltip(told, widgets::Side::Bottom).telling(true),
            None => more,
        };
        bar.form(FORMS.1)
            .start(Button::icon_only(icon::ARROW_LEFT, "Back to editing").small().on_press(back))
            .start(name())
            .middle(Element::from(Button::new(with).small().menu(compares)).description("Compare with"))
            .middle(TopBar::rule())
            .middle(naming(Button::icon_only(icon::TAG, "Name")))
            .middle(restore(Button::icon_only(icon::ROTATE_CCW, "Restore this version")))
            .end(more)
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
        let (count, zone) = (self.versions.len(), self.zone);
        for (name, listed) in self.days() {
            let rows = listed.iter().enumerate().map(|(place, &at)| {
                let (ends, picked) = ((place == 0, place + 1 == listed.len()), at == self.at);
                let (version, versions) = (&self.versions[at], self.versions.clone());
                // A long history has more rows than a frame has time for, so a row is built when it
                // comes near what shows. Until then its place is kept.
                let two = !version.authors.is_empty() || version.aside((at == 0, at + 1 == count), zone).1.is_some();
                let tall = 2.0 * tokens::SPACE_SM + HEAD + if two { tokens::SPACE_2XS + threads::FACE } else { 0.0 };
                Element::later(tall, move || Self::row(&versions, at, picked, ends, zone)).key(at).in_view(picked)
            });
            let list = Element::column().key(listed[0]).role(Role::ListBox).label(name.as_str()).children(rows);
            panel = panel.child(SidePanel::<Message>::section(&name)).child(list);
        }
        panel.into()
    }

    /// One version in the panel: its marker on the day's rail, what it is listed as, who wrote it and how
    /// much it changed. `ends` says whether it is the first and the last of its day.
    fn row(versions: &[Version], at: usize, picked: bool, (first, last): (bool, bool), zone: i32) -> Element<Message> {
        let version = &versions[at];
        let newest = at + 1 == versions.len();
        let small = TextStyle::control().size(tokens::TEXT_SM);
        let quiet = TextStyle::control().size(tokens::TEXT_XS);
        let (title, when) = version.aside((at == 0, newest), zone);
        let who = version.who();
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
            let put_in = Element::text(format!("+{}", count(version.added)), figures).fg(TONES[1].1);
            let taken_out = Element::text(format!("\u{2212}{}", count(version.removed)), figures).fg(TONES[0].1);
            head = head.child(Element::row().gap(tokens::SPACE_XS).child(put_in).child(taken_out).decorative());
        }
        let mut body =
            Element::column().grow(1.0).gap(tokens::SPACE_2XS).padding(Edges::xy(0.0, tokens::SPACE_SM)).child(head);
        let by: Vec<&str> = [who.as_deref(), when.as_deref()].into_iter().flatten().collect();
        if !by.is_empty() {
            let face = version.authors.first().map(|(name, _)| threads::face(name, false));
            let by = Element::text(by.join(" \u{b7} "), small).fg(tokens::FG_MUTED).shrink();
            let by = Element::row().align(Align::Center).gap(tokens::SPACE_SM).children(face).child(by);
            // As tall with a face as without, so that the row's height is known before it is built.
            body = body.child(by.height(threads::FACE));
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
            .gap(tokens::SPACE_SM)
            .padding(Edges::xy(tokens::SPACE_XS, 0.0))
            .radius(tokens::RADIUS_CONTROL)
            .hover(Look::bg(tokens::NEUTRAL_SOFT))
            .role(Role::ListBoxOption)
            .label(title)
            .description(told.join(", "))
            .selected(picked)
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
        for (block, band) in &self.before {
            // What is told is what is drawn there: the block of the document that the earlier side shows.
            let Some(before) = leaf.earlier.as_ref().and_then(|(earlier, _)| earlier.block(block)) else { continue };
            describe::held_back(block, &leaf.captions[1][0], &before.plain_text(), *band, canvas, nodes);
        }
    }
}

impl Leaf {
    /// A version to show: `later`, as it read or as it differs from `earlier`. `captions` are what the
    /// lines in its bands say. `last` is the version that showed until now, and `late` what a layout that
    /// is made new is handed: the faces of the system and the word lists.
    fn new(
        between: (Option<Revision>, Revision),
        earlier: Option<&(Document, Style)>,
        later: &(Document, Style),
        captions: [[String; 2]; 2],
        last: Option<Self>,
        late: (&Fonts, &WordLists),
    ) -> Self {
        let (doc, found) = match earlier {
            Some((earlier, _)) => compare(earlier, &later.0),
            None => (later.0.clone(), Differences::default()),
        };
        // A step shows nearly the same document, so the layouts go on from the last version's and lay
        // out only what differs. A layout of its own for every version made a step cost a whole page.
        let (laid, [page, other]) = last.map_or((None, [None, None]), |last| (last.laid, last.sheets()));
        let go_on = |sheet: Option<Sheet>, doc: &Document| match sheet {
            Some((was, layout, behind)) => {
                let behind = behind.and_then(|mut behind| {
                    behind.absorb(differing(&was, doc));
                    // A layout sets every block a change names at once, in view or not. A version far
                    // from the last differs in many, so it is laid out as new and only what shows is set.
                    (behind.blocks.len() <= NEAR).then_some(behind)
                });
                (layout, behind)
            }
            None => {
                let mut layout = DocLayout::new(Fonts { faces: letters_fonts::bundled() });
                layout.add_fallback_fonts(late.0);
                late.1.hand(&mut layout);
                // Nothing has a caret here, so a formula is typeset and never shows its source.
                layout.set_formula_source(false);
                (layout, None)
            }
        };
        let (layout, behind) = go_on(page, &doc);
        let (mut earlier, mut spare, mut second) = (None, other, None);
        if !found.pairs.is_empty() {
            let mut first = doc.clone();
            for before in &found.pairs {
                review::put(&mut first.blocks, before.clone());
            }
            let (layout, behind) = go_on(spare.take(), &first);
            (earlier, second) = (Some((first, layout)), behind);
        }
        let said = found.sorts.iter().map(|(block, sort)| {
            let said = match sort {
                Sort::Added => captions[0][1].clone(),
                Sort::Removed => captions[0][0].clone(),
                Sort::Gained(words) => format!("Added: {words}"),
                Sort::Lost(words) => format!("Removed: {words}"),
                Sort::Rewritten => captions[1][1].clone(),
            };
            (block.clone(), said)
        });
        let text = |block: Option<&Block>| block.and_then(written).map(|text| text.id.clone());
        let pairs = found.pairs.iter().map(|before| {
            let after = doc.blocks.iter().find(|block| block.id == before.id);
            (before.id.clone(), [text(Some(before)), text(after)])
        });
        Self {
            between,
            said: said.collect(),
            pairs: pairs.collect(),
            doc,
            style: later.1.clone(),
            layout,
            earlier,
            found,
            captions,
            laid,
            behind: [behind, second],
            spare,
            estimated: true,
        }
    }

    /// The page and its layout, and those of an earlier side if it has one to hand on, for the version
    /// that shows next.
    fn sheets(self) -> [Option<Sheet>; 2] {
        let [page, earlier] = self.behind;
        let earlier = self.earlier.map(|(doc, layout)| (doc, layout, earlier));
        [Some((self.doc, self.layout, page)), earlier.or(self.spare)]
    }

    /// Sets the version for a column `width` wide, when it is not.
    fn lay(&mut self, width: f32, images: &Images, palette: Palette) {
        let wide = self.laid == Some(width);
        let mut layouts =
            [Some((&self.doc, &mut self.layout)), self.earlier.as_mut().map(|(doc, layout)| (&*doc, layout))];
        for (layout, behind) in layouts.iter_mut().zip(&mut self.behind) {
            let Some((doc, layout)) = layout else { continue };
            let change = behind.replace(Change::default());
            if !wide || change.as_ref() != Some(&Change::default()) {
                // A layout that finds its width or its style changed lays everything out again.
                layout.update(doc, &self.style, width, change.as_ref());
                dress(doc, layout, images, change.as_ref());
                self.estimated = true;
            }
            layout.set_palette(palette);
        }
        self.laid = Some(width);
    }

    /// The page as what differs splits it, in a page area `room` wide: with a part of its own, and the
    /// room of a band, for each stretch that only one version has and for each version of a rewritten
    /// block. The block as it read before stands over the block as it reads after, whatever the room.
    fn split(&self, room: f32) -> Split {
        let first = self.earlier.as_ref().map_or(&self.layout, |(_, layout)| layout);
        let centre = ((room - self.laid.unwrap_or(room)) / 2.0).max(0.0);
        // What lies in bands down the page's column: the block a band is under and where its stretch
        // is, with where the earlier version's column has a rewritten block.
        let runs = self.found.runs.iter().filter_map(|run| {
            let (from, to) = self.layout.band(&run.first).zip(self.layout.band(&run.last))?;
            Some((&run.first, (from.0, to.1), None))
        });
        let pairs = self.pairs.iter().filter_map(|(block, _)| {
            let (before, after) = first.band(block).zip(self.layout.band(block))?;
            Some((block, after, Some(before)))
        });
        let mut bands: Vec<_> = runs.chain(pairs).collect();
        bands.sort_by(|a, b| a.1.0.total_cmp(&b.1.0));

        let mut split = Split { arrangement: Arrangement::Stacked, height: 0.0, pieces: Vec::new() };
        let mut put = |side: Side, block: Option<&BlockId>, band: (f32, f32), head: f32, foot: f32| {
            if band.1 > band.0 {
                let rect = Rect::new(0.0, split.height, room, head + band.1 - band.0 + foot);
                let origin = Point { x: centre, y: rect.y + head - band.0 };
                split.pieces.push(Piece { side, block: block.cloned(), rect, head, origin, band });
                split.height += rect.height;
            }
        };
        // How far down the page's column the parts have shown.
        let mut shown = 0.0;
        for (block, band, before) in bands.into_iter() {
            if band.0 < shown {
                continue;
            }
            put(Side::Second, None, (shown, band.0), 0.0, 0.0);
            match before {
                Some(before) => {
                    put(Side::First, Some(block), before, ABOVE + CAPTION, UNDER);
                    // The band of what it reads now stands right under the band of what it read.
                    put(Side::Second, Some(block), band, CAPTION, UNDER);
                }
                None => put(Side::Second, Some(block), band, ABOVE + CAPTION, UNDER),
            }
            shown = band.1;
        }
        put(Side::Second, None, (shown, self.layout.height()), 0.0, 0.0);
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

/// What a layout of `old` is to take in to be one of `new`.
fn differing(old: &Document, new: &Document) -> Change {
    let was: HashMap<&BlockId, &Block> = old.walk().map(|block| (&block.id, block)).collect();
    let mut change = Change::default();
    let mut kept = 0;
    for block in new.walk() {
        match was.get(&block.id) {
            Some(was) if *was == block => kept += 1,
            Some(_) => {
                kept += 1;
                change.blocks.insert(block.id.clone());
            }
            None => _ = change.blocks.insert(block.id.clone()),
        }
    }
    if kept < was.len() {
        let is: HashSet<&BlockId> = new.walk().map(|block| &block.id).collect();
        change.blocks.extend(was.keys().filter(|id| !is.contains(*id)).map(|id| (*id).clone()));
    }
    change.structure = !old.walk().map(|block| &block.id).eq(new.walk().map(|block| &block.id));
    change
}

/// Gives a layout what the page's has beside the document: the colours of its code, and the pictures it
/// links to that are there to draw. `only` names the blocks that changed since it last had them.
fn dress(doc: &Document, layout: &mut DocLayout, images: &Images, only: Option<&Change>) {
    for block in doc.walk() {
        if let BlockKind::Code { language } = &block.kind
            && only.is_none_or(|change| change.blocks.contains(&block.id))
        {
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
