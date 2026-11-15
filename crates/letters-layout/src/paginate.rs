//! Splits the column into the pages of Print view and export.
//!
//! Pagination is a pass over leaves that are already laid out. Each knows how many lines it has and where
//! they end, so the pass shapes no text: it only decides before which lines a page starts. A page then shows
//! a band of the column, cut between lines.

use std::ops::Range;

use letters_model::BlockId;
use letters_render::{Point, Rect};

use crate::flow::{Frame, Leaf};
use crate::theme;

/// A document split into pages. See [`crate::DocLayout::paginate`].
///
/// It describes the layout as it was when the pages were made, so paginate again after an update.
#[derive(Clone, Debug, PartialEq)]
pub struct Pages {
    /// The size of a sheet of paper, in logical pixels.
    pub width: f32,
    pub height: f32,
    /// The part of a sheet that text is set in, from the sheet's top left.
    pub area: Rect,
    /// The pages in order. There is always at least one.
    pub sheets: Vec<Sheet>,
}

impl Pages {
    /// Where sheet `index` shows the column's top left. Add it to a place in the column, such as a caret, to
    /// draw that on the sheet. Take it from a point on the sheet to hit test there.
    pub fn origin(&self, index: usize) -> Option<Point> {
        let sheet = self.sheets.get(index)?;
        Some(Point { x: self.area.x, y: self.area.y - sheet.band.0 })
    }
}

/// A link on a sheet, for an export that makes links work. See [`crate::DocLayout::page_links`].
#[derive(Clone, Debug, PartialEq)]
pub struct PageLink {
    /// One line of the link's text, from the sheet's top left. For a line of a table of contents, the line
    /// across the whole table, and for a link preview its card.
    pub rect: Rect,
    /// Where the link leads, as the document stores it. Empty for one that leads to a place in the document.
    pub target: String,
    /// Where a link into the document leads: the sheet, counted from 0, and a point on it. A reference to
    /// a figure leads to the top left of the figure's picture, and a line of a table of contents to the top
    /// left of its heading, as [`crate::DocLayout::outline`] has it. `None` for a link that leads out of
    /// the document.
    pub place: Option<(usize, Point)>,
    /// The id of the [`letters_render::Role::Link`] part that holds the link's text on the sheet. Every
    /// rectangle of one link has the same one. `None` in a code block, where no text is told as a link.
    pub part: Option<u64>,
}

/// A heading and where print shows it, for an export's outline. See [`crate::DocLayout::outline`].
#[derive(Clone, Debug, PartialEq)]
pub struct Heading {
    pub block: BlockId,
    pub title: String,
    /// From 1 to 4.
    pub level: u8,
    /// The sheet the heading starts on.
    pub page: usize,
    /// The top left of the heading's text on that sheet.
    pub at: Point,
}

/// A top-level block on a sheet, for what Print view shows between blocks. See
/// [`crate::DocLayout::page_blocks`].
#[derive(Clone, Debug, PartialEq)]
pub struct PageBlock {
    /// Which of the document's blocks it is, counted from 0.
    pub index: usize,
    /// Whether the sheet shows the block from its start, and not the rest of one the sheet before began.
    pub starts: bool,
    /// Where what stands above the block ends, from the sheet's top. For the first block of a sheet that
    /// is the top of the text area.
    pub above: f32,
    /// Where the block starts on the sheet, with its own box, and where it ends there.
    pub top: f32,
    pub bottom: f32,
}

/// One page of a paginated document.
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    /// The blocks on the page in order, each with the lines of it that the page holds. A block that runs
    /// over the end of a page is on the next one too, with the rest of its lines.
    pub blocks: Vec<(BlockId, Range<usize>)>,
    /// Where the page starts and ends in the column.
    pub(crate) band: (f32, f32),
    /// The leaf and the line that the page starts with.
    pub(crate) first: (usize, usize),
}

/// Room for the rounding of positions that are whole pixels in sum.
const SLACK: f32 = 0.01;

/// What each leaf shows above its text and takes with it to a new page: its own box, and the top of each
/// quote or callout that it opens.
fn heads(leaves: &[Leaf], frames: &[Frame]) -> Vec<f32> {
    let mut heads: Vec<f32> = leaves.iter().map(|leaf| if leaf.boxed() { theme::BOX_PAD_Y } else { 0.0 }).collect();
    for frame in frames {
        heads[frame.first] = heads[frame.first].max(leaves[frame.first].top - frame.lead);
    }
    heads
}

/// The top-level blocks one sheet shows, in order, each with where it stands on the sheet.
pub(crate) fn blocks(leaves: &[Leaf], frames: &[Frame], pages: &Pages, index: usize) -> Vec<PageBlock> {
    let (Some(sheet), Some(origin)) = (pages.sheets.get(index), pages.origin(index)) else { return Vec::new() };
    let heads = heads(leaves, frames);
    // The sheet shows its leaves up to the one the next sheet starts with, and of that one the lines before.
    let (first, _) = sheet.first;
    let (next, upto) = pages.sheets.get(index + 1).map_or((leaves.len(), 0), |next| next.first);
    let end = (next + usize::from(upto > 0)).min(leaves.len());
    let mut blocks: Vec<PageBlock> = Vec::new();
    for at in first..end {
        let leaf = &leaves[at];
        let bottom = origin.y + (leaf.y + leaf.height + leaf.bottom).min(sheet.band.1);
        match blocks.last_mut().filter(|block| block.index == leaf.root) {
            Some(block) => block.bottom = bottom,
            None => {
                let starts = (at > first || sheet.first.1 == 0) && (at == 0 || leaves[at - 1].root != leaf.root);
                // The sheet starts where its first leaf is cut, which is all that is above it there.
                let (above, top) = match at.checked_sub(1).filter(|_| at > first) {
                    Some(before) => (leaves[before].span().1, leaf.y - heads[at]),
                    None => (sheet.band.0, sheet.band.0),
                };
                blocks.push(PageBlock {
                    index: leaf.root,
                    starts,
                    above: origin.y + above,
                    top: origin.y + top,
                    bottom,
                });
            }
        }
    }
    blocks
}

/// Where a page starts: before a line of a leaf, at a height in the column.
#[derive(Clone, Copy)]
struct Cut {
    leaf: usize,
    line: usize,
    y: f32,
}

/// Splits settled leaves into pages that hold `room` of the column each.
///
/// A leaf marked to break starts a page. A heading stays with the block after it, and the header of a table
/// with its first row. A block splits only where two of its lines stay and two go, and never inside a line of
/// code. A gallery splits between any two rows of pictures. A figure and a row of a table stay whole. A leaf
/// that no rule lets onto the page goes to the next one, unless it is all the page would hold: then the page
/// takes what fits.
pub(crate) fn sheets(leaves: &[Leaf], frames: &[Frame], room: f32) -> Vec<Sheet> {
    if leaves.is_empty() {
        return vec![Sheet { blocks: Vec::new(), band: (0.0, 0.0), first: (0, 0) }];
    }
    let heads = heads(leaves, frames);
    let top = |at: usize| leaves[at].y - heads[at];

    let mut cuts = vec![Cut { leaf: 0, line: 0, y: top(0) }];
    // The leaf to place, and how many of its lines earlier pages took.
    let (mut at, mut from) = (0, 0);
    while at < leaves.len() {
        let (leaf, page) = (&leaves[at], cuts[cuts.len() - 1]);
        let opens = (page.leaf, page.line) == (at, from);
        if leaf.breaks && from == 0 && !opens {
            cuts.push(Cut { leaf: at, line: 0, y: top(at) });
            continue;
        }
        let limit = page.y + room + SLACK;
        if leaf.y + leaf.height + leaf.bottom <= limit {
            (at, from) = (at + 1, 0);
            continue;
        }

        let (count, together) = (leaf.lines.max(1) as usize, leaf.together());
        let fits = leaf.lines_in(limit - leaf.y);
        let free = |before: &usize| leaf.joined.binary_search(&(*before as u32)).is_err();
        let cut = |before: usize| Cut { leaf: at, line: before, y: leaf.y + leaf.line_top(before) };
        let stay = from + if from == 0 { together } else { 1 };
        // What must stay whole splits only when a page of its own is still too small for it.
        let split = (opens || !leaf.whole())
            .then(|| (stay..=fits.min(count.saturating_sub(together))).rev().find(free))
            .flatten();
        if let Some(before) = split {
            cuts.push(cut(before));
            from = before;
        } else if !opens {
            // It goes to the next page whole, and takes along the headings it follows.
            let mut first = at;
            while first > page.leaf + 1 && leaves[first - 1].leads() {
                first -= 1;
            }
            cuts.push(Cut { leaf: first, line: 0, y: top(first) });
            (at, from) = (first, 0);
        } else {
            // It opens the page and the rules leave nowhere to split it, so the page takes what it can hold.
            // A piece taller than a page runs over.
            let held = (from + 1..=fits.min(count - 1)).rev().find(free);
            match held.or_else(|| (fits.max(from) + 1..count).find(free)) {
                Some(before) => {
                    cuts.push(cut(before));
                    from = before;
                }
                None => (at, from) = (at + 1, 0),
            }
        }
    }

    let end = leaves.last().map_or(0.0, |last| last.y + last.height + last.bottom);
    let sheet = |(index, page): (usize, &Cut)| {
        let next = cuts.get(index + 1);
        // Up to the leaf and the line that the next page starts with.
        let (last, upto) = next.map_or((leaves.len(), 0), |next| (next.leaf, next.line));
        let mut blocks = Vec::new();
        for (at, leaf) in leaves.iter().enumerate().take(last + 1).skip(page.leaf) {
            let start = if at == page.leaf { page.line } else { 0 };
            let end = if at == last { upto } else { leaf.lines.max(1) as usize };
            if let (Some(id), true) = (&leaf.id, start < end) {
                blocks.push((id.clone(), start..end));
            }
        }
        let bottom = next.map_or(end, |next| next.y).min(page.y + room);
        Sheet { blocks, band: (page.y, bottom), first: (page.leaf, page.line) }
    };
    cuts.iter().enumerate().map(sheet).collect()
}
